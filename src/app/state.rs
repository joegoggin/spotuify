use crate::config::ConfigBootstrap;

use super::action::Action;
use super::router::{Router, Screen, ScreenTransition};

/// Shared state for the running application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    /// Whether the main event loop should exit.
    pub should_quit: bool,
    /// Whether the terminal should be redrawn.
    pub needs_redraw: bool,
    /// Current screen route and navigation history.
    pub router: Router,
    /// Config bootstrap state loaded at startup.
    pub config: ConfigBootstrap,
}

impl AppState {
    /// Creates the initial app state with the first draw requested.
    pub fn new(config: ConfigBootstrap) -> Self {
        let initial_screen = if config.is_ready() {
            Screen::Auth
        } else {
            Screen::Setup
        };
        let mut state = Self {
            router: Router::with_initial_screen(initial_screen),
            config,
            ..Self::default()
        };
        state.apply(Action::RequestRedraw);
        state
    }

    /// Applies an action to the app state.
    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => {
                self.should_quit = true;
                self.needs_redraw = true;
            }
            Action::Tick => {
                self.needs_redraw = true;
            }
            Action::Resize { .. } => {
                self.needs_redraw = true;
            }
            Action::Navigate(transition) => {
                if self.router.apply(transition) {
                    self.needs_redraw = true;
                }
            }
            Action::Fatal(_) => {
                self.router
                    .apply(ScreenTransition::Replace(Screen::FatalError));
                self.needs_redraw = true;
            }
            Action::RequestRedraw => {
                self.needs_redraw = true;
            }
            Action::Rendered => {
                self.needs_redraw = false;
            }
        }
    }
}
