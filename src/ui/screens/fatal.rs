use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, PropPayload, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Style};
use tuirealm::ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tuirealm::state::State;

use crate::app::fatal::FatalError;
use crate::app::msg::Msg;

use super::common::{ScreenFrameState, handle_common_event};

/// Attribute used to send typed fatal errors into the fatal screen.
pub(crate) const FATAL_ERROR_ATTR: &str = "fatal.error";

/// Render state owned by the fatal error screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FatalScreenState {
    /// Per-screen terminal and tick state.
    frame: ScreenFrameState,
    /// Fatal error subsystem rendered on the fatal error screen.
    pub(crate) source: Option<&'static str>,
    /// Fatal error message rendered on the fatal error screen.
    pub(crate) message: Option<String>,
    /// Fatal error next-step details rendered on the fatal error screen.
    pub(crate) details: Option<String>,
}

impl FatalScreenState {
    /// Stores a fatal error for rendering.
    pub(crate) fn apply(&mut self, error: FatalError) {
        self.source = Some(error.source);
        self.message = Some(error.message);
        self.details = error.details;
    }
}

/// Fatal error screen component.
#[derive(Debug, Clone, Default)]
pub(crate) struct FatalScreen {
    /// Mutable render state for fatal error content and frame metadata.
    render_state: FatalScreenState,
}

impl FatalScreen {
    /// Builds the fatal screen widget from its own render state.
    fn render_widget(&self) -> Paragraph<'_> {
        let source = self.render_state.source.unwrap_or("Unknown subsystem");
        let message = self
            .render_state
            .message
            .as_deref()
            .unwrap_or("Unrecoverable error.");
        let mut text = format!("{source} encountered an unrecoverable error.\n\n{message}");
        if let Some(details) = self.render_state.details.as_deref()
            && !details.is_empty()
        {
            text.push_str("\n\nNext steps:\n");
            text.push_str(details);
        }
        text.push_str("\n\nPress q, Esc, or Ctrl-C to quit.");

        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" spotuify - fatal error ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            )
            .wrap(Wrap { trim: true })
    }
}

impl Component for FatalScreen {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(self.render_widget(), area);
    }

    fn query<'a>(&'a self, _attr: Attribute) -> Option<QueryResult<'a>> {
        None
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if let (Attribute::Custom(FATAL_ERROR_ATTR), AttrValue::Payload(PropPayload::Any(error))) =
            (attr, value)
            && let Some(error) = error.as_any().downcast_ref::<FatalError>()
        {
            self.render_state.apply(error.clone());
        }
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        CmdResult::Invalid(cmd)
    }
}

impl AppComponent<Msg, NoUserEvent> for FatalScreen {
    fn on(&mut self, event: &Event<NoUserEvent>) -> Option<Msg> {
        handle_common_event(&mut self.render_state.frame, event)
    }
}

#[cfg(test)]
mod tests {
    use tuirealm::props::PropBound;

    use super::*;

    #[test]
    fn fatal_state_stores_error_details() {
        let mut state = FatalScreenState::default();

        state.apply(
            FatalError::new("Spotify auth", "auth flow exploded").with_details("Check config."),
        );

        assert_eq!(state.source, Some("Spotify auth"));
        assert_eq!(state.message.as_deref(), Some("auth flow exploded"));
        assert_eq!(state.details.as_deref(), Some("Check config."));
    }

    #[test]
    fn fatal_component_accepts_typed_error_attr() {
        let mut screen = FatalScreen::default();
        let error =
            FatalError::new("Spotify auth", "auth flow exploded").with_details("Check config.");

        screen.attr(
            Attribute::Custom(FATAL_ERROR_ATTR),
            AttrValue::Payload(PropPayload::Any(error.to_any_prop())),
        );

        assert_eq!(screen.render_state.source, Some("Spotify auth"));
        assert_eq!(
            screen.render_state.message.as_deref(),
            Some("auth flow exploded")
        );
        assert_eq!(
            screen.render_state.details.as_deref(),
            Some("Check config.")
        );
    }
}
