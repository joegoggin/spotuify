use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};
use tuirealm::ratatui::layout::Rect;

use crate::app::msg::Msg;

/// Render state shared by each screen component, but owned per screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ScreenFrameState {
    /// Number of tick events observed while the screen is active.
    pub(crate) ticks: u64,
    /// Most recent terminal size observed while the screen is active.
    pub(crate) terminal_size: Option<(u16, u16)>,
}

impl ScreenFrameState {
    /// Returns a user-facing terminal size label, falling back to the draw area.
    pub(crate) fn size_label(&self, area: Rect) -> String {
        self.terminal_size
            .map(|(width, height)| format!("{width}x{height}"))
            .unwrap_or_else(|| format!("{}x{}", area.width, area.height))
    }

    /// Records a tick without overflowing the counter.
    fn record_tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Records the latest terminal dimensions for this screen.
    fn record_resize(&mut self, width: u16, height: u16) {
        self.terminal_size = Some((width, height));
    }
}

/// Handles app-wide keyboard and terminal events for a screen component.
pub(crate) fn handle_common_event(
    frame_state: &mut ScreenFrameState,
    event: &Event<NoUserEvent>,
) -> Option<Msg> {
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
            frame_state.record_resize(*width, *height);
            Some(Msg::WindowResize {
                width: *width,
                height: *height,
            })
        }
        Event::Tick => {
            frame_state.record_tick();
            Some(Msg::Tick)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_tick_event_updates_screen_frame_state() {
        let mut frame = ScreenFrameState::default();

        let msg = handle_common_event(&mut frame, &Event::Tick);

        assert_eq!(msg, Some(Msg::Tick));
        assert_eq!(frame.ticks, 1);
    }

    #[test]
    fn common_resize_event_updates_screen_frame_state() {
        let mut frame = ScreenFrameState::default();

        let msg = handle_common_event(&mut frame, &Event::WindowResize(120, 40));

        assert_eq!(
            msg,
            Some(Msg::WindowResize {
                width: 120,
                height: 40,
            })
        );
        assert_eq!(frame.terminal_size, Some((120, 40)));
    }

    #[test]
    fn common_quit_events_emit_quit_message() {
        let mut frame = ScreenFrameState::default();

        assert_eq!(
            handle_common_event(&mut frame, &Event::Keyboard(KeyEvent::from(Key::Esc))),
            Some(Msg::Quit)
        );
        assert_eq!(
            handle_common_event(&mut frame, &Event::Keyboard(KeyEvent::from(Key::Char('q')))),
            Some(Msg::Quit)
        );
        assert_eq!(
            handle_common_event(
                &mut frame,
                &Event::Keyboard(KeyEvent::new(Key::Char('c'), KeyModifiers::CONTROL)),
            ),
            Some(Msg::Quit)
        );
    }
}
