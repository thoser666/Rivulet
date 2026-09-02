//! Regression tests for the generated branding assets (thumbnail, Linux icon,
//! macOS .icns) and their packaging wiring.
//!
//! These tests keep the committed binaries honest: the macOS app icon must be
//! a structurally valid ICNS container (correct magic, chunk framing and a
//! PNG per supported size), the generator script must actually produce it, and
//! `build-app.sh` must copy it into the app bundle as `AppIcon.icns`.

use std::fs;
use std::path::{Path, PathBuf};

/// Resolve a repository-relative path from the `rivulet-core` manifest dir.
fn repo_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo_file(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
}

/// The PNG-based ICNS types produced by `scripts/generate-assets.sh`,
/// mirroring `iconutil`'s output, mapped to the expected pixel size.
const ICON_TYPES: &[(&str, u32)] = &[
    ("icp4", 16),
    ("icp5", 32),
    ("ic07", 128),
    ("ic08", 256),
    ("ic09", 512),
    ("ic10", 1024),
    ("ic11", 32),
    ("ic12", 64),
    ("ic13", 256),
    ("ic14", 512),
];

fn u32_be(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("4-byte window"))
}

/// Width/height from a PNG's IHDR chunk (offsets 16 and 20).
fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    (u32_be(bytes, 16), u32_be(bytes, 20))
}

/// Content digest of the pinned ImageMagick image (manifest-list digest, so it
/// stays multi-arch). ImageMagick publishes no official container image, so the
/// tag is additionally pinned by digest to guard against tag mutation.
const IMAGEMAGICK_DIGEST: &str =
    "sha256:87998ec1b8127b2f73f626f74f7b05e8827f9d7605fa52da5370588f7e53cee1";

#[test]
fn macos_icns_is_structurally_valid() {
    let bytes = fs::read(repo_file("packaging/rivulet.icns"))
        .expect("packaging/rivulet.icns must be committed");

    assert_eq!(&bytes[..4], b"icns", "ICNS must start with the icns magic");
    let total = u32_be(&bytes, 4) as usize;
    assert_eq!(total, bytes.len(), "ICNS length field must match file size");

    let mut seen: Vec<String> = Vec::new();
    let mut offset = 8usize;
    while offset < bytes.len() {
        assert!(
            offset + 8 <= bytes.len(),
            "truncated chunk header at offset {offset}"
        );
        let type_code = std::str::from_utf8(&bytes[offset..offset + 4])
            .expect("chunk type is ASCII")
            .to_string();
        let length = u32_be(&bytes, offset + 4) as usize;
        assert!(length >= 8, "{type_code}: chunk length {length} < 8");
        assert!(
            offset + length <= bytes.len(),
            "{type_code}: chunk overruns file"
        );

        let payload = &bytes[offset + 8..offset + length];
        assert!(
            payload.starts_with(b"\x89PNG\r\n\x1a\n"),
            "{type_code}: payload must be a PNG"
        );
        let expected = ICON_TYPES
            .iter()
            .find(|(t, _)| *t == type_code)
            .unwrap_or_else(|| panic!("unexpected ICNS type {type_code}"));
        let (width, height) = (u32_be(payload, 16), u32_be(payload, 20));
        let expected_size = expected.1;
        assert_eq!(
            (width, height),
            (expected_size, expected_size),
            "{type_code}: PNG must be {expected_size}x{expected_size}, got {width}x{height}"
        );
        seen.push(type_code);
        offset += length;
    }

    assert_eq!(offset, bytes.len(), "trailing bytes after last chunk");
    let mut sorted_seen = seen.clone();
    sorted_seen.sort();
    let mut sorted_expected: Vec<String> = ICON_TYPES.iter().map(|(t, _)| t.to_string()).collect();
    sorted_expected.sort();
    assert_eq!(
        sorted_seen, sorted_expected,
        "ICNS must contain exactly the canonical 10 icon types"
    );
}

