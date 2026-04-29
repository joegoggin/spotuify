use std::error::Error;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use rspotify::prelude::{BaseClient, OAuthClient};
use rspotify::{
    AuthCodePkceSpotify, ClientError, Config as SpotifyClientConfig, Credentials, OAuth, Token,
    scopes,
};
use url::Url;

use crate::config::{AppConfig, app_config_file_path};

const TOKEN_CACHE_FILE_NAME: &str = "spotify_token_cache.json";
const CALLBACK_RESPONSE_SUCCESS: &str =
    "Spotify authorization completed. You can return to spotuify.";
const HTTP_REQUEST_LIMIT: usize = 16 * 1024;
const HTTP_RESPONSE_OK: &str = "200 OK";
const HTTP_RESPONSE_BAD_REQUEST: &str = "400 Bad Request";

/// Status updates emitted while the Spotify auth flow runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthEvent {
    /// Auth flow startup began.
    Starting,
    /// A valid cached token was found and can be reused.
    Cached { cache_path: PathBuf },
    /// An expired cached token is being refreshed.
    RefreshingCachedToken { cache_path: PathBuf },
    /// An expired cached token was refreshed and can be reused.
    RefreshedCachedToken { cache_path: PathBuf },
    /// Cached credentials cannot be reused and browser reauthorization is required.
    ReauthorizationRequired { message: String },
    /// A browser authorization URL was generated.
    AuthorizationUrl {
        /// URL the user must visit to authorize the app.
        url: String,
        /// Local callback endpoint currently waiting for Spotify.
        callback_addr: String,
    },
    /// Browser launch failed, but the URL can still be copied manually.
    BrowserOpenFailed { message: String },
    /// The callback listener is waiting for Spotify to redirect back.
    WaitingForCallback { callback_addr: String },
    /// The callback code was received and is being exchanged for tokens.
    ExchangingCode,
    /// Tokens were received and persisted.
    Completed { cache_path: PathBuf },
    /// Auth cannot continue.
    Failed { message: String },
}

impl AuthEvent {
    /// Returns true when no more auth worker events are expected.
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Cached { .. }
                | Self::RefreshedCachedToken { .. }
                | Self::Completed { .. }
                | Self::Failed { .. }
        )
    }
}

/// Errors produced by the Spotify auth flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthError {
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

/// Starts the Spotify auth flow on a background thread.
pub(crate) fn spawn_auth_flow(config: AppConfig) -> Receiver<AuthEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        run_auth_flow(config, &sender);
    });

    receiver
}

fn run_auth_flow(config: AppConfig, sender: &Sender<AuthEvent>) {
    send_event(sender, AuthEvent::Starting);

    if let Err(err) = run_auth_flow_inner(config, sender) {
        send_event(
            sender,
            AuthEvent::Failed {
                message: err.to_string(),
            },
        );
    }
}

fn run_auth_flow_inner(config: AppConfig, sender: &Sender<AuthEvent>) -> Result<(), AuthError> {
    let mut session = AuthSession::from_config(&config)?;
    session.complete_with_local_callback(|event| send_event(sender, event))
}

fn send_event(sender: &Sender<AuthEvent>, event: AuthEvent) {
    let _ = sender.send(event);
}

/// Spotify authorization session state.
#[derive(Debug)]
pub(crate) struct AuthSession {
    client: AuthCodePkceSpotify,
    redirect: LocalRedirect,
    cache_path: PathBuf,
}

