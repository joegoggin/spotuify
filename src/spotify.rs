use std::error::Error;
use std::fmt;

use rspotify::http::Query;
use rspotify::prelude::BaseClient;
use rspotify::{AuthCodePkceSpotify, ClientError};
use serde::de::DeserializeOwned;
use serde_json::Value;

const TRANSIENT_RETRY_LIMIT: usize = 2;

/// Central wrapper for Spotify API access.
#[derive(Debug, Clone)]
pub(crate) struct SpotifyService {
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

/// Errors produced by the Spotify service layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpotifyServiceError {
    /// Spotify credentials cannot be refreshed automatically.
    ReauthorizationRequired(String),
    /// Spotify rate limited the request after retries were exhausted.
    RateLimited(String),
    /// Spotify or the network returned a transient failure after retries were exhausted.
    Transient(String),
    /// Spotify returned a non-auth, non-transient client error.
    Spotify(String),
    /// Spotify returned a response body that could not be decoded for the requested type.
    Decode(String),
}

impl SpotifyServiceError {
    fn from_client_error(err: ClientError) -> Self {
        if is_auth_expired_error(&err) {
            Self::ReauthorizationRequired(
                "Spotify credentials are invalid or expired; reauthorization required".to_owned(),
            )
        } else if let Some(status) = client_error_status(&err) {
            let message = spotify_client_error_message(err);
            Self::from_status(status, message)
        } else if is_transient_transport_error(&err) {
            Self::Transient(spotify_client_error_message(err))
        } else {
            Self::Spotify(spotify_client_error_message(err))
        }
    }

    fn refresh_failed(err: ClientError) -> Self {
        Self::ReauthorizationRequired(format!(
            "Spotify token refresh failed ({err}); reauthorization required"
        ))
    }

    fn decode_failed(err: serde_json::Error) -> Self {
        Self::Decode(format!("Spotify response JSON decode failed: {err}"))
    }

    fn unexpected_body(body: &str) -> Self {
        Self::Decode(format!(
            "Spotify response was expected to be empty but returned {} bytes",
            body.len()
        ))
    }

    fn from_status(status: u16, message: String) -> Self {
        if status == 429 {
            Self::RateLimited(message)
        } else if is_transient_status(status) {
            Self::Transient(message)
        } else {
            Self::Spotify(message)
        }
    }
}

impl fmt::Display for SpotifyServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReauthorizationRequired(message)
            | Self::RateLimited(message)
            | Self::Transient(message)
            | Self::Spotify(message)
            | Self::Decode(message) => formatter.write_str(message),
        }
    }
}

impl Error for SpotifyServiceError {}

fn run_with_refresh_retry<T, E>(
    mut request: impl FnMut() -> Result<T, E>,
    mut refresh: impl FnMut() -> Result<(), SpotifyServiceError>,
    is_refreshable_auth_error: impl Fn(&E) -> bool,
    map_error: impl Fn(E) -> SpotifyServiceError,
) -> Result<T, SpotifyServiceError> {
    let first_error = match request() {
        Ok(value) => return Ok(value),
        Err(err) => err,
    };

    if !is_refreshable_auth_error(&first_error) {
        return Err(map_error(first_error));
    }

    refresh()?;

    match request() {
        Ok(value) => Ok(value),
        Err(err) if is_refreshable_auth_error(&err) => {
            Err(SpotifyServiceError::ReauthorizationRequired(
                "Spotify rejected refreshed credentials; reauthorization required".to_owned(),
            ))
        }
        Err(err) => Err(map_error(err)),
    }
}

fn run_with_refresh_and_transient_retry<T, E>(
    mut request: impl FnMut() -> Result<T, E>,
    mut refresh: impl FnMut() -> Result<(), SpotifyServiceError>,
    is_refreshable_auth_error: impl Fn(&E) -> bool,
    is_transient_error: impl Fn(&E) -> bool,
    map_error: impl Fn(E) -> SpotifyServiceError,
    transient_retry_limit: usize,
) -> Result<T, SpotifyServiceError> {
    let mut refreshed = false;
    let mut transient_retries = 0;

    loop {
        match request() {
            Ok(value) => return Ok(value),
            Err(err) if is_refreshable_auth_error(&err) && !refreshed => {
                refreshed = true;
                refresh()?;
            }
            Err(err) if is_refreshable_auth_error(&err) => {
                return Err(SpotifyServiceError::ReauthorizationRequired(
                    "Spotify rejected refreshed credentials; reauthorization required".to_owned(),
                ));
            }
            Err(err) if is_transient_error(&err) && transient_retries < transient_retry_limit => {
                transient_retries += 1;
            }
            Err(err) => return Err(map_error(err)),
        }
    }
}

