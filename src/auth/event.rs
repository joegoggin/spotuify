use std::path::PathBuf;

/// Status updates emitted while the Spotify auth flow runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthEvent {
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
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Cached { .. }
                | Self::RefreshedCachedToken { .. }
                | Self::Completed { .. }
                | Self::Failed { .. }
        )
    }
}
