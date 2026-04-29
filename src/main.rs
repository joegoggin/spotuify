use std::error::Error;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use auth::AuthEvent;
use config::ConfigBootstrap;
use tuirealm::application::{Application, PollStrategy};
use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::listener::EventListenerCfg;
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Style};
use tuirealm::ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tuirealm::state::State;
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter, TerminalResult};

mod auth;
mod config;
#[allow(dead_code)]
mod spotify;

type AppResult<T> = Result<T, Box<dyn Error>>;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TICK_INTERVAL: Duration = Duration::from_millis(250);
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const SHELL_TICKS_ATTR: &str = "shell.ticks";
const SHELL_TERMINAL_WIDTH_ATTR: &str = "shell.terminal_width";
const SHELL_TERMINAL_HEIGHT_ATTR: &str = "shell.terminal_height";
const SHELL_ACTIVE_SCREEN_ATTR: &str = "shell.active_screen";
const SHELL_CONFIG_STATUS_ATTR: &str = "shell.config_status";
const SHELL_AUTH_STATUS_ATTR: &str = "shell.auth_status";
const SHELL_AUTH_URL_ATTR: &str = "shell.auth_url";

fn main() {
    if let Err(err) = run() {
        eprintln!("spotuify: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let mut model = Model::new()?;

    while !model.state.should_quit {
        for msg in model.app.tick(PollStrategy::Once(FRAME_INTERVAL))? {
            model.update(msg);
        }
        model.drain_auth_events();

        if model.state.needs_redraw {
            model.view()?;
            model.dispatch(Action::Rendered);
        }
    }

    Ok(())
}

/// Component identifiers mounted in the tui-realm application.
#[derive(Debug, Eq, PartialEq, Clone, Hash)]
enum Id {
    /// Top-level shell component.
    Shell,
}

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
    fn label(self) -> &'static str {
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

/// Messages emitted by components and converted into app-level actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Msg {
    /// Request application shutdown.
    Quit,
    /// Record a UI tick.
    Tick,
    /// Record the latest terminal size.
    WindowResize { width: u16, height: u16 },
    /// Request a screen navigation transition.
    Navigate(ScreenTransition),
}

/// Actions handled by the central state dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// Mark the app as ready to shut down.
    Quit,
    /// Advance time-dependent shell state.
    Tick,
    /// Store the latest terminal size.
    Resize { width: u16, height: u16 },
    /// Apply a screen navigation transition.
    Navigate(ScreenTransition),
    /// Apply a Spotify auth flow status update.
    AuthEvent(AuthEvent),
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
        }
    }
}

/// Screen navigation state for the running application.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Router {
    /// Screen history stack, with the active screen at the end.
    stack: Vec<Screen>,
}

impl Router {
    /// Creates router state with the default entry screen.
    fn new() -> Self {
        Self::with_initial_screen(Screen::Home)
    }

    /// Creates router state with a specific entry screen.
    fn with_initial_screen(screen: Screen) -> Self {
        Self {
            stack: vec![screen],
        }
    }

    /// Returns the active screen.
    fn current(&self) -> Screen {
        self.stack.last().copied().unwrap_or(Screen::Home)
    }

    /// Applies a transition and reports whether the stack changed.
    fn apply(&mut self, transition: ScreenTransition) -> bool {
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

/// Shared state for the running application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct AppState {
    /// Whether the main event loop should exit.
    should_quit: bool,
    /// Whether the terminal should be redrawn.
    needs_redraw: bool,
    /// Current screen route and navigation history.
    router: Router,
    /// Config bootstrap state loaded at startup.
    config: ConfigBootstrap,
    /// Spotify auth flow state for the running app.
    auth: AuthUiState,
    /// State rendered by the shell component.
    shell: ShellState,
}

impl AppState {
    /// Creates the initial app state with the first draw requested.
    fn new(config: ConfigBootstrap) -> Self {
        let initial_screen = if config.is_ready() {
            Screen::Auth
        } else {
            Screen::Setup
        };
        let auth = AuthUiState::from_config(&config);
        let mut state = Self {
            router: Router::with_initial_screen(initial_screen),
            auth,
            config,
            ..Self::default()
        };
        state.apply(Action::RequestRedraw);
        state
    }

