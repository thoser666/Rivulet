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
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(layer)
        .try_init()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    Ok(path)
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
}