fn is_auth_expired_error(err: &ClientError) -> bool {
    match err {
        ClientError::InvalidToken => true,
        ClientError::Http(http_error) => match http_error.as_ref() {
            rspotify::http::HttpError::StatusCode(response) => response.status() == 401,
            _ => false,
        },
        _ => false,
    }
}

fn is_transient_client_error(err: &ClientError) -> bool {
    client_error_status(err).is_some_and(is_transient_status) || is_transient_transport_error(err)
}

fn is_transient_transport_error(err: &ClientError) -> bool {
    match err {
        ClientError::Http(http_error) => match http_error.as_ref() {
            rspotify::http::HttpError::Transport(_) | rspotify::http::HttpError::Io(_) => true,
            rspotify::http::HttpError::StatusCode(_) => false,
        },
        _ => false,
    }
}

fn client_error_status(err: &ClientError) -> Option<u16> {
    match err {
        ClientError::Http(http_error) => match http_error.as_ref() {
            rspotify::http::HttpError::StatusCode(response) => Some(response.status()),
            _ => None,
        },
        _ => None,
    }
}

fn is_transient_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

fn spotify_client_error_message(err: ClientError) -> String {
    match err {
        ClientError::Http(http_error) => match *http_error {
            rspotify::http::HttpError::StatusCode(response) => {
                let status = response.status();
                match response.into_string() {
                    Ok(body) if !body.trim().is_empty() => {
                        format!("Spotify API returned HTTP {status}: {body}")
                    }
                    Ok(_) => format!("Spotify API returned HTTP {status}"),
                    Err(err) => format!(
                        "Spotify API returned HTTP {status}; could not read response body: {err}"
                    ),
                }
            }
            other => other.to_string(),
        },
        other => other.to_string(),
    }
}

