//! `rivulet import-obs` - map an OBS Studio scene collection onto Rivulet
//! scenes and sources ([#367](https://github.com/thoser666/Rivulet/issues/367)).
//!
//! The mapping itself lives in [`rivulet_core::obs_import`], which reports
//! everything it could not carry. This module is the CLI around it: parse the
//! flags, read one collection file, print the report, and optionally write a
//! scene collection the GUI can load.
//!
//! The two modes are deliberately exclusive:
//!
//! - `--dry-run` prints the report and writes nothing. It is the safe way to
//!   see what a migration would do before touching a file.
//! - `--out <file>` writes the imported collection atomically (sibling `.tmp`
//!   plus rename), so an interrupted write cannot leave a half-written
//!   collection behind.
//!
//! With neither flag the command refuses: a silent no-op would look like a
//! successful migration. Every difference lands in the report - an unsupported
//! OBS source type, a payload that has no Rivulet field, the collection's
//! global transition - because a migration tool that drops objects silently
//! fails the user weeks later, not now.

use std::io::Write;
use std::path::{Path, PathBuf};

use rivulet_core::{import_file, ObsImportError};

use crate::exit_code;
use crate::failure::RunFailure;

/// The `import-obs` flag surface, printed by `rivulet import-obs --help`.
pub const USAGE: &str = "\
rivulet import-obs - map an OBS Studio scene collection onto Rivulet scenes

USAGE:
    rivulet import-obs <collection.json> --dry-run [--json]
    rivulet import-obs <collection.json> --out <rivulet-scenes.json> [--json]

ARGS:
    <collection.json>         OBS scene collection (basic/scenes/<name>.json)

FLAGS:
        --dry-run             print the import report, write nothing
        --out <file>          write the imported scene collection (SceneManager JSON)
        --json                print the report as JSON
    -h, --help                show this help

The report names every difference: unsupported OBS source types, source
payloads without a Rivulet field, and the collection's global transition.
`--out` writes the scene tree only; the GUI's scene-collection format stores
scenes, not source bindings.
";

/// `rivulet import-obs` arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportArgs {
    /// The OBS collection to read.
    pub collection: Option<PathBuf>,
    /// Where to write the imported collection; `None` with `--dry-run`.
    pub out: Option<PathBuf>,
    /// Print the report and write nothing.
    pub dry_run: bool,
    /// Print the report as JSON instead of prose.
    pub json: bool,
    /// `--help`: show [`USAGE`] and exit.
    pub show_help: bool,
}

