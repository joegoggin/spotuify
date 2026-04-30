use std::error::Error;
use std::fmt;

use rspotify::ClientError;

use super::retry::{
    client_error_status, is_auth_expired_error, is_transient_status, is_transient_transport_error,
    spotify_client_error_message,
};

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
    /// Classifies an `rspotify` client error into service-level categories.
    pub(super) fn from_client_error(err: ClientError) -> Self {
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

    /// Maps token refresh failures to reauthorization-required errors.
    pub(super) fn refresh_failed(err: ClientError) -> Self {
        Self::ReauthorizationRequired(format!(
            "Spotify token refresh failed ({err}); reauthorization required"
        ))
    }

    /// Maps JSON decode failures from Spotify responses.
    pub(super) fn decode_failed(err: serde_json::Error) -> Self {
        Self::Decode(format!("Spotify response JSON decode failed: {err}"))
    }

    /// Builds a decode error for endpoints expected to return an empty body.
    pub(super) fn unexpected_body(body: &str) -> Self {
        Self::Decode(format!(
            "Spotify response was expected to be empty but returned {} bytes",
            body.len()
        ))
    }

    /// Maps HTTP status + message into the most specific service error variant.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
