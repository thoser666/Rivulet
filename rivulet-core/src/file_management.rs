//! Recording file management policy (M4).
//!
//! Point "Recording file management" of the M4 roadmap: split recordings by
//! time/size, configurable filename patterns, and auto-record alongside the
//! stream. This module is the deterministic policy model — like
//! [`crate::container`] it is pure and fully unit-testable. The engine applies
//! the resulting path/part decisions when it opens each recording file.

use serde::{Deserialize, Serialize};

/// Supported placeholders in a recording filename pattern.
///
/// Pattern text like `{name}_{date}_{time}` is expanded at recording start by
/// [`FileNamePattern::render`]. Unknown/unsupported placeholders are rejected
/// at validation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternToken {
    /// Custom base name entered by the user (e.g. the scene name).
    Name,
    /// ISO date (e.g. `2026-08-31`).
    Date,
    /// Time (e.g. `14-05-09`).
    Time,
    /// Zero-padded 2-digit sequence number (first file of a session is `01`).
    Sequence,
    /// The platform or stream label (e.g. `twitch`).
    Stream,
}

impl PatternToken {
    /// The literal placeholder text, including braces (`{name}`).
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Name => "{name}",
            Self::Date => "{date}",
            Self::Time => "{time}",
            Self::Sequence => "{seq}",
            Self::Stream => "{stream}",
        }
    }

    /// All supported tokens.
    pub fn all() -> &'static [PatternToken; 5] {
        &[
            PatternToken::Name,
            PatternToken::Date,
            PatternToken::Time,
            PatternToken::Sequence,
            PatternToken::Stream,
        ]
    }

    /// Parses a placeholder body (without braces), if it is a known token.
    fn from_body(body: &str) -> Option<Self> {
        match body {
            "name" => Some(Self::Name),
            "date" => Some(Self::Date),
            "time" => Some(Self::Time),
            "seq" => Some(Self::Sequence),
            "stream" => Some(Self::Stream),
            _ => None,
        }
    }
}

/// A validated filename pattern.
///
/// Built via [`FileNamePattern::new`], which rejects unknown placeholders and
/// filename-hostile characters so a pattern can never break the output path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNamePattern {
    /// The raw pattern with `{token}` placeholders.
    raw: String,
}

impl Default for FileNamePattern {
    /// A sensible default OBS-like pattern: `name_date_time`.
    fn default() -> Self {
        Self {
            raw: "{name}_{date}_{time}".to_string(),
        }
    }
}

impl FileNamePattern {
    /// Builds a pattern, validating placeholders and path safety.
    pub fn new(raw: impl Into<String>) -> Result<Self, String> {
        let raw = raw.into();
        if raw.trim().is_empty() {
            return Err("pattern must not be empty".to_string());
        }
        if raw
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        {
            return Err("pattern contains characters that are illegal in filenames".to_string());
        }
        Self::tokens_chars(&raw)?; // validates all placeholders
        Ok(Self { raw })
    }

    /// Extracts the ordered subset of [`PatternToken`]s in the pattern,
    /// rejecting any unknown `{...}` placeholder.
    pub fn tokens(&self) -> Result<Vec<PatternToken>, String> {
        Self::tokens_chars(&self.raw)
    }

    /// Tokenizes a raw pattern string.
    fn tokens_chars(raw: &str) -> Result<Vec<PatternToken>, String> {
        let mut tokens = Vec::new();
        let mut rest = raw;
        while !rest.is_empty() {
            if let Some(open) = rest.find('{') {
                // Plain text before the placeholder is ignorable.
                let after_open = &rest[open + 1..];
                let close = after_open
                    .find('}')
                    .ok_or_else(|| format!("unterminated placeholder in pattern {raw:?}"))?;
                let body = &after_open[..close];
                let token = PatternToken::from_body(body)
                    .ok_or_else(|| format!("unknown placeholder {{{body}}} in pattern {raw:?}"))?;
                tokens.push(token);
                rest = &after_open[close + 1..];
            } else {
                break;
            }
        }
        Ok(tokens)
    }

    /// Renders the pattern to a concrete base filename.
    ///
    /// # Arguments
    /// - `vals` supplies the value for each [`PatternToken`].
    /// - `sequence` is a 1-based number formatted as `{:02}` for `{seq}`.
    pub fn render(&self, vals: &PatternValues, sequence: u32, stream_label: &str) -> String {
        let mut out = String::new();
        let mut rest = self.raw.as_str();
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after_open = &rest[open + 1..];
            let close = after_open
                .find('}')
                .expect("pattern was validated, placeholders are closed");
            let body = &after_open[..close];
            match PatternToken::from_body(body).expect("validated token") {
                PatternToken::Name => out.push_str(&sanitize_component(&vals.name)),
                PatternToken::Date => out.push_str(&vals.date),
                PatternToken::Time => out.push_str(&vals.time),
                PatternToken::Sequence => out.push_str(&format!("{sequence:02}")),
                PatternToken::Stream => {
                    out.push_str(&sanitize_component(&stream_label.to_ascii_lowercase()))
                }
            }
            rest = &after_open[close + 1..];
        }
        // Append the literal tail (e.g. a trailing separator the user wrote).
        out.push_str(rest);
        // Collapse doubled separators and strip leading/trailing ones, so a
        // free-text token ending in `_` next to a literal `_` cannot produce
        // an ugly `__` filename.
        let mut prev_underscore = false;
        let mut collapsed = String::with_capacity(out.len());
        for c in out.trim_matches('_').chars() {
            if c == '_' {
                if prev_underscore {
                    continue;
                }
                prev_underscore = true;
            } else {
                prev_underscore = false;
            }
            collapsed.push(c);
        }
        let collapsed = collapsed.trim_matches('_').to_string();
        if collapsed.trim().is_empty() {
            vals.name.clone()
        } else {
            collapsed
        }
    }
}

