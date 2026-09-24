use std::fs;

#[cfg(target_os = "windows")]
#[test]
fn webview2_loader_is_colocated_with_the_gui_executable() {
    use std::path::{Path, PathBuf};

    // CARGO_MANIFEST_DIR is rivulet-gui; the GUI exe lands in the workspace
    // profile directory (target/debug | target/release).
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gui crate must live in the workspace");
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let profile_dir: PathBuf = workspace.join("target").join(profile);

    // The browser-source adapter (#227) made the GUI binary import
    // WebView2Loader.dll. Windows resolves import DLLs only next to the
    // executable (or on PATH), never from cargo's build-script output — a
    // missing copy means the exe dies silently before the first log line.
    // The build script colocates the x64 loader with the exe; this guard
    // fails CI the moment that copy step regresses.
    let exe = profile_dir.join("rivulet-gui.exe");
    let loader = profile_dir.join("WebView2Loader.dll");
    assert!(
        exe.exists(),
        "the GUI exe must be built for this test (cargo builds bin targets during `cargo test`): {}",
        exe.display()
    );
    assert!(
        loader.exists(),
        "WebView2Loader.dll must sit next to the GUI exe, otherwise the binary dies silently at startup"
    );
}

#[test]
fn webview2_loader_build_step_is_wired() {
    // Cross-platform source pin: the build script must keep the colocate
    // step (gated on Windows) so the copy cannot silently disappear, and it
    // must keep locating the loader inside webview2-com-sys's build output.
    let script = fs::read_to_string("build.rs").expect("build.rs must be readable");
    assert!(script.contains("fn copy_webview2_loader"));
    assert!(script.contains("WebView2Loader.dll"));
    assert!(script.contains("webview2-com-sys-"));
    assert!(script.contains("cfg(target_os = \"windows\")"));
}
