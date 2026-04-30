use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Style};
use tuirealm::ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tuirealm::state::State;

use crate::app::msg::Msg;

use super::common::{ScreenFrameState, handle_common_event};

/// Render state owned by the home screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HomeScreenState {
    /// Per-screen terminal and tick state.
    frame: ScreenFrameState,
}

/// Home screen component.
#[derive(Debug, Clone, Default)]
pub(crate) struct HomeScreen {
    /// Mutable render state for home screen frame metadata.
    render_state: HomeScreenState,
}

impl HomeScreen {
    /// Builds the home screen widget from its own render state.
    fn render_widget(&self, area: Rect) -> Paragraph<'_> {
        let size = self.render_state.frame.size_label(area);
        let text = format!(
            "Home\n\nSpotuify is ready.\n\nEvent loop: active\nDraw cycle: active\nTerminal: raw mode + alternate screen\nSize: {size}\nTicks: {}\n\nPress q, Esc, or Ctrl-C to quit.",
            self.render_state.frame.ticks
        );

        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" spotuify - home ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Green)),
            )
            .wrap(Wrap { trim: true })
    }
}

impl Component for HomeScreen {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(self.render_widget(area), area);
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

impl AppComponent<Msg, NoUserEvent> for HomeScreen {
    fn on(&mut self, event: &Event<NoUserEvent>) -> Option<Msg> {
        handle_common_event(&mut self.render_state.frame, event)
    }
}
