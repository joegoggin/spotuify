//! Spotify PKCE authentication orchestration.
//!
//! This module starts a background worker that emits
//! [`AuthEvent`](crate::auth::AuthEvent) values
//! while it validates cached credentials, opens browser auth, waits for the
//! local redirect callback, and exchanges auth codes for tokens.

/// Local HTTP callback listener for Spotify redirect handling.
mod callback;
/// Errors surfaced by the authentication pipeline.
pub mod error;
/// Worker status events consumed by the UI.
pub mod event;
/// Redirect URI parsing and callback URL validation helpers.
mod redirect;
/// Auth session lifecycle and Spotify client/token interactions.
pub mod session;

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::config::AppConfig;

pub use event::AuthEvent;

use error::AuthError;
use session::AuthSession;

/// Starts the Spotify auth flow on a background thread.
///
/// # Examples
///
/// ```no_run
/// use spotuify::auth::spawn_auth_flow;
/// use spotuify::config::{AppConfig, SpotifyConfig};
///
/// let rx = spawn_auth_flow(AppConfig {
///     spotify: SpotifyConfig {
///         client_id: Some("spotify-client-id".to_owned()),
///         redirect_uri: Some("http://127.0.0.1:8888/callback".to_owned()),
///     },
/// });
///
/// let _ = rx; // consumed by the app event loop in real usage.
/// ```
pub fn spawn_auth_flow(config: AppConfig) -> Receiver<AuthEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        run_auth_flow(config, &sender);
    });

    receiver
}

/// Sends startup and terminal auth events for a single worker execution.
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

/// Builds an auth session and runs the local-callback completion flow.
fn run_auth_flow_inner(config: AppConfig, sender: &Sender<AuthEvent>) -> Result<(), AuthError> {
    let mut session = AuthSession::from_config(&config)?;
    session.complete_with_local_callback(|event| send_event(sender, event))
}

/// Best-effort event send that ignores receiver disconnects.
fn send_event(sender: &Sender<AuthEvent>, event: AuthEvent) {
    let _ = sender.send(event);
}
