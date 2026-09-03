//! Update checking and installation for Rivulet.
//!
//! The updater queries the GitHub Releases API for the newest Rivulet release,
//! compares it with the running version and downloads/launches the matching
//! platform package (MSI on Windows, AppImage on Linux, DMG on macOS).
//!
//! Rivulet publishes all releases as prereleases (alpha channel), so the
//! updater reads the *latest release* from the `releases` endpoint (which
//! includes prereleases) instead of `releases/latest` (which does not).

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context as _;
use sha2::{Digest, Sha256};

/// Repository the updates are fetched from.
#[derive(Debug, Clone)]
pub struct UpdateSource {
    pub owner: String,
    pub repo: String,
    pub api_base: String,
}

impl Default for UpdateSource {
    fn default() -> Self {
        Self {
            owner: "thoser666".into(),
            repo: "rivulet".into(),
            api_base: "https://api.github.com".into(),
        }
    }
}

impl UpdateSource {
    /// Base URL for release *download* assets (the API base with the API host
    /// swapped for the regular GitHub host). Keeps tests hermetic: a local
    /// `api_base` yields a local download base.
    pub fn download_base(&self) -> String {
        self.api_base.replace("api.github.com", "github.com")
    }
}

/// A downloadable release asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// Information about an available update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Version of the new release, e.g. `"0.5.0-alpha.2"` (leading `v` stripped).
    pub version: String,
    /// Full release tag, e.g. `"v0.5.0-alpha.2"`.
    pub tag: String,
    /// Link to the release notes page.
    pub html_url: String,
    /// Platform package, if a matching asset was found.
    pub asset: Option<Asset>,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, serde::Deserialize)]
struct GitHubAsset {
    name: String,
    #[serde(default)]
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

/// Check whether a newer release exists for the given running version.
pub fn check_for_update(current: &str) -> anyhow::Result<Option<UpdateInfo>> {
    check_for_update_from(&UpdateSource::default(), current)
}

/// Same as [`check_for_update`], but against a custom source (used in tests).
pub fn check_for_update_from(
    source: &UpdateSource,
    current: &str,
) -> anyhow::Result<Option<UpdateInfo>> {
    let url = format!(
        "{}/repos/{}/{}/releases?per_page=1",
        source.api_base, source.owner, source.repo
    );
    let response = ureq::get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "rivulet-updater")
        .call()?;
    let body = response.into_body().read_to_string()?;
    parse_latest_release(body.as_bytes(), current)
}

/// Parse the newest release from a GitHub API response body.
///
/// Returns `Ok(None)` when no release exists, the release is older than or
/// equal to `current`, or its version is not valid SemVer. Public so the
/// fuzz target (`fuzz/fuzz_targets/parse_latest_release.rs`) exercises the
/// same untrusted-network-input path as production code.
pub fn parse_latest_release(json: &[u8], current: &str) -> anyhow::Result<Option<UpdateInfo>> {
    let releases: Vec<GitHubRelease> = serde_json::from_slice(json)?;
    let Some(release) = releases.into_iter().next() else {
        return Ok(None);
    };
    let version = release.tag_name.trim_start_matches('v');
    let current_version = semver::Version::parse(current)
        .map_err(|e| anyhow::anyhow!("invalid current version `{current}`: {e}"))?;
    let Ok(new_version) = semver::Version::parse(version) else {
        return Ok(None);
    };
    if new_version <= current_version {
        return Ok(None);
    }
    let asset = release
        .assets
        .into_iter()
        .find(|a| asset_matches_platform(&a.name, std::env::consts::OS))
        .map(|a| Asset {
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
        });
    Ok(Some(UpdateInfo {
        version: version.to_string(),
        tag: release.tag_name,
        html_url: release.html_url,
        asset,
    }))
}

/// Whether the asset name belongs to the given operating system.
fn asset_matches_platform(name: &str, os: &str) -> bool {
    match os {
        "windows" => name.ends_with(".msi") && name.contains("windows-x86_64"),
        "linux" => name.ends_with(".AppImage") && name.contains("linux-x86_64"),
        "macos" => name.ends_with(".dmg") && name.contains("macos-aarch64"),
        _ => false,
    }
}

/// Name of the checksum manifest attached to every release by the release
/// workflow. Every installer must be verified against it before it is run.
pub const CHECKSUMS_ASSET_NAME: &str = "SHA256SUMS";

