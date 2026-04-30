use std::path::PathBuf;

use spotuify::app::action::Action;
use spotuify::app::fatal::{FatalError, fatal_from_auth_event};
use spotuify::app::msg::Msg;
use spotuify::app::router::{Screen, ScreenTransition};
use spotuify::app::state::AppState;
use spotuify::auth::event::AuthEvent;
use spotuify::config::{AppConfig, ConfigBootstrap, ConfigIssue, SpotifyConfig};

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
fn fatal_error_navigation_message_converts_to_action() {
    let fatal = FatalError::new("Spotify auth", "boom");

    assert_eq!(
        Action::from(Msg::Fatal(fatal.clone())),
        Action::Fatal(fatal)
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
fn dispatcher_requests_redraw_on_tick() {
    let mut state = AppState::default();

    state.apply(Action::Tick);

    assert!(state.needs_redraw);
}

#[test]
fn dispatcher_requests_redraw_on_resize() {
    let mut state = AppState::default();

    state.apply(Action::Resize {
        width: 120,
        height: 40,
    });

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
fn dispatcher_routes_to_fatal_error_screen() {
    let mut state = AppState::default();

    state.apply(Action::Fatal(
        FatalError::new("Spotify auth", "auth flow exploded").with_details("Check config."),
    ));

    assert_eq!(state.router.current(), Screen::FatalError);
    assert_eq!(state.router.stack, vec![Screen::FatalError]);
    assert!(state.needs_redraw);
}

#[test]
fn dispatcher_replaces_existing_screen_on_fatal() {
    let mut state = AppState::default();
    state.apply(Action::Navigate(ScreenTransition::Replace(Screen::Auth)));
    state.apply(Action::Rendered);

    state.apply(Action::Fatal(FatalError::new("Spotify auth", "boom")));

    assert_eq!(state.router.current(), Screen::FatalError);
    assert_eq!(state.router.stack, vec![Screen::FatalError]);
    assert!(state.needs_redraw);
}

#[test]
fn auth_failed_event_produces_fatal_error() {
    let event = AuthEvent::Failed {
        message: "Spotify token endpoint returned HTTP 400: invalid_grant".to_owned(),
    };

    let fatal = fatal_from_auth_event(&event).expect("Failed event should produce fatal");

    assert_eq!(fatal.source, "Spotify auth");
    assert_eq!(
        fatal.message,
        "Spotify token endpoint returned HTTP 400: invalid_grant"
    );
    assert!(fatal.details.is_some());
}

#[test]
fn non_failed_auth_events_do_not_produce_fatal_error() {
    assert!(fatal_from_auth_event(&AuthEvent::Starting).is_none());
    assert!(
        fatal_from_auth_event(&AuthEvent::Cached {
            cache_path: PathBuf::from("/tmp/token.json"),
        })
        .is_none()
    );
    assert!(
        fatal_from_auth_event(&AuthEvent::ReauthorizationRequired {
            message: "reauth required".to_owned(),
        })
        .is_none()
    );
    assert!(
        fatal_from_auth_event(&AuthEvent::Completed {
            cache_path: PathBuf::from("/tmp/token.json"),
        })
        .is_none()
    );
}
