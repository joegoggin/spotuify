use std::error::Error;
use std::time::Duration;

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

type AppResult<T> = Result<T, Box<dyn Error>>;

const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const TICK_INTERVAL: Duration = Duration::from_millis(250);
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const SHELL_TICKS_ATTR: &str = "shell.ticks";
const SHELL_TERMINAL_WIDTH_ATTR: &str = "shell.terminal_width";
const SHELL_TERMINAL_HEIGHT_ATTR: &str = "shell.terminal_height";

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    /// Mark the app as ready to shut down.
    Quit,
    /// Advance time-dependent shell state.
    Tick,
    /// Store the latest terminal size.
    Resize { width: u16, height: u16 },
    /// Apply a screen navigation transition.
    Navigate(ScreenTransition),
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
        Self {
            stack: vec![Screen::Home],
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
#[derive(Debug, Clone, PartialEq, Eq)]
struct AppState {
    /// Whether the main event loop should exit.
    should_quit: bool,
    /// Whether the terminal should be redrawn.
    needs_redraw: bool,
    /// Current screen route and navigation history.
    router: Router,
    /// State rendered by the shell component.
    shell: ShellState,
}

impl AppState {
    /// Creates the initial app state with the first draw requested.
    fn new() -> Self {
        let mut state = Self::default();
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
            Action::RequestRedraw => {
                self.needs_redraw = true;
            }
            Action::Rendered => {
                self.needs_redraw = false;
            }
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            should_quit: false,
            needs_redraw: false,
            router: Router::default(),
            shell: ShellState::default(),
        }
    }
}

/// Shell-specific data owned by the app state.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ShellState {
    /// Number of tick events processed by the app.
    ticks: u64,
    /// Most recent terminal size observed from resize events.
    terminal_size: Option<(u16, u16)>,
}

/// Runtime model tying tui-realm, terminal IO, and app state together.
struct Model {
    app: Application<Id, Msg, NoUserEvent>,
    state: AppState,
    terminal: CrosstermTerminalAdapter,
}

impl Model {
    /// Creates a runtime model and initializes the terminal.
    fn new() -> AppResult<Self> {
        Ok(Self {
            app: Self::init_app()?,
            state: AppState::new(),
            terminal: Self::init_terminal()?,
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
        let text = format!(
            "App shell running\n\nEvent loop: active\nDraw cycle: active\nTerminal: raw mode + alternate screen\nSize: {size}\nTicks: {}\n\nPress q, Esc, or Ctrl-C to quit.",
            self.render_state.ticks
        );

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

    #[test]
    fn app_state_starts_with_initial_redraw_requested() {
        let state = AppState::new();

        assert!(!state.should_quit);
        assert!(state.needs_redraw);
        assert_eq!(state.router.current(), Screen::Home);
        assert_eq!(state.router.stack, vec![Screen::Home]);
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
                terminal_size: None,
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

        assert_eq!(
            shell.render_state,
            ShellState {
                ticks: 42,
                terminal_size: Some((80, 24)),
            }
        );
    }
}
