use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, NoUserEvent};
use tuirealm::props::{AttrValue, Attribute, PropPayload, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Style};
use tuirealm::ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tuirealm::state::State;

use crate::app::msg::Msg;
use crate::auth::AuthEvent;
use crate::config::ConfigBootstrap;

use super::common::{ScreenFrameState, handle_common_event};

/// Attribute used to send typed auth worker events into the auth screen.
pub(crate) const AUTH_EVENT_ATTR: &str = "auth.event";

/// User-facing auth state owned by the auth screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthScreenState {
    /// Per-screen terminal and tick state.
    frame: ScreenFrameState,
    /// Current auth status label.
    pub(crate) status: String,
    /// Authorization URL shown when browser launch fails or manual copy is needed.
    pub(crate) authorize_url: Option<String>,
    /// Browser launch failure retained while the callback listener waits.
    pub(crate) browser_open_error: Option<String>,
}

impl AuthScreenState {
    /// Creates initial auth UI state from startup config readiness.
    pub(crate) fn from_config(config: &ConfigBootstrap) -> Self {
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
    pub(crate) fn apply(&mut self, event: AuthEvent) {
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

impl Default for AuthScreenState {
    fn default() -> Self {
        Self {
            frame: ScreenFrameState::default(),
            status: "Spotify auth not started".to_owned(),
            authorize_url: None,
            browser_open_error: None,
        }
    }
}

/// Spotify authentication screen component.
#[derive(Debug, Clone, Default)]
pub(crate) struct AuthScreen {
    /// Mutable render state for auth status labels and metadata.
    render_state: AuthScreenState,
}

impl AuthScreen {
    /// Creates the auth screen with startup config render state.
    pub(crate) fn from_config(config: &ConfigBootstrap) -> Self {
        Self {
            render_state: AuthScreenState::from_config(config),
        }
    }

    /// Builds the auth screen widget from its own render state.
    fn render_widget(&self, area: Rect) -> Paragraph<'_> {
        let size = self.render_state.frame.size_label(area);
        let mut text = format!(
            "Spotify Auth\n\nStatus: {}\n\nEvent loop: active\nDraw cycle: active\nTerminal: raw mode + alternate screen\nSize: {size}\nTicks: {}",
            self.render_state.status, self.render_state.frame.ticks
        );
        if let Some(auth_url) = self.render_state.authorize_url.as_deref()
            && !auth_url.is_empty()
        {
            text.push_str("\n\nSpotify auth URL:\n");
            text.push_str(auth_url);
        }
        text.push_str("\n\nPress q, Esc, or Ctrl-C to quit.");

        Paragraph::new(text)
            .block(
                Block::default()
                    .title(" spotuify - auth ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: true })
    }
}

impl Component for AuthScreen {
    fn view(&mut self, frame: &mut Frame, area: Rect) {
        frame.render_widget(self.render_widget(area), area);
    }

    fn query<'a>(&'a self, _attr: Attribute) -> Option<QueryResult<'a>> {
        None
    }

    fn attr(&mut self, attr: Attribute, value: AttrValue) {
        if let (Attribute::Custom(AUTH_EVENT_ATTR), AttrValue::Payload(PropPayload::Any(event))) =
            (attr, value)
            && let Some(event) = event.as_any().downcast_ref::<AuthEvent>()
        {
            self.render_state.apply(event.clone());
        }
    }

    fn state(&self) -> State {
        State::None
    }

    fn perform(&mut self, cmd: Cmd) -> CmdResult {
        CmdResult::Invalid(cmd)
    }
}

impl AppComponent<Msg, NoUserEvent> for AuthScreen {
    fn on(&mut self, event: &Event<NoUserEvent>) -> Option<Msg> {
        handle_common_event(&mut self.render_state.frame, event)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tuirealm::props::PropBound;

    use super::*;

    #[test]
    fn auth_state_renders_cached_token_refresh_events() {
        let mut state = AuthScreenState::default();

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
    fn auth_state_renders_reauthorization_required_message() {
        let mut state = AuthScreenState {
            status: "old status".to_owned(),
            authorize_url: Some("https://accounts.spotify.com/authorize".to_owned()),
            browser_open_error: Some("browser error".to_owned()),
            ..AuthScreenState::default()
        };

        state.apply(AuthEvent::ReauthorizationRequired {
            message: "Spotify reauthorization required".to_owned(),
        });

        assert_eq!(state.status, "Spotify reauthorization required");
        assert_eq!(state.authorize_url, None);
        assert_eq!(state.browser_open_error, None);
    }

    #[test]
    fn auth_component_accepts_typed_auth_event_attr() {
        let mut screen = AuthScreen::default();
        let event = AuthEvent::AuthorizationUrl {
            url: "https://accounts.spotify.com/authorize".to_owned(),
            callback_addr: "127.0.0.1:8888".to_owned(),
        };

        screen.attr(
            Attribute::Custom(AUTH_EVENT_ATTR),
            AttrValue::Payload(PropPayload::Any(event.to_any_prop())),
        );

        assert_eq!(
            screen.render_state.status,
            "Spotify authorization URL generated; waiting on 127.0.0.1:8888"
        );
        assert_eq!(
            screen.render_state.authorize_url.as_deref(),
            Some("https://accounts.spotify.com/authorize")
        );
    }
}
