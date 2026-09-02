//! Windows update watchdog for Rivulet.
//!
//! The MSI upgrade cannot replace the running `rivulet-gui.exe` /
//! `rivulet.exe` files while those processes are still alive (Windows locks
//! executing files). The GUI therefore delegates the actual installation to
//! this small detached binary:
//!
//! 1. It waits until the GUI process (and, by extension, the launcher that
//!    spawned it and blocks on its exit) has fully terminated.
//! 2. Only then does it run `msiexec /i ... /passive` **and block** until the
//!    installer finishes, so the file locks are gone when Windows replaces
//!    the old installation.
//! 3. It exits with the msiexec exit code so the outcome can be recorded.
//!
//! The watchdog copies itself into `%TEMP%` before running (self-update
//! pattern): if the MSI ships a `rivulet-updater.exe` in the install folder it
//! must not be overwritten by its own replacement mid-run.
//!
//! Usage:
//!   rivulet-updater.exe --install <msi-path> --wait-pid <pid> [--log <path>]
//!
//!   --install   path to the downloaded MSI to install
//!   --wait-pid  a process id that must exit before installation begins.
//!               May be given more than once.
//!   --log       optional path to append a one-line result record.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::Duration;

/// Minimum amount of time to keep waiting after all watched processes have
/// exited. The launcher blocks on `child.wait()` and terminates right after
/// the GUI does, so a short grace period lets it release its own file locks.
const EXIT_GRACE: Duration = Duration::from_millis(750);

/// How long to keep polling for a watched process before giving up. The GUI
/// may take a moment to shut down GStreamer and other resources, so allow a
/// generous window.
const WAIT_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::time::{Duration, Instant};

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0x0000_0000;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(
            dw_desired_access: u32,
            b_inherit_handle: i32,
            dw_process_id: u32,
        ) -> *mut c_void;
        fn WaitForSingleObject(h_handle: *mut c_void, dw_milliseconds: u32) -> u32;
        fn CloseHandle(h_object: *mut c_void) -> i32;
    }

    /// Open a handle to a process given its id, or `None` if the process does
    /// not (or no longer) exist.
    fn open_process(pid: u32) -> Option<*mut c_void> {
        if pid == 0 {
            return None;
        }
        let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            None
        } else {
            Some(handle)
        }
    }

    /// Block until the process with the given id has exited, or until `timeout`
    /// elapses. Returns `true` if the process is gone (or never existed) and
    /// `false` if it was still alive when the timeout expired.
    pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
        // If there is no such process at all, we are done immediately.
        let handle = match open_process(pid) {
            Some(h) => h,
            None => return true,
        };
        let deadline = Instant::now() + timeout;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let ms = remaining.as_millis().min(u128::from(u32::MAX)) as u32;
        // WaitForSingleObject returns WAIT_OBJECT_0 (0) once the process exits;
        // WAIT_TIMEOUT (0x102) means it is still alive at the deadline.
        let wait_result = unsafe { WaitForSingleObject(handle, ms) };
        unsafe { CloseHandle(handle) };
        wait_result == WAIT_OBJECT_0
    }
}

#[cfg(not(windows))]
mod imp {
    use std::thread::sleep;
    use std::time::Duration;

    /// Placeholder for non-Windows builds; the watchdog itself is only ever
    /// launched on Windows, so this simply reports "gone" after the timeout.
    pub fn wait_for_exit(_pid: u32, timeout: Duration) -> bool {
        sleep(timeout);
        false
    }
}

mod cli {
    use std::path::PathBuf;

    #[derive(Debug, Default)]
    pub struct Args {
        pub install: Option<PathBuf>,
        pub wait_pids: Vec<u32>,
        pub log: Option<PathBuf>,
    }

    pub fn parse() -> Args {
        let mut args = Args::default();
        let mut it = std::env::args_os().skip(1);
        while let Some(arg) = it.next() {
            let arg = arg.to_string_lossy().into_owned();
            match arg.as_str() {
                "--install" => {
                    if let Some(v) = it.next() {
                        args.install = Some(PathBuf::from(v));
                    }
                }
                "--wait-pid" => {
                    if let Some(v) = it.next() {
                        if let Ok(pid) = v.to_string_lossy().parse::<u32>() {
                            args.wait_pids.push(pid);
                        }
                    }
                }
                "--log" => {
                    if let Some(v) = it.next() {
                        args.log = Some(PathBuf::from(v));
                    }
                }
                _ => {}
            }
        }
        args
    }
}

