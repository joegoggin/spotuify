use rspotify::http::Query;
use rspotify::prelude::BaseClient;
use rspotify::{AuthCodePkceSpotify, ClientError};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::SpotifyServiceError;
use super::retry::{
    TRANSIENT_RETRY_LIMIT, is_auth_expired_error, is_transient_client_error,
    run_with_refresh_and_transient_retry, run_with_refresh_retry,
};

/// Central wrapper for Spotify API access.
#[derive(Debug, Clone)]
pub(crate) struct SpotifyService {
    /// Authenticated Spotify client used for all request methods.
    client: AuthCodePkceSpotify,
}

impl SpotifyService {
    /// Creates a service from an authenticated Spotify client.
    pub(crate) fn new(client: AuthCodePkceSpotify) -> Self {
        Self { client }
    }

    /// Returns the underlying rspotify client.
    pub(crate) fn client(&self) -> &AuthCodePkceSpotify {
        &self.client
    }

    /// Sends an authenticated GET request to a Spotify API endpoint and decodes JSON.
    pub(crate) fn get_json<T>(
        &self,
        endpoint: &str,
        query: &Query<'_>,
    ) -> Result<T, SpotifyServiceError>
    where
        T: DeserializeOwned,
    {
        let response =
            self.request_with_transient_retry(|client| client.api_get(endpoint, query))?;
        decode_json_response(&response)
    }

    /// Sends an authenticated POST request to a Spotify API endpoint and decodes JSON.
    pub(crate) fn post_json<T>(
        &self,
        endpoint: &str,
        payload: &Value,
    ) -> Result<T, SpotifyServiceError>
    where
        T: DeserializeOwned,
    {
        let response =
            self.request_with_transient_retry(|client| client.api_post(endpoint, payload))?;
        decode_json_response(&response)
    }

    /// Sends an authenticated PUT request to a Spotify API endpoint and decodes JSON.
    pub(crate) fn put_json<T>(
        &self,
        endpoint: &str,
        payload: &Value,
    ) -> Result<T, SpotifyServiceError>
    where
        T: DeserializeOwned,
    {
        let response =
            self.request_with_transient_retry(|client| client.api_put(endpoint, payload))?;
        decode_json_response(&response)
    }

    /// Sends an authenticated PUT request to a Spotify API endpoint that returns no body.
    pub(crate) fn put_empty(
        &self,
        endpoint: &str,
        payload: &Value,
    ) -> Result<(), SpotifyServiceError> {
        let response =
            self.request_with_transient_retry(|client| client.api_put(endpoint, payload))?;
        decode_empty_response(&response)
    }

    /// Runs a Spotify request, refreshes expired auth once, and retries once.
    pub(crate) fn request_with_auth_retry<T>(
        &self,
        mut request: impl FnMut(&AuthCodePkceSpotify) -> Result<T, ClientError>,
    ) -> Result<T, SpotifyServiceError> {
        run_with_refresh_retry(
            || request(&self.client),
            || self.refresh_token(),
            is_auth_expired_error,
            SpotifyServiceError::from_client_error,
        )
    }

    /// Runs a Spotify request with auth-refresh and transient retry handling.
    fn request_with_transient_retry(
        &self,
        mut request: impl FnMut(&AuthCodePkceSpotify) -> Result<String, ClientError>,
    ) -> Result<String, SpotifyServiceError> {
        run_with_refresh_and_transient_retry(
            || request(&self.client),
            || self.refresh_token(),
            is_auth_expired_error,
            is_transient_client_error,
            SpotifyServiceError::from_client_error,
            TRANSIENT_RETRY_LIMIT,
        )
    }

    /// Refreshes the current Spotify token and verifies that a usable token remains loaded.
    pub(crate) fn refresh_token(&self) -> Result<(), SpotifyServiceError> {
        self.client
            .refresh_token()
            .map_err(SpotifyServiceError::refresh_failed)?;

        if self.has_usable_token()? {
            Ok(())
        } else {
            Err(SpotifyServiceError::ReauthorizationRequired(
                "Spotify token refresh did not return a usable access token; reauthorization required"
                    .to_owned(),
            ))
        }
    }

    /// Returns true when the client currently holds an unexpired access token.
    fn has_usable_token(&self) -> Result<bool, SpotifyServiceError> {
        let token_store = self.client.get_token();
        let token_store = token_store.lock().map_err(|_| {
            SpotifyServiceError::Spotify("Spotify token lock was poisoned".to_owned())
        })?;

        Ok(token_store
            .as_ref()
            .is_some_and(|token| !token.is_expired()))
    }
}

/// Decodes a JSON string response into the caller's requested type.
fn decode_json_response<T>(response: &str) -> Result<T, SpotifyServiceError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(response).map_err(SpotifyServiceError::decode_failed)
}

/// Validates that a Spotify endpoint response body is empty.
fn decode_empty_response(response: &str) -> Result<(), SpotifyServiceError> {
    if response.trim().is_empty() {
        Ok(())
    } else {
        Err(SpotifyServiceError::unexpected_body(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_json_response_maps_invalid_body_to_decode_error() {
        let err = decode_json_response::<serde_json::Value>("not json")
            .expect_err("invalid JSON should be mapped to decode error");

        assert!(matches!(err, SpotifyServiceError::Decode(_)));
    }

    #[test]
    fn decode_empty_response_accepts_empty_success_body() {
        decode_empty_response("").expect("empty body should be accepted");
        decode_empty_response(" \n\t ").expect("whitespace body should be accepted");
    }

    #[test]
    fn decode_empty_response_rejects_unexpected_body() {
        let err = decode_empty_response("{}").expect_err("non-empty body should be rejected");

        assert_eq!(
            err,
            SpotifyServiceError::Decode(
                "Spotify response was expected to be empty but returned 2 bytes".to_owned()
            )
        );
    }

    #[test]
    fn service_runs_successful_request_without_refresh() {
        let service = SpotifyService::new(AuthCodePkceSpotify::default());

        let result = service
            .request_with_auth_retry(|client| {
                assert!(client.creds.id.is_empty());
                Ok::<_, ClientError>("ok")
            })
            .expect("successful request should pass through");

        assert_eq!(result, "ok");
        assert!(service.client().creds.id.is_empty());
    }

    #[test]
    fn service_refresh_without_loaded_token_requires_reauthorization() {
        let service = SpotifyService::new(AuthCodePkceSpotify::default());

        let err = service
            .refresh_token()
            .expect_err("missing token should require reauthorization");

        assert!(matches!(
            err,
            SpotifyServiceError::ReauthorizationRequired(_)
        ));
    }
}
