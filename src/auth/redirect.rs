//! Redirect URI parsing and callback target validation.

use std::net::IpAddr;

use url::Url;

use super::error::AuthError;

/// Parsed local redirect URI information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LocalRedirect {
    /// URI scheme (must be `http` for local callback usage).
    pub(super) scheme: String,
    /// Redirect host, restricted to explicit loopback IP literals.
    pub(super) host: String,
    /// Redirect port where the callback listener binds.
    pub(super) port: u16,
    /// Redirect path that callback requests must match.
    pub(super) path: String,
}

impl LocalRedirect {
    /// Parses and validates a configured redirect URI for local callback usage.
    pub(super) fn from_uri(uri: &str) -> Result<Self, AuthError> {
        let parsed = Url::parse(uri)?;
        let scheme = parsed.scheme();
        if scheme != "http" {
            return Err(AuthError::InvalidRedirectUri(
                "local callback listener requires an http redirect URI".to_owned(),
            ));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| {
                AuthError::InvalidRedirectUri("redirect URI is missing a host".to_owned())
            })?
            .to_owned();
        if !is_loopback_ip_literal(&host) {
            return Err(AuthError::InvalidRedirectUri(format!(
                "redirect host must be an explicit loopback IP literal like 127.0.0.1 or ::1, got {host}"
            )));
        }

        let port = parsed.port().ok_or_else(|| {
            AuthError::InvalidRedirectUri("redirect URI is missing a port".to_owned())
        })?;
        let path = parsed.path().to_owned();

        Ok(Self {
            scheme: scheme.to_owned(),
            host,
            port,
            path,
        })
    }

    /// Returns host:port display form for listener status messages.
    pub(super) fn bind_addr(&self) -> String {
        format!("{}:{}", self.host_for_url(), self.port)
    }

    /// Returns the URL origin reconstructed from validated redirect parts.
    fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.bind_addr())
    }

    /// Normalizes a callback request target into a validated absolute URL.
    pub(super) fn callback_url_from_request_target(
        &self,
        request_target: &str,
    ) -> Result<String, AuthError> {
        let callback_url = if request_target.starts_with("http://")
            || request_target.starts_with("https://")
        {
            Url::parse(request_target).map_err(|err| AuthError::InvalidCallback(err.to_string()))?
        } else {
            Url::parse(&format!("{}{}", self.origin(), request_target))
                .map_err(|err| AuthError::InvalidCallback(err.to_string()))?
        };

        if callback_url.scheme() != self.scheme {
            return Err(AuthError::InvalidCallback(
                "callback scheme did not match configured redirect URI".to_owned(),
            ));
        }

        let callback_host = callback_url.host_str().unwrap_or_default();
        if !callback_host.eq_ignore_ascii_case(&self.host) {
            return Err(AuthError::InvalidCallback(
                "callback host did not match configured redirect URI".to_owned(),
            ));
        }

        if callback_url.port_or_known_default() != Some(self.port) {
            return Err(AuthError::InvalidCallback(
                "callback port did not match configured redirect URI".to_owned(),
            ));
        }

        if callback_url.path() != self.path {
            return Err(AuthError::InvalidCallback(
                "callback path did not match configured redirect URI".to_owned(),
            ));
        }

        Ok(callback_url.to_string())
    }

    /// Formats an IP host for URL usage (including IPv6 bracket wrapping).
    fn host_for_url(&self) -> String {
        if self.host.contains(':') && !self.host.starts_with('[') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }
}

/// Returns true when the host is a loopback IP literal (`127.0.0.1`, `::1`).
fn is_loopback_ip_literal(host: &str) -> bool {
    host.parse::<IpAddr>()
        .map(|addr| addr.is_loopback())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_redirect_rejects_non_local_hosts() {
        let err = LocalRedirect::from_uri("http://example.com:8888/callback")
            .expect_err("non-local redirect should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn local_redirect_rejects_localhost_hostname() {
        let err = LocalRedirect::from_uri("http://localhost:8888/callback")
            .expect_err("localhost redirect should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn local_redirect_rejects_https_scheme() {
        let err = LocalRedirect::from_uri("https://127.0.0.1:8888/callback")
            .expect_err("https redirect should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn local_redirect_rejects_missing_explicit_port() {
        let err = LocalRedirect::from_uri("http://127.0.0.1/callback")
            .expect_err("redirect without explicit port should fail");

        assert!(matches!(err, AuthError::InvalidRedirectUri(_)));
    }

    #[test]
    fn callback_url_is_reconstructed_from_origin_form_target() {
        let redirect = LocalRedirect::from_uri("http://127.0.0.1:8888/callback")
            .expect("redirect should parse");

        let callback_url = redirect
            .callback_url_from_request_target("/callback?code=code-123&state=state-123")
            .expect("callback URL should be reconstructed");

        assert_eq!(
            callback_url,
            "http://127.0.0.1:8888/callback?code=code-123&state=state-123"
        );
    }

    #[test]
    fn callback_url_rejects_unexpected_path() {
        let redirect = LocalRedirect::from_uri("http://127.0.0.1:8888/callback")
            .expect("redirect should parse");

        let err = redirect
            .callback_url_from_request_target("/wrong?code=code-123&state=state-123")
            .expect_err("wrong path should fail");

        assert!(matches!(err, AuthError::InvalidCallback(_)));
    }
}