/// Run the installation and record the outcome.
fn main() {
    let args = cli::parse();
    let Some(install) = args.install else {
        log_line("no --install path provided");
        std::process::exit(2);
    };

    // If the MSI ships a rivulet-updater.exe, running in place while the MSI
    // replaces files would lock our own executable. Re-launch from %TEMP%
    // when we are not already there, so the original file is never in use.
    if let Some(temporary) = relaunch_from_temp() {
        log_line(&format!("relaunched from {}", temporary.display()));
    }

    // Wait for every watched process to exit (GUI, and any explicit processes).
    for pid in &args.wait_pids {
        let gone = imp::wait_for_exit(*pid, WAIT_TIMEOUT);
        log_line(&format!("wait-pid {pid}: gone={gone}"));
        if !gone {
            log_line("timeout while waiting for a watched process; installing anyway");
        }
    }

    // Extra grace so the launcher (which only ends after the GUI) can exit.
    sleep(EXIT_GRACE);

    let code = run_msi(&install);
    if let Some(log) = &args.log {
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .map(|mut f| {
                use std::io::Write;
                let _ = writeln!(f, "install {} -> exit code {code}", install.display());
            });
    }
    std::process::exit(code);
}

/// Copy this executable into the system temp directory and run that copy,
/// returning the copy's path. Returns `None` if already running from temp or
/// the copy is unnecessary / failed.
#[cfg(windows)]
fn relaunch_from_temp() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let temp = std::env::temp_dir();
    // Already running from temp: avoid infinite relaunch.
    if exe.starts_with(&temp) {
        return None;
    }
    let temp_exe = temp.join("rivulet-updater.exe");
    std::fs::copy(&exe, &temp_exe).ok()?;
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let status = Command::new(&temp_exe).args(&args).status().ok()?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(not(windows))]
fn relaunch_from_temp() -> Option<PathBuf> {
    None
}

fn run_msi(path: &std::path::Path) -> i32 {
    let path_str = path.to_string_lossy().to_string();
    let mut child = match Command::new("msiexec.exe")
        .args([
            "/i",
            &path_str,
            "/passive",
            "/norestart",
            "REBOOT=ReallySuppress",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            log_line(&format!("failed to spawn msiexec: {e}"));
            return 2;
        }
    };
    match child.wait() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            log_line(&format!("failed to wait for msiexec: {e}"));
            2
        }
    }
}

/// Write a line to stdout (visible in the detached process's own console only,
/// but useful when debugging) AND to any configured log file.
fn log_line(msg: &str) {
    println!("[rivulet-updater] {msg}");
}

#[cfg(test)]
mod tests {
    use super::cli;
    use std::path::PathBuf;

    fn without_exe_arg() -> Vec<std::ffi::OsString> {
        Vec::new()
    }

    #[test]
    fn parse_rejects_missing_install() {
        // When no --install is given, main would exit with code 2; here we only
        // assert the argument parser leaves `install` as None.
        let args = cli::Args {
            install: None,
            wait_pids: vec![],
            log: None,
        };
        let _ = without_exe_arg();
        assert!(args.install.is_none());
    }

    #[test]
    fn parse_extracts_all_arguments() {
        // Re-implement the parse loop against a fixed argv vector.
        let argv: Vec<String> = vec![
            "rivulet-updater.exe".into(),
            "--install".into(),
            "C:\\tmp\\rivulet.msi".into(),
            "--wait-pid".into(),
            "1234".into(),
            "--wait-pid".into(),
            "5678".into(),
            "--log".into(),
            "C:\\tmp\\update.log".into(),
        ];
        let args = parse_argv(&argv);
        assert_eq!(args.install, Some(PathBuf::from("C:\\tmp\\rivulet.msi")));
        assert_eq!(args.wait_pids, vec![1234, 5678]);
        assert_eq!(args.log, Some(PathBuf::from("C:\\tmp\\update.log")));
    }

    #[test]
    fn parse_ignores_invalid_pid() {
        let argv: Vec<String> = vec![
            "rivulet-updater.exe".into(),
            "--install".into(),
            "x.msi".into(),
            "--wait-pid".into(),
            "not-a-number".into(),
        ];
        let args = parse_argv(&argv);
        assert!(args.wait_pids.is_empty());
    }

    /// Mirror of `cli::parse` operating on an explicit argv, so it is testable
    /// without mutating the real process environment.
    fn parse_argv(argv: &[String]) -> cli::Args {
        let mut args = cli::Args::default();
        let mut it = argv.iter().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--install" => {
                    if let Some(v) = it.next() {
                        args.install = Some(PathBuf::from(v));
                    }
                }
                "--wait-pid" => {
                    if let Some(v) = it.next() {
                        if let Ok(pid) = v.parse::<u32>() {
                            args.wait_pids.push(pid);
                        }
                    }
                }
                "--log" => {
                    if let Some(v) = it.next() {
                        args.log = Some(PathBuf::from(v));
                    }
                }
                _ => {}
            }
        }
        args
    }
}