impl AuthSession {
    /// Creates an auth session from validated app config.
    pub(crate) fn from_config(config: &AppConfig) -> Result<Self, AuthError> {
        let client_id = config
            .spotify
            .client_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(AuthError::MissingConfig("spotify.client_id"))?;
        let redirect_uri = config
            .spotify
            .redirect_uri
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(AuthError::MissingConfig("spotify.redirect_uri"))?;
        let cache_path = app_config_file_path(TOKEN_CACHE_FILE_NAME)
            .ok_or(AuthError::TokenCachePathUnavailable)?;
        let redirect = LocalRedirect::from_uri(redirect_uri)?;
        let credentials = Credentials::new_pkce(client_id);
        let oauth = OAuth {
            redirect_uri: redirect_uri.to_owned(),
            scopes: scopes!(
                "user-read-private",
                "user-read-playback-state",
                "user-read-currently-playing",
                "user-modify-playback-state",
                "user-library-read",
                "playlist-read-private",
                "playlist-read-collaborative"
            ),
            ..OAuth::default()
        };
        let client_config = SpotifyClientConfig {
            cache_path: cache_path.clone(),
            token_cached: true,
            token_refreshing: true,
            ..SpotifyClientConfig::default()
        };
        let client = AuthCodePkceSpotify::with_config(credentials, oauth, client_config);

        Ok(Self {
            client,
            redirect,
            cache_path,
        })
    }

    /// Generates the Spotify authorization URL.
    pub(crate) fn authorize_url(&mut self) -> Result<String, AuthError> {
        self.client
            .get_authorize_url(None)
            .map_err(|err| AuthError::Spotify(err.to_string()))
    }

    /// Runs the full browser/callback/code exchange flow.
    pub(crate) fn complete_with_local_callback(
        &mut self,
        mut on_event: impl FnMut(AuthEvent),
    ) -> Result<(), AuthError> {
        if self.try_cached_token(&mut on_event)? {
            return Ok(());
        }

        let listener = self.bind_local_callback()?;
        let callback_addr = listener.callback_addr();
        let authorize_url = self.authorize_url()?;

        on_event(AuthEvent::AuthorizationUrl {
            url: authorize_url.clone(),
            callback_addr: callback_addr.clone(),
        });

        if let Err(err) = open::that(&authorize_url) {
            on_event(AuthEvent::BrowserOpenFailed {
                message: err.to_string(),
            });
        }

        on_event(AuthEvent::WaitingForCallback { callback_addr });

        let callback = listener.receive_authorization_code(&self.client)?;
        on_event(AuthEvent::ExchangingCode);
        self.ensure_cache_parent_dir()?;

        match self.client.request_token(&callback.code) {
            Ok(()) => {
                callback.respond(HTTP_RESPONSE_OK, CALLBACK_RESPONSE_SUCCESS);
                on_event(AuthEvent::Completed {
                    cache_path: self.cache_path.clone(),
                });
                Ok(())
            }
            Err(err) => {
                let error = AuthError::Spotify(spotify_client_error_message(err));
                callback.respond(HTTP_RESPONSE_BAD_REQUEST, &browser_failure_body(&error));
                Err(error)
            }
        }
    }

    fn try_cached_token(&self, on_event: &mut impl FnMut(AuthEvent)) -> Result<bool, AuthError> {
        if !self.cache_path.exists() {
            return Ok(false);
        }

        match self.client.read_token_cache(true) {
            Ok(Some(token)) if token.is_expired() => self.refresh_cached_token(token, on_event),
            Ok(Some(token)) => {
                self.replace_client_token(Some(token))?;
                on_event(AuthEvent::Cached {
                    cache_path: self.cache_path.clone(),
                });
                Ok(true)
            }
            Ok(None) => {
                on_event(AuthEvent::ReauthorizationRequired {
                    message:
                        "Cached Spotify token is missing required scopes; reauthorization required"
                            .to_owned(),
                });
                Ok(false)
            }
            Err(err) => {
                on_event(AuthEvent::ReauthorizationRequired {
                    message: format!(
                        "Cached Spotify token could not be read ({err}); reauthorization required"
                    ),
                });
                Ok(false)
            }
        }
    }

