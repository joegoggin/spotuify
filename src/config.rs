use std::fs;
use std::io;
use std::path::PathBuf;

use serde::Deserialize;

/// XDG config directory prefix used by this application.
const APP_CONFIG_DIR: &str = "spotuify";
/// Primary user config file name resolved under the XDG config directory.
const CONFIG_FILE_NAME: &str = "config.toml";

/// Application configuration loaded from the user's config file.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    /// Spotify-specific configuration required by the auth flow.
    pub spotify: SpotifyConfig,
}

impl AppConfig {
    /// Parses application configuration from TOML.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use spotuify::config::AppConfig;
    ///
    /// let config = AppConfig::from_toml(
    ///     r#"
    ///         [spotify]
    ///         client_id = "client-id"
    ///         redirect_uri = "http://127.0.0.1:8888/callback"
    ///     "#,
    /// ).unwrap();
    ///
    /// assert_eq!(config.spotify.client_id.as_deref(), Some("client-id"));
    /// ```
    pub fn from_toml(input: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(input)
    }

    /// Returns every required field that is absent or blank.
    fn missing_required_fields(&self) -> Vec<RequiredConfigField> {
        let mut missing = Vec::new();

        if is_blank(self.spotify.client_id.as_deref()) {
            missing.push(RequiredConfigField::SpotifyClientId);
        }

        if is_blank(self.spotify.redirect_uri.as_deref()) {
            missing.push(RequiredConfigField::SpotifyRedirectUri);
        }

        missing
    }
}

/// Spotify settings used to start the authorization flow.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SpotifyConfig {
    /// Spotify application client ID.
    pub client_id: Option<String>,
    /// Redirect URI registered for the Spotify application.
    pub redirect_uri: Option<String>,
}

/// Startup config bootstrap state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigBootstrap {
    /// Config loaded successfully and has the required Spotify settings.
    Ready {
        /// Path used to load the config file.
        path: PathBuf,
        /// Parsed and validated config.
        config: AppConfig,
    },
    /// Startup needs to route through setup before auth can continue.
    NeedsSetup {
        /// Expected config path, when it can be resolved.
        path: Option<PathBuf>,
        /// Reason setup is required.
        issue: ConfigIssue,
    },
}

impl ConfigBootstrap {
    /// Loads config from the default XDG config path.
    pub fn load() -> Self {
        match expected_config_path() {
            Some(path) => Self::load_from_path(path),
            None => Self::NeedsSetup {
                path: None,
                issue: ConfigIssue::PathUnavailable("$HOME is not set".to_owned()),
            },
        }
    }

    /// Loads config from a specific file path.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use spotuify::config::ConfigBootstrap;
    ///
    /// let bootstrap = ConfigBootstrap::load_from_path("/tmp/spotuify/config.toml");
    /// assert!(matches!(
    ///     bootstrap,
    ///     ConfigBootstrap::Ready { .. } | ConfigBootstrap::NeedsSetup { .. }
    /// ));
    /// ```
    pub fn load_from_path(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Self::NeedsSetup {
                    path: Some(path),
                    issue: ConfigIssue::MissingFile,
                };
            }
            Err(err) => {
                return Self::NeedsSetup {
                    path: Some(path),
                    issue: ConfigIssue::ReadFailed(err.to_string()),
                };
            }
        };

        let config = match AppConfig::from_toml(&contents) {
            Ok(config) => config,
            Err(err) => {
                return Self::NeedsSetup {
                    path: Some(path),
                    issue: ConfigIssue::ParseFailed(err.to_string()),
                };
            }
        };

        let missing_fields = config.missing_required_fields();
        if missing_fields.is_empty() {
            Self::Ready { path, config }
        } else {
            Self::NeedsSetup {
                path: Some(path),
                issue: ConfigIssue::MissingRequiredFields(missing_fields),
            }
        }
    }

    /// Reports whether startup can proceed directly to auth.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Builds a concise user-facing config status for the current shell.
    pub fn status_label(&self) -> String {
        match self {
            Self::Ready { path, config } => {
                let redirect_uri = config
                    .spotify
                    .redirect_uri
                    .as_deref()
                    .unwrap_or("configured redirect URI");
                format!(
                    "loaded {}; Spotify settings complete for {redirect_uri}",
                    path.display()
                )
            }
            Self::NeedsSetup { path, issue } => {
                let path = path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "unresolved config path".to_owned());
                format!("{issue} ({path})")
            }
        }
    }
}

impl Default for ConfigBootstrap {
    fn default() -> Self {
        Self::NeedsSetup {
            path: None,
            issue: ConfigIssue::MissingFile,
        }
    }
}

/// Reason startup cannot use the current config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigIssue {
    /// The expected config file does not exist.
    MissingFile,
    /// The expected config file path could not be resolved.
    PathUnavailable(String),
    /// The config file exists but cannot be read.
    ReadFailed(String),
    /// The config file is not valid TOML for the app config schema.
    ParseFailed(String),
    /// One or more required Spotify fields are absent or blank.
    MissingRequiredFields(Vec<RequiredConfigField>),
}

impl std::fmt::Display for ConfigIssue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFile => write!(formatter, "missing config file; setup required"),
            Self::PathUnavailable(message) => {
                write!(formatter, "could not resolve config path: {message}")
            }
            Self::ReadFailed(message) => write!(formatter, "could not read config: {message}"),
            Self::ParseFailed(message) => write!(formatter, "invalid config TOML: {message}"),
            Self::MissingRequiredFields(fields) => {
                let fields = fields
                    .iter()
                    .map(|field| field.key())
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(formatter, "missing required config fields: {fields}")
            }
        }
    }
}

/// Required Spotify config fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredConfigField {
    /// `spotify.client_id`
    SpotifyClientId,
    /// `spotify.redirect_uri`
    SpotifyRedirectUri,
}

impl RequiredConfigField {
    /// Returns the TOML key for the required field.
    fn key(self) -> &'static str {
        match self {
            Self::SpotifyClientId => "spotify.client_id",
            Self::SpotifyRedirectUri => "spotify.redirect_uri",
        }
    }
}

/// Resolves the expected user config file path.
fn expected_config_path() -> Option<PathBuf> {
    app_config_file_path(CONFIG_FILE_NAME)
}

/// Resolves a file path inside the application's XDG config directory.
pub fn app_config_file_path(file_name: &str) -> Option<PathBuf> {
    xdg::BaseDirectories::with_prefix(APP_CONFIG_DIR).get_config_file(file_name)
}

/// Returns true when a required string field is missing or all whitespace.
fn is_blank(value: Option<&str>) -> bool {
    value.map(str::trim).unwrap_or_default().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_config_toml() {
        let config = AppConfig::from_toml(
            r#"
                [spotify]
                client_id = "client-id"
                redirect_uri = "http://127.0.0.1:8888/callback"
            "#,
        )
        .expect("valid config should parse");

        assert_eq!(config.spotify.client_id.as_deref(), Some("client-id"));
        assert_eq!(
            config.spotify.redirect_uri.as_deref(),
            Some("http://127.0.0.1:8888/callback")
        );
        assert!(config.missing_required_fields().is_empty());
    }
}
