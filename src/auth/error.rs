use std::error::Error;
use std::fmt;
use std::io;

use rspotify::ClientError;

/// Errors produced by the Spotify auth flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Required app configuration is missing.
    MissingConfig(&'static str),
    /// XDG could not resolve the token cache path.
    TokenCachePathUnavailable,
    /// The configured redirect URI cannot be used by the local callback listener.
    InvalidRedirectUri(String),
    /// The browser callback request was missing or malformed.
    InvalidCallback(String),
    /// Spotify redirected back with an OAuth error instead of an auth code.
    AuthorizationRejected {
        /// OAuth error identifier returned by Spotify.
        error: String,
        /// Optional OAuth error description returned by Spotify.
        description: Option<String>,
    },
    /// Filesystem or socket IO failed.
    Io(String),
    /// Spotify client authorization failed.
    Spotify(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConfig(field) => write!(formatter, "missing required config: {field}"),
            Self::TokenCachePathUnavailable => {
                write!(formatter, "could not resolve token cache path")
            }
            Self::InvalidRedirectUri(message) => {
                write!(formatter, "invalid redirect URI: {message}")
            }
            Self::InvalidCallback(message) => write!(formatter, "invalid callback: {message}"),
            Self::AuthorizationRejected { error, description } => {
                write!(formatter, "Spotify authorization returned {error}")?;
                if let Some(description) = description {
                    write!(formatter, ": {description}")?;
                }
                Ok(())
            }
            Self::Io(message) => write!(formatter, "auth IO failed: {message}"),
            Self::Spotify(message) => write!(formatter, "Spotify auth failed: {message}"),
        }
    }
}

impl Error for AuthError {}

impl From<io::Error> for AuthError {
    fn from(err: io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

impl From<url::ParseError> for AuthError {
    fn from(err: url::ParseError) -> Self {
        Self::InvalidRedirectUri(err.to_string())
    }
}

/// Converts a Spotify client error into a compact, user-facing message.
pub(super) fn spotify_client_error_message(err: ClientError) -> String {
    match err {
        ClientError::Http(http_error) => match *http_error {
            rspotify::http::HttpError::StatusCode(response) => {
                let status = response.status();
                match response.into_string() {
                    Ok(body) if !body.trim().is_empty() => {
                        format!("Spotify token endpoint returned HTTP {status}: {body}")
                    }
                    Ok(_) => format!("Spotify token endpoint returned HTTP {status}"),
                    Err(err) => format!(
                        "Spotify token endpoint returned HTTP {status}; could not read response body: {err}"
                    ),
                }
            }
            other => other.to_string(),
        },
        other => other.to_string(),
    }
}