    fn refresh_cached_token(
        &self,
        token: Token,
        on_event: &mut impl FnMut(AuthEvent),
    ) -> Result<bool, AuthError> {
        on_event(AuthEvent::RefreshingCachedToken {
            cache_path: self.cache_path.clone(),
        });
        self.replace_client_token(Some(token))?;

        match self.client.refresh_token() {
            Ok(()) if self.has_usable_client_token()? => {
                on_event(AuthEvent::RefreshedCachedToken {
                    cache_path: self.cache_path.clone(),
                });
                Ok(true)
            }
            Ok(()) => {
                self.replace_client_token(None)?;
                on_event(AuthEvent::ReauthorizationRequired {
                    message: "Cached Spotify token did not include a refresh token; reauthorization required"
                        .to_owned(),
                });
                Ok(false)
            }
            Err(err) => {
                self.replace_client_token(None)?;
                on_event(AuthEvent::ReauthorizationRequired {
                    message: format!(
                        "Cached Spotify token refresh failed ({}); reauthorization required",
                        spotify_client_error_message(err)
                    ),
                });
                Ok(false)
            }
        }
    }

    fn replace_client_token(&self, token: Option<Token>) -> Result<(), AuthError> {
        let token_store = self.client.get_token();
        let mut token_store = token_store
            .lock()
            .map_err(|_| AuthError::Spotify("Spotify token lock was poisoned".to_owned()))?;
        *token_store = token;

        Ok(())
    }

    fn has_usable_client_token(&self) -> Result<bool, AuthError> {
        let token_store = self.client.get_token();
        let token_store = token_store
            .lock()
            .map_err(|_| AuthError::Spotify("Spotify token lock was poisoned".to_owned()))?;

        Ok(token_store
            .as_ref()
            .is_some_and(|token| !token.is_expired()))
    }

    fn bind_local_callback(&self) -> Result<CallbackListener, AuthError> {
        CallbackListener::bind(self.redirect.clone())
    }

    fn ensure_cache_parent_dir(&self) -> Result<(), AuthError> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        Ok(())
    }
}

fn spotify_client_error_message(err: ClientError) -> String {
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

/// Parsed local redirect URI information.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalRedirect {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

impl LocalRedirect {
    fn from_uri(uri: &str) -> Result<Self, AuthError> {
        let parsed = Url::parse(uri)?;
        let scheme = parsed.scheme();
        if scheme != "http" {
            return Err(AuthError::InvalidRedirectUri(
                "local callback listener requires an http redirect URI".to_owned(),
            ));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| {
                AuthError::InvalidRedirectUri("redirect URI is missing a host".to_owned())
            })?
            .to_owned();
        if !is_loopback_ip_literal(&host) {
            return Err(AuthError::InvalidRedirectUri(format!(
                "redirect host must be an explicit loopback IP literal like 127.0.0.1 or ::1, got {host}"
            )));
        }

        let port = parsed.port_or_known_default().ok_or_else(|| {
            AuthError::InvalidRedirectUri("redirect URI is missing a port".to_owned())
        })?;
        let path = parsed.path().to_owned();

        Ok(Self {
            scheme: scheme.to_owned(),
            host,
            port,
            path,
        })
    }

    fn bind_addr(&self) -> String {
        format!("{}:{}", self.host_for_url(), self.port)
    }

    fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.bind_addr())
    }

    fn callback_url_from_request_target(&self, request_target: &str) -> Result<String, AuthError> {
        let callback_url = if request_target.starts_with("http://")
            || request_target.starts_with("https://")
        {
            Url::parse(request_target).map_err(|err| AuthError::InvalidCallback(err.to_string()))?
        } else {
            Url::parse(&format!("{}{}", self.origin(), request_target))
                .map_err(|err| AuthError::InvalidCallback(err.to_string()))?
        };

        if callback_url.scheme() != self.scheme {
            return Err(AuthError::InvalidCallback(
                "callback scheme did not match configured redirect URI".to_owned(),
            ));
        }

        let callback_host = callback_url.host_str().unwrap_or_default();
        if !callback_host.eq_ignore_ascii_case(&self.host) {
            return Err(AuthError::InvalidCallback(
                "callback host did not match configured redirect URI".to_owned(),
            ));
        }

        if callback_url.port_or_known_default() != Some(self.port) {
            return Err(AuthError::InvalidCallback(
                "callback port did not match configured redirect URI".to_owned(),
            ));
        }

        if callback_url.path() != self.path {
            return Err(AuthError::InvalidCallback(
                "callback path did not match configured redirect URI".to_owned(),
            ));
        }

        Ok(callback_url.to_string())
    }

    fn host_for_url(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }
}

