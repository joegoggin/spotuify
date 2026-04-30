use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use tuirealm::application::{Application, PollStrategy};
use tuirealm::event::NoUserEvent;
use tuirealm::listener::EventListenerCfg;
use tuirealm::props::{AttrValue, Attribute, PropBound, PropPayload};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalResult};

use crate::AppResult;
use crate::auth::{self, AuthEvent};
use crate::config::ConfigBootstrap;
use crate::ui::Id;
use crate::ui::screens::auth::AUTH_EVENT_ATTR;
use crate::ui::screens::fatal::FATAL_ERROR_ATTR;
use crate::ui::screens::{AuthScreen, FatalScreen, HomeScreen, SetupScreen};

use super::action::Action;
use super::fatal::fatal_from_auth_event;
use super::msg::Msg;
use super::router::{Screen, ScreenTransition};
use super::state::AppState;

/// Poll interval used for keyboard input listener events.
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Tick interval used to emit periodic app events.
const TICK_INTERVAL: Duration = Duration::from_millis(250);
/// Frame poll interval for tui-realm update cycles.
const FRAME_INTERVAL: Duration = Duration::from_millis(50);

/// Runtime model tying tui-realm, terminal IO, and app state together.
pub struct Model {
    /// Mounted tui-realm application and component tree.
    app: Application<Id, Msg, NoUserEvent>,
    /// Mutable application state reduced from incoming actions.
    state: AppState,
    /// Terminal adapter used for rendering and raw-mode lifecycle.
    terminal: CrosstermTerminalAdapter,
    /// Auth worker channel when auth is currently active.
    auth_rx: Option<Receiver<AuthEvent>>,
}

impl Model {
    /// Creates a runtime model and initializes the terminal.
    pub fn new() -> AppResult<Self> {
        let config = ConfigBootstrap::load();
        let state = AppState::new(config);
        let app = Self::init_app(&state)?;
        let terminal = Self::init_terminal()?;
        let auth_rx = match &state.config {
            ConfigBootstrap::Ready { config, .. } => Some(auth::spawn_auth_flow(config.clone())),
            ConfigBootstrap::NeedsSetup { .. } => None,
        };

        Ok(Self {
            app,
            state,
            terminal,
            auth_rx,
        })
    }

    /// Drives the app event loop until the user quits.
    pub fn run(&mut self) -> AppResult<()> {
        while !self.state.should_quit {
            for msg in self.app.tick(PollStrategy::Once(FRAME_INTERVAL))? {
                self.update(msg)?;
            }
            self.drain_auth_events()?;

            if self.state.needs_redraw {
                self.view()?;
                self.dispatch(Action::Rendered)?;
            }
        }

        Ok(())
    }

    /// Mounts and activates the tui-realm components.
    fn init_app(state: &AppState) -> AppResult<Application<Id, Msg, NoUserEvent>> {
        let mut app = Application::init(
            EventListenerCfg::default()
                .crossterm_input_listener(INPUT_POLL_INTERVAL, 3)
                .tick_interval(TICK_INTERVAL),
        );

        app.mount(Id::Home, Box::<HomeScreen>::default(), Vec::new())?;
        app.mount(
            Id::Setup,
            Box::new(SetupScreen::from_config(&state.config)),
            Vec::new(),
        )?;
        app.mount(
            Id::Auth,
            Box::new(AuthScreen::from_config(&state.config)),
            Vec::new(),
        )?;
        app.mount(Id::FatalError, Box::<FatalScreen>::default(), Vec::new())?;
        app.active(&Id::from_screen(state.router.current()))?;

        Ok(app)
    }

    /// Initializes the terminal adapter for interactive rendering.
    fn init_terminal() -> TerminalResult<CrosstermTerminalAdapter> {
        let mut terminal = CrosstermTerminalAdapter::new()?;
        terminal.enable_raw_mode()?;
        terminal.enter_alternate_screen()?;
        terminal.clear_screen()?;

        Ok(terminal)
    }

    /// Dispatches a component message through the central update pipeline.
    fn update(&mut self, msg: Msg) -> AppResult<()> {
        self.dispatch(Action::from(msg))
    }

    /// Drains any pending Spotify auth status updates from the worker thread.
    fn drain_auth_events(&mut self) -> AppResult<()> {
        let mut events = Vec::new();
        let mut clear_receiver = false;

        if let Some(receiver) = &self.auth_rx {
            loop {
                match receiver.try_recv() {
                    Ok(event) => {
                        if event.is_terminal() {
                            clear_receiver = true;
                        }
                        events.push(event);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        clear_receiver = true;
                        break;
                    }
                }
            }
        }

        for event in events {
            self.sync_auth_event(event.clone())?;
            self.dispatch(action_for_auth_event(&event))?;
        }

        if clear_receiver {
            self.auth_rx = None;
        }

        Ok(())
    }

    /// Applies an action to the app state.
    fn dispatch(&mut self, action: Action) -> AppResult<()> {
        if let Action::Fatal(error) = &action {
            self.sync_fatal_error(error.clone())?;
        }

        self.state.apply(action);
        self.activate_current_screen()?;

        Ok(())
    }

    /// Draws the current UI state.
    fn view(&mut self) -> AppResult<()> {
        let active_id = Id::from_screen(self.state.router.current());
        self.app.active(&active_id)?;
        self.terminal
            .draw(|frame| self.app.view(&active_id, frame, frame.area()))
            .map(|_| ())?;

        Ok(())
    }

    /// Activates the mounted component that matches the current route.
    fn activate_current_screen(&mut self) -> AppResult<()> {
        self.app
            .active(&Id::from_screen(self.state.router.current()))?;
        Ok(())
    }

    /// Sends an auth worker event into the auth screen component.
    fn sync_auth_event(&mut self, event: AuthEvent) -> AppResult<()> {
        self.app.attr(
            &Id::Auth,
            Attribute::Custom(AUTH_EVENT_ATTR),
            AttrValue::Payload(PropPayload::Any(event.to_any_prop())),
        )?;

        Ok(())
    }

    /// Sends a fatal error into the fatal screen component.
    fn sync_fatal_error(&mut self, error: super::fatal::FatalError) -> AppResult<()> {
        self.app.attr(
            &Id::FatalError,
            Attribute::Custom(FATAL_ERROR_ATTR),
            AttrValue::Payload(PropPayload::Any(error.to_any_prop())),
        )?;

        Ok(())
    }
}

/// Maps auth worker events into app-level state transitions.
fn action_for_auth_event(event: &AuthEvent) -> Action {
    if let Some(fatal) = fatal_from_auth_event(event) {
        Action::Fatal(fatal)
    } else if matches!(
        event,
        AuthEvent::Cached { .. }
            | AuthEvent::RefreshedCachedToken { .. }
            | AuthEvent::Completed { .. }
    ) {
        Action::Navigate(ScreenTransition::Replace(Screen::Home))
    } else {
        Action::RequestRedraw
    }
}
