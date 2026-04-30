//! Application runtime orchestration.
//!
//! This module owns state transitions, screen routing, and the event loop model
//! that bridges terminal IO with tui-realm components.

/// State transition commands produced by app messages.
pub mod action;
/// Fatal error model and adapters from terminal auth failures.
pub mod fatal;
/// Core event loop model, terminal setup, and component synchronization.
mod model;
/// Message types emitted by UI components.
pub mod msg;
/// Screen routing primitives and navigation state.
pub mod router;
/// Mutable app state and reducer-style action application.
pub mod state;

/// Main runtime model used to start and drive the application.
pub use model::Model;
