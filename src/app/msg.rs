use super::fatal::FatalError;
use super::router::ScreenTransition;

/// Messages emitted by components and converted into app-level actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Request application shutdown.
    Quit,
    /// Record a UI tick.
    Tick,
    /// Record the latest terminal size.
    WindowResize { width: u16, height: u16 },
    /// Request a screen navigation transition.
    Navigate(ScreenTransition),
    /// Surface an unrecoverable error and route to the fatal error screen.
    Fatal(FatalError),
}
