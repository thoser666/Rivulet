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
    assert!(thumbnail.exists(), "docs/thumbnail.png must be committed");
    assert!(
        linux_icon.exists(),
        "packaging/rivulet.png must be committed"
    );
    assert!(social.exists(), "docs/social-preview.png must be committed");

    let mut header = [0u8; 8];
    for path in [&thumbnail, &linux_icon, &social] {
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