/// Build the release-download URL of the checksum manifest for `tag`.
fn checksums_url(source: &UpdateSource, tag: &str) -> String {
    format!(
        "{}/{}/{}/releases/download/{}/{}",
        source.download_base(),
        source.owner,
        source.repo,
        tag,
        CHECKSUMS_ASSET_NAME
    )
}

/// Parse a `SHA256SUMS` manifest into `(filename, hex-digest)` pairs.
///
/// Accepts the standard `sha256sum` output format (`<digest>  <name>`, with
/// an optional `*` binary-mode marker) and skips blank or malformed lines so
/// a single stray line cannot hide the entry we are looking for.
pub fn parse_checksums(manifest: &str) -> Vec<(String, String)> {
    manifest
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(2, char::is_whitespace);
            let digest = parts.next()?.trim().to_string();
            let name = parts
                .next()?
                .trim()
                .trim_start_matches('*')
                .trim()
                .to_string();
            // NUL bytes cannot appear in a real manifest (the digest is hex
            // and the name is a release asset path); a hostile manifest that
            // smuggles one in is malformed and must be skipped, not emitted.
            if digest.is_empty()
                || name.is_empty()
                || digest.contains('\u{0}')
                || name.contains('\u{0}')
            {
                return None;
            }
            Some((name, digest))
        })
        .collect()
}

/// Hex-encode a SHA-256 digest.
fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Verify that the file at `path` has the given SHA-256 digest.
///
/// The expected digest must be a well-formed 64-character hex string; a
/// malformed manifest entry is rejected *before* hashing (fail closed).
pub fn verify_checksum(path: &Path, expected: &str) -> anyhow::Result<()> {
    let expected = expected.trim().to_ascii_lowercase();
    let well_formed = expected.len() == 64
        && expected
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !well_formed {
        anyhow::bail!("checksum manifest contains an invalid sha256 digest: {expected:?}");
    }
    let data = std::fs::read(path)
        .map_err(|e| anyhow::anyhow!("cannot read {} for checksum: {e}", path.display()))?;
    let actual = sha256_hex(&data);
    if actual != expected {
        anyhow::bail!(
            "checksum mismatch for {}: expected {expected}, got {actual} — the download is corrupt or was tampered with; the installer was NOT started",
            path.display()
        )
    }
    Ok(())
}

/// Download the checksum manifest for the release `tag` from the default
/// source (`https://github.com/<owner>/<repo>/releases/download/<tag>/SHA256SUMS`).
pub fn download_checksums(tag: &str) -> anyhow::Result<String> {
    download_checksums_from(&UpdateSource::default(), tag)
}

/// Same as [`download_checksums`], but against a custom source (used in tests).
pub fn download_checksums_from(source: &UpdateSource, tag: &str) -> anyhow::Result<String> {
    fetch_checksums_from_url(&checksums_url(source, tag))
}

/// Fetch the checksum manifest body from an explicit URL.
fn fetch_checksums_from_url(url: &str) -> anyhow::Result<String> {
    let response = ureq::get(url)
        .header("User-Agent", "rivulet-updater")
        .call()?;
    Ok(response.into_body().read_to_string()?)
}

/// Verify a downloaded release asset against the release's checksum manifest.
///
/// Downloads `SHA256SUMS` for `tag`, looks up `asset_name` and compares the
/// digest with the file at `path`. Fails when the manifest cannot be fetched,
/// the asset is not listed, or the digest differs — in every failure case the
/// caller must NOT run the installer.
pub fn verify_downloaded_asset(path: &Path, tag: &str, asset_name: &str) -> anyhow::Result<()> {
    verify_downloaded_asset_from(&UpdateSource::default(), path, tag, asset_name)
}

/// Same as [`verify_downloaded_asset`], but against a custom source (tests).
pub fn verify_downloaded_asset_from(
    source: &UpdateSource,
    path: &Path,
    tag: &str,
    asset_name: &str,
) -> anyhow::Result<()> {
    let manifest = download_checksums_from(source, tag)?;
    let expected = parse_checksums(&manifest)
        .into_iter()
        .find(|(name, _)| name == asset_name)
        .map(|(_, digest)| digest)
        .with_context(|| {
            format!(
                "{CHECKSUMS_ASSET_NAME} for {tag} has no entry for {asset_name}; refusing to install an unverified file"
            )
        })?;
    verify_checksum(path, &expected)
}

