//! `rivulet render` — headless rendering from code (M7 W3, issue #189).
//!
//! Three modes over one declarative scene config ([`SceneRenderConfig`]):
//!
//! - `--frame N --png out.png` renders a single still, byte-identical on every
//!   run. This is the mode a CI job uses to prove a composition still looks
//!   like what it is supposed to look like.
//! - `--video out.mp4 --frames N` renders N frames through the encoder under
//!   an injected virtual clock.
//! - `--config-dir scenes/ --out-dir renders/` renders every config in a
//!   directory, one job per config, and emits a machine-readable summary.
//!
//! Every path here is testable without spawning the binary: [`run_render`]
//! only translates a parsed [`RenderArgs`] into exit codes, and the batch
//! driver is a plain function over directories.

use rivulet_core::render::{render_video, RenderError, RenderVideoTarget, SceneRenderConfig};
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::exit_code;

/// Outcome of one batch job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// The job rendered and wrote its outputs.
    Succeeded,
    /// The job failed; `error` says why.
    Failed,
}

/// One config file's result in a batch run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchRenderJob {
    /// Config path, as given on the command line.
    pub config: String,
    pub status: JobStatus,
    /// Rendered container, relative to the output directory.
    pub video: Option<PathBuf>,
    /// Rendered still of `--frame`, relative to the output directory.
    pub png: Option<PathBuf>,
    pub frames: u32,
    /// Duration derived from the frame count and fps, not measured.
    pub duration_secs: f64,
    /// Populated when `status` is `failed`.
    pub error: Option<String>,
}

/// The machine-readable result of a batch run.
///
/// Bumping `schema_version` is required for any breaking change: a CI job
/// consuming this summary must be able to tell that a field it reads is gone
/// rather than silently getting a default.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BatchRenderSummary {
    pub schema_version: u32,
    pub out_dir: PathBuf,
    pub jobs: Vec<BatchRenderJob>,
    pub succeeded: usize,
    pub failed: usize,
}

impl BatchRenderSummary {
    /// Schema version of the emitted summary.
    pub const SCHEMA_VERSION: u32 = 1;

    /// True when every job succeeded.
    pub fn is_success(&self) -> bool {
        self.failed == 0
    }

    /// The summary as pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Parsed `rivulet render` arguments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderArgs {
    /// Single scene config to render.
    pub config: Option<PathBuf>,
    /// Directory of scene configs for a batch run.
    pub config_dir: Option<PathBuf>,
    /// Directory the batch run writes into.
    pub out_dir: Option<PathBuf>,
    /// Which still frame to render.
    pub frame: Option<u64>,
    /// How many frames a video render pushes.
    pub frames: u32,
    /// Single-mode still output.
    pub png: Option<PathBuf>,
    /// Single-mode video output.
    pub video: Option<PathBuf>,
    pub container: Option<RenderVideoTarget>,
    /// Where the batch summary is written (default: `<out-dir>/summary.json`).
    pub summary: Option<PathBuf>,
    /// Print the summary to stdout as well as writing it.
    pub json: bool,
}

/// Default frame count for a video render.
const DEFAULT_FRAMES: u32 = 60;

/// Default number of frames for a batch job's still.
const DEFAULT_BATCH_STILL_FRAME: u64 = 0;