fn is_loopback_ip_literal(host: &str) -> bool {
    host.parse::<IpAddr>()
        .map(|addr| addr.is_loopback())
        .unwrap_or(false)
}

/// Temporary callback listener for the Spotify auth redirect.
#[derive(Debug)]
struct CallbackListener {
    listener: TcpListener,
    redirect: LocalRedirect,
}

impl CallbackListener {
    fn bind(redirect: LocalRedirect) -> Result<Self, AuthError> {
        let listener = TcpListener::bind((redirect.host.as_str(), redirect.port))?;

        Ok(Self { listener, redirect })
    }

    fn callback_addr(&self) -> String {
        self.redirect.bind_addr()
    }

    fn receive_authorization_code(
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
struct CallbackRequest {
    stream: TcpStream,
    code: String,
}

impl CallbackRequest {
    fn respond(mut self, status: &str, body: &str) {
        write_browser_response(&mut self.stream, status, body);
    }
}

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

fn write_browser_response(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

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

fn browser_failure_body(err: &AuthError) -> String {
    format!("Spotify authorization failed.\n\n{err}\n\nReturn to spotuify for details.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SpotifyConfig;

    fn required_test_scopes() -> std::collections::HashSet<String> {
        scopes!(
            "user-read-private",
            "user-read-playback-state",
            "user-read-currently-playing",
            "user-modify-playback-state",
            "user-library-read",
            "playlist-read-private",
            "playlist-read-collaborative"
        )
    }

    fn valid_config() -> AppConfig {
        AppConfig {
            spotify: SpotifyConfig {
                client_id: Some("client-id".to_owned()),
                redirect_uri: Some("http://127.0.0.1:8888/callback".to_owned()),
            },
        }
    }

    fn temp_token_path(test_name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "spotuify-{test_name}-{}-token.json",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    fn test_auth_session(test_name: &str) -> AuthSession {
        let cache_path = temp_token_path(test_name);
        let redirect_uri = "http://127.0.0.1:8888/callback";
        let credentials = Credentials::new_pkce("client-id");
        let oauth = OAuth {
            redirect_uri: redirect_uri.to_owned(),
            scopes: required_test_scopes(),
            ..OAuth::default()
        };
        let client_config = SpotifyClientConfig {
            cache_path: cache_path.clone(),
            token_cached: true,
            token_refreshing: true,
            ..SpotifyClientConfig::default()
        };
        let client = AuthCodePkceSpotify::with_config(credentials, oauth, client_config);

        AuthSession {
            client,
            redirect: LocalRedirect::from_uri(redirect_uri).expect("redirect should parse"),
            cache_path,
        }
    }

    fn expired_token(refresh_token: Option<&str>) -> Token {
        Token {
            access_token: "expired-access-token".to_owned(),
            refresh_token: refresh_token.map(str::to_owned),
            scopes: required_test_scopes(),
            ..Token::default()
        }
    }

    #[test]
    fn auth_session_builds_from_valid_config() {
        let session = AuthSession::from_config(&valid_config()).expect("auth session should build");

        assert_eq!(session.client.creds.id, "client-id");
        assert_eq!(session.redirect.host, "127.0.0.1");
        assert_eq!(session.redirect.port, 8888);
        assert_eq!(session.redirect.path, "/callback");
        assert!(session.cache_path.ends_with(TOKEN_CACHE_FILE_NAME));
    }

    #[test]
    fn auth_session_rejects_missing_client_id() {
        let mut config = valid_config();
        config.spotify.client_id = Some(" ".to_owned());

        assert_eq!(
            AuthSession::from_config(&config).expect_err("blank client ID should fail"),
            AuthError::MissingConfig("spotify.client_id")
        );
    }

    #[test]
    fn local_redirect_rejects_non_local_hosts() {
        let err = LocalRedirect::from_uri("http://example.com:8888/callback")
            .expect_err("non-local redirect should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn local_redirect_rejects_localhost_hostname() {
        let err = LocalRedirect::from_uri("http://localhost:8888/callback")
            .expect_err("localhost redirect should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn cached_expired_token_without_refresh_token_falls_back_to_reauthorization() {
        let session = test_auth_session("expired-without-refresh");
        expired_token(None)
            .write_cache(&session.cache_path)
            .expect("token cache should be writable");
        let mut events = Vec::new();

        let reused_cache = session
            .try_cached_token(&mut |event| events.push(event))
            .expect("cache fallback should not fail auth flow");

        let _ = fs::remove_file(&session.cache_path);
        assert!(!reused_cache);
        assert_eq!(
            events.first(),
            Some(&AuthEvent::RefreshingCachedToken {
                cache_path: session.cache_path.clone(),
            })
        );
        assert!(matches!(
            events.last(),
            Some(AuthEvent::ReauthorizationRequired { message })
                if message.contains("reauthorization required")
        ));
    }

    #[test]
    fn malformed_cached_token_falls_back_to_reauthorization() {
        let session = test_auth_session("malformed-cache");
        fs::write(&session.cache_path, "not valid token json")
            .expect("token cache should be writable");
        let mut events = Vec::new();

        let reused_cache = session
            .try_cached_token(&mut |event| events.push(event))
            .expect("malformed cache should not fail auth flow");

        let _ = fs::remove_file(&session.cache_path);
        assert!(!reused_cache);
        assert!(matches!(
            events.as_slice(),
            [AuthEvent::ReauthorizationRequired { message }]
                if message.contains("could not be read")
        ));
    }

    #[test]
    fn local_redirect_rejects_https_scheme() {
        let err = LocalRedirect::from_uri("https://127.0.0.1:8888/callback")
            .expect_err("https redirect should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn callback_url_is_reconstructed_from_origin_form_target() {
        let redirect = LocalRedirect::from_uri("http://127.0.0.1:8888/callback")
            .expect("redirect should parse");

        let callback_url = redirect
            .callback_url_from_request_target("/callback?code=code-123&state=state-123")
            .expect("callback URL should be reconstructed");

        assert_eq!(
            callback_url,
            "http://127.0.0.1:8888/callback?code=code-123&state=state-123"
        );
    }

    #[test]
    fn callback_url_rejects_unexpected_path() {
        let redirect = LocalRedirect::from_uri("http://127.0.0.1:8888/callback")
            .expect("redirect should parse");

        let err = redirect
            .callback_url_from_request_target("/wrong?code=code-123&state=state-123")
            .expect_err("wrong path should fail");

        assert!(matches!(err, AuthError::InvalidCallback(_)));
    }

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

    #[test]
    fn auth_event_marks_terminal_states() {
        assert!(
            AuthEvent::Cached {
                cache_path: PathBuf::from("/tmp/token.json"),
            }
            .is_terminal()
        );
        assert!(
            AuthEvent::Completed {
                cache_path: PathBuf::from("/tmp/token.json"),
            }
            .is_terminal()
        );
        assert!(
            AuthEvent::RefreshedCachedToken {
                cache_path: PathBuf::from("/tmp/token.json"),
            }
            .is_terminal()
        );
        assert!(
            AuthEvent::Failed {
                message: "failed".to_owned(),
            }
            .is_terminal()
        );
        assert!(!AuthEvent::Starting.is_terminal());
        assert!(
            !AuthEvent::RefreshingCachedToken {
                cache_path: PathBuf::from("/tmp/token.json"),
            }
            .is_terminal()
        );
        assert!(
            !AuthEvent::ReauthorizationRequired {
                message: "reauth required".to_owned(),
            }
            .is_terminal()
        );
    }
}
