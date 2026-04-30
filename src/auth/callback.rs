//! Local HTTP callback handling for Spotify OAuth redirects.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use rspotify::AuthCodePkceSpotify;
use rspotify::prelude::OAuthClient;
use url::Url;

use super::error::AuthError;
use super::redirect::LocalRedirect;

/// Plain-text body returned to the browser after a successful token exchange.
pub(super) const CALLBACK_RESPONSE_SUCCESS: &str =
    "Spotify authorization completed. You can return to spotuify.";
/// Maximum callback HTTP request size accepted by the local listener.
const HTTP_REQUEST_LIMIT: usize = 16 * 1024;
/// HTTP status used for successful callback responses.
pub(super) const HTTP_RESPONSE_OK: &str = "200 OK";
/// HTTP status used for malformed callback or auth failure responses.
pub(super) const HTTP_RESPONSE_BAD_REQUEST: &str = "400 Bad Request";

/// Temporary callback listener for the Spotify auth redirect.
#[derive(Debug)]
pub(super) struct CallbackListener {
    /// Bound TCP listener waiting for Spotify redirects.
    listener: TcpListener,
    /// Parsed redirect configuration used to validate callback targets.
    redirect: LocalRedirect,
}

impl CallbackListener {
    /// Binds the callback listener to the redirect host and port.
    pub(super) fn bind(redirect: LocalRedirect) -> Result<Self, AuthError> {
        let listener = TcpListener::bind((redirect.host.as_str(), redirect.port))?;

        Ok(Self { listener, redirect })
    }

    /// Returns the callback address shown in status updates.
    pub(super) fn callback_addr(&self) -> String {
        self.redirect.bind_addr()
    }

    /// Accepts one callback request and extracts its authorization code.
    pub(super) fn receive_authorization_code(
        &self,
        client: &AuthCodePkceSpotify,
    ) -> Result<CallbackRequest, AuthError> {
        let (mut stream, _) = self.listener.accept()?;
        let code_result = self.read_authorization_code(&mut stream, client);

        match code_result {
            Ok(code) => Ok(CallbackRequest { stream, code }),
            Err(err) => {
                write_browser_response(
                    &mut stream,
                    HTTP_RESPONSE_BAD_REQUEST,
                    &browser_failure_body(&err),
                );
                Err(err)
            }
        }
    }

    /// Parses and validates a callback request into a Spotify auth code.
    fn read_authorization_code(
        &self,
        stream: &mut TcpStream,
        client: &AuthCodePkceSpotify,
    ) -> Result<String, AuthError> {
        let request_target = read_request_target(stream)?;
        let callback_url = self
            .redirect
            .callback_url_from_request_target(&request_target)?;
        if let Some(err) = authorization_error_from_callback_url(&callback_url) {
            return Err(err);
        }

        client.parse_response_code(&callback_url).ok_or_else(|| {
            AuthError::InvalidCallback(
                "Spotify callback did not include a valid code and state".to_owned(),
            )
        })
    }
}

/// Pending callback response after the code has been parsed.
#[derive(Debug)]
pub(super) struct CallbackRequest {
    /// Open stream used to return the browser response.
    stream: TcpStream,
    /// Authorization code parsed from the callback request.
    pub(super) code: String,
}

impl CallbackRequest {
    /// Sends an HTTP response and closes the callback stream.
    pub(super) fn respond(mut self, status: &str, body: &str) {
        write_browser_response(&mut self.stream, status, body);
    }
}

/// Reads a single HTTP request target from the callback stream.
fn read_request_target(stream: &mut TcpStream) -> Result<String, AuthError> {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];

    loop {
        let bytes_read = stream.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }

        request.extend_from_slice(&buffer[..bytes_read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }

        if request.len() > HTTP_REQUEST_LIMIT {
            return Err(AuthError::InvalidCallback(
                "callback request exceeded maximum size".to_owned(),
            ));
        }
    }

    let request = String::from_utf8_lossy(&request);
    let request_line = request
        .lines()
        .next()
        .ok_or_else(|| AuthError::InvalidCallback("callback request was empty".to_owned()))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| AuthError::InvalidCallback("callback request method missing".to_owned()))?;
    let target = parts
        .next()
        .ok_or_else(|| AuthError::InvalidCallback("callback request target missing".to_owned()))?;
    let _version = parts
        .next()
        .ok_or_else(|| AuthError::InvalidCallback("callback request version missing".to_owned()))?;

    if method != "GET" {
        return Err(AuthError::InvalidCallback(format!(
            "callback request used unsupported method {method}"
        )));
    }

    Ok(target.to_owned())
}

/// Writes a minimal plain-text HTTP response for callback browser UX.
fn write_browser_response(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

/// Extracts OAuth error query params from a callback URL when present.
fn authorization_error_from_callback_url(callback_url: &str) -> Option<AuthError> {
    let parsed = Url::parse(callback_url).ok()?;
    let mut error = None;
    let mut description = None;

    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "error" => error = Some(value.into_owned()),
            "error_description" => description = Some(value.into_owned()),
            _ => {}
        }
    }

    error.map(|error| AuthError::AuthorizationRejected { error, description })
}

/// Builds a browser-visible failure body with an actionable auth message.
pub(super) fn browser_failure_body(err: &AuthError) -> String {
    format!("Spotify authorization failed.\n\n{err}\n\nReturn to spotuify for details.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_url_reports_spotify_authorization_error() {
        let err = authorization_error_from_callback_url(
            "http://127.0.0.1:8888/callback?error=access_denied&error_description=User%20denied%20access&state=state-123",
        )
        .expect("callback error should be parsed");

        assert_eq!(
            err,
            AuthError::AuthorizationRejected {
                error: "access_denied".to_owned(),
                description: Some("User denied access".to_owned()),
            }
        );
        assert_eq!(
            err.to_string(),
            "Spotify authorization returned access_denied: User denied access"
        );
    }

    #[test]
    fn browser_failure_body_includes_actionable_error() {
        let body = browser_failure_body(&AuthError::Spotify(
            "Spotify token endpoint returned HTTP 400: invalid_grant".to_owned(),
        ));

        assert!(body.contains("Spotify authorization failed."));
        assert!(body.contains("invalid_grant"));
    }
}