/// Parse `render` flags (everything after the subcommand).
///
/// Rendering has its own flag vocabulary — it configures a composition, not a
/// recording — so it gets its own parser instead of sharing `record`'s flags,
/// which do not apply here.
pub fn parse_render_args(args: &[String]) -> Result<RenderArgs, String> {
    let mut parsed = RenderArgs::default();
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
            "-c" | "--config" => parsed.config = Some(PathBuf::from(value(&mut index)?)),
            "--config-dir" => parsed.config_dir = Some(PathBuf::from(value(&mut index)?)),
            "--out-dir" => parsed.out_dir = Some(PathBuf::from(value(&mut index)?)),
            "--frame" => {
                parsed.frame = Some(
                    value(&mut index)?
                        .parse()
                        .map_err(|_| format!("--frame: not a frame number ({})", args[index]))?,
                )
            }
            "--frames" => {
                parsed.frames = value(&mut index)?
                    .parse()
                    .map_err(|_| format!("--frames: not a number ({})", args[index]))?
            }
            "--png" => parsed.png = Some(PathBuf::from(value(&mut index)?)),
            "--video" => parsed.video = Some(PathBuf::from(value(&mut index)?)),
            "--container" => {
                let raw = value(&mut index)?;
                parsed.container = Some(parse_container(&raw)?);
            }
            "--summary" => parsed.summary = Some(PathBuf::from(value(&mut index)?)),
            "--json" => parsed.json = true,
            other => return Err(format!("unknown flag {other:?} (for `rivulet render`)")),
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_container(raw: &str) -> Result<RenderVideoTarget, String> {
    match raw {
        "mp4" => Ok(RenderVideoTarget::Mp4),
        "mkv" => Ok(RenderVideoTarget::Mkv),
        other => Err(format!(
            "--container: unknown render container {other:?} (expected \"mp4\" or \"mkv\")"
        )),
    }
}

/// Load and validate a scene render config.
///
/// Parse and validation errors are both reported against the file, because
/// from the caller's side "this config is unusable" is one failure.
pub fn load_scene_config(path: &Path) -> Result<SceneRenderConfig, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("{}: cannot read config ({error})", path.display()))?;
    let config: SceneRenderConfig = toml::from_str(&raw)
        .map_err(|error| format!("{}: invalid config ({error})", path.display()))?;
    config
        .validate()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(config)
}

/// Collect the scene configs of a batch directory, in a stable order.
///
/// Sorted by path so the summary is reproducible across filesystems, which
/// `read_dir` does not guarantee.
pub fn collect_batch_configs(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| format!("--config-dir: cannot read {} ({error})", dir.display()))?;
    let mut configs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        })
        .collect();
    configs.sort();
    Ok(configs)
}

/// Renders one config to a container file.
///
/// Injected into [`render_batch_with`] so the batch bookkeeping — ordering,
/// per-job status, output paths, failure isolation — can be tested without
/// an encoder in the loop. The production path passes [`render_video`].
pub type VideoEncoderFn =
    dyn Fn(&SceneRenderConfig, &Path, u32, RenderVideoTarget) -> Result<(), RenderError>;

/// Render every config in `config_dir` into `out_dir` using the real encoder.
pub fn render_batch(
    config_dir: &Path,
    out_dir: &Path,
    frames: u32,
    container: RenderVideoTarget,
    still_frame: u64,
) -> Result<BatchRenderSummary, String> {
    render_batch_with(
        config_dir,
        out_dir,
        frames,
        container,
        still_frame,
        &|config, video, frames, container| {
            render_video(
                config,
                video,
                frames,
                container,
                rivulet_core::VideoEncoder::Software,
            )
            .map(|_| ())
        },
    )
}

/// Render every config in `config_dir` into `out_dir`, one job per config.
///
/// A job that fails does **not** stop the batch: a CI runner wants to know
/// about every broken config, not just the first. The per-job error lands in
/// the summary, and [`BatchRenderSummary::is_success`] decides the process exit
/// code.
pub fn render_batch_with(
    config_dir: &Path,
    out_dir: &Path,
    frames: u32,
    container: RenderVideoTarget,
    still_frame: u64,
    encode: &VideoEncoderFn,
) -> Result<BatchRenderSummary, String> {
    let configs = collect_batch_configs(config_dir)?;
    if configs.is_empty() {
        return Err(format!(
            "--config-dir: {} contains no .toml configs",
            config_dir.display()
        ));
    }
    if frames == 0 {
        return Err("--frames: must be non-zero".to_string());
    }
    std::fs::create_dir_all(out_dir)
        .map_err(|error| format!("--out-dir: cannot create {} ({error})", out_dir.display()))?;

    let mut jobs = Vec::with_capacity(configs.len());
    for config_path in &configs {
        jobs.push(render_one_batch_job(
            config_path,
            out_dir,
            frames,
            container,
            still_frame,
            encode,
        ));
    }

    let succeeded = jobs
        .iter()
        .filter(|job| job.status == JobStatus::Succeeded)
        .count();
    let failed = jobs.len() - succeeded;
    Ok(BatchRenderSummary {
        schema_version: BatchRenderSummary::SCHEMA_VERSION,
        out_dir: out_dir.to_path_buf(),
        jobs,
        succeeded,
        failed,
    })
}

