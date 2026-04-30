//! Terminal UI layer built on tui-realm components.
//!
//! This module maps logical app screens to concrete component identifiers and
//! exposes the screen submodules used by the runtime model.

pub(crate) mod screens;

use crate::app::router::Screen;

/// Component identifiers mounted in the tui-realm application.
#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub(crate) enum Id {
    /// Home screen component.
    Home,
    /// First-run setup screen component.
    Setup,
    /// Spotify authentication screen component.
    Auth,
    /// Fatal error screen component.
    FatalError,
}

impl Id {
    /// Returns the component identifier for a logical screen route.
    pub(crate) fn from_screen(screen: Screen) -> Self {
        match screen {
            Screen::Home => Self::Home,
            Screen::Setup => Self::Setup,
            Screen::Auth => Self::Auth,
            Screen::FatalError => Self::FatalError,
        }
    }
}
