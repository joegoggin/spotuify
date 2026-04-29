use std::error::Error;
use std::fmt;

use rspotify::prelude::BaseClient;
use rspotify::{AuthCodePkceSpotify, ClientError};

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
    /// Spotify returned a non-auth client error.
    Spotify(String),
}

impl SpotifyServiceError {
    fn from_client_error(err: ClientError) -> Self {
        if is_auth_expired_error(&err) {
            Self::ReauthorizationRequired(
                "Spotify credentials are invalid or expired; reauthorization required".to_owned(),
            )
        } else {
            Self::Spotify(err.to_string())
        }
    }

    fn refresh_failed(err: ClientError) -> Self {
        Self::ReauthorizationRequired(format!(
            "Spotify token refresh failed ({err}); reauthorization required"
        ))
    }
}

impl fmt::Display for SpotifyServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReauthorizationRequired(message) | Self::Spotify(message) => {
                formatter.write_str(message)
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    enum TestError {
        Auth,
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