/// Sanitizes a free-text component to a filename-safe token, preserving case
/// but replacing path-hostile whitespace/characters with `_`.
fn sanitize_component(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

/// Concrete values used to expand a [`FileNamePattern`] at recording start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternValues {
    /// Base name (usually the scene name or a user label).
    pub name: String,
    /// ISO date component (e.g. `2026-08-31`).
    pub date: String,
    /// Time component (e.g. `14-05-09`).
    pub time: String,
}

impl PatternValues {
    /// Builds `PatternValues` from a wall clock timestamp.
    pub fn from_datetime(now: chrono::DateTime<chrono::Local>) -> Self {
        Self {
            name: String::new(),
            date: now.format("%Y-%m-%d").to_string(),
            time: now.format("%H-%M-%S").to_string(),
        }
    }
}

/// How a recording is split into multiple files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SplitBy {
    /// No splitting.
    #[default]
    None,
    /// Split every N seconds.
    Duration { seconds: u64 },
    /// Split when the output file reaches MiB.
    Size { megabytes: u64 },
}

impl SplitBy {
    /// Whether splitting is enabled.
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Recording file-management settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingFileSettings {
    /// Filename pattern applied at recording start.
    pub filename_pattern: FileNamePattern,
    /// How (and whether) the recording is split.
    pub split: SplitBy,
    /// Whether a recording auto-starts alongside the stream.
    pub auto_record_with_stream: bool,
    /// Directory in which auto-recorded files are written.
    pub auto_record_dir: String,
}

impl Default for RecordingFileSettings {
    fn default() -> Self {
        Self {
            filename_pattern: FileNamePattern::default(),
            split: SplitBy::None,
            auto_record_with_stream: false,
            auto_record_dir: "recordings".to_string(),
        }
    }
}

/// Tracks progression through a (possibly split) recording session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingSession {
    /// The directory containing the output files.
    pub dir: String,
    /// The container extension (e.g. `mkv`).
    pub extension: String,
    /// The current 1-based part number.
    pub part: u32,
    /// Bytes written so far to the current part.
    pub bytes_written: u64,
    /// Seconds elapsed so far in the current part.
    pub seconds_elapsed: u64,
}

impl RecordingSession {
    /// Begins a session in the given directory with the container extension.
    pub fn start(dir: impl Into<String>, extension: impl Into<String>) -> Self {
        Self {
            dir: dir.into(),
            extension: extension.into(),
            part: 1,
            bytes_written: 0,
            seconds_elapsed: 0,
        }
    }

    /// The current part's filename (the pattern minus the extension).
    pub fn current_stem(
        &self,
        pattern: &FileNamePattern,
        vals: &PatternValues,
        stream: &str,
    ) -> String {
        pattern.render(vals, self.part, stream)
    }

    /// The current part's full output path (dir + stem + .ext).
    pub fn current_path(
        &self,
        pattern: &FileNamePattern,
        vals: &PatternValues,
        stream: &str,
    ) -> String {
        let stem = self.current_stem(pattern, vals, stream);
        format!(
            "{}/{stem}.{}",
            self.dir.trim_end_matches('/'),
            self.extension
        )
    }

    /// Records bytes written to the current part.
    pub fn record_bytes(&mut self, written: u64) {
        self.bytes_written += written;
    }

    /// Records elapsed seconds for the current part.
    pub fn record_seconds(&mut self, seconds: u64) {
        self.seconds_elapsed += seconds;
    }

    /// Whether the current part must be closed (splitted) because the split
    /// rule boundary has been crossed — duration or size, whichever applies.
    pub fn should_split(&self, split: SplitBy) -> bool {
        match split {
            SplitBy::None => false,
            SplitBy::Duration { seconds } => self.seconds_elapsed >= seconds,
            SplitBy::Size { megabytes } => {
                self.bytes_written >= megabytes.saturating_mul(1024 * 1024)
            }
        }
    }

