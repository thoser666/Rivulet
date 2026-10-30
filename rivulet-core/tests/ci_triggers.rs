//! Regression tests for CI trigger hygiene: one push to a pull-request branch
//! must start each workflow exactly once, and a superseded pull-request run is
//! cancelled instead of queueing a second one behind it.
//!
//! `ci.yml` used to pair `push: branches: [ "**" ]` with `pull_request`, so
//! every branch push started the full 19-job matrix a second time (#346). The
//! duplicate was not merely expensive: in #341 a hung `apt-get install` in one
//! of the two runs kept the required `CI` check pending for two hours although
//! the very same job had already passed in the other run.

use std::fs;
use std::path::{Path, PathBuf};

fn repo_file(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

fn read(rel: &str) -> String {
    fs::read_to_string(repo_file(rel)).unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
}

/// Every workflow file checked into `.github/workflows`, sorted by name.
fn workflows() -> Vec<(String, String)> {
    let dir = repo_file(".github/workflows");
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dir.display()))
        .filter_map(|entry| {
            entry
                .ok()
                .map(|e| e.file_name().to_string_lossy().into_owned())
        })
        .filter(|name| name.ends_with(".yml") || name.ends_with(".yaml"))
        .collect();
    names.sort();
    assert!(
        names.len() >= 10,
        "expected the repository's workflows, found {:?}",
        names
    );
    names
        .into_iter()
        .map(|name| {
            let rel = format!(".github/workflows/{name}");
            (name, read(&rel))
        })
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Lines of the top-level block that starts with `key:` (the key line itself
/// excluded), up to the next top-level key.
fn top_level_block<'a>(text: &'a str, key: &str) -> Option<Vec<&'a str>> {
    let mut lines: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if !inside {
            if trimmed == format!("{key}:") {
                inside = true;
            }
            continue;
        }
        let is_comment = trimmed.trim_start().starts_with('#');
        if !trimmed.is_empty() && !is_comment && indent(trimmed) == 0 {
            break;
        }
        if !trimmed.is_empty() {
            lines.push(trimmed);
        }
    }
    inside.then_some(lines)
}

/// Lines of one event trigger (`push`, `pull_request`, ...) inside the `on:`
/// block, the event line included.
fn event_lines<'a>(trigger: &[&'a str], event: &str) -> Option<Vec<&'a str>> {
    let mut lines: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in trigger {
        let header = format!("{event}:");
        if !inside {
            if indent(line) == 2 && line.trim_start().starts_with(&header) {
                inside = true;
                lines.push(line);
            }
            continue;
        }
        if indent(line) <= 2 && !line.trim_start().starts_with('#') {
            break;
        }
        lines.push(line);
    }
    inside.then_some(lines)
}

/// The `branches:` filter of an event trigger, without the YAML list syntax.
fn branch_filter(event: &[&str]) -> Option<Vec<String>> {
    let line = event
        .iter()
        .find(|line| line.trim_start().starts_with("branches:"))?;
    let value = line.split_once(':')?.1;
    Some(
        value
            .trim()
            .trim_matches(|c| c == '[' || c == ']')
            .split(',')
            .map(|item| {
                item.trim()
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string()
            })
            .filter(|item| !item.is_empty())
            .collect(),
    )
}

/// A push trigger outside `pull_request`'s coverage is the duplicate: every
/// branch push would start the workflow a second time. Restricting `branches:`
/// to the integration and release refs (as `ci.yml` does) keeps the coverage
/// for refs that never get a `pull_request` run without paying twice.
#[test]
fn no_workflow_runs_twice_for_one_pull_request_push() {
    for (name, body) in workflows() {
        let trigger = top_level_block(&body, "on")
            .unwrap_or_else(|| panic!("{name}: has no `on:` trigger block"));
        let Some(push) = event_lines(&trigger, "push") else {
            continue;
        };
        if event_lines(&trigger, "pull_request").is_none() {
            continue;
        }
        let filter = branch_filter(&push).unwrap_or_else(|| {
            panic!(
                "{name}: an unconditional `push` trigger next to `pull_request` runs every \
                 branch push twice; restrict `branches:` to develop, master and release/**"
            )
        });
        assert!(
            !filter.iter().any(|branch| branch == "**"),
            "{name}: `push: branches: [ \"**\" ]` next to `pull_request` runs every \
             branch push twice; restrict `branches:` to develop, master and release/**"
        );
    }
}

/// A workflow that runs on pull requests must cancel a superseded run for the
/// same ref. The cancellation has to be gated: pushes to `develop`, scheduled
/// runs and manual dispatches must always finish.
#[test]
fn pull_request_workflows_cancel_superseded_runs() {
    for (name, body) in workflows() {
        let trigger = top_level_block(&body, "on")
            .unwrap_or_else(|| panic!("{name}: has no `on:` trigger block"));
        if event_lines(&trigger, "pull_request").is_none() {
            continue;
        }
        let concurrency = top_level_block(&body, "concurrency").unwrap_or_else(|| {
            panic!(
                "{name}: runs on pull requests without a `concurrency` group, so each push \
                 stacks another full run instead of cancelling the superseded one"
            )
        });
        assert!(
            concurrency
                .iter()
                .any(|line| line.contains("group:") && line.contains("github.ref")),
            "{name}: the concurrency group must be per ref (github.workflow-github.ref)"
        );
        let cancel = concurrency
            .iter()
            .find(|line| line.trim_start().starts_with("cancel-in-progress:"))
            .unwrap_or_else(|| panic!("{name}: concurrency block without cancel-in-progress"));
        assert!(
            cancel.contains("${{"),
            "{name}: cancel-in-progress must be gated, not a literal (a bare `true` would \
             cancel pushes to develop and scheduled runs)"
        );
        assert!(
            cancel.contains("event_name") || cancel.contains("ref_type"),
            "{name}: cancel-in-progress must only cancel pull-request runs \
             (`github.event_name == 'pull_request'`), never develop or scheduled runs"
        );
    }
}
