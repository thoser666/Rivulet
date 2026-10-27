# Downstream-consumer smoke crate (M8 #297)

This directory holds the **external** consumer the M8 exit evidence asks for:
a crate that is not a workspace member, that never sees the working tree, and
that uses `rivulet-core` the way a foreign developer would — as a dependency
from the registry.

Run it with:

```sh
bash scripts/run-downstream-smoke.sh
```

The script

1. runs `cargo package -p rivulet-core --no-verify`, which produces the
   `.crate` file `cargo publish` would upload (normalized manifest, no
   `dev-dependencies`, no test binaries),
2. unpacks that tarball into a build directory under `target/`,
3. writes this crate's manifest from [`Cargo.toml.in`](Cargo.toml.in) with the
   path dependency pointing at the **unpacked artifact**, and
4. builds and runs [`src/main.rs`](src/main.rs) against it.

A committed `Cargo.toml` is deliberately absent: a `path = "../rivulet-core"`
dependency would silently re-test the working tree, which is the failure mode
this smoke crate exists to rule out.

## What the consumer proves

`src/main.rs` is an ordinary embedding program. It

- validates its configuration through the public typed gate, including the
  refusal of a broken configuration, before GStreamer is touched,
- records a **video-only** session end to end: `RecordingJob::start`, RGBA
  frames it generates itself, a `Progress` tick off the session's own event
  stream, pause/resume, `stop()`,
- compares the returned `SessionReport` **and the file on disk** against what
  it pushed, and exits non-zero if anything differs,
- then repeats the same session **with audio**, and
- finally queries structured feature detection (`detect_available_encoders_with_info`)
  for a video encoder without parsing log output.

Because the build happens against the packaged artifact, a public item that
exists only in the working tree (a `pub` API behind an internal path, a missing
re-export) fails here rather than at the next release. The crate's dependency
list is `rivulet-core` alone — it does not even name `tokio`, despite receiving
a `tokio` broadcast receiver from `RecordingJob::start`, because it only calls
methods on it.

### The audio leg and `RIVULET_DOWNSTREAM_REQUIRE_AUDIO`

Audio encoding needs `avenc_aac` (from `gst-libav`). On a host without it,
`RecordingJob::start` refuses with the public typed
`JobError::MissingElements` — the same contract `rivulet-core`'s own suite pins
instead of skipping. The smoke crate therefore accepts that refusal *and says
so in its output* rather than failing, which keeps it runnable on any checkout.

CI sets `RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1`, which turns a missing audio leg
into a failure: the positive audio evidence must not be replaceable by the
refusal branch without anyone noticing. The video-only session has no audio
dependency and must always run.

## Platform scope

CI runs the script on Linux in the `Build & Test` job, where GStreamer and the
plugins the session needs are installed, with
`RIVULET_DOWNSTREAM_REQUIRE_AUDIO=1`. The script itself is plain `bash` and
works on any platform whose checkout can run `cargo`; it is not wired into the
Windows/macOS jobs because it rebuilds the consumer in a second target
directory and those runners would pay that cost for no extra signal.
