//! Screen-specific tui-realm components.

/// Spotify auth status screen and event rendering.
pub(crate) mod auth;
/// Shared per-screen event handling and frame metadata helpers.
mod common;
/// Fatal error screen for unrecoverable application failures.
pub(crate) mod fatal;
/// Default landing screen.
pub(crate) mod home;
/// Setup screen shown when config bootstrap is incomplete.
pub(crate) mod setup;

pub(crate) use auth::AuthScreen;
pub(crate) use fatal::FatalScreen;
pub(crate) use home::HomeScreen;
pub(crate) use setup::SetupScreen;
