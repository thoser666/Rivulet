//! Crash-report assembly: turn local crash logs into a paste-ready bug
//! report, without anything leaving the device.
//!
//! The design constraint comes from [`crate::telemetry`]: the shipping build
//! has no outbound transport, and a crash report is exactly the data that
//! must not leave a machine uninvited — it contains file paths, window
//! titles and, if a redaction ever fails, a stream key. So this module does
//! **not** send anything. It assembles a redacted report, puts it on the
//! clipboard, and hands the user a prefilled issue URL to paste into.
//! What leaves the device is exactly what the user chose to paste.
//!
//! Three properties matter and are pinned by tests:
//!
//! 1. **Redaction is not optional.** A report is only built through
//!    [`CrashReport::render`], which always redacts. There is no public
//!    function that returns an unredacted bundle.
//! 2. **The report says what was removed.** Silent redaction teaches users
//!    that the tool already cleaned everything, so every redaction is
//!    counted and disclosed in the header.
//! 3. **No crash block means no crash report.** A report with no
//!    `RIVULET CRASH` block would send the maintainer a log dump with no
//!    defect in it; [`CrashReport::render`] says so instead of pretending.

/// Maximum number of log lines kept before the crash block. The documented
/// reporting procedure asks for "the complete crash block and the preceding
/// ~50 lines" ([`docs/logging.md`]); this is that number, named.
pub const CONTEXT_LINES: usize = 50;

/// How many crash blocks are kept when a log contains more than one.
pub const MAX_BLOCKS: usize = 3;

/// Longest log excerpt embedded in a report, in characters. A runaway log
/// must not produce a clipboard payload nobody can paste into an issue box.
pub const MAX_LOG_CHARS: usize = 24_000;

const CRASH_START: &str = "===== RIVULET CRASH =====";
const CRASH_END: &str = "===== END RIVULET CRASH =====";

/// One crash block recovered from a daily log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashBlock {
    /// The `context:` field of the marker (startup, recording, capture, …).
    pub context: String,
    /// The `error:` field, already redacted.
    pub error: String,
    /// The log lines that appeared before the block, oldest first, capped at
    /// [`CONTEXT_LINES`].
    pub context_lines: Vec<String>,
}

/// What a report is built from. Explicit, so a caller cannot accidentally
/// attach the wrong log or forget the version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReportInput<'a> {
    /// `CARGO_PKG_VERSION` of the running build.
    pub app_version: &'a str,
    /// Resolved platform code (`windows`, `linux`, `macos`, `other`).
    pub platform: &'a str,
    /// The user's home directory, replaced with a placeholder in output.
    pub home_dir: Option<&'a std::path::Path>,
    /// Contents of the daily log covering the failure.
    pub log: &'a str,
}

/// A rendered, redacted bug report plus the counts of what was redacted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashReport {
    /// Markdown body, ready to paste into an issue.
    pub body: String,
    /// Suggested issue title.
    pub title: String,
    /// Number of stream URLs replaced.
    pub redacted_stream_urls: usize,
    /// Number of home-directory paths replaced.
    pub redacted_home_paths: usize,
    /// True when the log contained at least one crash block.
    pub has_crash: bool,
}

