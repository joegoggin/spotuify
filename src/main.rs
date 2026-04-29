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

fn main() {
    if let Err(err) = run() {
        eprintln!("spotuify: {err}");
        std::process::exit(1);
    }
}

fn run() -> AppResult<()> {
    let mut model = Model::new()?;

    while !model.quit {
        for msg in model.app.tick(PollStrategy::Once(FRAME_INTERVAL))? {
            model.update(msg);
        }

        if model.redraw {
            model.view()?;
            model.redraw = false;
        }
    }

    Ok(())
}

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
enum Id {
    Shell,
}

#[derive(Debug, PartialEq)]
enum Msg {
    Quit,
    Redraw,
}

struct Model {
    app: Application<Id, Msg, NoUserEvent>,
    quit: bool,
    redraw: bool,
    terminal: CrosstermTerminalAdapter,
}

impl Model {
    fn new() -> AppResult<Self> {
        Ok(Self {
            app: Self::init_app()?,
            quit: false,
            redraw: true,
            terminal: Self::init_terminal()?,
        })
    }

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

    fn init_terminal() -> TerminalResult<CrosstermTerminalAdapter> {
        let mut terminal = CrosstermTerminalAdapter::new()?;
        terminal.enable_raw_mode()?;
        terminal.enter_alternate_screen()?;
        terminal.clear_screen()?;

        Ok(terminal)
    }

    fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Quit => self.quit = true,
            Msg::Redraw => self.redraw = true,
        }
    }

    fn view(&mut self) -> TerminalResult<()> {
        self.terminal
            .draw(|frame| self.app.view(&Id::Shell, frame, frame.area()))
            .map(|_| ())
    }
}

#[derive(Default)]
struct Shell {
    ticks: u64,
    terminal_size: Option<(u16, u16)>,
}

impl Component for Shell {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        let size = self
            .terminal_size
            .map(|(width, height)| format!("{width}x{height}"))
            .unwrap_or_else(|| format!("{}x{}", area.width, area.height));
        let text = format!(
            "App shell running\n\nEvent loop: active\nDraw cycle: active\nTerminal: raw mode + alternate screen\nSize: {size}\nTicks: {}\n\nPress q, Esc, or Ctrl-C to quit.",
            self.ticks
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

    fn attr(&mut self, _attr: Attribute, _value: AttrValue) {}

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
            Event::WindowResize(width, height) => {
                self.terminal_size = Some((*width, *height));
                Some(Msg::Redraw)
            }
            Event::Tick => {
                self.ticks += 1;
                Some(Msg::Redraw)
            }
            _ => None,
        }
    }
}
