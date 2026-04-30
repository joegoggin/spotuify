//! `spotuify` is a terminal UI Spotify client with local OAuth callback auth.
//!
//! The crate is organized around a small runtime model:
//! - [`app`] drives the event loop and routing.
//! - [`auth`] manages Spotify PKCE authentication and token caching.
//! - [`spotify`] provides retry-aware Spotify API helpers.
//! - [`ui`] renders screen components with tui-realm.
//!
//! Typical entrypoints:
//! - Binary startup via `main` (see `src/main.rs`).
//! - Programmatic runtime startup via [`app::Model`].

use std::error::Error;

/// Application runtime and state management.
pub mod app;
/// Spotify authentication flow and status events.
pub mod auth;
/// User config loading and bootstrap validation.
pub mod config;
#[allow(dead_code)]
/// Spotify API service primitives and retry policies.
pub mod spotify;
/// Terminal screen components and IDs.
pub mod ui;

/// Shared fallible result type used across the crate.
pub type AppResult<T> = Result<T, Box<dyn Error>>;
