//! Bootstrap diagnostics that can run before the GUI's tracing subscriber.
//!
//! This crate deliberately has no third-party dependencies. It is suitable for
//! a tiny native launcher or an early `main` call and can therefore report
//! failures that happen before GStreamer or egui is initialized.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const LOG_PREFIX: &str = "rivulet-";

/// Resolve the same per-user log directory used by the GUI logger.
pub fn default_log_directory() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local_app_data).join("Rivulet").join("logs");
        }
    }

    #[cfg(not(windows))]
    if let Some(data_dir) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(data_dir).join("Rivulet").join("logs");
    }

    std::env::temp_dir().join("Rivulet").join("logs")
}

/// Return a per-user directory for Windows crash dumps.
pub fn default_crash_dump_directory() -> PathBuf {
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("Rivulet")
            .join("crash-dumps");
    }
    std::env::temp_dir().join("Rivulet").join("crash-dumps")
}

/// Build today's log path using the local date without depending on chrono.
pub fn today_log_path(directory: &Path) -> PathBuf {
    directory.join(format!("{LOG_PREFIX}{}.log", local_date_string()))
}

/// Append a startup event before Rust tracing is available.
pub fn write_pre_rust_event(path: &Path, context: &str, message: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "===== RIVULET PRE-RUST EVENT =====\ncontext: {context}\nmessage: {message}\nepoch_ms: {}\n===== END RIVULET PRE-RUST EVENT =====",
        unix_epoch_millis()
    )
}

/// Append a marker for failures that happen before Rust tracing is available.
pub fn write_pre_rust_diagnostic(path: &Path, context: &str, message: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "===== RIVULET PRE-RUST DIAGNOSTIC =====\ncontext: {context}\nerror: {message}\nepoch_ms: {}\n===== END RIVULET PRE-RUST DIAGNOSTIC =====",
        unix_epoch_millis()
    )
}

/// Install Windows Error Reporting LocalDumps for a packaged executable.
///
/// This does not require administrator privileges. It is opt-in because it
/// changes the user's WER configuration. The dump directory is created by the
/// caller and receives a full dump when the process crashes outside Rust.
#[cfg(windows)]
pub fn configure_windows_local_dumps(
    dump_directory: &Path,
    executable_name: &str,
) -> io::Result<()> {
    use std::process::Command;

    fs::create_dir_all(dump_directory)?;
    let key = format!(
        r"HKCU\Software\Microsoft\Windows\Windows Error Reporting\LocalDumps\{executable_name}"
    );
    let status = Command::new("reg.exe")
        .args([
            "ADD",
            &key,
            "/v",
            "DumpType",
            "/t",
            "REG_DWORD",
            "/d",
            "2",
            "/f",
        ])
        .status()?;
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("reg.exe failed with {status}"),
        ));
    }
    let dump_path = dump_directory.to_string_lossy().into_owned();
    let status = Command::new("reg.exe")
        .args([
            "ADD",
            &key,
            "/v",
            "DumpFolder",
            "/t",
            "REG_EXPAND_SZ",
            "/d",
            &dump_path,
            "/f",
        ])
        .status()?;
    if !status.success() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("reg.exe failed with {status}"),
        ));
    }
    Ok(())
}

/// Non-Windows builds keep the launcher testable without changing the host.
#[cfg(not(windows))]
pub fn configure_windows_local_dumps(
    _dump_directory: &Path,
    _executable_name: &str,
) -> io::Result<()> {
    Ok(())
}

fn unix_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn local_date_string() -> String {
    // Civil date conversion from Unix days (Howard Hinnant's public-domain
    // algorithm), avoiding a dependency in the pre-Rust bootstrap path.
    let days = (unix_epoch_millis() / 86_400_000) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_creates_parent_and_machine_searchable_marker() {
        let dir = std::env::temp_dir().join(format!("rivulet-launcher-{}", unix_epoch_millis()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rivulet-2026-08-25.log");
        write_pre_rust_event(
            &path,
            "launcher-start",
            "pre-Rust launcher started the GUI process",
        )
        .unwrap();
        write_pre_rust_diagnostic(&path, "launcher", "missing DLL").unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("RIVULET PRE-RUST EVENT"));
        assert!(content.contains("RIVULET PRE-RUST DIAGNOSTIC"));
        assert!(content.contains("context: launcher"));
        assert!(content.contains("error: missing DLL"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn local_date_has_iso_shape() {
        let date = local_date_string();
        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
    }
}
