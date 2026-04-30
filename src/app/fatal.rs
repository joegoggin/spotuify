use crate::auth::AuthEvent;

/// User-facing description of an unrecoverable error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FatalError {
    /// Short subsystem label shown as the headline (e.g. "Spotify auth").
    pub source: &'static str,
    /// One-line summary of what failed.
    pub message: String,
    /// Optional next steps for the user.
    pub details: Option<String>,
}

impl FatalError {
    /// Creates a fatal error with a subsystem label and message.
    pub fn new(source: &'static str, message: impl Into<String>) -> Self {
        Self {
            source,
            message: message.into(),
            details: None,
        }
    }

    /// Adds optional next-step details to the fatal error.
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

/// Returns a fatal-error description for any auth event that ends the flow unrecoverably.
pub fn fatal_from_auth_event(event: &AuthEvent) -> Option<FatalError> {
    match event {
        AuthEvent::Failed { message } => Some(
            FatalError::new("Spotify auth", message.clone()).with_details(
                "Verify your Spotify client ID and redirect URI in the app config, then restart spotuify.",
            ),
        ),
        _ => None,
    }
}