/// Download the asset to `dest`.
pub fn download_asset(asset: &Asset, dest: &Path) -> anyhow::Result<()> {
    let response = ureq::get(&asset.url)
        .header("User-Agent", "rivulet-updater")
        .call()?;
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(dest)?;
    std::io::copy(&mut reader, &mut file)?;
    Ok(())
}

/// Download the asset to `dest`, updating `progress` with bytes downloaded.
pub fn download_asset_with_progress(
    asset: &Asset,
    dest: &Path,
    progress: Arc<AtomicU64>,
) -> anyhow::Result<()> {
    let response = ureq::get(&asset.url)
        .header("User-Agent", "rivulet-updater")
        .call()?;
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(dest)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        progress.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(())
}

/// Windows Installer exit status indicating success with a reboot required.
pub const WINDOWS_INSTALLER_REBOOT_REQUIRED: i32 = 3010;

/// Classify a Windows Installer exit code. MSI 3010 is a successful install;
/// it only indicates that Windows recommends a reboot to finish applying it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn windows_installer_exit_success(code: i32) -> bool {
    code == 0 || code == WINDOWS_INSTALLER_REBOOT_REQUIRED
}

/// Launch the platform installer for a downloaded package.
///
/// Returns `Ok(true)` when the application should quit right afterwards (the
/// installer replaces the running files) and `Ok(false)` when it can stay open
/// (the macOS DMG is only opened).
///
/// On Windows this launches the [`rivulet-updater`] watchdog binary detached.
/// The watchdog waits for the watched process ids (the running GUI and its
/// launcher) to fully terminate, then runs `msiexec` to completion. Because
/// the executing files are locked by Windows while they run, installation must
/// not start until every Rivulet process has exited — launching msiexec
/// directly from the still-running GUI leaves the old files in place (only the
/// registry/shortcuts are updated), which is the exact bug this design fixes.
pub fn install_asset(path: &Path) -> anyhow::Result<bool> {
    install_asset_with_watch(path, None, &[])
}

/// Install the downloaded asset, delegating to a watchdog binary that waits for
/// the given process ids to exit before starting the platform installer.
///
/// `watchdog` overrides the path to the watchdog executable (used in tests);
/// when `None` it is resolved next to the running executable.
pub fn install_asset_with_watch(
    path: &Path,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] watchdog: Option<&Path>,
    #[cfg_attr(not(target_os = "windows"), allow(unused_variables))] wait_pids: &[u32],
) -> anyhow::Result<bool> {
    if !path.exists() {
        anyhow::bail!("installer file does not exist: {}", path.display());
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        let path_str = path.to_string_lossy().to_string();

        // The caller (GUI) must quit AFTER this returns, so the watchdog takes
        // over the install. If no watchdog binary is available we fall back to
        // starting msiexec detached (the previous, fragile behaviour).
        let watchdog_path = watchdog.map(ToOwned::to_owned).or_else(resolve_watchdog);
        let Some(watchdog_path) = watchdog_path else {
            // Fallback: launch msiexec directly, detached. Not ideal (see above)
            // but keeps the flow functional if the watchdog is missing.
            std::process::Command::new("msiexec.exe")
                .args([
                    "/i",
                    &path_str,
                    "/passive",
                    "/norestart",
                    "REBOOT=ReallySuppress",
                ])
                .creation_flags(0x08000000)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            return Ok(true);
        };

        let mut cmd = std::process::Command::new(&watchdog_path);
        cmd.arg("--install").arg(&path_str);
        for pid in wait_pids {
            cmd.arg("--wait-pid").arg(pid.to_string());
        }
        cmd.creation_flags(0x08000000);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.spawn()?;
        Ok(true)
    }

    #[cfg(target_os = "linux")]
    {
        let _ = watchdog;
        let _ = wait_pids;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        let mut child = std::process::Command::new(path).spawn()?;
        // Wait for the AppImage to finish extracting/running so the file
        // is safe to clean up.
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("installer exited with {status}");
        }
        Ok(true)
    }

    #[cfg(target_os = "macos")]
    {
        let _ = watchdog;
        let _ = wait_pids;
        let mut child = std::process::Command::new("open").arg(path).spawn()?;
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("open exited with {status} — is the DMG file valid?");
        }
        Ok(false)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        let _ = watchdog;
        let _ = wait_pids;
        anyhow::bail!("automatic updates are not supported on this platform")
    }
}