#[test]
fn generator_script_produces_all_assets() {
    let script = read("scripts/generate-assets.sh");
    assert!(
        script.contains("packaging/rivulet.icns"),
        "generate-assets.sh must emit the macOS icon"
    );
    assert!(
        script.contains("png2icns.py"),
        "generate-assets.sh must pack the ICNS via png2icns.py"
    );
    assert!(
        script.contains("docs/social-preview.png"),
        "generate-assets.sh must emit the social preview"
    );
    assert!(
        script.contains("docs/opengraph.png"),
        "generate-assets.sh must emit the OpenGraph fallback"
    );
    assert!(
        script.contains("docs/assets/rivulet-app-icon-512.png"),
        "generate-assets.sh must emit the Discord app icon"
    );

    let packer = read("scripts/png2icns.py");
    assert!(
        packer.contains("icns") && packer.contains("struct.pack"),
        "png2icns.py must assemble the ICNS container"
    );
}

#[test]
fn build_app_wires_the_icon_into_the_bundle() {
    let script = read("packaging/macos/build-app.sh");
    assert!(
        script.contains("packaging/rivulet.icns"),
        "build-app.sh must read the committed ICNS"
    );
    assert!(
        script.contains("Contents/Resources/AppIcon.icns"),
        "build-app.sh must install the icon as AppIcon.icns"
    );

    // The Info.plist must reference the installed icon so macOS finds it.
    assert!(
        script.contains("CFBundleIconFile") && script.contains("<string>AppIcon</string>"),
        "Info.plist must declare CFBundleIconFile=AppIcon"
    );
}

#[test]
fn other_branding_assets_exist() {
    let thumbnail = repo_file("docs/thumbnail.png");
    let linux_icon = repo_file("packaging/rivulet.png");
    let social = repo_file("docs/social-preview.png");
    let opengraph = repo_file("docs/opengraph.png");
    assert!(thumbnail.exists(), "docs/thumbnail.png must be committed");
    assert!(
        linux_icon.exists(),
        "packaging/rivulet.png must be committed"
    );
    assert!(social.exists(), "docs/social-preview.png must be committed");
    assert!(opengraph.exists(), "docs/opengraph.png must be committed");

    let mut header = [0u8; 8];
    for path in [&thumbnail, &linux_icon, &social, &opengraph] {
        let bytes = fs::read(path).expect("asset readable");
        header.copy_from_slice(&bytes[..8]);
        assert_eq!(
            &header,
            b"\x89PNG\r\n\x1a\n",
            "{} must be a PNG",
            path.display()
        );
    }
}

#[test]
fn discord_app_icon_contract_holds() {
    let bytes = fs::read(repo_file("docs/assets/rivulet-app-icon-512.png"))
        .expect("docs/assets/rivulet-app-icon-512.png must be committed");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "must be a PNG");
    let (width, height) = png_dimensions(&bytes);
    assert_eq!(
        (width, height),
        (512, 512),
        "Discord app icons must be 512x512 (Discord's recommended size)"
    );
}

#[test]
fn social_preview_has_opengraph_dimensions() {
    let bytes = fs::read(repo_file("docs/social-preview.png"))
        .expect("docs/social-preview.png must be committed");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "must be a PNG");
    assert_eq!(
        png_dimensions(&bytes),
        (1280, 640),
        "GitHub social previews must be 1280x640 (2:1)"
    );
}
#[test]
fn opengraph_fallback_has_standard_dimensions() {
    let bytes =
        fs::read(repo_file("docs/opengraph.png")).expect("docs/opengraph.png must be committed");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "must be a PNG");
    assert_eq!(
        png_dimensions(&bytes),
        (1200, 630),
        "OpenGraph fallbacks must be 1200x630 (1.91:1)"
    );
}

#[test]
fn release_workflows_attach_social_images() {
    // There is no public API for the Settings -> Social preview upload, so the
    // images are attached to every release instead: that gives them a stable
    // URL (releases/latest/download/...) a website can point og:image at.
    for name in ["ci.yml", "release.yml"] {
        let content = read(&format!(".github/workflows/{name}"));
        assert!(
            content.contains("docs/opengraph.png") && content.contains("docs/social-preview.png"),
            "{name} must attach the social images to GitHub releases"
        );
    }
}

