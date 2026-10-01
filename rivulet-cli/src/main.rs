//! `rivulet` — headless command-line companion for the Rivulet engine
//! (M7 W1, issue #186). The binary is a thin wrapper around the
//! [`rivulet_cli`] library: JSON status events on stdout, human diagnostics
//! on stderr, documented exit codes, graceful SIGINT/SIGTERM.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use rivulet_cli::{
    config::RecordConfig, exit_code, inspect_json, inspect_with_features, stdout_stderr_sinks,
    RecordJob, RunFailure,
};

const USAGE: &str = "\
rivulet — headless recording with the Rivulet engine (M7)

USAGE:
    rivulet record --config <file.toml> [flags]
    rivulet record --output <file.mp4> [flags]   # minimal inline config
    rivulet inspect --config <file.toml> [--json] # print the pipeline, don't record
    rivulet render --config <scene.toml> --frame <n> --png <out.png>
    rivulet render --config-dir <scenes/> --out-dir <renders/>   # batch
    rivulet help                                 # show this help

FLAGS (record):
    -c, --config <file>       TOML config file (see spec § CLI surface reference)
    -o, --output <file>       output path (overrides config output.path)
    -d, --duration <secs>     stop after N seconds (overrides config)
    -w, --width <px>          video width (default 640)
    -h, --height <px>         video height (default 360)
    -f, --fps <n>             video frame rate (default 30)
    -a, --audio               enable the silent test-audio track
        --container <fmt>     mp4 | mkv | mov | mpegts (default mp4)
        --dry-run             print the pipeline and exit without recording

FLAGS (inspect):
    -c, --config <file>       TOML config file
    -o, --output <file>       output path (overrides config output.path)
        --container <fmt>     mp4 | mkv | mov | mpegts (default mp4)
    -a, --audio               include the audio branch in the pipeline
        --json                machine-readable report (pipeline + capabilities)

FLAGS (render):
    -c, --config <file>       scene composition TOML
        --config-dir <dir>    render every .toml in <dir> (batch mode)
        --out-dir <dir>       output directory for batch mode
        --frame <n>           which still frame to render (default 0)
        --frames <n>          frames a video render pushes (default 60)
        --png <file>          write a still frame as PNG
        --video <file>        write the render as a video container
        --container <fmt>     mp4 | mkv (default mp4)
        --summary <file>      batch summary JSON (default <out-dir>/summary.json)
        --json                print the batch summary to stdout as well

STREAMS:
    stdout  JSON status events (started/progress/stopped, or failed on error)
    stderr  human-readable diagnostics

EXIT CODES:
    0  success (including graceful SIGINT/SIGTERM stop)
    1  runtime failure (engine error, IO error)
    2  invalid usage or config (the error names the offending key)

record/inspect failures end with a machine-readable object naming the stage:
    {\"event\": \"failed\", \"stage\": \"engine\", \"exit_code\": 1, \"message\": ...}
";

/// Which subcommand the arguments selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    Record,
    Inspect,
    Render,
}

struct RecordArgs {
    command: Command,
    config_path: Option<PathBuf>,
    overrides: RecordConfig,
    /// `inspect --json`: emit the machine-readable report.
    json: bool,
    /// `record --dry-run`: build the pipeline and stop.
    dry_run: bool,
    show_help: bool,
    /// `render`: its own flags. Rendering configures a composition rather than
    /// a recording, so its vocabulary does not overlap with `record`'s.
    render: rivulet_cli::render::RenderArgs,
}

fn parse_args(args: &[String]) -> Result<RecordArgs, String> {
    let mut command = Command::Record;
    let mut rest = args;
    if let Some(first) = args.first() {
        match first.as_str() {
            "record" => {
                command = Command::Record;
                rest = &args[1..];
            }
            "inspect" => {
                command = Command::Inspect;
                rest = &args[1..];
            }
            "render" => {
                command = Command::Render;
                rest = &args[1..];
            }
            "help" | "--help" | "-h" => {
                return Ok(RecordArgs {
                    command,
                    config_path: None,
                    overrides: RecordConfig::default(),
                    json: false,
                    dry_run: false,
                    show_help: true,
                    render: rivulet_cli::render::RenderArgs::default(),
                })
            }
            other => {
                return Err(format!(
                    "unknown command {other:?} (expected \"record\", \"inspect\" or \"render\")"
                ))
            }
        }
    }

    let mut parsed = RecordArgs {
        command,
        config_path: None,
        overrides: RecordConfig::default(),
        json: false,
        dry_run: false,
        show_help: false,
        render: rivulet_cli::render::RenderArgs::default(),
    };

    // `render` has its own flag vocabulary, so it is parsed by its own parser
    // and never sees the recording flags below.
    if command == Command::Render {
        if rest.iter().any(|arg| arg == "--help" || arg == "-h") {
            parsed.show_help = true;
            return Ok(parsed);
        }
        parsed.render = rivulet_cli::render::parse_render_args(rest)?;
        return Ok(parsed);
    }
    let mut i = 0;
    while i < rest.len() {
        let flag = rest[i].as_str();
        let value = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            rest.get(*i)
                .cloned()
                .ok_or_else(|| format!("missing value for {flag}"))
        };
        match flag {
            "--help" => {
                parsed.show_help = true;
            }
            "-c" | "--config" => parsed.config_path = Some(PathBuf::from(value(&mut i)?)),
            "-o" | "--output" => parsed.overrides.output.path = Some(PathBuf::from(value(&mut i)?)),
            "-d" | "--duration" => {
                parsed.overrides.duration_secs =
                    Some(value(&mut i)?.parse().map_err(|_| {
                        format!("--duration: not a number of seconds ({})", rest[i])
                    })?)
            }
            "-w" | "--width" => {
                parsed.overrides.video.width = value(&mut i)?
                    .parse()
                    .map_err(|_| format!("--width: not a number ({})", rest[i]))?
            }
            "--height" => {
                parsed.overrides.video.height = value(&mut i)?
                    .parse()
                    .map_err(|_| format!("--height: not a number ({})", rest[i]))?
            }
            "-f" | "--fps" => {
                parsed.overrides.video.fps = value(&mut i)?
                    .parse()
                    .map_err(|_| format!("--fps: not a number ({})", rest[i]))?
            }
            "-a" | "--audio" => parsed.overrides.audio.enabled = true,
            "--container" => parsed.overrides.output.container = Some(value(&mut i)?),
            "--json" => parsed.json = true,
            "--dry-run" => parsed.dry_run = true,
            other => return Err(format!("unknown flag {other:?}")),
        }
        i += 1;
    }
    // Recording-only flags are a usage error under `inspect` rather than a
    // silent no-op: the user asked for something this subcommand cannot do.
    if command == Command::Inspect {
        for (present, flag) in [
            (parsed.overrides.duration_secs.is_some(), "--duration"),
            (parsed.dry_run, "--dry-run"),
        ] {
            if present {
                return Err(format!("{flag}: only valid for `rivulet record`"));
            }
        }
        if parsed.overrides.video.width != RecordConfig::default().video.width
            || parsed.overrides.video.height != RecordConfig::default().video.height
            || parsed.overrides.video.fps != RecordConfig::default().video.fps
        {
            return Err(
                "--width/--height/--fps: only valid for `rivulet record` (set video.* in the config)"
                    .to_string(),
            );
        }
    }
    Ok(parsed)
}