    /// Applies an action to the app state.
    fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => {
                self.should_quit = true;
                self.needs_redraw = true;
            }
            Action::Tick => {
                self.shell.ticks = self.shell.ticks.saturating_add(1);
                self.needs_redraw = true;
            }
            Action::Resize { width, height } => {
                self.shell.terminal_size = Some((width, height));
                self.needs_redraw = true;
            }
            Action::Navigate(transition) => {
                if self.router.apply(transition) {
                    self.needs_redraw = true;
                }
            }
            Action::AuthEvent(event) => {
                self.auth.apply(event);
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

/// User-facing auth state rendered by the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AuthUiState {
    /// Current auth status label.
    status: String,
    /// Authorization URL shown when browser launch fails or manual copy is needed.
    authorize_url: Option<String>,
    /// Browser launch failure retained while the callback listener waits.
    browser_open_error: Option<String>,
}

impl AuthUiState {
    /// Creates initial auth UI state from startup config readiness.
    fn from_config(config: &ConfigBootstrap) -> Self {
        if config.is_ready() {
            Self {
                status: "Spotify auth starting".to_owned(),
                ..Self::default()
            }
        } else {
            Self {
                status: "Spotify auth waiting for setup".to_owned(),
                ..Self::default()
            }
        }
    }

    /// Applies an auth worker event to the rendered status.
    fn apply(&mut self, event: AuthEvent) {
        match event {
            AuthEvent::Starting => {
                self.status = "Spotify auth starting".to_owned();
                self.authorize_url = None;
                self.browser_open_error = None;
            }
            AuthEvent::Cached { cache_path } => {
                self.status = format!(
                    "Spotify session loaded from cached token at {}",
                    cache_path.display()
                );
                self.authorize_url = None;
                self.browser_open_error = None;
            }
            AuthEvent::RefreshingCachedToken { cache_path } => {
                self.status = format!(
                    "Spotify cached token at {} expired; refreshing session",
                    cache_path.display()
                );
                self.authorize_url = None;
                self.browser_open_error = None;
            }
            AuthEvent::RefreshedCachedToken { cache_path } => {
                self.status = format!(
                    "Spotify session refreshed and saved at {}",
                    cache_path.display()
                );
                self.authorize_url = None;
                self.browser_open_error = None;
            }
            AuthEvent::ReauthorizationRequired { message } => {
                self.status = message;
                self.authorize_url = None;
                self.browser_open_error = None;
            }
            AuthEvent::AuthorizationUrl { url, callback_addr } => {
                self.status =
                    format!("Spotify authorization URL generated; waiting on {callback_addr}");
                self.authorize_url = Some(url);
            }
            AuthEvent::BrowserOpenFailed { message } => {
                self.status = format!(
                    "Could not open browser automatically; copy the auth URL below ({message})"
                );
                self.browser_open_error = Some(message);
            }
            AuthEvent::WaitingForCallback { callback_addr } => {
                self.status = if self.browser_open_error.is_some() {
                    format!(
                        "Waiting for Spotify callback on {callback_addr}; browser did not open automatically"
                    )
                } else {
                    format!("Waiting for Spotify callback on {callback_addr}")
                };
            }
            AuthEvent::ExchangingCode => {
                self.status = "Spotify callback received; exchanging code for tokens".to_owned();
            }
            AuthEvent::Completed { cache_path } => {
                self.status = format!(
                    "Spotify session authenticated and saved at {}",
                    cache_path.display()
                );
                self.authorize_url = None;
                self.browser_open_error = None;
            }
            AuthEvent::Failed { message } => {
                self.status = format!("Spotify auth failed: {message}");
                self.browser_open_error = None;
            }
        }
    }
}

impl Default for AuthUiState {
    fn default() -> Self {
        Self {
            status: "Spotify auth not started".to_owned(),
            authorize_url: None,
            browser_open_error: None,
        }
    }
}

/// Shell-specific data owned by the app state.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellState {
    /// Number of tick events processed by the app.
    ticks: u64,
    /// Most recent terminal size observed from resize events.
    terminal_size: Option<(u16, u16)>,
    /// Active screen label rendered by the shell.
    active_screen: String,
    /// User-facing config bootstrap status rendered by the shell.
    config_status: String,
    /// User-facing Spotify auth status rendered by the shell.
    auth_status: String,
    /// Authorization URL rendered when user action may be required.
    auth_url: Option<String>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            ticks: 0,
            terminal_size: None,
            active_screen: Screen::Home.label().to_owned(),
            config_status: "config not checked".to_owned(),
            auth_status: "Spotify auth not started".to_owned(),
            auth_url: None,
        }
    }
}

