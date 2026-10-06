//! Crash records: the trace a killed run leaves behind, plus the seam a
//! future upload transport would plug into.
//!
//! Why this exists next to [`crate::crash_report`]: the report builder reads
//! the daily log, but a panic kills the process before anything gets there.
//! The release profile is `panic = "abort"` ([`Cargo.toml`]), so the hook
//! still runs, but the process is gone the instant it returns — there is no
//! unwinding, no background thread and no time for a network round trip.
//! Everything a future upload needs must therefore be on disk *before* the
//! hook returns.
//!
//! Three properties matter, and tests pin all three:
//!
//! 1. **A crash survives the crash.** [`install_panic_hook`] writes a record
//!    synchronously; [`read_latest`] finds it again on the next launch.
//! 2. **Disk usage is bounded.** A crash loop must not fill the user's
//!    disk, so [`write_record`] keeps only the newest [`MAX_CRASH_RECORDS`].
//! 3. **No transport ships.** [`CrashReportSink`] mirrors the
//!    [`TelemetrySink`](crate::telemetry::TelemetrySink) pattern: the
//!    trait exists and is tested, and the shipping application installs
//!    none. `docs/telemetry.md` requires a future transport to be reviewed
//!    separately, so "the seam exists" and "data leaves the device" are
//!    deliberately separate facts.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// How many crash records are kept on disk. A user who crashes on every
/// launch must not accumulate one file per launch forever.
pub const MAX_CRASH_RECORDS: usize = 5;

/// File-name prefix of a persisted crash record.
pub const CRASH_RECORD_PREFIX: &str = "crash-";

/// File-name suffix of a persisted crash record.
pub const CRASH_RECORD_SUFFIX: &str = ".txt";

/// Context value used when a record is rendered into the report's log
/// excerpt, so the maintainer can tell a real panic from a caught failure.
const PANIC_CONTEXT: &str = "panic";

/// One crashed run, as far as the hook could observe it.
///
/// Deliberately minimal: a message and a location. Stack traces, thread
/// names and memory contents are exactly the user context that
/// `docs/telemetry.md` rules out, and the daily log already carries the
/// surrounding context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashRecord {
    /// Seconds since the Unix epoch, as recorded by the hook.
    pub timestamp_unix: u64,
    /// `file:line` of the panic site, or a placeholder when the runtime did
    /// not report one.
    pub location: String,
    /// The panic payload rendered as text.
    pub message: String,
}

impl CrashRecord {
    /// Render the record to the exact on-disk format.
    ///
    /// Three `key: value` lines and a trailing newline, so the file stays
    /// readable when a user opens it by hand and parseable by [`Self::parse`].
    pub fn render(&self) -> String {
        format!(
            "timestamp: {}\nlocation: {}\nmessage: {}\n",
            self.timestamp_unix, self.location, self.message
        )
    }

    /// Parse a record back from [`Self::render`] output.
    ///
    /// Returns `None` for anything that does not have all three fields, so a
    /// truncated or hand-edited file is ignored rather than turned into a
    /// half-filled crash report.
    pub fn parse(raw: &str) -> Option<Self> {
        let mut timestamp = None;
        let mut location = None;
        let mut message = None;
        for line in raw.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            match key.trim() {
                "timestamp" => timestamp = value.trim().parse::<u64>().ok(),
                "location" => location = Some(value.trim().to_owned()),
                "message" => message = Some(value.trim().to_owned()),
                _ => {}
            }
        }
        Some(Self {
            timestamp_unix: timestamp?,
            // An empty location is allowed: the runtime may not report one,
            // and inventing a placeholder string here would only mislead.
            location: location?,
            message: message?,
        })
    }

    /// Stable identifier for "the same crash".
    ///
    /// Hashes location and message, not the timestamp, so a crash loop
    /// produces one fingerprint instead of a new one per launch. A transport
    /// needs this to find the existing issue for a defect that already has
    /// one instead of filing a duplicate.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.location.as_bytes());
        hasher.update([0u8]);
        hasher.update(self.message.as_bytes());
        let digest = hasher.finalize();
        // 16 hex characters is 64 bits: enough to separate real defects,
        // short enough to read in a label or issue comment.
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Render the record as a `RIVULET CRASH` block.
    ///
    /// Deliberately the same marker [`crate::crash_report`] already parses:
    /// a pending crash then flows through the *existing* redaction and
    /// block handling instead of introducing a second, less-tested path.
    pub fn as_log_block(&self) -> String {
        format!(
            "===== RIVULET CRASH =====\ncontext: {PANIC_CONTEXT}\nerror: {} at {}\n===== END RIVULET CRASH =====\n",
            self.message, self.location
        )
    }
}

