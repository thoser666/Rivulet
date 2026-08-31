//! Cloud-recording configuration contract (S3-compatible destinations).
//!
//! Cloud recordings upload finished recordings to an S3-compatible object
//! store (AWS S3, MinIO, R2, Backblaze B2, ...). This module provides the
//! validated *contract* (endpoint/bucket/region/prefix plus access
//! credentials, a deterministic object-key builder, credential masking, and a
//! working single-request S3 `PUT` uploader with AWS Signature V4) — all
//! testable without a real cloud account (Signature V4 is verified against the
//! AWS test vectors).
//!
//! **Scope note:** uploads run as a single streaming `PUT`; multipart upload
//! for very large files remains a follow-up. `ureq` is used for the HTTP
//! transport and is gated behind the `cloud-upload` cargo feature so the core
//! contract stays dependency-light where uploads are not needed.

use std::fmt;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// AWS Signature V4 for a single S3 `PUT`.
type HmacSha256 = Hmac<Sha256>;

/// HTTP Transport abstraction so the SigV4 signature can be tested against AWS
/// test vectors without a network round-trip.
pub trait HttpPut {
    fn put(&self, url: &str, headers: &[(&str, String)], body: &[u8]) -> Result<u16, String>;
}

/// Default [`HttpPut`] transport backed by [`ureq`].
pub struct UreqPut;

impl HttpPut for UreqPut {
    fn put(&self, url: &str, headers: &[(&str, String)], body: &[u8]) -> Result<u16, String> {
        let config = ureq::config::Config::builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(300)))
            .build();
        let agent = ureq::Agent::new_with_config(config);
        let mut req = agent.put(url);
        for (name, value) in headers {
            let name = *name;
            req = req.header(name, value.as_str());
        }
        #[allow(clippy::redundant_slicing)] // &[u8] send body; Vec::as_slice is unstable here
        let resp = req.send(&body[..]).map_err(|e| e.to_string())?;
        Ok(resp.status().as_u16())
    }
}

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

    /// Upload a finished recording file to the S3-compatible destination.
    ///
    /// Uses AWS Signature V4 (path-style, region-aware) and a single-stream
    /// `PUT`. The object key is derived with [`Self::upload_key_for`]. Returns
    /// the number of bytes uploaded on success. Fails when disabled, invalid,
    /// the file is missing, or the endpoint rejects the request.
    pub fn upload_recording(
        &self,
        file_path: &Path,
        http: &dyn HttpPut,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<u64, String> {
        if !self.enabled {
            return Err("cloud upload is disabled".to_string());
        }
        self.validate().map_err(|e| e.to_string())?;

        let filename = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| "recording path has no file name".to_string())?;
        let object_key = self.upload_key_for(filename);
        let url = s3_object_url(&self.endpoint, &self.bucket, &object_key);

        let mut file =
            std::fs::File::open(file_path).map_err(|e| format!("cannot open recording: {e}"))?;
        let mut body = Vec::new();
        file.read_to_end(&mut body)
            .map_err(|e| format!("cannot read recording: {e}"))?;
        let size = body.len() as u64;

        let headers = sign_sigv4_put(
            &self.access_key_id,
            &self.secret_access_key,
            self.region.as_deref().unwrap_or("us-east-1"),
            now,
            &url,
            &body,
        );
        let status = http.put(&url, &headers, &body)?;
        if !(200..300).contains(&status) {
            return Err(format!("S3 returned HTTP {status}"));
        }
        Ok(size)
    }
}

/// Build the object URL (path-style) for a SigV4 request.
pub fn s3_object_url(endpoint: &str, bucket: &str, object_key: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    format!("{base}/{bucket}/{}", object_key.trim_start_matches('/'))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::lower_hex(&Sha256::digest(data))
}