/// Parse `import-obs` flags (everything after the subcommand).
///
/// The flag combination is validated here rather than in [`run_import`], so a
/// mistake exits with the usage status and a message that names the fix
/// instead of reading a file first.
pub fn parse_import_args(args: &[String]) -> Result<ImportArgs, String> {
    let mut parsed = ImportArgs::default();
    let mut positionals: Vec<PathBuf> = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = |index: &mut usize| -> Result<String, String> {
            *index += 1;
            args.get(*index)
                .cloned()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag {
            "-h" | "--help" => parsed.show_help = true,
            "--dry-run" => parsed.dry_run = true,
            "--json" => parsed.json = true,
            "--out" => parsed.out = Some(PathBuf::from(value(&mut index)?)),
            other if other.starts_with('-') => {
                return Err(format!(
                    "unknown flag {other:?} (expected --out, --dry-run, --json or --help)"
                ))
            }
            other => positionals.push(PathBuf::from(other)),
        }
        index += 1;
    }
    if parsed.show_help {
        return Ok(parsed);
    }
    parsed.collection = match positionals.len() {
        0 => {
            return Err(
                "missing the OBS collection file (usage: rivulet import-obs <collection.json> \
                 --out <rivulet-scenes.json>)"
                    .to_string(),
            )
        }
        1 => Some(positionals.remove(0)),
        _ => {
            return Err(format!(
                "expected one OBS collection file, got {} ({})",
                positionals.len(),
                positionals
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
    };
    match (parsed.dry_run, parsed.out.is_some()) {
        (true, true) => {
            Err("--dry-run and --out are mutually exclusive: a dry run writes nothing".to_string())
        }
        (false, false) => Err(
            "choose what to do with the import: --dry-run to inspect it, or --out <file> to \
             write the scene collection"
                .to_string(),
        ),
        _ => Ok(parsed),
    }
}

/// Run `rivulet import-obs` against the process's streams.
pub fn run_import(args: &ImportArgs) -> i32 {
    run_import_with(args, &mut std::io::stdout(), &mut std::io::stderr())
}

/// The testable core of [`run_import`]: reports go to `out`, the extra human
/// notes (and failures) to `err`.
fn run_import_with<O: Write, E: Write>(args: &ImportArgs, out: &mut O, err: &mut E) -> i32 {
    let Some(collection) = args.collection.as_deref() else {
        return fail(
            RunFailure::usage(
                "missing the OBS collection file (usage: rivulet import-obs <collection.json> \
                 --out <rivulet-scenes.json>)",
            ),
            out,
            err,
        );
    };
    let imported = match import_file(collection) {
        Ok(imported) => imported,
        Err(error) => return fail(import_failure(error), out, err),
    };

    if args.json {
        match serde_json::to_string_pretty(&imported.report) {
            Ok(json) => {
                let _ = writeln!(out, "{json}");
            }
            Err(error) => {
                return fail(
                    RunFailure::usage(format!("could not serialize the import report: {error}")),
                    out,
                    err,
                )
            }
        }
    } else {
        let _ = write!(out, "{}", imported.report.describe());
    }

    if let Some(path) = args.out.as_deref() {
        let json = match imported.scenes.export_json() {
            Ok(json) => json,
            Err(error) => {
                return fail(
                    RunFailure::usage(format!("could not serialize the scene collection: {error}")),
                    out,
                    err,
                )
            }
        };
        if let Err(error) = write_atomically(path, &json) {
            return fail(
                RunFailure::output(format!("could not write {}: {error}", path.display())),
                out,
                err,
            );
        }
        let _ = writeln!(out, "wrote {}", path.display());
        if imported.report.sources > 0 {
            let _ = writeln!(
                err,
                "note: {} source(s) were mapped with their bindings; the scene-collection \
                 format stores scenes only ({} is the scene tree)",
                imported.report.sources,
                path.display()
            );
        }
    }
    exit_code::OK
}

/// An input problem is a configuration problem (exit 2), not a runtime crash:
/// the collection file is this command's config, and the message says what is
/// wrong with it.
fn import_failure(error: ObsImportError) -> RunFailure {
    RunFailure::config(error.to_string())
}

fn fail<O: Write, E: Write>(failure: RunFailure, out: &mut O, err: &mut E) -> i32 {
    let _ = writeln!(out, "{}", failure.to_json());
    let _ = writeln!(err, "{}", failure.describe());
    failure.exit_code()
}

/// Write `json` to `path` through a sibling temporary file.
///
/// The rename is what makes the write atomic: a reader either sees the old
/// collection or the complete new one, never a truncated file. A failed write
/// removes the temporary file so a retry does not trip over it.
fn write_atomically(path: &Path, json: &str) -> std::io::Result<()> {
    let temporary = path.with_extension("tmp");
    if let Err(error) = std::fs::write(&temporary, json) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn run(values: &[&str]) -> (ImportArgs, i32, String, String) {
        let parsed = parse_import_args(&args(values)).expect("arguments must parse");
        let mut out: Vec<u8> = Vec::new();
        let mut err: Vec<u8> = Vec::new();
        let code = run_import_with(&parsed, &mut out, &mut err);
        (
            parsed,
            code,
            String::from_utf8(out).expect("utf-8 stdout"),
            String::from_utf8(err).expect("utf-8 stderr"),
        )
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rivulet-cli-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    const COLLECTION: &str = r#"{
      "name": "Stream",
      "current_scene": "Game",
      "scene_order": [{"name": "Intro"}, {"name": "Game"}],
      "sources": [
        {"name": "Intro", "id": "scene", "settings": {}},
        {
          "name": "Game",
          "id": "scene",
          "settings": {"items": [
            {"name": "Facecam", "bounds_type": 2, "bounds": {"x": 640.0, "y": 360.0}}
          ]}
        },
        {"name": "Facecam", "id": "v4l2_input", "settings": {"device_id": "/dev/video0"}}
      ]
    }"#;

    #[test]
    fn a_dry_run_requires_the_collection_and_one_mode() {
        assert!(parse_import_args(&args(&["collection.json", "--dry-run"])).is_ok());
        assert!(parse_import_args(&args(&["collection.json", "--out", "out.json"])).is_ok());
        let missing_mode = parse_import_args(&args(&["collection.json"])).unwrap_err();
        assert!(missing_mode.contains("--out"), "{missing_mode}");
        let both = parse_import_args(&args(&[
            "collection.json",
            "--dry-run",
            "--out",
            "out.json",
        ]))
        .unwrap_err();
        assert!(both.contains("mutually exclusive"), "{both}");
        let missing_file = parse_import_args(&args(&["--dry-run"])).unwrap_err();
        assert!(missing_file.contains("collection.json"), "{missing_file}");
        let too_many = parse_import_args(&args(&["a.json", "b.json", "--dry-run"])).unwrap_err();
        assert!(too_many.contains("one OBS collection"), "{too_many}");
        let unknown = parse_import_args(&args(&["a.json", "--write"])).unwrap_err();
        assert!(unknown.contains("--write"), "{unknown}");
        assert!(parse_import_args(&args(&["--help"])).unwrap().show_help);
    }

    #[test]
    fn a_dry_run_reports_and_writes_nothing() {
        let dir = scratch("dry-run");
        let collection = dir.join("obs.json");
        std::fs::write(&collection, COLLECTION).unwrap();
        let out_path = dir.join("scenes.json");
        let (_, code, stdout, stderr) = run(&[collection.to_str().unwrap(), "--dry-run", "--json"]);
        assert_eq!(code, exit_code::OK, "{stderr}");
        assert!(stdout.contains("\"collection\": \"Stream\""), "{stdout}");
        assert!(
            stdout.contains("payload") || stdout.contains("\"notes\""),
            "{stdout}"
        );
        assert!(!out_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn out_writes_a_collection_the_gui_can_load() {
        let dir = scratch("out");
        let collection = dir.join("obs.json");
        std::fs::write(&collection, COLLECTION).unwrap();
        let target = dir.join("rivulet-scenes.json");
        let (_, code, stdout, stderr) = run(&[
            collection.to_str().unwrap(),
            "--out",
            target.to_str().unwrap(),
        ]);
        assert_eq!(code, exit_code::OK, "{stderr}");
        assert!(
            stdout.contains("scenes: 2 primary, 0 folder(s)"),
            "{stdout}"
        );
        assert!(stdout.contains("wrote"), "{stdout}");
        assert!(
            stderr.contains("the scene-collection format stores scenes only"),
            "{stderr}"
        );
        let written = std::fs::read_to_string(&target).unwrap();
        let manager = rivulet_core::SceneManager::import_json(&written)
            .expect("the GUI must be able to load what we write");
        let names: Vec<&str> = manager
            .scenes()
            .iter()
            .map(|scene| scene.name.as_str())
            .collect();
        assert_eq!(names, ["Intro", "Game"]);
        assert!(!dir.join("rivulet-scenes.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn out_replaces_an_existing_collection() {
        let dir = scratch("replace");
        let collection = dir.join("obs.json");
        std::fs::write(&collection, COLLECTION).unwrap();
        let target = dir.join("rivulet-scenes.json");
        std::fs::write(&target, "{\"stale\":true}").unwrap();
        let (_, code, _, stderr) = run(&[
            collection.to_str().unwrap(),
            "--out",
            target.to_str().unwrap(),
        ]);
        assert_eq!(code, exit_code::OK, "{stderr}");
        let written = std::fs::read_to_string(&target).unwrap();
        assert!(
            rivulet_core::SceneManager::import_json(&written).is_ok(),
            "the stale file must be replaced atomically: {written}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_collection_exits_with_the_config_status() {
        let (_, code, stdout, stderr) = run(&["does-not-exist-obs.json", "--dry-run"]);
        assert_eq!(code, exit_code::USAGE);
        assert!(stdout.contains("\"event\":\"failed\""), "{stdout}");
        assert!(stderr.contains("does-not-exist-obs.json"), "{stderr}");
    }

    #[test]
    fn invalid_json_and_an_empty_collection_are_refused() {
        let dir = scratch("invalid");
        let invalid = dir.join("invalid.json");
        std::fs::write(&invalid, "{ not json").unwrap();
        let (_, code, stdout, _) = run(&[invalid.to_str().unwrap(), "--dry-run"]);
        assert_eq!(code, exit_code::USAGE);
        assert!(
            stdout.contains("not a valid OBS scene collection"),
            "{stdout}"
        );

        let empty = dir.join("empty.json");
        std::fs::write(&empty, r#"{"sources":[]}"#).unwrap();
        let (_, code, stdout, _) = run(&[empty.to_str().unwrap(), "--dry-run"]);
        assert_eq!(code, exit_code::USAGE);
        assert!(stdout.contains("no scenes"), "{stdout}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_prose_report_names_every_difference() {
        let dir = scratch("prose");
        let collection = dir.join("obs.json");
        std::fs::write(
            &collection,
            r#"{
              "name": "Differences",
              "current_scene": "Main",
              "scene_order": [{"name": "Main"}],
              "transition": "Cut",
              "sources": [
                {"name": "Main", "id": "scene", "settings": {"items": [
                  {"name": "Logo"},
                  {"name": "Plugin"}
                ]}},
                {"name": "Logo", "id": "image_source", "settings": {"file": "logo.png"}},
                {"name": "Plugin", "id": "obs_nvidia_denoiser", "settings": {}}
              ]
            }"#,
        )
        .unwrap();
        let (_, code, stdout, _) = run(&[collection.to_str().unwrap(), "--dry-run"]);
        assert_eq!(code, exit_code::OK);
        assert!(stdout.contains("differences ("), "{stdout}");
        assert!(stdout.contains("no Rivulet counterpart"), "{stdout}");
        assert!(
            stdout.contains("Fade") || stdout.contains("Cut"),
            "{stdout}"
        );
        assert!(
            stdout.contains("logo.png") || stdout.contains("\"file\""),
            "{stdout}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