    /// Advances to the next part of a split session, resetting per-part state.
    pub fn next_part(&mut self) {
        self.part += 1;
        self.bytes_written = 0;
        self.seconds_elapsed = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pattern_validates() {
        let p = FileNamePattern::default();
        assert!(p.tokens().is_ok());
        assert_eq!(
            p.tokens().unwrap(),
            vec![PatternToken::Name, PatternToken::Date, PatternToken::Time]
        );
    }

    #[test]
    fn rejects_empty_pattern() {
        assert!(FileNamePattern::new("").is_err());
    }

    #[test]
    fn rejects_unknown_placeholder() {
        assert!(FileNamePattern::new("{bogus}_clip").is_err());
    }

    #[test]
    fn rejects_unterminated_placeholder() {
        assert!(FileNamePattern::new("clip_{date").is_err());
    }

    #[test]
    fn rejects_path_hostile_characters() {
        for bad in [
            "a/b", "a\\b", "a:b", "a*b", "a?b", "a\"b", "a<b", "a>b", "a|b",
        ] {
            assert!(FileNamePattern::new(bad).is_err(), "bad: {bad}");
        }
    }

    #[test]
    fn tokens_parse_known_placeholders() {
        let p = FileNamePattern::new("{stream}_{seq}_{name}").unwrap();
        assert_eq!(
            p.tokens().unwrap(),
            vec![
                PatternToken::Stream,
                PatternToken::Sequence,
                PatternToken::Name
            ]
        );
    }

    #[test]
    fn render_swaps_all_placeholders() {
        let p = FileNamePattern::new("{name}_{date}_{time}").unwrap();
        let vals = PatternValues {
            name: "CoolScene".to_string(),
            date: "2026-08-31".to_string(),
            time: "14-05-09".to_string(),
        };
        let out = p.render(&vals, 3, "twitch");
        assert_eq!(out, "CoolScene_2026-08-31_14-05-09");
    }

    #[test]
    fn render_zero_pads_sequence() {
        let p = FileNamePattern::new("sess_{seq}").unwrap();
        let vals = PatternValues::from_datetime(chrono::Local::now());
        assert_eq!(p.render(&vals, 1, ""), "sess_01");
        assert_eq!(p.render(&vals, 12, ""), "sess_12");
    }

    #[test]
    fn render_sanitizes_stream_component() {
        let p = FileNamePattern::new("{stream}_gone_live").unwrap();
        let vals = PatternValues::from_datetime(chrono::Local::now());
        let out = p.render(&vals, 1, "Twitch!");
        assert_eq!(out, "twitch_gone_live");
    }

    #[test]
    fn render_falls_back_to_name_when_output_empty() {
        let p = FileNamePattern::new("{name}").unwrap();
        let vals = PatternValues {
            name: "fallback".to_string(),
            date: "".to_string(),
            time: "".to_string(),
        };
        assert_eq!(p.render(&vals, 1, ""), "fallback");
    }

    #[test]
    fn session_starts_at_part_one() {
        let s = RecordingSession::start("/tmp/out", "mkv");
        assert_eq!(s.part, 1);
        assert_eq!(s.extension, "mkv");
        assert!(!s.should_split(SplitBy::None));
    }

    #[test]
    fn session_current_path_includes_dir_and_extension() {
        let s = RecordingSession::start("/tmp/out", "mkv");
        let p = FileNamePattern::new("{name}_{seq}").unwrap();
        let vals = PatternValues {
            name: "game".to_string(),
            date: "".to_string(),
            time: "".to_string(),
        };
        assert_eq!(s.current_path(&p, &vals, ""), "/tmp/out/game_01.mkv");
    }

    #[test]
    fn duration_split_triggers_at_threshold() {
        let mut s = RecordingSession::start("/tmp/out", "mkv");
        s.record_seconds(59);
        assert!(!s.should_split(SplitBy::Duration { seconds: 60 }));
        s.record_seconds(1);
        assert!(s.should_split(SplitBy::Duration { seconds: 60 }));
    }

    #[test]
    fn size_split_triggers_at_threshold() {
        let mut s = RecordingSession::start("/tmp/out", "mkv");
        let split = SplitBy::Size { megabytes: 2 };
        s.record_bytes(2 * 1024 * 1024 - 1);
        assert!(!s.should_split(split));
        s.record_bytes(1);
        assert!(s.should_split(split));
    }

    #[test]
    fn next_part_resets_per_part_state_and_increments() {
        let mut s = RecordingSession::start("/tmp/out", "mkv");
        s.record_bytes(999);
        s.record_seconds(30);
        s.next_part();
        assert_eq!(s.part, 2);
        assert_eq!(s.bytes_written, 0);
        assert_eq!(s.seconds_elapsed, 0);
    }

    #[test]
    fn auto_record_defaults_off() {
        let s = RecordingFileSettings::default();
        assert!(!s.auto_record_with_stream);
        assert_eq!(s.auto_record_dir, "recordings");
    }

    #[test]
    fn pattern_values_from_datetime() {
        // Format-aware check that is independent of the local timezone.
        let now = chrono::Local::now();
        let vals = PatternValues::from_datetime(now);
        // Date always matches %Y-%m-%d regardless of zone.
        let expected_date = now.format("%Y-%m-%d").to_string();
        let expected_time = now.format("%H-%M-%S").to_string();
        assert_eq!(vals.date, expected_date);
        assert_eq!(vals.time, expected_time);
    }
}
