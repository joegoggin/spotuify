use std::fs;
use std::path::PathBuf;

use rspotify::prelude::{BaseClient, OAuthClient};
use rspotify::{
    AuthCodePkceSpotify, Config as SpotifyClientConfig, Credentials, OAuth, Token, scopes,
};

use crate::config::{AppConfig, app_config_file_path};

use super::callback::{
    CALLBACK_RESPONSE_SUCCESS, CallbackListener, HTTP_RESPONSE_BAD_REQUEST, HTTP_RESPONSE_OK,
    browser_failure_body,
};
use super::error::{AuthError, spotify_client_error_message};
use super::event::AuthEvent;
use super::redirect::LocalRedirect;

/// File name used for Spotify token cache persistence under XDG config dir.
const TOKEN_CACHE_FILE_NAME: &str = "spotify_token_cache.json";

/// Spotify authorization session state.
#[derive(Debug)]
pub struct AuthSession {
    /// Spotify client configured for PKCE auth and token caching.
    client: AuthCodePkceSpotify,
    /// Validated local redirect settings used by callback listener.
    redirect: LocalRedirect,
    /// Absolute path where cached Spotify tokens are persisted.
    cache_path: PathBuf,
}

impl AuthSession {
    /// Creates an auth session from validated app config.
    pub fn from_config(config: &AppConfig) -> Result<Self, AuthError> {
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
    pub fn authorize_url(&mut self) -> Result<String, AuthError> {
        self.client
            .get_authorize_url(None)
            .map_err(|err| AuthError::Spotify(err.to_string()))
    }

    /// Runs the full browser/callback/code exchange flow.
    pub fn complete_with_local_callback(
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
        self.ensure_cache_file_permissions()?;

        match self.client.request_token(&callback.code) {
            Ok(()) => {
                self.harden_cache_file_permissions()?;
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

    /// Attempts to reuse cached credentials before browser auth is required.
    fn try_cached_token(&self, on_event: &mut impl FnMut(AuthEvent)) -> Result<bool, AuthError> {
        if !self.cache_path.exists() {
            return Ok(false);
        }
        self.harden_cache_file_permissions()?;

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

    /// Refreshes an expired cached token and returns whether reuse succeeded.
    fn refresh_cached_token(
        &self,
        token: Token,
        on_event: &mut impl FnMut(AuthEvent),
    ) -> Result<bool, AuthError> {
        on_event(AuthEvent::RefreshingCachedToken {
            cache_path: self.cache_path.clone(),
        });
        let refresh_token = token.refresh_token.clone();
        self.replace_client_token(Some(token))?;
        self.ensure_cache_file_permissions()?;

        match self.client.refresh_token() {
            Ok(()) => {
                self.preserve_refresh_token_if_missing(refresh_token)?;
                if self.has_usable_client_token()? {
                    on_event(AuthEvent::RefreshedCachedToken {
                        cache_path: self.cache_path.clone(),
                    });
                    Ok(true)
                } else {
                    self.replace_client_token(None)?;
                    on_event(AuthEvent::ReauthorizationRequired {
                        message: "Cached Spotify token did not include a refresh token; reauthorization required"
                            .to_owned(),
                    });
                    Ok(false)
                }
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

    /// Replaces the in-memory Spotify client token atomically.
    fn replace_client_token(&self, token: Option<Token>) -> Result<(), AuthError> {
        let token_store = self.client.get_token();
        let mut token_store = token_store
            .lock()
            .map_err(|_| AuthError::Spotify("Spotify token lock was poisoned".to_owned()))?;
        *token_store = token;

        Ok(())
    }

    /// Returns true when an unexpired access token is currently loaded.
    fn has_usable_client_token(&self) -> Result<bool, AuthError> {
        let token_store = self.client.get_token();
        let token_store = token_store
            .lock()
            .map_err(|_| AuthError::Spotify("Spotify token lock was poisoned".to_owned()))?;

        Ok(token_store
            .as_ref()
            .is_some_and(|token| !token.is_expired()))
    }

    /// Creates a callback listener bound to the configured local redirect.
    fn bind_local_callback(&self) -> Result<CallbackListener, AuthError> {
        CallbackListener::bind(self.redirect.clone())
    }

    /// Ensures the token cache parent directory exists before write attempts.
    fn ensure_cache_parent_dir(&self) -> Result<(), AuthError> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }

        Ok(())
    }

    /// Ensures the token cache exists with user-only permissions before token writes.
    fn ensure_cache_file_permissions(&self) -> Result<(), AuthError> {
        self.ensure_cache_parent_dir()?;
        let _file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.cache_path)?;
        self.harden_cache_file_permissions()
    }

    /// Restricts token cache access to the current user on Unix platforms.
    fn harden_cache_file_permissions(&self) -> Result<(), AuthError> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::set_permissions(&self.cache_path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(())
    }

    /// Restores the existing refresh token when Spotify omits one in a refresh response.
    fn preserve_refresh_token_if_missing(
        &self,
        refresh_token: Option<String>,
    ) -> Result<(), AuthError> {
        let Some(refresh_token) = refresh_token else {
            return Ok(());
        };
        let token_store = self.client.get_token();
        let mut token_store = token_store
            .lock()
            .map_err(|_| AuthError::Spotify("Spotify token lock was poisoned".to_owned()))?;

        if let Some(token) = token_store.as_mut()
            && token.refresh_token.is_none()
        {
            token.refresh_token = Some(refresh_token);
            token.write_cache(&self.cache_path).map_err(|err| {
                AuthError::Spotify(format!("could not write Spotify token cache: {err}"))
            })?;
        }

        drop(token_store);
        self.harden_cache_file_permissions()
    }
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

    #[cfg(unix)]
    #[test]
    fn valid_cached_token_permissions_are_hardened_when_reused() {
        use std::os::unix::fs::PermissionsExt;

        let session = test_auth_session("valid-cache-permissions");
        fs::write(
            &session.cache_path,
            r#"{
                "access_token": "cached-access-token",
                "expires_in": 3600,
                "expires_at": "2999-01-01T00:00:00Z",
                "refresh_token": "cached-refresh-token",
                "scope": "user-read-private user-read-playback-state user-read-currently-playing user-modify-playback-state user-library-read playlist-read-private playlist-read-collaborative"
            }"#,
        )
        .expect("token cache should be writable");
        fs::set_permissions(&session.cache_path, fs::Permissions::from_mode(0o644))
            .expect("test should loosen cache permissions");
        let mut events = Vec::new();

        let reused_cache = session
            .try_cached_token(&mut |event| events.push(event))
            .expect("valid cache should be reusable");

        let mode = fs::metadata(&session.cache_path)
            .expect("token cache metadata should be readable")
            .permissions()
            .mode()
            & 0o777;
        let _ = fs::remove_file(&session.cache_path);
        assert!(reused_cache);
        assert_eq!(mode, 0o600);
        assert!(matches!(events.as_slice(), [AuthEvent::Cached { .. }]));
    }

    #[test]
    fn missing_refresh_token_is_preserved_after_refresh_response() {
        let session = test_auth_session("preserve-refresh-token");
        session
            .replace_client_token(Some(Token {
                access_token: "refreshed-access-token".to_owned(),
                refresh_token: None,
                scopes: required_test_scopes(),
                ..Token::default()
            }))
            .expect("test token should be loaded");

        session
            .preserve_refresh_token_if_missing(Some("cached-refresh-token".to_owned()))
            .expect("refresh token should be restored");

        let token_store = session.client.get_token();
        let token_store = token_store
            .lock()
            .expect("test token lock should not poison");
        assert_eq!(
            token_store
                .as_ref()
                .and_then(|token| token.refresh_token.as_deref()),
            Some("cached-refresh-token")
        );
        drop(token_store);

        let cached_token =
            Token::from_cache(&session.cache_path).expect("token cache should be readable");
        let _ = fs::remove_file(&session.cache_path);
        assert_eq!(
            cached_token.refresh_token.as_deref(),
            Some("cached-refresh-token")
        );
    }
}
