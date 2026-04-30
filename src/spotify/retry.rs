use rspotify::ClientError;

use super::error::SpotifyServiceError;

/// Maximum number of transient retries attempted per Spotify request.
pub(super) const TRANSIENT_RETRY_LIMIT: usize = 2;

/// Runs a request once, refreshes auth on first auth-expired error, and retries once.
pub(super) fn run_with_refresh_retry<T, E>(
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

/// Runs a request with one auth refresh attempt and bounded transient retries.
pub(super) fn run_with_refresh_and_transient_retry<T, E>(
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

/// Returns true when a client error indicates expired/invalid Spotify auth.
pub(super) fn is_auth_expired_error(err: &ClientError) -> bool {
    match err {
        ClientError::InvalidToken => true,
        ClientError::Http(http_error) => match http_error.as_ref() {
            rspotify::http::HttpError::StatusCode(response) => response.status() == 401,
            _ => false,
        },
        _ => false,
    }
}

/// Returns true when a client error is safe to retry transiently.
pub(super) fn is_transient_client_error(err: &ClientError) -> bool {
    client_error_status(err).is_some_and(is_transient_status) || is_transient_transport_error(err)
}

/// Returns true when an underlying transport or IO layer failed transiently.
pub(super) fn is_transient_transport_error(err: &ClientError) -> bool {
    match err {
        ClientError::Http(http_error) => match http_error.as_ref() {
            rspotify::http::HttpError::Transport(_) | rspotify::http::HttpError::Io(_) => true,
            rspotify::http::HttpError::StatusCode(_) => false,
        },
        _ => false,
    }
}

/// Extracts an HTTP status code when the client error wraps a status response.
pub(super) fn client_error_status(err: &ClientError) -> Option<u16> {
    match err {
        ClientError::Http(http_error) => match http_error.as_ref() {
            rspotify::http::HttpError::StatusCode(response) => Some(response.status()),
            _ => None,
        },
        _ => None,
    }
}

/// Returns true when an HTTP status should be treated as transient.
pub(super) fn is_transient_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// Converts an `rspotify` client error into a compact user-facing message.
pub(super) fn spotify_client_error_message(err: ClientError) -> String {
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
}