/// File name for a record timestamp.
///
/// Zero-padded to a fixed width so lexicographic order equals chronological
/// order — that is what lets the rotation below sort by file name instead of
/// parsing every file back.
fn record_file_name(timestamp_unix: u64) -> String {
    format!("{CRASH_RECORD_PREFIX}{timestamp_unix:016x}{CRASH_RECORD_SUFFIX}")
}

/// Every record in `directory`, oldest first, parsed.
///
/// Files that do not parse are skipped rather than treated as errors: a
/// half-written record from a crash during a crash must not stop the next
/// launch from reading the ones that are intact.
pub fn read_all(directory: &Path) -> Vec<CrashRecord> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut by_name: BTreeMap<String, CrashRecord> = BTreeMap::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(CRASH_RECORD_PREFIX) || !name.ends_with(CRASH_RECORD_SUFFIX) {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(record) = CrashRecord::parse(&raw) {
            by_name.insert(name, record);
        }
    }
    by_name.into_values().collect()
}

/// The most recent crash record on disk, if any.
///
/// This is what the next launch calls to answer "did the last run die?".
pub fn read_latest(directory: &Path) -> Option<CrashRecord> {
    read_all(directory).pop()
}

/// Persist one record, rotating older ones out.
///
/// Returns the path written. Rotation keeps the newest
/// [`MAX_CRASH_RECORDS`] files by name, which is chronological order; the
/// file just written is always among them.
pub fn write_record(directory: &Path, record: &CrashRecord) -> io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(record_file_name(record.timestamp_unix));
    std::fs::write(&path, record.render())?;
    rotate(directory);
    Ok(path)
}

/// Delete all records. Called once the user has dealt with a crash, so a
/// crash they already reported is not offered again on the next launch.
pub fn clear(directory: &Path) -> io::Result<usize> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Ok(0);
    };
    let mut removed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(CRASH_RECORD_PREFIX)
            && name.ends_with(CRASH_RECORD_SUFFIX)
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Drop the oldest records until at most [`MAX_CRASH_RECORDS`] remain.
///
/// Best-effort by design: a failure to delete must not turn a crash report
/// into a failed crash report.
fn rotate(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with(CRASH_RECORD_PREFIX) && name.ends_with(CRASH_RECORD_SUFFIX))
        .collect();
    if names.len() <= MAX_CRASH_RECORDS {
        return;
    }
    // Fixed-width hex names sort chronologically, so the front is oldest.
    names.sort();
    let excess = names.len() - MAX_CRASH_RECORDS;
    for name in names.into_iter().take(excess) {
        let _ = std::fs::remove_file(directory.join(name));
    }
}

/// Transport for a rendered crash report.
///
/// Mirrors [`TelemetrySink`](crate::telemetry::TelemetrySink) on purpose:
/// one trait, one `&mut self` method, a blanket impl for closures, and
/// **no sink installed by the shipping application**. A caller that has
/// wired a sink gets a URL back and copies that; a caller without one falls
/// back to the clipboard plus a prefilled issue URL.
///
/// Returning `None` means "not delivered" — a sink that cannot reach its
/// service must not pretend the report is safely stored somewhere.
pub trait CrashReportSink {
    /// Upload a report and return the URL it is reachable under.
    fn upload(&mut self, report: &crate::crash_report::CrashReport) -> Option<String>;
}

impl<F> CrashReportSink for F
where
    F: FnMut(&crate::crash_report::CrashReport) -> Option<String>,
{
    fn upload(&mut self, report: &crate::crash_report::CrashReport) -> Option<String> {
        self(report)
    }
}