/// Runtime model tying tui-realm, terminal IO, and app state together.
struct Model {
    app: Application<Id, Msg, NoUserEvent>,
    state: AppState,
    terminal: CrosstermTerminalAdapter,
    auth_rx: Option<Receiver<AuthEvent>>,
}

impl Model {
    /// Creates a runtime model and initializes the terminal.
    fn new() -> AppResult<Self> {
        let config = ConfigBootstrap::load();
        let auth_rx = match &config {
            ConfigBootstrap::Ready { config, .. } => Some(auth::spawn_auth_flow(config.clone())),
            ConfigBootstrap::NeedsSetup { .. } => None,
        };

        Ok(Self {
            app: Self::init_app()?,
            state: AppState::new(config),
            terminal: Self::init_terminal()?,
            auth_rx,
        })
    }

    /// Mounts and activates the tui-realm components.
    fn init_app() -> AppResult<Application<Id, Msg, NoUserEvent>> {
        let mut app = Application::init(
            EventListenerCfg::default()
                .crossterm_input_listener(INPUT_POLL_INTERVAL, 3)
                .tick_interval(TICK_INTERVAL),
        );

        app.mount(Id::Shell, Box::<Shell>::default(), Vec::new())?;
        app.active(&Id::Shell)?;

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
    fn update(&mut self, msg: Msg) {
        self.dispatch(Action::from(msg));
    }

    /// Drains any pending Spotify auth status updates from the worker thread.
    fn drain_auth_events(&mut self) {
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
            self.dispatch(Action::AuthEvent(event));
        }

        if clear_receiver {
            self.auth_rx = None;
        }
    }

    /// Applies an action to the app state.
    fn dispatch(&mut self, action: Action) {
        self.state.apply(action);
    }

    /// Draws the current UI state.
    fn view(&mut self) -> AppResult<()> {
        self.sync_shell_state()?;
        self.terminal
            .draw(|frame| self.app.view(&Id::Shell, frame, frame.area()))
            .map(|_| ())?;

        Ok(())
    }

    /// Copies shell state into the shell component's render snapshot.
    fn sync_shell_state(&mut self) -> AppResult<()> {
        let ticks = self.state.shell.ticks.min(isize::MAX as u64) as isize;

        self.app.attr(
            &Id::Shell,
            Attribute::Custom(SHELL_TICKS_ATTR),
            AttrValue::Number(ticks),
        )?;

        if let Some((width, height)) = self.state.shell.terminal_size {
            self.app.attr(
                &Id::Shell,
                Attribute::Custom(SHELL_TERMINAL_WIDTH_ATTR),
                AttrValue::Size(width),
            )?;
            self.app.attr(
                &Id::Shell,
                Attribute::Custom(SHELL_TERMINAL_HEIGHT_ATTR),
                AttrValue::Size(height),
            )?;
        }

        self.app.attr(
            &Id::Shell,
            Attribute::Custom(SHELL_ACTIVE_SCREEN_ATTR),
            AttrValue::String(self.state.router.current().label().to_owned()),
        )?;
        self.app.attr(
            &Id::Shell,
            Attribute::Custom(SHELL_CONFIG_STATUS_ATTR),
            AttrValue::String(self.state.config.status_label()),
        )?;
        self.app.attr(
            &Id::Shell,
            Attribute::Custom(SHELL_AUTH_STATUS_ATTR),
            AttrValue::String(self.state.auth.status.clone()),
        )?;
        self.app.attr(
            &Id::Shell,
            Attribute::Custom(SHELL_AUTH_URL_ATTR),
            AttrValue::String(self.state.auth.authorize_url.clone().unwrap_or_default()),
        )?;

        Ok(())
    }
}

/// Top-level shell component.
#[derive(Default)]
struct Shell {
    render_state: ShellState,
}

