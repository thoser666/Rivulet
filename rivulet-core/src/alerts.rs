//! Stream alert overlay import (follows/subs/donations) via the browser source.
//!
//! Rivulet does not re-implement alert hosting. Instead — exactly like OBS —
//! streamers point a browser source at their **alert widget URL** provided by
//! a third-party service (Streamlabs Alert Box or StreamElements Overlays).
//! The widget page renders the animated alerts and talks to the service's own
//! backend; Rivulet only renders the page and forwards pointer/keyboard input
//! (interaction can be disabled so the overlay never steals focus).
//!
//! This module knows the URL shapes of the two common providers so the GUI can
//! offer a guided import (provider + token → ready-to-load browser URL) and can
//! validate a pasted overlay URL, without ever storing the token outside the
//! app's own scene configuration.

use std::fmt;

/// The alert provider whose overlay URL shape is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AlertProvider {
    /// Streamlabs Alert Box (`streamlabs.com` widget URL).
    #[default]
    Streamlabs,
    /// StreamElements Overlays (`streamelements.com/overlay/<token>`).
    StreamElements,
    /// Any other https URL (still validated, but no token expansion).
    Custom,
}

impl AlertProvider {
    /// All providers in UI display order.
    pub fn all() -> &'static [AlertProvider] {
        &[
            AlertProvider::Streamlabs,
            AlertProvider::StreamElements,
            AlertProvider::Custom,
        ]
    }

    /// i18n key for the provider label.
    pub fn i18n_key(self) -> &'static str {
        match self {
            AlertProvider::Streamlabs => "alert_provider_streamlabs",
            AlertProvider::StreamElements => "alert_provider_streamelements",
            AlertProvider::Custom => "alert_provider_custom",
        }
    }

    /// Stable English label (used by tests and as fallback).
    pub fn label(self) -> &'static str {
        match self {
            AlertProvider::Streamlabs => "Streamlabs",
            AlertProvider::StreamElements => "StreamElements",
            AlertProvider::Custom => "Custom",
        }
    }

    /// Turn a provider token into the widget URL the browser source loads.
    /// Returns `None` when the token is empty or the provider is `Custom`
    /// (custom URLs are entered verbatim).
    pub fn widget_url(&self, token: &str) -> Option<String> {
        let token = token.trim();
        if token.is_empty() {
            return None;
        }
        match self {
            AlertProvider::Streamlabs => {
                Some(format!("https://streamlabs.com/alert-box/v2/{token}"))
            }
            AlertProvider::StreamElements => {
                Some(format!("https://streamelements.com/overlay/{token}"))
            }
            AlertProvider::Custom => None,
        }
    }

    /// Guess the provider from a widget URL the user pasted. Used to prefill
    /// the provider dropdown so the token can be extracted for editing.
    pub fn guess_from_url(url: &str) -> AlertProvider {
        let url = url.trim().to_ascii_lowercase();
        if url.contains("streamlabs.com") {
            AlertProvider::Streamlabs
        } else if url.contains("streamelements.com") {
            AlertProvider::StreamElements
        } else {
            AlertProvider::Custom
        }
    }
}

/// Errors returned while importing an alert overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertOverlayError {
    /// The widget URL is not an https URL (overlay scripts require https).
    NotHttps(String),
    /// The token looks structurally wrong for the selected provider.
    MalformedToken {
        provider: AlertProvider,
        token: String,
    },
    /// A custom URL was requested without a URL.
    MissingUrl,
}

impl fmt::Display for AlertOverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotHttps(url) => write!(f, "alert overlay URL must use https: {url}"),
            Self::MalformedToken { provider, token } => {
                write!(f, "invalid {} alert token: {token}", provider.label())
            }
            Self::MissingUrl => write!(f, "custom alert overlays need a full https URL"),
        }
    }
}

impl std::error::Error for AlertOverlayError {}

/// Validate an alert overlay URL (https only, no whitespace). This mirrors
/// [`crate::browser_source::BrowserSource::validate_url`] but restricts the
/// scheme to https because alert widgets depend on the service's secure
/// backend and often on camera/mic-free iframe permissions.
pub fn validate_overlay_url(url: &str) -> Result<(), AlertOverlayError> {
    let trimmed = url.trim();
    if trimmed != url || url.chars().any(char::is_whitespace) {
        return Err(AlertOverlayError::NotHttps(url.to_owned()));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(AlertOverlayError::NotHttps(url.to_owned()));
    };
    if !scheme.eq_ignore_ascii_case("https") || rest.is_empty() || rest.starts_with('/') {
        return Err(AlertOverlayError::NotHttps(url.to_owned()));
    }
    Ok(())
}

