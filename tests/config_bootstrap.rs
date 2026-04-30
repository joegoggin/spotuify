use std::fs;
use std::path::PathBuf;

use spotuify::config::{ConfigBootstrap, ConfigIssue, RequiredConfigField};

fn temp_config_path(test_name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "spotuify-{test_name}-{}-config.toml",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    path
}

#[test]
fn load_from_path_reports_missing_file() {
    let path = temp_config_path("missing");

    let bootstrap = ConfigBootstrap::load_from_path(path.clone());

    assert_eq!(
        bootstrap,
        ConfigBootstrap::NeedsSetup {
            path: Some(path),
            issue: ConfigIssue::MissingFile,
        }
    );
}

#[test]
fn load_from_path_reports_invalid_toml() {
    let path = temp_config_path("invalid-toml");
    fs::write(&path, "not toml =").expect("test config should be writable");

    let bootstrap = ConfigBootstrap::load_from_path(path.clone());
    let _ = fs::remove_file(&path);

    assert!(matches!(
        bootstrap,
        ConfigBootstrap::NeedsSetup {
            path: Some(_),
            issue: ConfigIssue::ParseFailed(_),
        }
    ));
}

#[test]
fn load_from_path_reports_missing_client_id() {
    let path = temp_config_path("missing-client-id");
    fs::write(
        &path,
        r#"
            [spotify]
            redirect_uri = "http://127.0.0.1:8888/callback"
        "#,
    )
    .expect("test config should be writable");

    let bootstrap = ConfigBootstrap::load_from_path(path.clone());
    let _ = fs::remove_file(&path);

    assert_eq!(
        bootstrap,
        ConfigBootstrap::NeedsSetup {
            path: Some(path),
            issue: ConfigIssue::MissingRequiredFields(vec![RequiredConfigField::SpotifyClientId,]),
        }
    );
}

#[test]
fn load_from_path_reports_blank_redirect_uri() {
    let path = temp_config_path("blank-redirect-uri");
    fs::write(
        &path,
        r#"
            [spotify]
            client_id = "client-id"
            redirect_uri = "   "
        "#,
    )
    .expect("test config should be writable");

    let bootstrap = ConfigBootstrap::load_from_path(path.clone());
    let _ = fs::remove_file(&path);

    assert_eq!(
        bootstrap,
        ConfigBootstrap::NeedsSetup {
            path: Some(path),
            issue: ConfigIssue::MissingRequiredFields(vec![
                RequiredConfigField::SpotifyRedirectUri,
            ]),
        }
    );
}
