//! Update checking and installation for Rivulet.
//!
//! The updater queries the GitHub Releases API for the newest Rivulet release,
//! compares it with the running version and downloads/launches the matching
//! platform package (MSI on Windows, AppImage on Linux, DMG on macOS).
//!
//! Rivulet publishes all releases as prereleases (alpha channel), so the
//! updater reads the *latest release* from the `releases` endpoint (which
//! includes prereleases) instead of `releases/latest` (which does not).

use std::path::Path;

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
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "rivulet-updater")
        .call()?;
    let body = response.into_string()?;
    parse_latest_release(body.as_bytes(), current)
}

/// Parse the newest release from a GitHub API response body.
///
/// Returns `Ok(None)` when no release exists, the release is older than or
/// equal to `current`, or its version is not valid SemVer.
fn parse_latest_release(json: &[u8], current: &str) -> anyhow::Result<Option<UpdateInfo>> {
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

/// Download the asset to `dest`.
pub fn download_asset(asset: &Asset, dest: &Path) -> anyhow::Result<()> {
    let response = ureq::get(&asset.url)
        .set("User-Agent", "rivulet-updater")
        .call()?;
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(dest)?;
    std::io::copy(&mut reader, &mut file)?;
    Ok(())
}

/// Launch the platform installer for a downloaded package.
///
/// Returns `Ok(true)` when the application should quit right afterwards (the
/// installer replaces the running files) and `Ok(false)` when it can stay open
/// (the macOS DMG is only opened).
pub fn install_asset(path: &Path) -> anyhow::Result<bool> {
    #[cfg(target_os = "windows")]
    {
        let path = path.to_string_lossy();
        std::process::Command::new("msiexec")
            .args([
                "/i",
                path.as_ref(),
                "/passive",
                "/norestart",
                "REBOOT=ReallySuppress",
            ])
            .spawn()?;
        Ok(true)
    }

    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        std::process::Command::new(path).spawn()?;
        Ok(true)
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
        Ok(false)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("automatic updates are not supported on this platform")
    }
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

    /// The asset name the tests expect for the host platform.
    fn expected_asset_name() -> String {
        match std::env::consts::OS {
            "windows" => "rivulet-windows-x86_64.msi".to_string(),
            "linux" => "rivulet-linux-x86_64.AppImage".to_string(),
            "macos" => "rivulet-macos-aarch64.dmg".to_string(),
            _ => panic!("unsupported test platform"),
        }
    }
}
