#![cfg_attr(windows, windows_subsystem = "windows")]

use rivulet_launcher::{
    configure_windows_local_dumps, default_crash_dump_directory, default_log_directory,
    today_log_path, write_pre_rust_diagnostic,
};
use std::process::Command;

fn target_executable_name() -> &'static str {
    #[cfg(windows)]
    {
        "rivulet-gui.exe"
    }
    #[cfg(not(windows))]
    {
        "rivulet-gui"
    }
}

fn main() {
    let log_directory = std::env::var_os("RIVULET_LAUNCHER_LOG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(default_log_directory);
    let log_path = today_log_path(&log_directory);
    let launcher_dir = match std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(ToOwned::to_owned))
    {
        Some(path) => path,
        None => {
            let _ = write_pre_rust_diagnostic(
                &log_path,
                "launcher",
                "could not resolve launcher directory",
            );
            std::process::exit(1);
        }
    };
    let executable = std::env::var_os("RIVULET_GUI_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| launcher_dir.join(target_executable_name()));
    let _ = rivulet_launcher::write_pre_rust_event(
        &log_path,
        "launcher-start",
        &format!("starting {}", target_executable_name()),
    );

    if std::env::var_os("RIVULET_ENABLE_CRASH_DUMPS").is_some() {
        let dump_dir = default_crash_dump_directory();
        if let Err(error) = configure_windows_local_dumps(&dump_dir, target_executable_name()) {
            let _ = write_pre_rust_diagnostic(&log_path, "wer", &error.to_string());
        }
    }

    match Command::new(&executable)
        .args(std::env::args_os().skip(1))
        .current_dir(&launcher_dir)
        .spawn()
        .and_then(|mut child| child.wait())
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            let message = status
                .code()
                .map(|code| format!("{} exited with code {code}", target_executable_name()))
                .unwrap_or_else(|| {
                    format!(
                        "{} terminated by signal or exception",
                        target_executable_name()
                    )
                });
            let _ = write_pre_rust_diagnostic(&log_path, "process-exit", &message);
            std::process::exit(status.code().unwrap_or(1));
        }
        Err(error) => {
            let _ = write_pre_rust_diagnostic(&log_path, "launcher", &error.to_string());
            std::process::exit(1);
        }
    }
}
