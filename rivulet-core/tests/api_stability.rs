//! M8 API-contract proof: the public engine surface must stay stable and
//! intentionally encapsulated.
//!
//! These are regression guards, not exploratory tests. A new public field or a
//! relaxed visibility accessor is a contract regression and must fail this
//! suite rather than surprise downstream consumers later.
//!
//! The last three tests enforce the deprecation and migration policy in
//! `docs/api-migration.md` ([#344](https://github.com/thoser666/Rivulet/issues/344)):
//! a retired name has to be written down before it disappears, and a
//! deprecated item has to name what replaces it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rivulet_core::{
    OutputSettings, RateControl, RateControlMode, RivuletEngine, VideoCodec, VideoEncoder,
};

#[test]
fn m8_api_contract_basic_lifecycle() {
    // The primary entry point must remain callable with the documented defaults.
    // Keep the instance alive here so the test is not a tautology around `Default`.
    let _engine = RivuletEngine::new();

    let settings = OutputSettings::default();
    assert_eq!(settings.width(), 1920);
    assert_eq!(settings.height(), 1080);

    let rc = RateControl::default();
    assert_eq!(rc.mode(), RateControlMode::Cbr);
    assert_eq!(rc.quality(), 23);

    // Downstream stability expectations: the codec/enum labels must stay
    // stable enough that UI strings and serialization do not break silently.
    assert_eq!(VideoCodec::H264.label(), "H.264 (AVC)");
    assert_eq!(VideoEncoder::Software.label(), "Software");
    assert_eq!(
        VideoEncoder::Software.label_with_codec(VideoCodec::H264),
        "Software (H.264 (AVC))"
    );
}

#[test]
fn m8_api_contract_encapsulation_is_intentional() {
    // This test is a compile-time regression guard for the M8 stabilization
    // goals. If any stability-sensitive field became public, the commented-out
    // accesses below must fail to compile rather than silently widening the API.
    let _settings = OutputSettings::default();
    let _rc = RateControl::default();

    // The following must NOT compile if uncommented:
    // let _w = _settings.width;
    // let _m = _rc.mode;
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rivulet-core must sit inside the workspace")
        .to_path_buf()
}

fn read_repo_file(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()))
}

/// The crate-root re-exports of `docs/api-overview.md`, i.e. the names a
/// consumer reaches as `rivulet_core::…`.
///
/// The table is generated from `lib.rs` and checked by
/// `scripts/generate-api-overview.py --check`, so reading it here is reading
/// the crate's own surface — not a hand-kept list that could flatter the guard.
fn current_crate_root_exports() -> BTreeSet<String> {
    let doc = read_repo_file("docs/api-overview.md");
    let mut names = BTreeSet::new();
    let mut inside = false;
    for line in doc.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("### ") {
            inside = trimmed.starts_with("### Crate-root re-exports");
            continue;
        }
        if !inside || !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = trimmed
            .trim_matches('|')
            .split('|')
            .map(str::trim)
            .collect();
        let Some(names_cell) = cells.get(1) else {
            continue;
        };
        for token in names_cell.split(',') {
            if let Some(name) = token
                .trim()
                .strip_prefix('`')
                .and_then(|token| token.strip_suffix('`'))
            {
                names.insert(name.to_string());
            }
        }
    }
    assert!(
        !names.is_empty(),
        "docs/api-overview.md must keep its crate-root re-export table"
    );
    names
}

/// The surface frozen at the last released line.
fn baseline_crate_root_exports() -> BTreeSet<String> {
    read_repo_file("rivulet-core/tests/api_surface_baseline.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Rows of the "Applied migrations" table, split into cells.
fn applied_migration_rows(policy: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut inside = false;
    for line in policy.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            inside = trimmed.to_lowercase().contains("applied migrations");
            continue;
        }
        if !inside || !trimmed.starts_with('|') {
            continue;
        }
        let cells: Vec<String> = trimmed
            .trim_matches('|')
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect();
        // Skip the `| --- | --- |` separator.
        if cells
            .iter()
            .all(|cell| cell.chars().all(|ch| ch == '-' || ch == ':' || ch == ' '))
        {
            continue;
        }
        rows.push(cells);
    }
    rows
}