fn decode_json_response<T>(response: &str) -> Result<T, SpotifyServiceError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(response).map_err(SpotifyServiceError::decode_failed)
}

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

    #[derive(Debug)]
    enum TestError {
        Auth,
        Transient,
        Other,
    }

    fn map_test_error(err: TestError) -> SpotifyServiceError {
        SpotifyServiceError::Spotify(format!("{err:?}"))
    }

    #[test]
    fn retry_helper_returns_success_without_refresh() {
        let mut attempts = 0;
        let mut refreshes = 0;

        let result = run_with_refresh_retry(
            || {
                attempts += 1;
                Ok::<_, TestError>("ok")
            },
            || {
                refreshes += 1;
                Ok(())
            },
            |err| matches!(err, TestError::Auth),
            map_test_error,
        )
        .expect("successful request should pass through");

        assert_eq!(result, "ok");
        assert_eq!(attempts, 1);
        assert_eq!(refreshes, 0);
    }

    #[test]
    fn retry_helper_refreshes_and_retries_once_for_auth_error() {
        let mut attempts = 0;
        let mut refreshes = 0;

        let result = run_with_refresh_retry(
            || {
                attempts += 1;
                if attempts == 1 {
                    Err(TestError::Auth)
                } else {
                    Ok("ok")
                }
            },
            || {
                refreshes += 1;
                Ok(())
            },
            |err| matches!(err, TestError::Auth),
            map_test_error,
        )
        .expect("second request should pass after refresh");

        assert_eq!(result, "ok");
        assert_eq!(attempts, 2);
        assert_eq!(refreshes, 1);
    }

    #[test]
    fn retry_helper_does_not_refresh_non_auth_errors() {
        let mut refreshes = 0;

        let err = run_with_refresh_retry(
            || Err::<(), _>(TestError::Other),
            || {
                refreshes += 1;
                Ok(())
            },
            |err| matches!(err, TestError::Auth),
            map_test_error,
        )
        .expect_err("non-auth error should be returned");

        assert_eq!(err, SpotifyServiceError::Spotify("Other".to_owned()));
        assert_eq!(refreshes, 0);
    }

    #[test]
    fn retry_helper_returns_reauth_after_repeated_auth_error() {
        let mut attempts = 0;
        let mut refreshes = 0;

        let err = run_with_refresh_retry(
            || {
                attempts += 1;
                Err::<(), _>(TestError::Auth)
            },
            || {
                refreshes += 1;
                Ok(())
            },
            |err| matches!(err, TestError::Auth),
            map_test_error,
        )
        .expect_err("repeated auth error should require reauthorization");

        assert!(matches!(
            err,
            SpotifyServiceError::ReauthorizationRequired(_)
        ));
        assert_eq!(attempts, 2);
        assert_eq!(refreshes, 1);
    }

    #[test]
    fn invalid_token_is_auth_expired_error() {
        assert!(is_auth_expired_error(&ClientError::InvalidToken));
    }

    #[test]
    fn resilient_retry_helper_retries_transient_errors_to_success() {
        let mut attempts = 0;
        let mut refreshes = 0;

        let result = run_with_refresh_and_transient_retry(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(TestError::Transient)
                } else {
                    Ok("ok")
                }
            },
            || {
                refreshes += 1;
                Ok(())
            },
            |err| matches!(err, TestError::Auth),
            |err| matches!(err, TestError::Transient),
            map_test_error,
            TRANSIENT_RETRY_LIMIT,
        )
        .expect("transient retry should eventually return success");

        assert_eq!(result, "ok");
        assert_eq!(attempts, 3);
        assert_eq!(refreshes, 0);
    }

    #[test]
    fn resilient_retry_helper_returns_final_transient_error_after_limit() {
        let mut attempts = 0;

        let err = run_with_refresh_and_transient_retry(
            || {
                attempts += 1;
                Err::<(), _>(TestError::Transient)
            },
            || Ok(()),
            |err| matches!(err, TestError::Auth),
            |err| matches!(err, TestError::Transient),
            |_| SpotifyServiceError::Transient("final transient".to_owned()),
            TRANSIENT_RETRY_LIMIT,
        )
        .expect_err("transient retry limit should be enforced");

        assert_eq!(
            err,
            SpotifyServiceError::Transient("final transient".to_owned())
        );
        assert_eq!(attempts, TRANSIENT_RETRY_LIMIT + 1);
    }

    #[test]
    fn resilient_retry_helper_refreshes_auth_once_across_transient_retries() {
        let mut attempts = 0;
        let mut refreshes = 0;

        let result = run_with_refresh_and_transient_retry(
            || {
                attempts += 1;
                match attempts {
                    1 => Err(TestError::Transient),
                    2 => Err(TestError::Auth),
                    _ => Ok("ok"),
                }
            },
            || {
                refreshes += 1;
                Ok(())
            },
            |err| matches!(err, TestError::Auth),
            |err| matches!(err, TestError::Transient),
            map_test_error,
            TRANSIENT_RETRY_LIMIT,
        )
        .expect("auth refresh should compose with transient retries");

        assert_eq!(result, "ok");
        assert_eq!(attempts, 3);
        assert_eq!(refreshes, 1);
    }

    #[test]
    fn transient_status_detection_covers_rate_limit_and_server_errors() {
        for status in [429, 500, 502, 503, 504] {
            assert!(is_transient_status(status));
        }
    }

    #[test]
    fn transient_status_detection_rejects_non_transient_client_errors() {
        for status in [400, 403, 404] {
            assert!(!is_transient_status(status));
        }
    }

    #[test]
    fn status_mapping_keeps_rate_limits_distinct_from_server_errors() {
        assert_eq!(
            SpotifyServiceError::from_status(429, "rate limit".to_owned()),
            SpotifyServiceError::RateLimited("rate limit".to_owned())
        );
        assert_eq!(
            SpotifyServiceError::from_status(503, "unavailable".to_owned()),
            SpotifyServiceError::Transient("unavailable".to_owned())
        );
        assert_eq!(
            SpotifyServiceError::from_status(404, "missing".to_owned()),
            SpotifyServiceError::Spotify("missing".to_owned())
        );
    }

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
