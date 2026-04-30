/// Logical screens that can be reached through app navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    /// Default app entry screen.
    Home,
    /// First-run setup and config validation screen.
    Setup,
    /// Spotify authentication screen.
    Auth,
    /// Fatal error recovery screen.
    FatalError,
}

impl Screen {
    /// Returns the display label for a screen route.
    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Setup => "Setup",
            Self::Auth => "Auth",
            Self::FatalError => "Fatal Error",
        }
    }
}

/// Navigation transitions supported by the app router.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenTransition {
    /// Add a screen to the top of the navigation stack.
    Push(Screen),
    /// Replace the current screen with another screen.
    Replace(Screen),
    /// Return to the previous screen when history exists.
    Back,
}

/// Screen navigation state for the running application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Router {
    /// Screen history stack, with the active screen at the end.
    pub stack: Vec<Screen>,
}

impl Router {
    /// Creates router state with the default entry screen.
    pub fn new() -> Self {
        Self::with_initial_screen(Screen::Home)
    }

    /// Creates router state with a specific entry screen.
    pub fn with_initial_screen(screen: Screen) -> Self {
        Self {
            stack: vec![screen],
        }
    }

    /// Returns the active screen.
    pub fn current(&self) -> Screen {
        self.stack.last().copied().unwrap_or(Screen::Home)
    }

    /// Applies a transition and reports whether the stack changed.
    pub fn apply(&mut self, transition: ScreenTransition) -> bool {
        match transition {
            ScreenTransition::Push(screen) if self.current() != screen => {
                self.stack.push(screen);
                true
            }
            ScreenTransition::Push(_) => false,
            ScreenTransition::Replace(screen) if self.current() != screen => {
                if let Some(current) = self.stack.last_mut() {
                    *current = screen;
                } else {
                    self.stack.push(screen);
                }

                true
            }
            ScreenTransition::Replace(_) => false,
            ScreenTransition::Back if self.stack.len() > 1 => {
                self.stack.pop();
                true
            }
            ScreenTransition::Back => false,
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}
