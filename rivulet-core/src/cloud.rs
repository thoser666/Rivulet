//! Cloud-recording configuration contract (S3-compatible destinations).
//!
//! Cloud recordings upload finished recordings to an S3-compatible object
//! store (AWS S3, MinIO, R2, Backblaze B2, ...). This module provides the
//! validated *contract*: an endpoint/bucket/region/prefix plus access
//! credentials, a deterministic object-key builder, and credential masking so
//! secrets never leak into logs or `Debug` output — all testable without any
//! network access.
//!
//! **Scope note:** the actual upload (S3 `PUT` against the endpoint) is a
//! follow-up; this module deliberately stops at the validated, secret-safe
//! configuration that the uploader and the GUI can target.

use std::fmt;

/// Maximum accepted bucket length (S3 allows 3..=63 characters).
pub const MAX_BUCKET_LEN: usize = 63;
/// Minimum accepted bucket length.
pub const MIN_BUCKET_LEN: usize = 3;

/// S3-compatible cloud destination for finished recordings.
///
/// `Debug` is implemented manually so the secret access key is never printed.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct CloudRecording {
    /// Base endpoint, e.g. `https://s3.eu-central-1.amazonaws.com` or
    /// `https://minio.example.internal`. Must include a scheme.
    pub endpoint: String,
    /// Bucket name (S3 naming rules: lowercase letters, digits, dots, hyphens).
    pub bucket: String,
    /// Optional region string, e.g. `eu-central-1`.
    pub region: Option<String>,
    /// Optional object-key prefix, e.g. `rivulet/recordings`. Slashes are
    /// normalised; the default is empty (bucket root).
    pub prefix: Option<String>,
    /// Access key id for the bucket. Masked in `Debug`/logs.
    pub access_key_id: String,
    /// Secret access key for the bucket. Masked in `Debug`/logs.
    pub secret_access_key: String,
    /// Whether uploading is enabled. Off by default so no recording is ever
    /// uploaded unintentionally.
    pub enabled: bool,
}

/// Mask a credential, keeping at most the first two characters.
pub fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let keep: String = value.chars().take(2).collect();
    format!("{keep}••••")
}

impl CloudRecording {
    pub fn new(endpoint: impl Into<String>, bucket: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            ..Default::default()
        }
    }

    pub fn with_credentials(mut self, id: impl Into<String>, secret: impl Into<String>) -> Self {
        self.access_key_id = id.into();
        self.secret_access_key = secret.into();
        self
    }

    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Validate the destination without touching the network.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.enabled {
            return Ok(());
        }
        if self.endpoint.trim().is_empty() {
            return Err("cloud endpoint must not be empty");
        }
        let has_scheme = self.endpoint.trim_start().starts_with("https://")
            || self.endpoint.trim_start().starts_with("http://");
        if !has_scheme {
            return Err("cloud endpoint must include a scheme (https:// or http://)");
        }
        if self.bucket.len() < MIN_BUCKET_LEN || self.bucket.len() > MAX_BUCKET_LEN {
            return Err("bucket name must be between 3 and 63 characters");
        }
        if !self
            .bucket
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        {
            return Err("bucket name may only contain lowercase letters, digits, dots and hyphens");
        }
        if self.access_key_id.trim().is_empty() || self.secret_access_key.trim().is_empty() {
            return Err("cloud credentials must be set when enabled");
        }
        Ok(())
    }

    /// Deterministic object key for a finished recording filename.
    ///
    /// The configured prefix is normalised (leading/trailing slashes removed)
    /// so `recordings/` and `recordings` produce the same key shape.
    pub fn upload_key_for(&self, filename: &str) -> String {
        let prefix = self.prefix.as_deref().unwrap_or("").trim_matches('/');
        let name = filename.trim_start_matches('/');
        if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        }
    }

    /// Credentials with the secret masked, safe for logs and `Debug` output.
    pub fn masked_credentials(&self) -> (String, String) {
        (
            mask_secret(&self.access_key_id),
            mask_secret(&self.secret_access_key),
        )
    }
}

impl fmt::Debug for CloudRecording {
    /// Never prints the raw secret access key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudRecording")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("prefix", &self.prefix)
            .field("access_key_id", &mask_secret(&self.access_key_id))
            .field("secret_access_key", &mask_secret(&self.secret_access_key))
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled_and_empty() {
        let c = CloudRecording::default();
        assert!(!c.enabled);
        assert!(c.validate().is_ok(), "disabled config needs no validation");
        assert_eq!(c.masked_credentials(), (String::new(), String::new()));
    }

    #[test]
    fn validation_enforces_endpoint_and_bucket_rules() {
        // Missing scheme -> error.
        let c = CloudRecording::new("s3.example.com", "bucket").enabled(true);
        assert!(c.validate().is_err());
        // Valid HTTPS endpoint + valid bucket -> ok.
        let c = CloudRecording::new("https://s3.example.com", "rivulet-recordings")
            .with_credentials("AKIA...", "s3cr3t")
            .enabled(true);
        assert!(c.validate().is_ok());
        // Bucket too short / invalid characters.
        assert!(CloudRecording::new("https://s3.example.com", "ab")
            .with_credentials("k", "s")
            .enabled(true)
            .validate()
            .is_err());
        assert!(CloudRecording::new("https://s3.example.com", "Bad_Bucket!")
            .with_credentials("k", "s")
            .enabled(true)
            .validate()
            .is_err());
        // Missing credentials.
        assert!(CloudRecording::new("https://s3.example.com", "bucket")
            .enabled(true)
            .validate()
            .is_err());
    }

    #[test]
    fn upload_key_joins_normalised_prefix() {
        let c = CloudRecording::new("https://s3.example.com", "bucket")
            .with_prefix("rivulet/recordings/");
        assert_eq!(
            c.upload_key_for("2026-08-31_game.mp4"),
            "rivulet/recordings/2026-08-31_game.mp4"
        );
        let c = CloudRecording::new("https://s3.example.com", "bucket");
        assert_eq!(c.upload_key_for("clip.mp4"), "clip.mp4");
        let c = CloudRecording::new("https://s3.example.com", "bucket").with_prefix("/lead/");
        assert_eq!(c.upload_key_for("/deep/clip.mp4"), "lead/deep/clip.mp4");
    }

    #[test]
    fn secrets_are_masked_in_debug_and_masked_credentials() {
        let c = CloudRecording::new("https://s3.example.com", "bucket")
            .with_credentials("AKIA1234567890", "super-secret-value");
        let (id, secret) = c.masked_credentials();
        assert_eq!(id, "AK••••");
        assert_eq!(secret, "su••••");
        assert!(!format!("{c:?}").contains("super-secret-value"));
        assert!(format!("{c:?}").contains("su••••"));
        assert!(!format!("{c:?}").contains("AKIA1234567890"));
    }

    #[test]
    fn mask_secret_handles_short_and_empty_values() {
        assert_eq!(mask_secret(""), "");
        assert_eq!(mask_secret("a"), "a••••");
        assert_eq!(mask_secret("ab"), "ab••••");
    }
}
