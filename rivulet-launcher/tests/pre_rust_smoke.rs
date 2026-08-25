use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn missing_gui_binary_writes_pre_rust_diagnostic() {
    let root = std::env::temp_dir().join(format!(
        "rivulet-pre-rust-smoke-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos()
    ));
    let log_dir = root.join("logs");
    let missing_gui = root.join("missing").join(if cfg!(windows) {
        "rivulet-gui.exe"
    } else {
        "rivulet-gui"
    });
    fs::create_dir_all(&log_dir).expect("create isolated log directory");

    let launcher = std::env::var_os("CARGO_BIN_EXE_rivulet-launcher")
        .map(PathBuf::from)
        .expect("Cargo must provide the launcher binary path");
    let result = Command::new(launcher)
        .env("RIVULET_LAUNCHER_LOG_DIR", &log_dir)
        .env("RIVULET_GUI_PATH", &missing_gui)
        .output()
        .expect("launch the pre-Rust launcher");

    assert!(
        !result.status.success(),
        "missing GUI must make launcher fail"
    );
    let log_path = fs::read_dir(&log_dir)
        .expect("read isolated log directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "log"))
        .expect("launcher must create a daily log");
    let log = fs::read_to_string(log_path).expect("read launcher log");
    assert!(log.contains("===== RIVULET PRE-RUST DIAGNOSTIC ====="));
    assert!(log.contains("context: launcher"));
    assert!(log.contains("===== END RIVULET PRE-RUST DIAGNOSTIC ====="));

    let _ = fs::remove_dir_all(root);
}