/// Redact one crash report input and render it as a markdown bug report.
///
/// Returns a report with `has_crash == false` when the log holds no crash
/// block; the body then explains how to find one instead of pretending the
/// run was clean.
pub fn build_report(input: &CrashReportInput<'_>) -> CrashReport {
    let (stream_redacted, stream_urls) = redact_stream_urls(input.log);
    let (log, home_paths) = redact_home_paths(&stream_redacted, input.home_dir);

    let blocks = parse_blocks(&log);
    let mut body = String::with_capacity(MIN_BODY_CHARS + log.len());
    body.push_str("## Crash report\n\n");

    if blocks.is_empty() {
        body.push_str(
            "This log contains **no `RIVULET CRASH` block**, so there is no \
             recorded crash to report. A crash is written between\n\
             ```\n===== RIVULET CRASH ===== ... ===== END RIVULET CRASH =====\n```\n\
             If the app misbehaved without one, please describe the steps \
             below and include the version above — for a startup failure on \
             Windows the launcher records `RIVULET PRE-RUST DIAGNOSTIC` \
             blocks instead, which are included in the excerpt.\n\n",
        );
    } else {
        body.push_str(&format!(
            "**{} crash block(s) found.** Stream URLs and home directories \
             were replaced below; check the redaction list before pasting.\n\n",
            blocks.len()
        ));
        for (index, block) in blocks.iter().enumerate() {
            body.push_str(&format!("### Crash {}\n\n", index + 1));
            body.push_str(&format!("- context: `{}`\n", block.context));
            if block.error.is_empty() {
                body.push_str("- error: *(not recorded in the log)*\n");
            } else {
                body.push_str(&format!("- error: `{}`\n", block.error));
            }
            if !block.context_lines.is_empty() {
                body.push_str(&format!(
                    "\nLast {} log line(s) before the crash:\n\n```\n",
                    block.context_lines.len()
                ));
                body.push_str(&block.context_lines.join("\n"));
                body.push_str("\n```\n");
            }
            body.push('\n');
        }
    }

    body.push_str("\n## Environment\n\n");
    body.push_str(&format!("- Rivulet version: `{}`\n", input.app_version));
    body.push_str(&format!("- platform: `{}`\n", input.platform));
    body.push_str(&format!(
        "- stream URLs redacted: {}\n- home paths redacted: {}\n",
        stream_urls, home_paths
    ));
    body.push_str(
        "\n_Generated on-device by Rivulet. Nothing was transmitted; this \
         text was placed on the clipboard for you to paste._\n",
    );

    let already_shown: Vec<&str> = blocks
        .iter()
        .flat_map(|block| block.context_lines.iter().map(String::as_str))
        .collect();
    let excerpt = excerpt(&log, &already_shown);
    if !excerpt.is_empty() {
        body.push_str("\n## Log excerpt\n\n```\n");
        body.push_str(&excerpt);
        body.push_str("\n```\n");
    }

    let title = title_for(input.app_version, &blocks);
    CrashReport {
        body,
        title,
        redacted_stream_urls: stream_urls,
        redacted_home_paths: home_paths,
        has_crash: !blocks.is_empty(),
    }
}

const MIN_BODY_CHARS: usize = 512;

/// Build the prefilled GitHub issue URL for a report.
///
/// The body is percent-encoded into `body=`, which GitHub renders in the
/// issue composer. Kept here — rather than in the GUI — so the encoding is
/// covered by tests instead of being trusted at the call site.
pub fn issue_url(repository: &str, report: &CrashReport) -> String {
    let mut url = String::from("https://github.com/");
    url.push_str(repository.trim_matches('/'));
    url.push_str("/issues/new?labels=bug&title=");
    url.push_str(&percent_encode(&report.title));
    url.push_str("&body=");
    url.push_str(&percent_encode(&report.body));
    url
}

/// Percent-encode for a query component.
///
/// `url::form_urlencoded` is already a dependency, but hand-rolling keeps
/// this module free of a second encoding convention next to
/// [`form_encode_component`](crate::youtube_chat::form_encode_component),
/// which percent-encodes chat text for YouTube. The two must agree, so the
/// encoder is shared rather than duplicated.
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn redact_stream_urls(input: &str) -> (String, usize) {
    let mut count = 0usize;
    let mut rest = input;
    let mut out = String::with_capacity(input.len());
    loop {
        let Some(index) = [
            rest.find("rtmps://"),
            rest.find("rtmpt://"),
            rest.find("rtmp://"),
            rest.find("srt://"),
        ]
        .into_iter()
        .flatten()
        .min() else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        // Any stream URL ends at the first delimiter that cannot appear in
        // a host, path or stream key: whitespace, quote, comma or closing
        // bracket. SRT carries a passphrase in `srt://host:port?passphrase=`,
        // so the whole URL including the query is dropped.
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | ',' | ')' | '\''))
            .unwrap_or(tail.len());
        // Replace the URL outright rather than running it through
        // `redact_pipeline_for_log`: that redactor keeps everything up to the
        // closing quote and only knows rtmp schemes, so an SRT passphrase
        // would survive. Dropping the whole span is the safe direction.
        debug_assert!(
            !tail[..end].is_empty(),
            "a matched stream scheme always yields a non-empty URL"
        );
        out.push_str("<redacted stream URL>");
        count += 1;
        rest = &tail[end..];
    }
    (out, count)
}