impl Component for Shell {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let size = self
            .render_state
            .terminal_size
            .map(|(width, height)| format!("{width}x{height}"))
            .unwrap_or_else(|| format!("{}x{}", area.width, area.height));
        let mut text = format!(
            "App shell running\n\nScreen: {}\nConfig: {}\nAuth: {}\n\nEvent loop: active\nDraw cycle: active\nTerminal: raw mode + alternate screen\nSize: {size}\nTicks: {}",
            self.render_state.active_screen,
            self.render_state.config_status,
            self.render_state.auth_status,
            self.render_state.ticks
        );
        if let Some(auth_url) = self.render_state.auth_url.as_deref()
            && !auth_url.is_empty()
        {
            text.push_str("\n\nSpotify auth URL:\n");
            text.push_str(auth_url);
        }
        text.push_str("\n\nPress q, Esc, or Ctrl-C to quit.");

        let widget = Paragraph::new(text)
            .block(
                Block::default()
                    .title(" spotuify ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .wrap(Wrap { trim: true });

        frame.render_widget(widget, area);
    }

    fn query<'a>(&'a self, _attr: Attribute) -> Option<QueryResult<'a>> {
        None
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        match (attr, value) {
            (Attribute::Custom(SHELL_TICKS_ATTR), AttrValue::Number(ticks)) if ticks >= 0 => {
                self.render_state.ticks = ticks as u64;
            }
            (Attribute::Custom(SHELL_TERMINAL_WIDTH_ATTR), AttrValue::Size(width)) => {
                let (_, height) = self.render_state.terminal_size.unwrap_or_default();
                self.render_state.terminal_size = Some((width, height));
            }
            (Attribute::Custom(SHELL_TERMINAL_HEIGHT_ATTR), AttrValue::Size(height)) => {
                let (width, _) = self.render_state.terminal_size.unwrap_or_default();
                self.render_state.terminal_size = Some((width, height));
            }
            (Attribute::Custom(SHELL_ACTIVE_SCREEN_ATTR), AttrValue::String(active_screen)) => {
                self.render_state.active_screen = active_screen;
            }
            (Attribute::Custom(SHELL_CONFIG_STATUS_ATTR), AttrValue::String(config_status)) => {
                self.render_state.config_status = config_status;
            }
            (Attribute::Custom(SHELL_AUTH_STATUS_ATTR), AttrValue::String(auth_status)) => {
                self.render_state.auth_status = auth_status;
            }
            (Attribute::Custom(SHELL_AUTH_URL_ATTR), AttrValue::String(auth_url)) => {
                self.render_state.auth_url = if auth_url.is_empty() {
                    None
                } else {
                    Some(auth_url)
                };
            }
            _ => {}
        }
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        CmdResult::Invalid(cmd)
    }
}