#[test]
fn asset_drift_check_is_wired_up() {
    let checker = read("scripts/check-assets.py");
    assert!(
        checker.contains("git show") && checker.contains("RGBA"),
        "check-assets.py must compare decoded pixels against HEAD"
    );
    // The Discord app icon must be part of the drift check so it is
    // regenerated together with the rest of the brand assets.
    assert!(
        checker.contains("docs/assets/rivulet-app-icon-512.png"),
        "check-assets.py must verify the Discord app icon against HEAD"
    );
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("scripts/generate-assets.sh") && ci.contains("scripts/check-assets.py"),
        "CI must regenerate assets and check them against the committed files"
    );
    assert!(
        ci.contains(IMAGEMAGICK_DIGEST),
        "the asset check must pin the ImageMagick image by content digest"
    );
    assert!(
        ci.contains("safe.directory"),
        "the pinned container must mark the mounted repo as a safe git directory"
    );
    assert!(
        ci.contains("--entrypoint bash"),
        "the CI docker run must override the entrypoint to bash, otherwise \
         the ImageMagick entrypoint intercepts the command"
    );

    let generator = read("scripts/generate-assets.sh");
    assert!(
        generator.contains("fonts/DejaVuSans-Bold.ttf")
            && generator.contains("fonts/DejaVuSans.ttf"),
        "generate-assets.sh must pin the committed fonts so text is reproducible"
    );
    assert!(
        generator.contains("-filter Mitchell"),
        "generate-assets.sh must pin the resize filter for cross-version determinism"
    );
}

#[test]
fn installers_ship_discord_assets_and_docs() {
    // The Discord Rich Presence setup files (app icon for the member list,
    // Rich Presence artwork for the profile card, setup docs) must ship in
    // every installer so users can complete the configuration offline.
    let required = [
        "docs/assets/rivulet-app-icon-512.png",
        "docs/assets/rivulet-rich-presence-1024.png",
        "docs/assets/rivulet-rich-presence-512.png",
        "docs/activity-status.md",
    ];
    for rel in required {
        assert!(
            repo_file(rel).exists(),
            "{rel} must be committed so the installers can ship it"
        );
    }

    // Windows: the portable bundle step copies the files into the bundle dir;
    // the MSI harvests the whole bundle dir, so the bundle is the single
    // staging point for both Windows installers.
    let portable = read("packaging/windows/build-portable.ps1");
    for needle in [
        "rivulet-app-icon-512.png",
        "rivulet-rich-presence-1024.png",
        "rivulet-rich-presence-512.png",
        "activity-status.md",
        "$discordDir",
    ] {
        assert!(
            portable.contains(needle),
            "build-portable.ps1 must stage {needle} for the Discord setup"
        );
    }

    // Linux: the AppImage ships them under usr/share/doc/rivulet/discord.
    let appimage = read("packaging/linux/build-appimage.sh");
    assert!(
        appimage.contains("usr/share/doc/rivulet/discord"),
        "build-appimage.sh must install the Discord assets into the AppImage doc dir"
    );
    assert!(
        appimage.contains("rivulet-app-icon-512.png")
            && appimage.contains("rivulet-rich-presence-1024.png")
            && appimage.contains("activity-status.md"),
        "build-appimage.sh must copy the Discord setup files"
    );

    // macOS: the app bundle ships them under Contents/Resources/discord.
    let app = read("packaging/macos/build-app.sh");
    assert!(
        app.contains("Contents/Resources/discord"),
        "build-app.sh must install the Discord assets into the bundle Resources"
    );
    assert!(
        app.contains("rivulet-app-icon-512.png")
            && app.contains("rivulet-rich-presence-1024.png")
            && app.contains("activity-status.md"),
        "build-app.sh must copy the Discord setup files"
    );

    // The formal GitHub release additionally attaches the three files so they
    // are downloadable without installing anything.
    let release = read(".github/workflows/release.yml");
    assert!(
        release.contains("docs/assets/rivulet-app-icon-512.png")
            && release.contains("docs/assets/rivulet-rich-presence-1024.png")
            && release.contains("docs/activity-status.md"),
        "release.yml must attach the Discord setup files as release assets"
    );
}

#[test]
fn docker_wrapper_regenerates_assets_in_the_pinned_image() {
    let wrapper = read("scripts/generate-assets-docker.sh");
    assert!(
        wrapper.contains(IMAGEMAGICK_DIGEST),
        "the docker wrapper must pin the ImageMagick image by content digest"
    );
    assert!(
        wrapper.contains("scripts/generate-assets.sh"),
        "the docker wrapper must invoke the generator script"
    );
    assert!(
        wrapper.contains("--check") && wrapper.contains("scripts/check-assets.py"),
        "the docker wrapper's --check mode must verify output against the committed files"
    );
    assert!(
        wrapper.contains("docker") && wrapper.contains("podman"),
        "the docker wrapper must support both Docker and Podman runtimes"
    );
}
