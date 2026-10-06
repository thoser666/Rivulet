use chrono::{Local, NaiveDate};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub const DEFAULT_RETENTION_DAYS: u32 = 14;
const LOG_PREFIX: &str = "rivulet-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogConfig {
    pub directory: PathBuf,
    pub retention_days: u32,
}

impl LogConfig {
    pub fn new(directory: PathBuf, retention_days: u32) -> Self {
        Self {
            directory,
            retention_days: retention_days.max(1),
        }
    }

    pub fn default_directory() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Rivulet")
            .join("logs")
    }
}

/// Directory the crash records live in.
///
/// Deliberately next to, but not inside, the log directory: a crash record
/// is a one-line marker the panic hook writes synchronously while the
/// process is dying, not a log stream. Sharing the directory would let the
/// retention pass delete an unreported crash, because a crash can happen on
/// a day whose log was never written at all.
pub fn crash_record_directory() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Rivulet")
        .join("crashes")
}

pub fn log_path(directory: &Path, date: NaiveDate) -> PathBuf {
    directory.join(format!("{LOG_PREFIX}{}.log", date.format("%Y-%m-%d")))
}

pub fn remove_expired_logs(
    directory: &Path,
    retention_days: u32,
    today: NaiveDate,
) -> io::Result<usize> {
    let cutoff = today - chrono::Duration::days(i64::from(retention_days.max(1)));
    let mut removed = 0;
    if !directory.is_dir() {
        return Ok(0);
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let Some(date_text) = name
            .strip_prefix(LOG_PREFIX)
            .and_then(|s| s.strip_suffix(".log"))
        else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(date_text, "%Y-%m-%d") else {
            continue;
        };
        if date < cutoff {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn crash_marker(context: &str, error: &str) -> String {
    format!("===== RIVULET CRASH =====\ncontext: {context}\nerror: {error}\n===== END RIVULET CRASH =====")
}

pub fn log_crash(path: &Path, context: &str, error: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{}", crash_marker(context, error))
}

pub fn init(config: &LogConfig) -> io::Result<PathBuf> {
    std::fs::create_dir_all(&config.directory)?;
    let today = Local::now().date_naive();
    let _ = remove_expired_logs(&config.directory, config.retention_days, today)?;
    let path = log_path(&config.directory, today);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let file = Mutex::new(file);
    let layer = fmt::layer().with_ansi(false).with_writer(file);
    // Reading the env via `from_default_env` would filter out *everything*
    // when `RUST_LOG` is unset, leaving the daily log file empty (not even
    // startup messages). Default to `info` so the crash logs stay useful out
    // of the box, while an explicit `RUST_LOG` still overrides it.
    let filter = EnvFilter::new(resolve_filter_spec(std::env::var("RUST_LOG").ok()));
    tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    Ok(path)
}

/// Resolve the `RUST_LOG` value into the filter spec used by [`init`], with a
/// user-friendly fallback: an unset or empty variable selects `info`, so the
/// daily log always captures startup and Discord/engine diagnostics without
/// requiring the user to configure anything.
fn resolve_filter_spec(env: Option<String>) -> String {
    env.filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "info".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_is_daily_and_stable() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 25).unwrap();
        assert_eq!(
            log_path(Path::new("logs"), date),
            PathBuf::from("logs/rivulet-2026-08-25.log")
        );
    }

    #[test]
    fn retention_removes_only_old_rivulet_logs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            log_path(dir.path(), NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            "old",
        )
        .unwrap();
        std::fs::write(
            log_path(dir.path(), NaiveDate::from_ymd_opt(2026, 8, 20).unwrap()),
            "new",
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.log"), "keep").unwrap();
        let removed =
            remove_expired_logs(dir.path(), 3, NaiveDate::from_ymd_opt(2026, 8, 25).unwrap())
                .unwrap();
        assert_eq!(removed, 2);
        assert!(dir.path().join("notes.log").exists());
    }

    #[test]
    fn crash_marker_is_machine_searchable_and_human_clear() {
        let marker = crash_marker("startup", "pipeline failed");
        assert!(marker.contains("RIVULET CRASH"));
        assert!(marker.contains("context: startup"));
        assert!(marker.contains("error: pipeline failed"));
    }

    /// A crash record must not sit inside the log directory: the retention
    /// pass deletes by age, and a crash can happen on a day whose log was
    /// never written at all, so sharing the directory would silently drop
    /// exactly the report the user has not sent yet.
    #[test]
    fn crash_records_are_kept_out_of_the_rotating_log_directory() {
        let records = crash_record_directory();
        let logs = LogConfig::default_directory();
        assert!(records.ends_with("crashes"), "{records:?}");
        assert_ne!(records, logs);
        assert!(
            !records.starts_with(&logs),
            "the retention pass must not be able to reach a crash record: {records:?}"
        );
    }

    #[test]
    fn filter_spec_defaults_to_info_when_rust_log_unset() {
        // Regression: EnvFilter::from_default_env() with RUST_LOG unset filters
        // out everything, so the daily log stayed empty and Discord/engine
        // diagnostics were invisible. The fallback must enable info level.
        assert_eq!(resolve_filter_spec(None), "info");
        assert_eq!(resolve_filter_spec(Some(String::new())), "info");
        assert_eq!(resolve_filter_spec(Some("   ".to_owned())), "info");
    }

    #[test]
    fn filter_spec_honors_explicit_rust_log() {
        assert_eq!(resolve_filter_spec(Some("debug".to_owned())), "debug");
        assert_eq!(
            resolve_filter_spec(Some("rivulet=debug,info".to_owned())),
            "rivulet=debug,info"
        );
    }
}
