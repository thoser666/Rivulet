//! Regression test for the new `check_sanity.py` wrapper (CS-001 issue 1, C55 m6/p8).

use std::path::PathBuf;
use std::process::Command;

fn sanity_py() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.push("check_sanity.py");
    path
}

/// Resolve the Python interpreter at run time.
///
/// `env!("PYTHON")` was deliberately dropped: it is a *compile-time* probe
/// that fails the whole build on any machine (CI included) where the variable
/// happens not to be exported. The wrapper itself is a Python script, so a
/// working interpreter is a runtime prerequisite, discovered here by trying
/// the explicit override first and then the platform's usual names.
fn python() -> String {
    if let Ok(p) = std::env::var("PYTHON") {
        if !p.is_empty() {
            return p;
        }
    }
    let candidates: &[&str] = if cfg!(windows) {
        &["python", "python3"]
    } else {
        &["python3", "python"]
    };
    for cand in candidates {
        if let Ok(out) = std::process::Command::new(cand).arg("--version").output() {
            if out.status.success() {
                return (*cand).to_string();
            }
        }
    }
    panic!("no working Python interpreter found (tried $PYTHON, {candidates:?})");
}

#[test]
fn check_sanity_script_exists_and_runs_as_a_fire_and_forget_wrapper() {
    let path = sanity_py();
    assert!(
        path.exists(),
        "check_sanity.py must be committed next to the workspace root (found {path:?})"
    );

    // Do NOT use a homegrown runner helper here: the point of the refactor is
    // that every invocation goes through a single `subprocess.run` with captured
    // stdout/stderr, and the exit code is carried directly. Sink the output so
    // pytest stays quiet; the wrapper already replays what matters.
    let out = Command::new(python())
        .arg("-u")
        .arg(&path)
        .output()
        .expect("check_sanity.py must be executable by a Python on PATH");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // A graceful skip because pytest is absent is acceptable for a sanity check.
    let skipped_due_to_missing_pytest =
        stdout.contains("SKIP: pytest not installed") && out.status.code() == Some(0);
    assert!(
        skipped_due_to_missing_pytest || out.status.success(),
        "check_sanity.py must exit 0 when pytest is missing (graceful skip), or 0 when the suite passes.\n\
         exit = {:?}\n\
         stdout:\n{}\n\
         stderr:\n{}",
        out.status,
        stdout,
        stderr,
    );

    // The refactored wrapper prints its own diagnostics; it must not lose them by
    // falling back to a helper that truncated only stderr.tail onto stdout.
    assert!(
        stdout.contains("Python:"),
        "wrapper must print the python it runs on"
    );
    assert!(
        stdout.contains("pytest") || stdout.contains("SKIP"),
        "wrapper must report the pytest availability check"
    );

    // The wrapper must not hide a real stack trace behind a tail of stderr whose
    // first 400 chars landed on stdout. The marker is printed unconditionally
    // after the lint step, so it is present on every path (empty tail is fine) —
    // this is the output contract callers key on.
    assert!(
        stderr.contains("STDERR (tail):") || stdout.contains("STDERR (tail):"),
        "wrapper must print the cargo-check stderr tail marker (empty tail is fine)\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[test]
fn check_sanity_does_not_define_per_command_runner_helpers() {
    let source = std::fs::read_to_string(sanity_py()).expect("check_sanity.py readable");
    // The original homegrown helpers (one function per command, try/except that
    // lost process output) must be gone. The file should expose a single `_run`
    // that delegates to `subprocess.run(..., capture_output=True, text=True)`.
    assert!(
        source.contains("subprocess.run") && source.contains("capture_output=True"),
        "check_sanity.py must use a single subprocess.run with capture_output=True, not bespoke runner helpers"
    );
    assert!(
        source.contains("pytest.returncode") || source.contains("r.returncode"),
        "the wrapper must carry exit codes directly instead of swallowing them"
    );
}