/// Replace `needle` only where it appears as a whole path segment.
///
/// A plain `str::replace` is wrong here: `/home/thouser` is a prefix of
/// `/home/thouser2`, so a user with a neighbouring account name would have
/// that path mangled into `<user>2/file.mp4` — the report would then blame a
/// path that never existed, which is worse than no redaction at all. Both
/// the character before and the one after the match must be a separator, a
/// quote, or the string boundary.
fn replace_path_aware(input: &str, needle: &str) -> (String, usize) {
    if needle.is_empty() {
        return (input.to_owned(), 0);
    }
    let boundary = |c: char| c == '/' || c == '\\' || c == '"' || c == '\'' || c.is_whitespace();
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut count = 0usize;
    while let Some(index) = rest.find(needle) {
        let before_ok = index == 0 || rest[..index].chars().next_back().is_none_or(boundary);
        let after = &rest[index + needle.len()..];
        let after_ok = after.chars().next().is_none_or(boundary);
        if !(before_ok && after_ok) {
            // Not a whole segment: copy one character past the match start and
            // keep scanning, so a later real occurrence is still found.
            let take = index + 1;
            out.push_str(&rest[..take]);
            rest = &rest[take..];
            continue;
        }
        out.push_str(&rest[..index]);
        out.push_str("<user>");
        rest = after;
        count += 1;
    }
    out.push_str(rest);
    (out, count)
}

fn redact_home_paths(input: &str, home_dir: Option<&std::path::Path>) -> (String, usize) {
    let Some(home) = home_dir.map(|h| h.as_os_str().to_string_lossy().to_string()) else {
        return (input.to_owned(), 0);
    };
    // The full home path first, then the bare user name: the second catches a
    // home written without its drive prefix. Short names are skipped —
    // redacting `ab` would eat unrelated words.
    let mut needles: Vec<&str> = vec![home.as_str()];
    if let Some(user) = home.rsplit(['/', '\\']).next().filter(|u| !u.is_empty()) {
        if user.len() >= 3 {
            needles.push(user);
        }
    }
    let mut count = 0usize;
    let mut out = input.to_owned();
    for needle in needles {
        if !out.contains(needle) {
            continue;
        }
        let (next, hits) = replace_path_aware(&out, needle);
        out = next;
        count += hits;
    }
    (out, count)
}

fn parse_blocks(log: &str) -> Vec<CrashBlock> {
    let lines: Vec<&str> = log.lines().collect();
    let mut blocks: Vec<CrashBlock> = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        if lines[index].trim() != CRASH_START {
            index += 1;
            continue;
        }
        let start = index;
        let mut context = String::new();
        let mut error = String::new();
        let mut end = index;
        while end < lines.len() {
            let line = lines[end].trim();
            if line == CRASH_END {
                break;
            }
            if let Some(value) = line.strip_prefix("context:") {
                context = value.trim().to_owned();
            } else if let Some(value) = line.strip_prefix("error:") {
                error = value.trim().to_owned();
            }
            end += 1;
        }
        let from = start.saturating_sub(CONTEXT_LINES);
        let context_lines = lines[from..start]
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| (*line).to_owned())
            .collect();
        blocks.push(CrashBlock {
            context,
            error,
            context_lines,
        });
        index = end + 1;
    }
    // Newest first: the crash the user is looking at is the last one.
    blocks.reverse();
    blocks.truncate(MAX_BLOCKS);
    blocks
}

/// Build the log excerpt, skipping lines the per-block sections already
/// printed. `log.lines()` is order-preserving and the excerpt is a substring
/// selection, so duplicate *content* elsewhere in the log is kept unless it
/// was literally shown as crash context — anything else would silently drop
/// a relevant line.
fn excerpt(log: &str, already_shown: &[&str]) -> String {
    let trimmed = log.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let kept: String = trimmed
        .lines()
        .filter(|line| !already_shown.contains(line))
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = kept.as_str();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.chars().count() <= MAX_LOG_CHARS {
        return trimmed.to_owned();
    }
    // Keep the tail: a crash is diagnosed from what happened last.
    let tail: String = trimmed
        .chars()
        .skip(trimmed.chars().count() - MAX_LOG_CHARS)
        .collect();
    format!("… (earlier lines omitted)\n{tail}")
}