/// Guard installed by [`install_panic_hook`], so the hook is only set once.
///
/// A second installation would replace the first, and the previous hook's
/// output would be lost — so the second call is a no-op rather than a
/// silent overwrite.
static HOOK_INSTALLED: std::sync::OnceLock<()> = std::sync::OnceLock::new();

/// Install a panic hook that persists a crash record to `directory`.
///
/// The record has to reach the disk before this function returns: with
/// `panic = "abort"` there is no unwind, so a background task started here
/// would never be scheduled. The hook therefore does the minimum
/// synchronously — build the record, write it, rotate — and does nothing
/// else. It also must not panic, or the abort message would be replaced by
/// a second, more confusing one; every fallible step is ignored on purpose.
///
/// The previously installed hook, if any, is preserved and still runs, so
/// this composes with a logging backend instead of replacing it.
///
/// Calling this more than once does nothing after the first call.
pub fn install_panic_hook(directory: PathBuf) {
    HOOK_INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let location = info
                .location()
                .map(|location| format!("{}:{}", location.file(), location.line()))
                .unwrap_or_else(|| "unknown".to_owned());
            let message = panic_message(info);
            // A record write that fails is not worth reporting: the user is
            // already looking at a panic.
            let _ = write_record(
                &directory,
                &CrashRecord {
                    timestamp_unix: now_unix(),
                    location,
                    message,
                },
            );
            previous(info);
        }));
    });
}