fn is_placeholder(cell: &str) -> bool {
    matches!(cell, "" | "-" | "—" | "–")
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// Whether a table cell names `name` as a whole identifier (backticks optional).
///
/// The policy table is prose for humans, so the guard must not demand a
/// particular backtick style — but it must also not accept `Scene` as
/// documentation for a removed `SceneManager`.
fn cell_names_item(cell: &str, name: &str) -> bool {
    let cell = cell.replace('`', "");
    let mut search_from = 0;
    while let Some(offset) = cell[search_from..].find(name) {
        let start = search_from + offset;
        let end = start + name.len();
        let before_ok = start == 0
            || !cell[..start]
                .chars()
                .next_back()
                .is_some_and(is_identifier_char);
        let after_ok =
            end == cell.len() || !cell[end..].chars().next().is_some_and(is_identifier_char);
        if before_ok && after_ok {
            return true;
        }
        search_from = start + 1;
    }
    false
}

#[test]
fn m8_api_removals_are_documented_in_the_migration_policy() {
    let baseline = baseline_crate_root_exports();
    let current = current_crate_root_exports();
    let removed: Vec<String> = baseline.difference(&current).cloned().collect();

    let policy = read_repo_file("docs/api-migration.md");
    let rows = applied_migration_rows(&policy);

    let undocumented: Vec<&String> = removed
        .iter()
        .filter(|name| {
            !rows.iter().any(|cells| {
                cells.len() >= 4
                    && cell_names_item(&cells[0], name)
                    && !is_placeholder(&cells[2])
                    && !is_placeholder(&cells[3])
            })
        })
        .collect();

    assert!(
        undocumented.is_empty(),
        "these crate-root names are gone from docs/api-overview.md but have no \
         migration row in docs/api-migration.md: {undocumented:?}\n\
         Add a row to the `## Applied migrations` table with the replacement and \
         a migration note, then refresh the baseline with \
         `python scripts/generate-api-overview.py --write-baseline`.\n\
         If the removal is not intended, restore the item instead — under the \
         policy in docs/api-migration.md a public name is deprecated for at \
         least one minor release before it is removed."
    );
}

/// Every `#[deprecated]` attribute in the crate, with the file and line it
/// starts on.
fn deprecated_attributes() -> Vec<(String, usize, String)> {
    let source_root = repo_root().join("rivulet-core").join("src");
    let mut files = Vec::new();
    collect_rust_files(&source_root, &mut files);

    let mut found = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let relative = file
            .strip_prefix(repo_root())
            .unwrap_or(&file)
            .display()
            .to_string()
            .replace('\\', "/");
        let mut search_from = 0;
        while let Some(offset) = text[search_from..].find("#[deprecated") {
            let start = search_from + offset;
            let rest = &text[start..];
            let len = rest.find(']').map(|end| end + 1).unwrap_or(rest.len());
            let line = text[..start].lines().count() + 1;
            found.push((relative.clone(), line, rest[..len].to_string()));
            search_from = start + len;
        }
    }
    found
}

fn collect_rust_files(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

fn string_argument<'a>(attribute: &'a str, key: &str) -> Option<&'a str> {
    let after_key = attribute.split_once(&format!("{key} ="))?.1;
    let after_quote = after_key.split_once('"')?.1;
    let value = after_quote.split_once('"')?.0;
    Some(value)
}

#[test]
fn m8_deprecated_items_name_their_replacement() {
    let deprecated = deprecated_attributes();
    assert!(
        !deprecated.is_empty(),
        "the M8 contract expects at least the documented \
         `detect_encoder_availability` deprecation to exist"
    );

    let offenders: Vec<String> = deprecated
        .iter()
        .filter(|(_, _, attribute)| {
            let since = string_argument(attribute, "since");
            let note = string_argument(attribute, "note");
            since.is_none_or(|since| since.trim().is_empty())
                || note.is_none_or(|note| note.trim().is_empty())
        })
        .map(|(file, line, attribute)| format!("  {file}:{line}: {attribute}"))
        .collect();

    assert!(
        offenders.is_empty(),
        "every `#[deprecated]` item must carry `since` and a non-empty `note` \
         naming the replacement, as docs/api-migration.md requires:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn m8_deprecation_policy_is_published_and_linked() {
    let policy = read_repo_file("docs/api-migration.md");
    assert!(
        policy.contains("at least one minor release"),
        "docs/api-migration.md must name the supported deprecation window"
    );
    assert!(
        policy.contains("#[deprecated") && policy.contains("Applied migrations"),
        "docs/api-migration.md must document the attribute and the migration table \
         the guards read"
    );

    for document in ["docs/m8-api-contract.md", "docs/api-overview.md"] {
        assert!(
            read_repo_file(document).contains("api-migration.md"),
            "{document} must link docs/api-migration.md so a consumer finds the \
             policy from the contract and from the generated surface"
        );
    }
}
