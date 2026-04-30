//! Spotify API primitives used by the terminal app.

/// Error model used by Spotify service and retry layers.
pub(crate) mod error;
/// Retry helpers for auth-expired and transient Spotify failures.
mod retry;
/// High-level Spotify request helpers built on top of `rspotify`.
pub(crate) mod service;