/// Validate a token structurally for the selected provider. Streamlabs Alert
/// Box tokens and StreamElements overlay tokens are non-empty strings without
/// whitespace or URL-dangerous characters; they are not cryptographically
/// verified here (the service does that when the widget loads).
pub fn validate_token(provider: AlertProvider, token: &str) -> Result<(), AlertOverlayError> {
    let token = token.trim();
    if provider == AlertProvider::Custom {
        return Err(AlertOverlayError::MissingUrl);
    }
    if token.is_empty()
        || token.chars().any(|c| c.is_whitespace())
        || token
            .chars()
            .any(|c| matches!(c, '/' | '?' | '#' | '&' | '=' | '"' | '\''))
    {
        return Err(AlertOverlayError::MalformedToken {
            provider,
            token: token.to_owned(),
        });
    }
    Ok(())
}

/// Build the final browser-source URL for an alert overlay configuration.
/// Returns `Ok(url)` for the provider+token combination or a validated custom
/// URL; `Err` for structurally invalid input.
pub fn build_overlay_url(
    provider: AlertProvider,
    token: &str,
    custom_url: Option<&str>,
) -> Result<String, AlertOverlayError> {
    match provider {
        AlertProvider::Custom => {
            let url = custom_url.unwrap_or_default().trim().to_owned();
            validate_overlay_url(&url)?;
            Ok(url)
        }
        provider => {
            validate_token(provider, token)?;
            provider
                .widget_url(token)
                .ok_or(AlertOverlayError::MissingUrl)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_urls_match_the_known_service_shapes() {
        assert_eq!(
            AlertProvider::Streamlabs.widget_url("abc123"),
            Some("https://streamlabs.com/alert-box/v2/abc123".to_owned())
        );
        assert_eq!(
            AlertProvider::StreamElements.widget_url("xyz789"),
            Some("https://streamelements.com/overlay/xyz789".to_owned())
        );
        assert_eq!(AlertProvider::Custom.widget_url("anything"), None);
        assert_eq!(AlertProvider::Streamlabs.widget_url("  "), None);
    }

    #[test]
    fn provider_is_guessed_from_pasted_urls() {
        assert_eq!(
            AlertProvider::guess_from_url("https://streamlabs.com/alert-box/v2/abc"),
            AlertProvider::Streamlabs
        );
        assert_eq!(
            AlertProvider::guess_from_url("https://streamelements.com/overlay/abc"),
            AlertProvider::StreamElements
        );
        assert_eq!(
            AlertProvider::guess_from_url("https://example.com/overlay"),
            AlertProvider::Custom
        );
    }

    #[test]
    fn overlay_url_must_be_https_and_clean() {
        assert_eq!(
            validate_overlay_url("https://streamlabs.com/alert-box/v2/abc"),
            Ok(())
        );
        assert!(validate_overlay_url("http://streamlabs.com/alert-box/v2/abc").is_err());
        assert!(validate_overlay_url("javascript:alert(1)").is_err());
        assert!(validate_overlay_url("https://exa mple.com").is_err());
        assert!(validate_overlay_url("file:///tmp/x.html").is_err());
    }

    #[test]
    fn tokens_are_validated_structurally() {
        assert_eq!(validate_token(AlertProvider::Streamlabs, "abc123"), Ok(()));
        assert!(validate_token(AlertProvider::Streamlabs, "").is_err());
        assert!(validate_token(AlertProvider::StreamElements, "a b").is_err());
        assert!(validate_token(AlertProvider::Streamlabs, "a/b").is_err());
        assert_eq!(
            validate_token(AlertProvider::Custom, "whatever"),
            Err(AlertOverlayError::MissingUrl)
        );
    }

    #[test]
    fn build_overlay_url_combines_provider_and_token() {
        assert_eq!(
            build_overlay_url(AlertProvider::Streamlabs, "tok123", None).unwrap(),
            "https://streamlabs.com/alert-box/v2/tok123"
        );
        assert_eq!(
            build_overlay_url(AlertProvider::StreamElements, "tok123", None).unwrap(),
            "https://streamelements.com/overlay/tok123"
        );
        assert_eq!(
            build_overlay_url(
                AlertProvider::Custom,
                "",
                Some("https://cdn.example.com/alerts")
            )
            .unwrap(),
            "https://cdn.example.com/alerts"
        );
        assert!(build_overlay_url(AlertProvider::Custom, "", Some("http://x")).is_err());
        assert!(build_overlay_url(AlertProvider::Streamlabs, "", None).is_err());
    }

    #[test]
    fn browser_source_accepts_the_generated_url() {
        // The URL produced for the widgets must pass the browser source
        // validator, so import + navigate works end to end.
        for url in [
            build_overlay_url(AlertProvider::Streamlabs, "tok", None).unwrap(),
            build_overlay_url(AlertProvider::StreamElements, "tok", None).unwrap(),
        ] {
            assert_eq!(
                crate::browser_source::BrowserSource::validate_url(&url),
                Ok(())
            );
        }
    }
}
