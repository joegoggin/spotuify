use super::fatal::FatalError;
use super::msg::Msg;
use super::router::ScreenTransition;

/// Actions handled by the central state dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Mark the app as ready to shut down.
    Quit,
    /// Advance time-dependent shell state.
    Tick,
    /// Store the latest terminal size.
    Resize { width: u16, height: u16 },
    /// Apply a screen navigation transition.
    Navigate(ScreenTransition),
    /// Surface an unrecoverable error and route to the fatal error screen.
    Fatal(FatalError),
    /// Mark the UI as needing a redraw.
    RequestRedraw,
    /// Mark the current draw request as handled.
    Rendered,
}

impl From<Msg> for Action {
    fn from(msg: Msg) -> Self {
        match msg {
            Msg::Quit => Self::Quit,
            Msg::Tick => Self::Tick,
            Msg::WindowResize { width, height } => Self::Resize { width, height },
            Msg::Navigate(transition) => Self::Navigate(transition),
            Msg::Fatal(error) => Self::Fatal(error),
        }
    }
}
