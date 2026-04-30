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
use crate::config::ConfigBootstrap;

use super::common::{ScreenFrameState, handle_common_event};

/// Render state owned by the setup screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupScreenState {
    /// Per-screen terminal and tick state.
    frame: ScreenFrameState,
    /// User-facing startup config status.
    config_status: String,
}

impl SetupScreenState {
    /// Creates setup render state from startup config bootstrap data.
    pub(crate) fn from_config(config: &ConfigBootstrap) -> Self {
        Self {
            config_status: config.status_label(),
            ..Self::default()
        }
    }
}

impl Default for SetupScreenState {
    fn default() -> Self {
        Self {
            frame: ScreenFrameState::default(),
            config_status: "config not checked".to_owned(),
        }
    }
}

/// First-run setup screen component.
#[derive(Debug, Clone, Default)]
pub(crate) struct SetupScreen {
    /// Mutable render state for setup status and frame metadata.
    render_state: SetupScreenState,
}

impl SetupScreen {
    /// Creates the setup screen with startup config render state.
    pub(crate) fn from_config(config: &ConfigBootstrap) -> Self {
        Self {
            render_state: SetupScreenState::from_config(config),
        }
    }

    /// Builds the setup screen widget from its own render state.
    fn render_widget(&self, area: Rect) -> Paragraph<'_> {
        let size = self.render_state.frame.size_label(area);
        let text = format!(
            "Setup\n\nConfig: {}\n\nEvent loop: active\nDraw cycle: active\nTerminal: raw mode + alternate screen\nSize: {size}\nTicks: {}\n\nPress q, Esc, or Ctrl-C to quit.",
            self.render_state.config_status, self.render_state.frame.ticks
        );

        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" spotuify - setup ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .wrap(Wrap { trim: true })
    }
}

impl Component for SetupScreen {
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

impl AppComponent<Msg, NoUserEvent> for SetupScreen {
    fn on(&mut self, event: &Event<NoUserEvent>) -> Option<Msg> {
        handle_common_event(&mut self.render_state.frame, event)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::ConfigIssue;

    use super::*;

    #[test]
    fn setup_state_reflects_config_bootstrap_status() {
        let config = ConfigBootstrap::NeedsSetup {
            path: Some(PathBuf::from("/tmp/spotuify/config.toml")),
            issue: ConfigIssue::MissingFile,
        };

        let state = SetupScreenState::from_config(&config);

        assert_eq!(
            state.config_status,
            "missing config file; setup required (/tmp/spotuify/config.toml)"
        );
    }
}