/// Render one config to `<stem>.mp4` plus `<stem>.png` in `out_dir`.
fn render_one_batch_job(
    config_path: &Path,
    out_dir: &Path,
    frames: u32,
    container: RenderVideoTarget,
    still_frame: u64,
    encode: &VideoEncoderFn,
) -> BatchRenderJob {
    let stem = config_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scene".to_string());
    let extension = match container {
        RenderVideoTarget::Mp4 => "mp4",
        RenderVideoTarget::Mkv => "mkv",
    };
    // The job records paths relative to `summary.out_dir`, not absolute ones:
    // a summary that embeds a machine-specific absolute path is neither
    // reproducible nor diffable between runners.
    let video_name = PathBuf::from(format!("{stem}.{extension}"));
    let png_name = PathBuf::from(format!("{stem}.png"));
    let video = out_dir.join(&video_name);
    let png = out_dir.join(&png_name);

    let mut job = BatchRenderJob {
        config: config_path.display().to_string(),
        status: JobStatus::Failed,
        video: None,
        png: None,
        frames,
        duration_secs: 0.0,
        error: None,
    };

    let config = match load_scene_config(config_path) {
        Ok(config) => config,
        Err(error) => {
            // `load_scene_config` already names the file.
            job.error = Some(error);
            return job;
        }
    };

    // The still first: if it fails, there is no point encoding a video of the
    // same broken composition.
    if let Err(error) = config.write_frame_png(still_frame, &png) {
        job.error = Some(format!("{}: {error}", config_path.display()));
        return job;
    }
    job.png = Some(png_name);

    match encode(&config, &video, frames, container) {
        Ok(()) => {
            job.status = JobStatus::Succeeded;
            job.video = Some(video_name);
            job.duration_secs = f64::from(frames) / f64::from(config.fps);
        }
        Err(error) => job.error = Some(format!("{}: {error}", config_path.display())),
    }
    job
}

/// Execute `rivulet render` and return the process exit code.
///
/// Errors are printed to stderr rather than emitted as JSON: the documented
/// contract (spec § CLI surface reference) keeps stdout a machine-readable
/// stream, and the batch summary is written to a file or requested explicitly
/// with `--json`.
pub fn run_render(args: &RenderArgs) -> i32 {
    if args.config_dir.is_some() {
        return run_batch(args);
    }
    let Some(config_path) = args.config.as_ref() else {
        eprintln!(
            "error: `rivulet render` needs --config <file.toml> or --config-dir <dir>\n\n{USAGE}"
        );
        return exit_code::USAGE;
    };
    if args.png.is_none() && args.video.is_none() {
        eprintln!("error: `rivulet render` needs --png <file> or --video <file>\n\n{USAGE}");
        return exit_code::USAGE;
    }

    let config = match load_scene_config(config_path) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code::USAGE;
        }
    };

    if let Some(png) = &args.png {
        let frame = args.frame.unwrap_or(0);
        if let Err(error) = config.write_frame_png(frame, png) {
            eprintln!("error: {error}");
            return exit_code::RUNTIME;
        }
        println!(
            "{}",
            serde_json::json!({
                "event": "frame_rendered",
                "config": config_path.display().to_string(),
                "frame": frame,
                "png": png.display().to_string(),
                "width": config.canvas.width,
                "height": config.canvas.height,
            })
        );
    }

    if let Some(video) = &args.video {
        let frames = if args.frames == 0 {
            DEFAULT_FRAMES
        } else {
            args.frames
        };
        let container = args.container.unwrap_or(RenderVideoTarget::Mp4);
        match render_video(
            &config,
            video,
            frames,
            container,
            rivulet_core::VideoEncoder::Software,
        ) {
            Ok(report) => {
                println!("{}", serde_json::to_string(&report).unwrap_or_default());
            }
            Err(error) => {
                eprintln!("error: {error}");
                return exit_code::RUNTIME;
            }
        }
    }

    exit_code::OK
}