impl AppComponent<Msg, NoUserEvent> for Shell {
    fn on(&mut self, event: &Event<NoUserEvent>) -> Option<Msg> {
        match event {
            Event::Keyboard(KeyEvent { code: Key::Esc, .. })
            | Event::Keyboard(KeyEvent {
                code: Key::Char('q'),
                ..
            })
            | Event::Keyboard(KeyEvent {
                code: Key::Char('Q'),
                ..
            }) => Some(Msg::Quit),
            Event::Keyboard(KeyEvent {
                code: Key::Char('c'),
                modifiers,
            }) if modifiers.contains(KeyModifiers::CONTROL) => Some(Msg::Quit),
            Event::WindowResize(width, height) => Some(Msg::WindowResize {
                width: *width,
                height: *height,
            }),
            Event::Tick => Some(Msg::Tick),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    use crate::config::{AppConfig, ConfigIssue, SpotifyConfig};

    fn valid_config_bootstrap() -> ConfigBootstrap {
        ConfigBootstrap::Ready {
            path: PathBuf::from("/tmp/spotuify/config.toml"),
            config: AppConfig {
                spotify: SpotifyConfig {
                    client_id: Some("client-id".to_owned()),
                    redirect_uri: Some("http://127.0.0.1:8888/callback".to_owned()),
                },
            },
        }
    }

    fn missing_config_bootstrap() -> ConfigBootstrap {
        ConfigBootstrap::NeedsSetup {
            path: Some(PathBuf::from("/tmp/spotuify/config.toml")),
            issue: ConfigIssue::MissingFile,
        }
    }

    #[test]
    fn app_state_starts_with_setup_screen_for_missing_config() {
        let bootstrap = missing_config_bootstrap();

        let state = AppState::new(bootstrap.clone());

        assert!(!state.should_quit);
        assert!(state.needs_redraw);
        assert_eq!(state.router.current(), Screen::Setup);
        assert_eq!(state.router.stack, vec![Screen::Setup]);
        assert_eq!(state.config, bootstrap);
        assert_eq!(state.shell, ShellState::default());
    }

    #[test]
    fn app_state_starts_with_auth_screen_for_valid_config() {
        let bootstrap = valid_config_bootstrap();

        let state = AppState::new(bootstrap.clone());

        assert!(!state.should_quit);
        assert!(state.needs_redraw);
        assert_eq!(state.router.current(), Screen::Auth);
        assert_eq!(state.router.stack, vec![Screen::Auth]);
        assert_eq!(state.config, bootstrap);
        assert_eq!(state.shell, ShellState::default());
    }

    #[test]
    fn dispatcher_pushes_new_screen_and_requests_redraw() {
        let mut state = AppState::default();

        state.apply(Action::Navigate(ScreenTransition::Push(Screen::Setup)));

        assert_eq!(state.router.stack, vec![Screen::Home, Screen::Setup]);
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_ignores_push_of_current_screen() {
        let mut state = AppState::default();

        state.apply(Action::Navigate(ScreenTransition::Push(Screen::Home)));

        assert_eq!(state.router.stack, vec![Screen::Home]);
        assert!(!state.needs_redraw);
    }

    #[test]
    fn dispatcher_replaces_current_screen_and_requests_redraw() {
        let mut state = AppState::default();

        state.apply(Action::Navigate(ScreenTransition::Replace(Screen::Auth)));

        assert_eq!(state.router.stack, vec![Screen::Auth]);
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_navigates_back_when_history_exists() {
        let mut state = AppState::default();
        state.apply(Action::Navigate(ScreenTransition::Push(Screen::Setup)));
        state.apply(Action::Rendered);

        state.apply(Action::Navigate(ScreenTransition::Back));

        assert_eq!(state.router.stack, vec![Screen::Home]);
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_ignores_back_at_root_screen() {
        let mut state = AppState::default();

        state.apply(Action::Navigate(ScreenTransition::Back));

        assert_eq!(state.router.stack, vec![Screen::Home]);
        assert!(!state.needs_redraw);
    }

    #[test]
    fn navigation_messages_convert_to_actions() {
        let transition = ScreenTransition::Replace(Screen::FatalError);

        assert_eq!(
            Action::from(Msg::Navigate(transition)),
            Action::Navigate(transition)
        );
    }

    #[test]
    fn auth_ui_state_renders_cached_token_refresh_events() {
        let mut state = AuthUiState::default();

        state.apply(AuthEvent::RefreshingCachedToken {
            cache_path: PathBuf::from("/tmp/spotuify/token.json"),
        });

        assert_eq!(
            state.status,
            "Spotify cached token at /tmp/spotuify/token.json expired; refreshing session"
        );
        assert_eq!(state.authorize_url, None);
        assert_eq!(state.browser_open_error, None);

        state.apply(AuthEvent::RefreshedCachedToken {
            cache_path: PathBuf::from("/tmp/spotuify/token.json"),
        });

        assert_eq!(
            state.status,
            "Spotify session refreshed and saved at /tmp/spotuify/token.json"
        );
        assert_eq!(state.authorize_url, None);
        assert_eq!(state.browser_open_error, None);
    }

    #[test]
    fn auth_ui_state_renders_reauthorization_required_message() {
        let mut state = AuthUiState {
            status: "old status".to_owned(),
            authorize_url: Some("https://accounts.spotify.com/authorize".to_owned()),
            browser_open_error: Some("browser error".to_owned()),
        };

        state.apply(AuthEvent::ReauthorizationRequired {
            message: "Spotify reauthorization required".to_owned(),
        });

        assert_eq!(state.status, "Spotify reauthorization required");
        assert_eq!(state.authorize_url, None);
        assert_eq!(state.browser_open_error, None);
    }

    #[test]
    fn dispatcher_marks_app_for_shutdown() {
        let mut state = AppState::default();

        state.apply(Action::Quit);

        assert!(state.should_quit);
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_tracks_ticks() {
        let mut state = AppState::default();

        state.apply(Action::Tick);
        state.apply(Action::Tick);

        assert_eq!(state.shell.ticks, 2);
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_saturates_tick_count() {
        let mut state = AppState {
            shell: ShellState {
                ticks: u64::MAX,
                ..ShellState::default()
            },
            ..AppState::default()
        };

        state.apply(Action::Tick);

        assert_eq!(state.shell.ticks, u64::MAX);
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_tracks_terminal_size() {
        let mut state = AppState::default();

        state.apply(Action::Resize {
            width: 120,
            height: 40,
        });

        assert_eq!(state.shell.terminal_size, Some((120, 40)));
        assert!(state.needs_redraw);
    }

    #[test]
    fn dispatcher_handles_redraw_lifecycle() {
        let mut state = AppState::default();

        state.apply(Action::RequestRedraw);
        assert!(state.needs_redraw);

        state.apply(Action::Rendered);
        assert!(!state.needs_redraw);
    }

    #[test]
    fn shell_tick_event_emits_message_without_mutating_render_state() {
        let mut shell = Shell::default();

        let msg = shell.on(&Event::Tick);

        assert_eq!(msg, Some(Msg::Tick));
        assert_eq!(shell.render_state, ShellState::default());
    }

    #[test]
    fn shell_resize_event_emits_message_without_mutating_render_state() {
        let mut shell = Shell::default();

        let msg = shell.on(&Event::WindowResize(120, 40));

        assert_eq!(
            msg,
            Some(Msg::WindowResize {
                width: 120,
                height: 40,
            })
        );
        assert_eq!(shell.render_state, ShellState::default());
    }

    #[test]
    fn shell_quit_events_emit_quit_message() {
        let mut shell = Shell::default();

        assert_eq!(
            shell.on(&Event::Keyboard(KeyEvent::from(Key::Esc))),
            Some(Msg::Quit)
        );
        assert_eq!(
            shell.on(&Event::Keyboard(KeyEvent::from(Key::Char('q')))),
            Some(Msg::Quit)
        );
        assert_eq!(
            shell.on(&Event::Keyboard(KeyEvent::new(
                Key::Char('c'),
                KeyModifiers::CONTROL,
            ))),
            Some(Msg::Quit)
        );
        assert_eq!(shell.render_state, ShellState::default());
    }

    #[test]
    fn shell_attrs_update_render_snapshot() {
        let mut shell = Shell::default();

        shell.attr(Attribute::Custom(SHELL_TICKS_ATTR), AttrValue::Number(42));
        shell.attr(
            Attribute::Custom(SHELL_TERMINAL_WIDTH_ATTR),
            AttrValue::Size(80),
        );
        shell.attr(
            Attribute::Custom(SHELL_TERMINAL_HEIGHT_ATTR),
            AttrValue::Size(24),
        );
        shell.attr(
            Attribute::Custom(SHELL_ACTIVE_SCREEN_ATTR),
            AttrValue::String("Setup".to_owned()),
        );
        shell.attr(
            Attribute::Custom(SHELL_CONFIG_STATUS_ATTR),
            AttrValue::String("missing config file; setup required".to_owned()),
        );
        shell.attr(
            Attribute::Custom(SHELL_AUTH_STATUS_ATTR),
            AttrValue::String("Spotify auth starting".to_owned()),
        );
        shell.attr(
            Attribute::Custom(SHELL_AUTH_URL_ATTR),
            AttrValue::String("http://127.0.0.1:8888/authorize".to_owned()),
        );

        assert_eq!(
            shell.render_state,
            ShellState {
                ticks: 42,
                terminal_size: Some((80, 24)),
                active_screen: "Setup".to_owned(),
                config_status: "missing config file; setup required".to_owned(),
                auth_status: "Spotify auth starting".to_owned(),
                auth_url: Some("http://127.0.0.1:8888/authorize".to_owned()),
            }
        );
    }
}
