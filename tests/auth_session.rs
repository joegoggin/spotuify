use std::path::PathBuf;

use spotuify::auth::error::AuthError;
use spotuify::auth::event::AuthEvent;
use spotuify::auth::session::AuthSession;
use spotuify::config::{AppConfig, SpotifyConfig};

fn valid_config() -> AppConfig {
    AppConfig {
        spotify: SpotifyConfig {
            client_id: Some("client-id".to_owned()),
            redirect_uri: Some("http://127.0.0.1:8888/callback".to_owned()),
        },
    }
}

#[test]
fn auth_session_rejects_missing_client_id() {
    let mut config = valid_config();
    config.spotify.client_id = Some(" ".to_owned());

    assert_eq!(
        AuthSession::from_config(&config).expect_err("blank client ID should fail"),
        AuthError::MissingConfig("spotify.client_id")
    );
}

#[test]
fn auth_event_marks_terminal_states() {
    assert!(
        AuthEvent::Cached {
            cache_path: PathBuf::from("/tmp/token.json"),
        }
        .is_terminal()
    );
    assert!(
        AuthEvent::Completed {
            cache_path: PathBuf::from("/tmp/token.json"),
        }
        .is_terminal()
    );
    assert!(
        AuthEvent::RefreshedCachedToken {
            cache_path: PathBuf::from("/tmp/token.json"),
        }
        .is_terminal()
    );
    assert!(
        AuthEvent::Failed {
            message: "failed".to_owned(),
        }
        .is_terminal()
    );
    assert!(!AuthEvent::Starting.is_terminal());
    assert!(
        !AuthEvent::RefreshingCachedToken {
            cache_path: PathBuf::from("/tmp/token.json"),
        }
        .is_terminal()
    );
    assert!(
        !AuthEvent::ReauthorizationRequired {
            message: "reauth required".to_owned(),
        }
        .is_terminal()
    );
}