fn run_batch(args: &RenderArgs) -> i32 {
    let (Some(config_dir), Some(out_dir)) = (args.config_dir.as_ref(), args.out_dir.as_ref())
    else {
        eprintln!("error: batch mode needs --config-dir <dir> and --out-dir <dir>\n\n{USAGE}");
        return exit_code::USAGE;
    };
    let frames = if args.frames == 0 {
        DEFAULT_FRAMES
    } else {
        args.frames
    };
    let container = args.container.unwrap_or(RenderVideoTarget::Mp4);

    let summary = match render_batch(
        config_dir,
        out_dir,
        frames,
        container,
        args.frame.unwrap_or(DEFAULT_BATCH_STILL_FRAME),
    ) {
        Ok(summary) => summary,
        Err(error) => {
            eprintln!("error: {error}");
            return exit_code::USAGE;
        }
    };

    let summary_path = args
        .summary
        .clone()
        .unwrap_or_else(|| out_dir.join("summary.json"));
    if let Some(parent) = summary_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                eprintln!("error: creating {}: {error}", parent.display());
                return exit_code::RUNTIME;
            }
        }
    }

    let rendered = match summary.to_json() {
        Ok(json) => json,
        Err(error) => {
            eprintln!("error: encoding summary: {error}");
            return exit_code::RUNTIME;
        }
    };
    if let Err(error) = std::fs::write(&summary_path, format!("{rendered}\n")) {
        eprintln!("error: writing {}: {error}", summary_path.display());
        return exit_code::RUNTIME;
    }

    if args.json {
        println!("{rendered}");
    } else {
        println!(
            "rendered {}/{} configs into {} (summary: {})",
            summary.succeeded,
            summary.jobs.len(),
            out_dir.display(),
            summary_path.display()
        );
    }

    if summary.is_success() {
        exit_code::OK
    } else {
        // The per-job error already names its config, so it is printed as is.
        for job in summary
            .jobs
            .iter()
            .filter(|job| job.status == JobStatus::Failed)
        {
            eprintln!(
                "error: {}",
                job.error.as_deref().unwrap_or("failed without a reason")
            );
        }
        exit_code::RUNTIME
    }
}

/// The `render` section of `rivulet --help`.
pub const USAGE: &str = "\
USAGE:
    rivulet render --config <scene.toml> --frame <n> --png <out.png>
    rivulet render --config <scene.toml> --video <out.mp4> --frames <n>
    rivulet render --config-dir <scenes/> --out-dir <renders/>   # batch

FLAGS (render):
    -c, --config <file>       scene composition TOML (see spec § CLI surface reference)
        --config-dir <dir>    render every .toml in <dir> (batch mode)
        --out-dir <dir>       output directory for batch mode
        --frame <n>           which still frame to render (default 0)
        --frames <n>          how many frames a video render pushes (default 60)
        --png <file>          write a still frame as PNG
        --video <file>        write the render as a video container
        --container <fmt>     mp4 | mkv (default mp4)
        --summary <file>      batch summary JSON (default <out-dir>/summary.json)
        --json                print the batch summary to stdout as well