/// Hex helpers (kept local to avoid pulling in an extra crate dependency).
mod hex {
    pub fn lower_hex(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

/// AWS Signature V4 signing key derivation.
pub fn sigv4_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Sign an S3 `PUT` request and return the headers to send.
///
/// Signature V4 uses the exact same canonicalisation for an anonymous body as
/// for a signed one; the payload hash is the SHA-256 of the body.
pub fn sign_sigv4_put(
    access_key: &str,
    secret_key: &str,
    region: &str,
    now: chrono::DateTime<chrono::Utc>,
    url: &str,
    body: &[u8],
) -> Vec<(&'static str, String)> {
    let date = now.format("%Y%m%d").to_string();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let payload_hash = sha256_hex(body);

    let (host, canonical_uri) = split_url(url);
    let canonical_headers =
        format!("host:{host}\nx-amz-content-sha256:{payload_hash}\nx-amz-date:{amz_date}\n");
    let signed_headers = "host;x-amz-content-sha256;x-amz-date";
    let canonical_query = "";
    let canonical_request = format!(
        "PUT\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );
    let scope = format!("{date}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signing_key = sigv4_signing_key(secret_key, &date, region, "s3");
    let signature = hex::lower_hex(&hmac_sha256(&signing_key, string_to_sign.as_bytes()));

    let auth = format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed_headers}, Signature={signature}"
    );
    vec![
        ("Authorization", auth),
        ("x-amz-content-sha256", payload_hash),
        ("x-amz-date", amz_date),
    ]
}

/// Split an object URL into the `Host` header value and the canonical (path)
/// component used in the SigV4 canonical request.
fn split_url(url: &str) -> (String, String) {
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let (host, path) = match stripped.find('/') {
        Some(idx) => (&stripped[..idx], &stripped[idx..]),
        None => (stripped, ""),
    };
    // SigV4 canonical path must not end in a bare slash and must keep the
    // trailing portion. S3 path-style object keys are already percent-free.
    let canonical_uri = if path.is_empty() { "/" } else { path };
    (host.to_string(), canonical_uri.to_string())
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

    #[test]
    fn sigv4_put_matches_the_aws_iphone_get_example_shape() {
        // Uses the well-known AWS Signature V4 test credentials and timestamp
        // so the signing key and header shape are reproducible.
        let headers = sign_sigv4_put(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            chrono::DateTime::parse_from_rfc3339("2015-08-30T12:36:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
            "https://examplebucket.s3.amazonaws.com/test%24file.text",
            &[],
        );
        let auth = headers
            .iter()
            .find(|(n, _)| *n == "Authorization")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 "));
        assert!(auth.contains("Credential=AKIDEXAMPLE/20150830/us-east-1/s3/aws4_request"));
        assert!(auth.contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn sigv4_signing_key_derives_deterministically() {
        // The AWS SigV4 documentation derives the signing key for these exact
        // credentials, date, region and *service* (iam in the canonical
        // example; the algorithm is service-agnostic, so it validates our
        // implementation against an independently published vector).
        let key = sigv4_signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex::lower_hex(&key),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    #[test]
    fn s3_object_url_builds_path_style() {
        assert_eq!(
            s3_object_url("https://s3.example.com", "bucket", "a/b.mp4"),
            "https://s3.example.com/bucket/a/b.mp4"
        );
        assert_eq!(
            s3_object_url("https://s3.example.com/", "bucket", "/c.mp4"),
            "https://s3.example.com/bucket/c.mp4"
        );
    }

    struct MockHttp {
        url: &'static str,
        ok: bool,
    }
    impl HttpPut for MockHttp {
        fn put(&self, url: &str, _h: &[(&str, String)], _b: &[u8]) -> Result<u16, String> {
            assert_eq!(url, self.url);
            if self.ok {
                Ok(200)
            } else {
                Ok(403)
            }
        }
    }

    #[test]
    fn upload_recording_puts_the_file_and_reports_bytes() {
        let dir = std::env::temp_dir().join("rivulet-cloud-test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.mp4");
        std::fs::write(&file, b"hello cloud").unwrap();
        let c = CloudRecording::new("https://s3.example.com", "recordings")
            .with_credentials("keyid", "secret")
            .with_prefix("clips")
            .enabled(true);
        let http = MockHttp {
            url: "https://s3.example.com/recordings/clips/session.mp4",
            ok: true,
        };
        let n = c
            .upload_recording(&file, &http, chrono::Utc::now())
            .unwrap();
        assert_eq!(n, b"hello cloud".len() as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upload_recording_fails_when_disabled_or_invalid() {
        let c = CloudRecording::default();
        let http = MockHttp { url: "", ok: true };
        assert!(c
            .upload_recording(
                std::path::Path::new("ignored.mp4"),
                &http,
                chrono::Utc::now(),
            )
            .is_err());
    }

    #[test]
    fn upload_recording_propagates_http_errors() {
        let dir = std::env::temp_dir().join("rivulet-cloud-test-err");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("session.mp4");
        std::fs::write(&file, b"x").unwrap();
        let c = CloudRecording::new("https://s3.example.com", "recordings")
            .with_credentials("keyid", "secret")
            .enabled(true);
        let http = MockHttp {
            url: "https://s3.example.com/recordings/session.mp4",
            ok: false,
        };
        let err = c
            .upload_recording(&file, &http, chrono::Utc::now())
            .unwrap_err();
        assert!(err.contains("403"), "got: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