/// Seconds since the Unix epoch, or 0 if the clock is before it.
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Render the panic payload as text.
///
/// A payload that is neither `&str` nor `String` becomes a fixed marker
/// instead of a `{:?}` dump: a custom payload type would format its own
/// fields, and those fields are arbitrary application state — exactly the
/// kind of content that must not be persisted just because it panicked.
fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_owned();
    }
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    "non-string panic payload".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch directory that cleans itself up, so the on-disk tests do
    /// not write into the user's real data directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let unique = format!(
                "rivulet-crash-record-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            );
            let path = std::env::temp_dir().join(unique.replace(['(', ')', ' '], "_"));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn record(timestamp: u64, message: &str) -> CrashRecord {
        CrashRecord {
            timestamp_unix: timestamp,
            location: "rivulet_core::capture::run:412".to_owned(),
            message: message.to_owned(),
        }
    }

    #[test]
    fn a_record_round_trips_through_disk() {
        let dir = TempDir::new("roundtrip");
        let original = record(1_700_000_000, "buffer negotiation failed");
        let path = write_record(dir.path(), &original).expect("write");
        assert!(path.starts_with(dir.path()));
        assert_eq!(read_latest(dir.path()), Some(original));
    }

    #[test]
    fn the_newest_record_wins() {
        let dir = TempDir::new("newest");
        write_record(dir.path(), &record(1_700_000_000, "first")).unwrap();
        write_record(dir.path(), &record(1_700_000_900, "second")).unwrap();
        assert_eq!(read_latest(dir.path()).unwrap().message, "second");
    }

    #[test]
    fn a_crash_loop_cannot_fill_the_disk() {
        let dir = TempDir::new("bounded");
        for index in 0..MAX_CRASH_RECORDS * 3 {
            write_record(dir.path(), &record(1_700_000_000 + index as u64, "boom")).expect("write");
        }
        let files = std::fs::read_dir(dir.path()).unwrap().count();
        assert!(
            files <= MAX_CRASH_RECORDS,
            "rotation left {files} files, cap is {MAX_CRASH_RECORDS}"
        );
        assert_eq!(read_all(dir.path()).len(), MAX_CRASH_RECORDS);
    }

    #[test]
    fn rotation_keeps_the_newest_records() {
        let dir = TempDir::new("rotate-newest");
        for index in 0..MAX_CRASH_RECORDS * 2 {
            write_record(dir.path(), &record(1_700_000_000 + index as u64, "boom")).unwrap();
        }
        let kept = read_all(dir.path());
        assert_eq!(kept.len(), MAX_CRASH_RECORDS);
        let newest = kept.last().unwrap().timestamp_unix;
        assert_eq!(
            newest,
            1_700_000_000 + (MAX_CRASH_RECORDS * 2) as u64 - 1,
            "the newest record must survive rotation"
        );
    }

    #[test]
    fn a_truncated_record_is_ignored_instead_of_half_read() {
        let dir = TempDir::new("truncated");
        write_record(dir.path(), &record(1_700_000_000, "intact")).unwrap();
        // A crash during the write leaves a file with only its first line.
        std::fs::write(
            dir.path().join(record_file_name(1_700_000_500)),
            "timestamp: 1700000500\n",
        )
        .unwrap();
        let all = read_all(dir.path());
        assert_eq!(all.len(), 1, "the intact record must still be readable");
        assert_eq!(all[0].message, "intact");
    }

    #[test]
    fn an_unrelated_file_in_the_directory_is_not_mistaken_for_a_record() {
        let dir = TempDir::new("unrelated");
        write_record(dir.path(), &record(1_700_000_000, "intact")).unwrap();
        std::fs::write(dir.path().join("rivulet-2026-10-05.log"), "noise").unwrap();
        std::fs::write(dir.path().join("crash-notes.txt"), "not a record").unwrap();
        assert_eq!(read_all(dir.path()).len(), 1);
    }

    #[test]
    fn a_missing_directory_is_not_an_error() {
        let missing = std::env::temp_dir().join("rivulet-crash-record-does-not-exist-xyz");
        assert_eq!(read_latest(&missing), None);
        assert!(read_all(&missing).is_empty());
        assert_eq!(clear(&missing).expect("clear"), 0);
    }

    #[test]
    fn clearing_removes_the_reported_crash() {
        let dir = TempDir::new("clear");
        write_record(dir.path(), &record(1_700_000_000, "boom")).unwrap();
        write_record(dir.path(), &record(1_700_000_900, "boom")).unwrap();
        assert_eq!(clear(dir.path()).expect("clear"), 2);
        assert_eq!(read_latest(dir.path()), None);
    }

    #[test]
    fn the_fingerprint_is_stable_across_timestamps() {
        let first = CrashRecord {
            timestamp_unix: 1,
            ..record(0, "same crash")
        };
        let second = CrashRecord {
            timestamp_unix: 999,
            ..record(0, "same crash")
        };
        assert_eq!(
            first.fingerprint(),
            second.fingerprint(),
            "a crash loop must produce one fingerprint, not one per launch"
        );
        assert_eq!(first.fingerprint().len(), 16);
    }

    #[test]
    fn different_crashes_get_different_fingerprints() {
        assert_ne!(
            record(1, "buffer negotiation failed").fingerprint(),
            record(1, "device disappeared").fingerprint(),
        );
        assert_ne!(
            record(1, "boom").fingerprint(),
            CrashRecord {
                location: "other.rs:1".to_owned(),
                ..record(1, "boom")
            }
            .fingerprint(),
        );
    }

    #[test]
    fn a_record_becomes_a_crash_block_the_existing_parser_finds() {
        let dir = TempDir::new("block");
        write_record(dir.path(), &record(1_700_000_000, "device disappeared")).unwrap();
        let pending = read_latest(dir.path()).unwrap();
        let mut log = String::from("2026-10-05T10:00:00Z  INFO capture starting\n");
        log.push_str(&pending.as_log_block());

        let report = crate::crash_report::build_report(&crate::crash_report::CrashReportInput {
            app_version: "0.65.0-alpha.55",
            platform: "linux",
            home_dir: None,
            log: &log,
        });
        assert!(report.has_crash, "the pending record must read as a crash");
        assert!(report.body.contains("device disappeared"));
        assert!(
            report.body.contains("context: `panic`"),
            "a real panic must be distinguishable from a caught failure: {}",
            report.body
        );
    }

    #[test]
    fn a_record_is_redacted_like_any_other_log_content() {
        let dir = TempDir::new("redacted");
        let pending = CrashRecord {
            timestamp_unix: 1_700_000_000,
            location: "rivulet_core::run:1".to_owned(),
            // A panic message can carry a stream URL; redaction must still
            // run over it, because the block is fed through the log path.
            message: "failed to open rtmps://live.twitch.tv/app/SUPERSECRET".to_owned(),
        };
        std::fs::write(
            dir.path().join(record_file_name(pending.timestamp_unix)),
            pending.render(),
        )
        .unwrap();
        let mut log = read_latest(dir.path()).unwrap().as_log_block();
        log.push('\n');
        let report = crate::crash_report::build_report(&crate::crash_report::CrashReportInput {
            app_version: "0.65.0-alpha.55",
            platform: "linux",
            home_dir: None,
            log: &log,
        });
        assert!(
            !report.body.contains("SUPERSECRET"),
            "a stream key leaked out of a crash record: {}",
            report.body
        );
    }

    #[test]
    fn the_sink_receives_the_report_and_returns_the_url() {
        let mut seen_title = String::new();
        let mut sink = |report: &crate::crash_report::CrashReport| {
            seen_title = report.title.clone();
            Some("https://example.invalid/crash/1".to_owned())
        };
        let report = crate::crash_report::build_report(&crate::crash_report::CrashReportInput {
            app_version: "0.65.0-alpha.55",
            platform: "linux",
            home_dir: None,
            log: "===== RIVULET CRASH =====\ncontext: startup\nerror: boom\n===== END RIVULET CRASH =====\n",
        });
        let url = sink.upload(&report).expect("upload");
        assert_eq!(url, "https://example.invalid/crash/1");
        assert!(seen_title.contains("0.65.0-alpha.55"));
    }

    #[test]
    fn a_sink_that_cannot_reach_its_service_reports_no_url() {
        let mut sink = |_: &crate::crash_report::CrashReport| None;
        let report = crate::crash_report::build_report(&crate::crash_report::CrashReportInput {
            app_version: "0.65.0-alpha.55",
            platform: "linux",
            home_dir: None,
            log: "no crash\n",
        });
        assert_eq!(sink.upload(&report), None);
    }

    /// One test for the hook, not two: the installed hook is process-global
    /// (`HOOK_INSTALLED`), so two tests installing into different
    /// directories would race under the parallel test runner — the second
    /// would be a no-op and write into the first one's directory. Sequence
    /// the cases here instead.
    #[test]
    fn the_hook_records_a_panic_synchronously() {
        let dir = TempDir::new("hook");

        // A second installation must be ignored, or the first directory would
        // silently stop receiving records.
        install_panic_hook(dir.path().to_path_buf());
        install_panic_hook(std::env::temp_dir().join("rivulet-crash-record-never"));

        // Case 1: the common `&str` payload. The hook is left installed —
        // silencing it with `take_hook`/`set_hook` would replace the very
        // hook under test, so the panic message on stderr is the price.
        let caught = std::panic::catch_unwind(|| panic!("boom from the hook test"));
        assert!(
            caught.is_err(),
            "the panic must still propagate to the caller"
        );

        let record = read_latest(dir.path()).expect("the hook must have written a record");
        assert_eq!(record.message, "boom from the hook test");
        assert!(
            record.location.contains("crash_record"),
            "the hook must record where it panicked, got {:?}",
            record.location
        );

        // Case 2: a payload that is neither `&str` nor `String`.
        clear(dir.path()).expect("clear");
        let caught = std::panic::catch_unwind(|| std::panic::panic_any(42u32));
        assert!(caught.is_err());
        assert_eq!(
            read_latest(dir.path()).expect("record").message,
            "non-string panic payload",
        );

        // Case 3: the ignored second installation really did not take over —
        // writing the record is what creates the directory.
        assert!(
            !std::env::temp_dir()
                .join("rivulet-crash-record-never")
                .exists(),
            "the second installation must not have replaced the first"
        );
    }

    #[test]
    fn the_file_name_orders_chronologically_as_text() {
        // Rotation sorts by file name, so this property is load-bearing.
        let older = record_file_name(1_700_000_000);
        let newer = record_file_name(1_700_000_001);
        assert!(older < newer, "{older} must sort before {newer}");
        assert!(older.starts_with(CRASH_RECORD_PREFIX));
        assert!(older.ends_with(CRASH_RECORD_SUFFIX));
    }
}