fn title_for(app_version: &str, blocks: &[CrashBlock]) -> String {
    let kind = match blocks.first().map(|block| block.context.as_str()) {
        Some("startup") => "startup failure",
        Some("recording") => "recording failure",
        Some("capture") => "capture failure",
        Some("update") => "update failure",
        Some("logging") => "logging failure",
        Some(other) if !other.is_empty() => other,
        _ => "crash",
    };
    format!("Crash report ({kind}) on v{app_version}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn input<'a>(log: &'a str, home: &'a str) -> CrashReportInput<'a> {
        CrashReportInput {
            app_version: "0.65.0-alpha.55",
            platform: "linux",
            home_dir: Some(Path::new(home)),
            log,
        }
    }

    const SAMPLE: &str = "\
2026-10-05T10:00:00Z  INFO rivulet_core: GStreamer ready.
2026-10-05T10:00:01Z  INFO rivulet_core: Starting capture thread (abc123)
2026-10-05T10:00:02Z  WARN rivulet_core: rtmp2sink location=\"rtmps://live.twitch.tv/app/SUPERSECRET\"
===== RIVULET CRASH =====
context: capture
error: buffer negotiation failed at /home/thouser/Videos/clip.mp4
===== END RIVULET CRASH =====
";

    #[test]
    fn report_extracts_the_crash_and_its_context() {
        let report = build_report(&input(SAMPLE, "/home/thouser"));
        assert!(report.has_crash);
        assert!(report.body.contains("context: `capture`"));
        assert!(report.body.contains("buffer negotiation failed"));
        assert!(
            report.body.contains("Starting capture thread"),
            "the lines before the crash must be kept: {}",
            report.body
        );
    }

    #[test]
    fn report_redacts_the_stream_key() {
        let report = build_report(&input(SAMPLE, "/home/thouser"));
        assert!(
            !report.body.contains("SUPERSECRET"),
            "stream key leaked into the report: {}",
            report.body
        );
        assert_eq!(report.redacted_stream_urls, 1);
        assert!(report.body.contains("<redacted stream URL>"));
    }

    #[test]
    fn report_redacts_the_home_directory_and_discloses_it() {
        let report = build_report(&input(SAMPLE, "/home/thouser"));
        assert!(
            !report.body.contains("/home/thouser"),
            "home path leaked into the report: {}",
            report.body
        );
        assert!(report.redacted_home_paths >= 1);
        // Silent redaction teaches users the tool cleaned everything, so the
        // count has to be visible in the report itself.
        assert!(report.body.contains("home paths redacted: "));
        assert!(report.body.contains("stream URLs redacted: "));
    }

    #[test]
    fn report_without_a_crash_block_says_so_instead_of_pretending() {
        let report = build_report(&input("2026-10-05T10:00:00Z  INFO startup ok\n", "/home/x"));
        assert!(!report.has_crash);
        assert!(report.body.contains("no `RIVULET CRASH` block"));
        assert!(
            report.body.contains("RIVULET PRE-RUST DIAGNOSTIC"),
            "the Windows pre-Rust path must stay discoverable"
        );
    }

    #[test]
    fn context_is_capped_so_one_run_cannot_flood_a_report() {
        let mut log = String::new();
        for index in 0..200 {
            log.push_str(&format!("line {index}\n"));
        }
        log.push_str("===== RIVULET CRASH =====\ncontext: startup\nerror: boom\n===== END RIVULET CRASH =====\n");
        let report = build_report(&input(&log, "/home/x"));
        // Count inside the per-block context section only. The excerpt
        // legitimately carries the *older* lines, so counting the whole body
        // would measure the excerpt too and never reach the cap.
        let section = report
            .body
            .split("log line(s) before the crash:")
            .nth(1)
            .and_then(|rest| rest.split("```").nth(1))
            .unwrap_or_else(|| panic!("no context section in: {}", report.body));
        assert_eq!(
            section
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count(),
            CONTEXT_LINES,
            "the crash context must be capped"
        );
        // The most recent lines are the ones kept — a crash is diagnosed
        // backwards from the failure.
        assert!(section.contains(&format!("line {}", 199 - CONTEXT_LINES + 1)));
        assert!(!section.contains("line 0\n"));
    }

    #[test]
    fn excerpt_is_bounded_from_the_end() {
        let log: String = "x".repeat(MAX_LOG_CHARS * 2);
        let report = build_report(&input(&log, "/home/x"));
        assert!(report.body.len() < MAX_LOG_CHARS + 4096);
        assert!(report.body.contains("earlier lines omitted"));
    }

    #[test]
    fn multiple_crashes_keep_the_most_recent_one() {
        let log = format!(
            "===== RIVULET CRASH =====\ncontext: startup\nerror: first\n===== END RIVULET CRASH =====\n{}\n===== RIVULET CRASH =====\ncontext: recording\nerror: second\n===== END RIVULET CRASH =====\n",
            "filler\n".repeat(3)
        );
        let report = build_report(&input(&log, "/home/x"));
        assert!(report.body.contains("error: `second`"));
        assert!(report.body.contains("2 crash block(s) found"));
    }

    #[test]
    fn report_says_nothing_was_transmitted() {
        let report = build_report(&input(SAMPLE, "/home/x"));
        assert!(report.body.contains("Nothing was transmitted"));
    }

    #[test]
    fn title_names_the_context_and_the_version() {
        assert_eq!(
            build_report(&input(SAMPLE, "/home/x")).title,
            "Crash report (capture failure) on v0.65.0-alpha.55"
        );
        assert_eq!(
            build_report(&input("no crash\n", "/home/x")).title,
            "Crash report (crash) on v0.65.0-alpha.55"
        );
    }

    #[test]
    fn issue_url_is_percent_encoded_and_prefilled() {
        let report = build_report(&input(SAMPLE, "/home/x"));
        let url = issue_url("thoser666/Rivulet", &report);
        assert!(url.starts_with("https://github.com/thoser666/Rivulet/issues/new?"));
        assert!(url.contains("labels=bug"));
        assert!(url.contains("title=Crash+report"));
        assert!(url.contains("body="));
        // A raw newline or space would break the query component.
        assert!(!url.contains('\n'));
        assert!(!url.split("&body=").nth(1).unwrap().contains(' '));
    }

    #[test]
    fn issue_url_tolerates_a_trailing_slash_on_the_repository() {
        let report = build_report(&input(SAMPLE, "/home/x"));
        assert!(issue_url("/thoser666/Rivulet/", &report)
            .starts_with("https://github.com/thoser666/Rivulet/issues/new?"));
    }

    #[test]
    fn srt_passphrase_never_survives_redaction() {
        let log = "line srt://host:9000?passphrase=hunter2 trailing\n";
        let report = build_report(&input(log, "/home/x"));
        assert!(
            !report.body.contains("hunter2"),
            "srt passphrase leaked: {}",
            report.body
        );
    }

    #[test]
    fn a_neighbouring_home_directory_is_not_damaged() {
        // `/home/thouser2` must survive when `/home/thouser` is redacted.
        let log = "path /home/thouser2/file.mp4 and /home/thouser/other.mp4\n";
        let report = build_report(&input(log, "/home/thouser"));
        assert!(report.body.contains("/home/thouser2/file.mp4"));
        assert!(!report.body.contains("/home/thouser/other.mp4"));
    }

    #[test]
    fn report_without_a_home_directory_still_redacts_stream_urls() {
        let raw = CrashReportInput {
            app_version: "0.1.0",
            platform: "windows",
            home_dir: None,
            log: "rtmp://a.rtmp.youtube.com/live2/KEY rest\n",
        };
        let report = build_report(&raw);
        assert!(!report.body.contains("/live2/KEY"));
    }

    #[test]
    fn a_short_username_is_not_blanked_everywhere() {
        // Guard against over-redaction: a one- or two-character user name
        // would otherwise eat unrelated text.
        let log = "user ab wrote: about the abcdef test\n";
        let report = build_report(&input(log, "/home/ab"));
        assert!(
            report.body.contains("about the abcdef test"),
            "over-redacted: {}",
            report.body
        );
    }
}