/// Resolve the path to the `rivulet-updater` watchdog binary, preferring the
/// directory of the running executable (the install folder / dev target dir).
/// An explicit `RIVULET_UPDATER_PATH` environment variable takes precedence.
#[cfg(target_os = "windows")]
fn resolve_watchdog() -> Option<std::path::PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .parent()
        .map(ToOwned::to_owned)?;
    std::env::var_os("RIVULET_UPDATER_PATH")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let candidate = exe_dir.join("rivulet-updater.exe");
            candidate.exists().then_some(candidate)
        })
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
fn resolve_watchdog() -> Option<std::path::PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const RELEASE_JSON: &str = r#"[
      {
        "tag_name": "v0.5.0-alpha.2",
        "html_url": "https://github.com/thoser666/rivulet/releases/tag/v0.5.0-alpha.2",
        "prerelease": true,
        "assets": [
          { "name": "rivulet-windows-x86_64.msi", "browser_download_url": "https://example.com/r.msi", "size": 42 },
          { "name": "rivulet-linux-x86_64.AppImage", "browser_download_url": "https://example.com/r.AppImage", "size": 43 },
          { "name": "rivulet-macos-aarch64.dmg", "browser_download_url": "https://example.com/r.dmg", "size": 44 }
        ]
      }
    ]"#;

    #[test]
    fn newer_release_returns_update_info() {
        let info = parse_latest_release(RELEASE_JSON.as_bytes(), "0.5.0-alpha.1")
            .unwrap()
            .unwrap();
        assert_eq!(info.version, "0.5.0-alpha.2");
        assert_eq!(info.tag, "v0.5.0-alpha.2");
        assert_eq!(
            info.html_url,
            "https://github.com/thoser666/rivulet/releases/tag/v0.5.0-alpha.2"
        );
        assert_eq!(info.asset.unwrap().name, expected_asset_name());
    }

    #[test]
    fn same_or_older_release_is_ignored() {
        assert!(
            parse_latest_release(RELEASE_JSON.as_bytes(), "0.5.0-alpha.2")
                .unwrap()
                .is_none()
        );
        assert!(parse_latest_release(RELEASE_JSON.as_bytes(), "1.0.0")
            .unwrap()
            .is_none());
    }

    #[test]
    fn empty_releases_return_none() {
        assert!(parse_latest_release(b"[]", "0.5.0-alpha.1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn malformed_json_is_an_error() {
        assert!(parse_latest_release(b"not json", "0.5.0-alpha.1").is_err());
    }

    #[test]
    fn invalid_current_version_is_an_error() {
        assert!(parse_latest_release(RELEASE_JSON.as_bytes(), "not-a-version").is_err());
    }

    #[test]
    fn invalid_release_version_is_ignored() {
        let json = br#"[{"tag_name":"nonsense","assets":[]}]"#;
        assert!(parse_latest_release(json, "0.5.0-alpha.1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn asset_matches_platform_selects_packages() {
        assert!(asset_matches_platform(
            "rivulet-windows-x86_64.msi",
            "windows"
        ));
        assert!(!asset_matches_platform(
            "rivulet-windows-x86_64-portable.zip",
            "windows"
        ));
        assert!(asset_matches_platform(
            "rivulet-linux-x86_64.AppImage",
            "linux"
        ));
        assert!(asset_matches_platform("rivulet-macos-aarch64.dmg", "macos"));
        assert!(!asset_matches_platform(
            "rivulet-macos-aarch64.dmg",
            "windows"
        ));
        assert!(!asset_matches_platform(
            "rivulet-windows-x86_64.msi",
            "linux"
        ));
    }

    #[test]
    fn check_fetches_from_github_releases_endpoint() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = RELEASE_JSON.to_string();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        });

        let source = UpdateSource {
            owner: "thoser666".into(),
            repo: "rivulet".into(),
            api_base: format!("http://{addr}"),
        };
        let info = check_for_update_from(&source, "0.5.0-alpha.1")
            .unwrap()
            .unwrap();
        assert_eq!(info.version, "0.5.0-alpha.2");
    }

    #[test]
    fn download_asset_with_progress_tracks_bytes() {
        let payload = vec![0xABu8; 256 * 1024]; // 256 KB payload
        let body_len = payload.len();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_len
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&payload);
        });

        let asset = Asset {
            name: "test.bin".into(),
            url: format!("http://{addr}/test.bin"),
            size: body_len as u64,
        };
        let progress = Arc::new(AtomicU64::new(0));
        let dest = std::env::temp_dir().join("rivulet_test_download.bin");
        let result = download_asset_with_progress(&asset, &dest, Arc::clone(&progress));
        assert!(result.is_ok());
        assert_eq!(progress.load(Ordering::Relaxed), body_len as u64);
        assert_eq!(std::fs::metadata(&dest).unwrap().len() as usize, body_len);
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn download_asset_with_progress_starts_at_zero() {
        let payload = b"hello".to_vec();
        let body_len = payload.len();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_len
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&payload);
        });

        let asset = Asset {
            name: "tiny.bin".into(),
            url: format!("http://{addr}/tiny.bin"),
            size: 999,
        };
        let progress = Arc::new(AtomicU64::new(0));
        let dest = std::env::temp_dir().join("rivulet_test_tiny.bin");
        download_asset_with_progress(&asset, &dest, Arc::clone(&progress)).unwrap();
        assert_eq!(progress.load(Ordering::Relaxed), body_len as u64);
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn download_asset_with_progress_invalid_url_fails() {
        let asset = Asset {
            name: "fail.bin".into(),
            url: "http://127.0.0.1:1/nonexistent".into(),
            size: 100,
        };
        let progress = Arc::new(AtomicU64::new(0));
        let dest = std::env::temp_dir().join("rivulet_test_fail.bin");
        let result = download_asset_with_progress(&asset, &dest, Arc::clone(&progress));
        assert!(result.is_err());
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_install_delegates_to_watchdog() {
        // The Windows implementation must launch the detached watchdog binary
        // (not msiexec directly), pass the MSI path and the watched pids, and
        // return immediately so the GUI can close cleanly.
        let source = include_str!("lib.rs");
        assert!(source.contains("resolve_watchdog"));
        assert!(source.contains("--install"));
        assert!(source.contains("--wait-pid"));
        assert!(source.contains("creation_flags(0x08000000)"));
        assert!(source.contains("Stdio::null()"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn watchdog_delegates_to_msiexec_instead_of_lib() {
        // The watchdog binary itself (not the library) is what ultimately runs
        // msiexec, and it does so with a blocking wait. Verify the watchdog's
        // own sources by checking the bin file's contents.
        let bin = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/bin/rivulet-updater.rs"
        ));
        assert!(bin.contains("msiexec.exe"));
        assert!(
            bin.contains("child.wait()"),
            "watchdog must block until msiexec exits"
        );
        assert!(bin.contains("std::process::exit(code)"));
    }

    #[test]
    fn install_asset_returns_error_for_nonexistent_file() {
        let fake = std::env::temp_dir().join("rivulet_test_nonexistent.msi");
        let result = install_asset(&fake);
        assert!(
            result.is_err(),
            "install_asset must fail for nonexistent file"
        );
    }

    #[test]
    fn windows_installer_success_includes_reboot_required() {
        assert!(windows_installer_exit_success(0));
        assert!(windows_installer_exit_success(3010));
        assert!(!windows_installer_exit_success(1603));
        assert!(!windows_installer_exit_success(1));
    }

    #[test]
    fn install_asset_waits_for_process() {
        // Create a small script/batch that sleeps briefly, then verify
        // install_asset blocks until it exits.
        #[cfg(target_os = "linux")]
        {
            let script = std::env::temp_dir().join("rivulet_test_install_wait.sh");
            std::fs::write(&script, "#!/bin/sh\nsleep 0.1").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            let start = std::time::Instant::now();
            let result = install_asset(&script);
            let elapsed = start.elapsed();
            assert!(result.is_ok(), "install_asset should succeed: {result:?}");
            assert!(
                elapsed.as_millis() >= 50,
                "install_asset should have waited for the child (elapsed: {elapsed:?})"
            );
            let _ = std::fs::remove_file(&script);
        }
    }

    /// The asset name the tests expect for the host platform.
    fn expected_asset_name() -> String {
        match std::env::consts::OS {
            "windows" => "rivulet-windows-x86_64.msi".to_string(),
            "linux" => "rivulet-linux-x86_64.AppImage".to_string(),
            "macos" => "rivulet-macos-aarch64.dmg".to_string(),
            _ => panic!("unsupported test platform"),
        }
    }

    #[test]
    fn parse_checksums_handles_standard_and_binary_mode_lines() {
        let manifest = "\
abc123  rivulet-windows-x86_64.msi\n\
\n\
def456 *rivulet-linux-x86_64.AppImage\n\
garbage-without-digest\n\
789aaa  \n\
";
        let parsed = parse_checksums(manifest);
        assert_eq!(
            parsed,
            vec![
                ("rivulet-windows-x86_64.msi".into(), "abc123".into()),
                ("rivulet-linux-x86_64.AppImage".into(), "def456".into()),
            ]
        );
    }

    #[test]
    fn parse_checksums_skips_nul_poisoned_entries() {
        // libFuzzer regression: a manifest with NUL bytes must never emit
        // entries containing NUL (the fuzz contract rejects them).
        let manifest = "abc123  \u{0}evil-name\n\u{0}digest  rivulet.msi\n\
def456  valid-name\n";
        let parsed = parse_checksums(manifest);
        assert_eq!(parsed, vec![("valid-name".into(), "def456".into())]);
        assert!(
            parsed
                .iter()
                .all(|(n, d)| !n.contains('\u{0}') && !d.contains('\u{0}')),
            "no parsed entry may carry a NUL byte"
        );
    }

    #[test]
    fn verify_checksum_accepts_matching_file_and_rejects_tampering() {
        let dir = std::env::temp_dir();
        let file = dir.join("rivulet_test_checksum_ok.bin");
        std::fs::write(&file, b"trusted installer payload").unwrap();

        let digest = sha256_hex(b"trusted installer payload");
        verify_checksum(&file, &digest).expect("matching digest must pass");

        let other = sha256_hex(b"tampered payload");
        let err = verify_checksum(&file, &other).unwrap_err();
        assert!(
            err.to_string().contains("checksum mismatch"),
            "tampered file must be rejected: {err}"
        );
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn verify_checksum_fails_closed_on_malformed_digests() {
        let file = std::env::temp_dir().join("rivulet_test_checksum_malformed.bin");
        std::fs::write(&file, b"payload").unwrap();
        for bad in ["", "zz", &"a".repeat(63), &"A".repeat(63)] {
            let err = verify_checksum(&file, bad).expect_err("malformed digest must fail closed");
            assert!(
                err.to_string().contains("invalid sha256 digest"),
                "unexpected error for {bad:?}: {err}"
            );
        }
        // Uppercase hex IS accepted (normalized to lowercase).
        let digest = sha256_hex(b"payload");
        verify_checksum(&file, &digest.to_uppercase()).expect("uppercase hex must pass");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn verify_downloaded_asset_end_to_end_against_local_manifest() {
        let payload = b"the real installer bytes";
        let digest = sha256_hex(payload);
        let manifest = format!(
            "{digest}  rivulet-windows-x86_64.msi\n{digest}  rivulet-linux-x86_64.AppImage\n"
        );
        let body = manifest.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        // Every verify call fetches the manifest, so the server must accept
        // several sequential connections.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body.as_bytes());
            }
        });

        let source = UpdateSource {
            owner: "thoser666".into(),
            repo: "rivulet".into(),
            api_base: format!("http://{addr}"),
        };
        assert_eq!(source.download_base(), format!("http://{addr}"));

        let file = std::env::temp_dir().join("rivulet_test_verified_asset.bin");
        std::fs::write(&file, payload).unwrap();
        verify_downloaded_asset_from(&source, &file, "v9.9.9", "rivulet-windows-x86_64.msi")
            .expect("listed asset with matching digest must verify");

        // Tampered file must be rejected.
        std::fs::write(&file, b"evil bytes").unwrap();
        let err =
            verify_downloaded_asset_from(&source, &file, "v9.9.9", "rivulet-windows-x86_64.msi")
                .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");

        // Asset missing from the manifest must fail closed.
        let err = verify_downloaded_asset_from(&source, &file, "v9.9.9", "unknown-asset.bin")
            .unwrap_err();
        assert!(err.to_string().contains("no entry"), "{err}");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn checksums_url_points_at_the_release_download_folder() {
        let url = checksums_url(&UpdateSource::default(), "v1.2.3");
        assert_eq!(
            url,
            "https://github.com/thoser666/rivulet/releases/download/v1.2.3/SHA256SUMS"
        );
    }
}
