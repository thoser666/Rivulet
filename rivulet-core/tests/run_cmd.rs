//! Regression test for the `scripts/run.cmd` wrapper (CS-001 issue 3, C55 m6/p8).
//!
//! The wrapper is a Windows .cmd, so the tests that execute it run through the
//! same shell the project uses on Windows (`cmd /c`), gated to that platform —
//! `cmd` does not exist on the Linux/macOS CI legs. The file-shape assertions
//! run everywhere so a stale or broken wrapper is caught on every runner.
//!
//! The wrapper's session directory is the parent of `scripts/` (the worktree
//! root), so a target outside it must be refused, and only an explicit
//! allow-list of environment variables may reach the child command.

use std::path::{Path, PathBuf};

/// The repository root (parent of the `rivulet-core` crate directory).
fn repo_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root
}

fn run_cmd() -> PathBuf {
    repo_root().join("scripts").join("run.cmd")
}

/// File-shape checks that must hold on every platform: the wrapper exists,
/// reads as UTF-8, and looks like the auditable cmd wrapper rather than a
/// stale stub. If these fail, the Windows execution tests below would be
/// testing the wrong file.
#[test]
fn run_cmd_wrapper_exists_in_the_scripts_directory() {
    let path = run_cmd();
    assert!(
        path.exists(),
        "scripts/run.cmd must be committed (found {path:?})"
    );
    let text = std::fs::read_to_string(&path).expect("run.cmd must be readable UTF-8");
    assert!(
        text.contains("@echo off") && text.contains("setlocal") && text.contains("%~dp0"),
        "scripts/run.cmd looks like the updated cmd wrapper rather than a stale stub: {path:?}"
    );
    // The wrapper must refuse out-of-session targets and print the intended
    // command on refusal; these markers are what the execution tests key on.
    for marker in ["REJECTED", "Intended command", "SESSION_DIR"] {
        assert!(
            text.contains(marker),
            "scripts/run.cmd must contain the audit marker {marker:?}"
        );
    }
}

#[cfg(windows)]
mod windows_execution {
    use super::*;
    use std::process::{Command, Stdio};

    /// Configure a `cmd /c run.cmd ...` invocation from a chosen working
    /// directory, forwarding only the environment variables we explicitly add
    /// so the wrapper's allow-list behaviour is observable through the child's
    /// view. The caller finishes with `.output()`.
    fn run_cmd_command(args: &[&str], cwd: &Path, extra_env: &[(&str, &str)]) -> Command {
        let mut cmd = Command::new("cmd");
        cmd.arg("/c").arg(run_cmd());
        for a in args {
            cmd.arg(a);
        }
        cmd.current_dir(cwd);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.env_clear();
        cmd.env(
            "SYSTEMROOT",
            std::env::var("SYSTEMROOT")
                .as_deref()
                .unwrap_or("C:\\Windows"),
        );
        cmd.env(
            "SystemRoot",
            std::env::var("SystemRoot")
                .as_deref()
                .unwrap_or("C:\\Windows"),
        );
        cmd.env(
            "TEMP",
            std::env::var("TEMP")
                .as_deref()
                .unwrap_or("C:\\Windows\\Temp"),
        );
        cmd.env("PATHEXT", ".COM;.EXE;.BAT;.CMD");
        cmd.env(
            "COMSPEC",
            std::env::var("COMSPEC").as_deref().unwrap_or("cmd"),
        );
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        cmd
    }

    fn combined(out: &std::process::Output) -> String {
        [
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ]
        .concat()
    }

    #[test]
    fn run_cmd_rejects_targets_outside_the_session_directory() {
        // The session directory is the repo root; a system temp directory is
        // guaranteed to live outside it on every Windows runner.
        let tmp = tempfile::tempdir().expect("tempdir");
        let outside = tmp.path().join("outside_workspace");
        std::fs::create_dir(&outside).expect("can create outside dir");
        let listing_before = std::fs::read_dir(&outside)
            .expect("outside dir readable")
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();

        let target = outside.to_string_lossy().replace('\\', "/");
        let out = run_cmd_command(&["dir", "/b"], &outside, &[("TARGET_DIR", &target)])
            .output()
            .expect("run.cmd must be executable");

        assert!(
            !out.status.success(),
            "run.cmd must reject a target outside its own directory (exit was {:?})",
            out.status.code()
        );

        // Concrete "did the command run?" check: a refusal must leave the
        // outside directory exactly as it was.
        let listing_after = std::fs::read_dir(&outside)
            .expect("outside dir still readable")
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            listing_before, listing_after,
            "SECURITY REGRESSION: run.cmd executed a command that modified the outside directory"
        );

        let combined = combined(&out);
        // The wrapper must surface the intended command on refusal so the
        // audit line is inspectable rather than a silent block.
        assert!(
            combined.contains("REJECTED") && combined.contains("cmd /c dir /b"),
            "run.cmd must refuse and still print the full intended command, got:\n{combined}"
        );
    }

    #[test]
    fn run_cmd_keeps_existing_user_and_worktree_variables_away_from_the_command() {
        // Inside the session dir: a unique temp dir under the repo's gitignored
        // tmp/, so the wrapper accepts this target while the dir itself stays
        // outside version control and is cleaned up on drop.
        let session_tmp = repo_root().join("tmp");
        std::fs::create_dir_all(&session_tmp).expect("can create repo tmp dir");
        let work = tempfile::tempdir_in(&session_tmp).expect("temp dir inside the session");
        let work_path = work.path();

        let out = run_cmd_command(
            &[
                "python",
                "-c",
                r#"import os; open('probe.txt','w').write(os.environ.get('LEAK_TEST_VAR','missing'))"#,
            ],
            work_path,
            &[("LEAK_TEST_VAR", "user-controlled-secret-value")],
        )
        .output()
        .expect("wrapper must be runnable");

        let probe = work_path.join("probe.txt");
        let probe_text = std::fs::read_to_string(&probe).unwrap_or_default();
        assert!(
            !probe_text.contains("user-controlled-secret-value"),
            "run.cmd must not pass LEAK_TEST_VAR into the child command, got probe: {probe_text:?}"
        );
        // The command itself must have run (target is inside the session) and
        // seen the variable as absent, not merely been refused.
        assert_eq!(
            probe_text,
            "missing",
            "the child must run inside the session dir without the injected secret (exit {:?})\n{}",
            out.status.code(),
            combined(&out)
        );

        // The intended command must still be visible for the audit line.
        let combined = combined(&out);
        assert!(
            combined.contains("Intended command"),
            "run.cmd must report the intended command when running: {combined}"
        );
    }

    #[test]
    fn run_cmd_reports_the_full_intended_command_on_rejection() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&outside).expect("can create outside dir");
        let target = outside.to_string_lossy().replace('\\', "/");

        let out = run_cmd_command(
            &["echo", "sensitive-argument"],
            &outside,
            &[
                ("TARGET_DIR", &target),
                ("SENSITIVE_ENV", "should-not-leak"),
            ],
        )
        .output()
        .expect("wrapper must be runnable");

        let combined = combined(&out);
        // The precise command text must appear verbatim under a
        // machine-readable heading rather than only a vague refusal.
        assert!(
            combined.contains("REJECTED") && combined.contains("cmd /c echo sensitive-argument"),
            "run.cmd must print the full intended command block, combined output:\n{combined}"
        );
        // The sensitive *value* must never be echoed: the wrapper prints the
        // command line, not the caller's environment.
        assert!(
            !combined.contains("should-not-leak"),
            "run.cmd must not echo SENSITIVE_ENV raw into the output"
        );
    }
}
