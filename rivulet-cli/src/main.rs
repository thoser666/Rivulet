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

use rivulet_cli::{config::RecordConfig, exit_code, stdout_stderr_sinks, RecordJob};

const USAGE: &str = "\
rivulet — headless recording with the Rivulet engine (M7)

USAGE:
    rivulet record --config <file.toml> [flags]
    rivulet record --output <file.mp4> [flags]   # minimal inline config
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

STREAMS:
    stdout  JSON status events (started/progress/stopped; stable schema)
    stderr  human-readable diagnostics

EXIT CODES:
    0  success (including graceful SIGINT/SIGTERM stop)
    1  runtime failure (engine error, IO error)
    2  invalid usage or config (the error names the offending key)
";

struct RecordArgs {
    config_path: Option<PathBuf>,
    overrides: RecordConfig,
    show_help: bool,
}

fn parse_args(args: &[String]) -> Result<RecordArgs, String> {
    let mut sub = None;
    let mut rest = args;
    if let Some(first) = args.first() {
        if first == "record" {
            sub = Some("record");
            rest = &args[1..];
        } else if first == "help" || first == "--help" || first == "-h" {
            return Ok(RecordArgs {
                config_path: None,
                overrides: RecordConfig::default(),
                show_help: true,
            });
        } else {
            return Err(format!("unknown command {first:?} (expected \"record\")"));
        }
    }
    let _ = sub;

    let mut parsed = RecordArgs {
        config_path: None,
        overrides: RecordConfig::default(),
        show_help: false,
    };
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
            "--help" | "-h" if flag != "-h" || rest.len() == 1 => {
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
            other => return Err(format!("unknown flag {other:?}")),
        }
        i += 1;
    }
    Ok(parsed)
}

fn main() {
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return exit_code::USAGE;
        }
    };
    if parsed.show_help {
        print!("{USAGE}");
        let _ = std::io::stdout().flush();
        return exit_code::OK;
    }

    // Load config file, then apply flag overrides (flags win).
    let mut config = match &parsed.config_path {
        Some(path) => match RecordConfig::load(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                return exit_code::USAGE;
            }
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
        eprintln!("error: {e}\n\n{USAGE}");
        return exit_code::USAGE;
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
        Err(e) => {
            eprintln!("error: {e:#}");
            exit_code::RUNTIME
        }
    }
}