fn main() {
    let code = run();
    std::process::exit(code);
}

/// Emit a failure the documented way (M7 W5): the machine-readable `failed`
/// object on stdout, appended to the JSON status stream, plus the human line on
/// stderr. The two streams never mix.
fn report_failure(failure: &RunFailure) -> i32 {
    println!("{}", failure.to_json());
    eprintln!("{}", failure.describe());
    failure.exit_code()
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args(&args) {
        Ok(p) => p,
        Err(e) => {
            let failure = RunFailure::usage(e);
            println!("{}", failure.to_json());
            eprintln!("{}\n\n{USAGE}", failure.describe());
            return failure.exit_code();
        }
    };
    if parsed.show_help {
        // `render --help` shows the render surface on its own; `rivulet help`
        // shows the overall usage (which summarizes every subcommand).
        if parsed.command == Command::Render {
            print!("{}", rivulet_cli::render::USAGE);
        } else {
            print!("{USAGE}");
        }
        let _ = std::io::stdout().flush();
        return exit_code::OK;
    }

    if parsed.command == Command::Render {
        // Rendering builds a composition from a scene config, so it must not
        // go through the recording config path (and its `output.path` default).
        return rivulet_cli::render::run_render(&parsed.render);
    }

    // Load config file, then apply flag overrides (flags win).
    let mut config = match &parsed.config_path {
        Some(path) => match RecordConfig::load(path) {
            Ok(c) => c,
            Err(e) => return report_failure(&RunFailure::config(e)),
        },
        None => RecordConfig::default(),
    };
    if parsed.overrides.output.path.is_some() {
        config.output.path = parsed.overrides.output.path;
    }
    if parsed.overrides.output.container.is_some() {
        config.output.container = parsed.overrides.output.container;
    }
    if parsed.overrides.duration_secs.is_some() {
        config.duration_secs = parsed.overrides.duration_secs;
    }
    if parsed.overrides.video.width != config.video.width {
        config.video.width = parsed.overrides.video.width;
    }
    if parsed.overrides.video.height != config.video.height {
        config.video.height = parsed.overrides.video.height;
    }
    if parsed.overrides.video.fps != config.video.fps {
        config.video.fps = parsed.overrides.video.fps;
    }
    if parsed.overrides.audio.enabled {
        config.audio.enabled = true;
    }

    // Validate up front so a bad config exits 2 with the offending key.
    if let Err(e) = config.validate() {
        let failure = RunFailure::config(e).with_config(config.clone());
        println!("{}", failure.to_json());
        eprintln!("{}\n\n{USAGE}", failure.describe());
        return failure.exit_code();
    }

    if parsed.command == Command::Inspect {
        return run_inspect(config, parsed.json);
    }

    // A dry run reuses the record path's config-to-engine mapping and stops
    // before any frame is pushed, so no file is created.
    if parsed.dry_run {
        return match RecordJob::new(config).dry_run() {
            Ok(report) => {
                println!("{}", report.describe());
                exit_code::OK
            }
            Err(failure) => report_failure(&failure),
        };
    }

    // Graceful SIGINT/SIGTERM: flip the flag; the job stops between frames
    // and finalizes the container (spec: exit code 0 on graceful stop).
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .expect("installing the signal handler");
    }

    let (events, diags) = stdout_stderr_sinks();
    let mut job = RecordJob::new(config)
        .with_event_sink(events)
        .with_diagnostic_sink(diags);

    match job.run(|| stop.load(Ordering::SeqCst)) {
        Ok(path) => {
            eprintln!("done: {}", path.display());
            exit_code::OK
        }
        Err(failure) => report_failure(&failure),
    }
}

/// `rivulet inspect`: print the pipeline the engine would build, plus (with
/// `--json`) a capability report. Never records, never creates the output file.
fn run_inspect(config: RecordConfig, json: bool) -> i32 {
    let outcome = if json {
        inspect_json(config)
    } else {
        inspect_with_features(config).map(|report| report.describe())
    };
    match outcome {
        Ok(body) => {
            println!("{body}");
            exit_code::OK
        }
        Err(failure) => report_failure(&failure),
    }
}