";

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn scene_toml(name: &str) -> String {
        format!(
            r#"
name = "{name}"
canvas = {{ width = 64, height = 48 }}
fps = 30

[[layers]]
name = "Backdrop"
kind = "Color"
transform = {{ x = 0.0, y = 0.0, width = 64.0, height = 48.0, rotation = 0.0, opacity = 1.0 }}
animated = false

[[layers]]
name = "Motion"
kind = "GameCapture"
transform = {{ x = 8.0, y = 8.0, width = 32.0, height = 24.0, rotation = 0.0, opacity = 1.0 }}
"#
        )
    }

    #[test]
    fn flags_parse_into_every_mode() {
        let single = parse_render_args(&args(&[
            "--config",
            "scene.toml",
            "--frame",
            "12",
            "--png",
            "out.png",
        ]))
        .expect("parse");
        assert_eq!(single.config, Some(PathBuf::from("scene.toml")));
        assert_eq!(single.frame, Some(12));
        assert_eq!(single.png, Some(PathBuf::from("out.png")));

        let video = parse_render_args(&args(&[
            "--config",
            "s.toml",
            "--video",
            "o.mp4",
            "--frames",
            "30",
            "--container",
            "mkv",
        ]))
        .expect("parse");
        assert_eq!(video.frames, 30);
        assert_eq!(video.container, Some(RenderVideoTarget::Mkv));

        let batch = parse_render_args(&args(&[
            "--config-dir",
            "scenes",
            "--out-dir",
            "renders",
            "--json",
        ]))
        .expect("parse");
        assert_eq!(batch.config_dir, Some(PathBuf::from("scenes")));
        assert_eq!(batch.out_dir, Some(PathBuf::from("renders")));
        assert!(batch.json);
    }

    #[test]
    fn an_unknown_container_is_rejected_by_name() {
        let error = parse_render_args(&args(&["--container", "webm"])).unwrap_err();
        assert_eq!(
            error,
            "--container: unknown render container \"webm\" (expected \"mp4\" or \"mkv\")"
        );
    }

    #[test]
    fn a_missing_flag_value_names_the_flag() {
        assert_eq!(
            parse_render_args(&args(&["--config"])).unwrap_err(),
            "missing value for --config"
        );
    }

    #[test]
    fn a_non_numeric_frame_count_is_rejected() {
        let error = parse_render_args(&args(&["--frames", "many"])).unwrap_err();
        assert!(error.starts_with("--frames: not a number"), "got {error}");
    }

    #[test]
    fn a_loaded_config_validates_and_renders() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scene.toml");
        std::fs::write(&path, scene_toml("Single")).expect("write config");
        let config = load_scene_config(&path).expect("load");
        assert_eq!(config.name, "Single");
        assert_eq!(config.layers.len(), 2);
        assert_eq!(config.render_frame(0).rgba.len(), 64 * 48 * 4);
    }

    #[test]
    fn a_config_error_names_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "canvas = { width = 3, height = 4 }").expect("write");
        let error = load_scene_config(&path).unwrap_err();
        assert!(error.contains("broken.toml"), "must name the file: {error}");
        assert!(
            error.contains("canvas"),
            "must name the offending key: {error}"
        );
    }

    #[test]
    fn a_missing_file_is_reported_as_unreadable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = load_scene_config(&dir.path().join("absent.toml")).unwrap_err();
        assert!(error.contains("cannot read config"), "got {error}");
    }

    #[test]
    fn batch_configs_are_collected_in_a_stable_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["c.toml", "a.toml", "b.toml"] {
            std::fs::write(dir.path().join(name), scene_toml(name)).expect("write");
        }
        // A non-TOML file and a subdirectory must both be ignored, or a batch
        // run would try to render them as scenes.
        std::fs::write(dir.path().join("notes.md"), "ignored").expect("write");
        std::fs::create_dir(dir.path().join("nested")).expect("mkdir");

        let configs = collect_batch_configs(dir.path()).expect("collect");
        let names: Vec<_> = configs
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.toml", "b.toml", "c.toml"]);
    }

    #[test]
    fn an_empty_config_dir_is_a_usage_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = render_batch(
            dir.path(),
            &dir.path().join("out"),
            2,
            RenderVideoTarget::Mp4,
            0,
        )
        .unwrap_err();
        assert!(error.contains("no .toml configs"), "got {error}");
    }

    #[test]
    fn zero_frames_is_rejected_before_any_work() {
        let configs = tempfile::tempdir().expect("configs");
        std::fs::write(configs.path().join("a.toml"), scene_toml("A")).expect("write");
        let error = render_batch(
            configs.path(),
            &configs.path().join("out"),
            0,
            RenderVideoTarget::Mp4,
            0,
        )
        .unwrap_err();
        assert_eq!(error, "--frames: must be non-zero");
    }

    #[test]
    fn the_summary_is_machine_readable() {
        // The summary is what a CI job consumes; its shape must be stable and
        // self-describing.
        let summary = BatchRenderSummary {
            schema_version: BatchRenderSummary::SCHEMA_VERSION,
            out_dir: PathBuf::from("renders"),
            jobs: vec![
                BatchRenderJob {
                    config: "scenes/a.toml".to_string(),
                    status: JobStatus::Succeeded,
                    video: Some(PathBuf::from("renders/a.mp4")),
                    png: Some(PathBuf::from("renders/a.png")),
                    frames: 5,
                    duration_secs: 0.2,
                    error: None,
                },
                BatchRenderJob {
                    config: "scenes/b.toml".to_string(),
                    status: JobStatus::Failed,
                    video: None,
                    png: None,
                    frames: 5,
                    duration_secs: 0.0,
                    error: Some("canvas: width and height must be even".to_string()),
                },
            ],
            succeeded: 1,
            failed: 1,
        };

        let json: serde_json::Value =
            serde_json::from_str(&summary.to_json().expect("encode")).expect("decode");
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["succeeded"], 1);
        assert_eq!(json["failed"], 1);
        assert_eq!(json["jobs"][0]["status"], "succeeded");
        assert_eq!(json["jobs"][0]["video"], "renders/a.mp4");
        assert_eq!(json["jobs"][0]["png"], "renders/a.png");
        assert_eq!(json["jobs"][0]["duration_secs"], 0.2);
        assert_eq!(json["jobs"][1]["status"], "failed");
        assert!(
            json["jobs"][1]["error"]
                .as_str()
                .expect("error text")
                .contains("even"),
            "a failed job must say why"
        );
        assert!(!summary.is_success());
    }

    #[test]
    fn a_batch_run_reports_one_job_per_config_including_failures() {
        // AC: a batch run over >=2 configs produces a machine-readable summary
        // with per-job status and output paths. Two configs, one of them
        // broken, so the run also proves a failure is isolated and does not
        // abort the batch.
        let configs = tempfile::tempdir().expect("configs");
        let out = tempfile::tempdir().expect("out");
        std::fs::write(configs.path().join("good.toml"), scene_toml("Good")).expect("write");
        // Odd width: rejected by validation, so the job fails on its own.
        std::fs::write(
            configs.path().join("odd.toml"),
            scene_toml("Odd").replace("width = 64", "width = 63"),
        )
        .expect("write");

        // A stub encoder: the batch bookkeeping is what this test is about, and
        // the real encoder is covered end-to-end by the CI smoke job.
        let stub =
            |_config: &SceneRenderConfig, video: &Path, frames: u32, _target: RenderVideoTarget| {
                std::fs::write(video, vec![0u8; frames as usize])
                    .map_err(|error| RenderError::Io(format!("stub encoder: {error}")))
            };
        let summary = render_batch_with(
            configs.path(),
            out.path(),
            2,
            RenderVideoTarget::Mp4,
            0,
            &stub,
        )
        .expect("batch");

        assert_eq!(summary.jobs.len(), 2, "one job per config");
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 1);
        assert!(!summary.is_success());

        let good = summary
            .jobs
            .iter()
            .find(|job| job.config.ends_with("good.toml"))
            .expect("good job");
        let odd = summary
            .jobs
            .iter()
            .find(|job| job.config.ends_with("odd.toml"))
            .expect("odd job");

        assert_eq!(good.status, JobStatus::Succeeded);
        assert_eq!(good.png, Some(PathBuf::from("good.png")));
        assert_eq!(good.video, Some(PathBuf::from("good.mp4")));
        assert_eq!(good.duration_secs, 2.0 / 30.0);
        assert!(good.error.is_none());

        assert_eq!(odd.status, JobStatus::Failed);
        assert_eq!(odd.video, None);
        assert!(odd.error.as_ref().expect("error").contains("even"));

        // A still needs no encoder, so it is written even here.
        assert!(out.path().join("good.png").is_file());
    }

    #[test]
    fn a_failing_encoder_is_reported_per_job() {
        // The other isolation direction: a config that renders fine but whose
        // encoder fails must be reported as failed, not silently "succeeded".
        let configs = tempfile::tempdir().expect("configs");
        let out = tempfile::tempdir().expect("out");
        std::fs::write(configs.path().join("a.toml"), scene_toml("A")).expect("write");

        let stub = |_config: &SceneRenderConfig,
                    _video: &Path,
                    _frames: u32,
                    _target: RenderVideoTarget| {
            Err(RenderError::Engine("no encoder available".to_string()))
        };
        let summary = render_batch_with(
            configs.path(),
            out.path(),
            2,
            RenderVideoTarget::Mp4,
            0,
            &stub,
        )
        .expect("batch");

        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failed, 1);
        let job = &summary.jobs[0];
        assert_eq!(job.status, JobStatus::Failed);
        assert_eq!(job.video, None);
        assert!(job.png.is_some(), "the still was written before encoding");
        // Every per-job error names its own config, so the batch report can be
        // read without cross-referencing `job.config`.
        let error = job.error.as_deref().expect("error");
        assert!(error.contains("a.toml"), "{error}");
        assert!(
            error.ends_with("engine error: no encoder available"),
            "{error}"
        );
    }
}
