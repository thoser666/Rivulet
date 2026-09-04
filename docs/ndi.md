# NDI output (LAN monitor feed)

NDI (Network Device Interface) lets Rivulet publish the **encoded H.264 video**
of an active session as an NDI source on the local network — for LAN
contribution and monitoring (e.g. a second machine in the studio, a hardware
switcher, or NDI Tools). This implements the M5 roadmap item
[#77](https://github.com/thoser666/Rivulet/issues/77) ("NDI output selectable
as a destination").

## Requirements

- The **NewTek NDI GStreamer plugin** with the `ndisink` element must be
  installed in the GStreamer installation Rivulet uses (the plugin is
  platform-specific and **not** part of the standard `gstreamer1.0-plugins-*`
  packages).
- Publishing happens **next to a session**: recording, streaming, or dual
  output (record + stream). There is no standalone "NDI only" mode yet; the
  feed starts with the session and ends when the session stops.
- NDI is **off by default** — a feed is never announced unintentionally.

## Configuration

Two layers configure NDI:

**Engine (`rivulet-core`)**

```rust
engine.set_ndi_output(Some(NdiOutput::new("Rivulet Monitor", true)))?;
engine.set_ndi_output(None); // disable
```

`NdiOutput` validates the source name (non-empty, ≤ 80 chars) and an optional
group filter; hostile names (embedded quotes) are escaped so they cannot break
out of the GStreamer parse string. `set_ndi_output` returns `Err` for an
invalid configuration and does not mutate the engine state.

**GUI (Settings → “NDI output (LAN)”)**

- Enable/disable the feed.
- NDI source name (default `Rivulet`).
- Optional NDI group (e.g. VLAN workflows).
- Warnings for an empty name and for a missing `ndisink` plugin in the current
  GStreamer installation.

The settings are persisted and applied to the engine before every session
start (recording on every platform, streaming, dual output).

## How the feed is wired (pipeline)

The NDI sink taps the **encoded** video stream *before* the container/FLV
muxer, because NDI carries elementary H.264 and must not see the muxed
container bitstream:

- **Recording** and **streaming** pipelines: the video branch fans out through
  a tee (`ndi_vtee`) that feeds the session muxer (`mux`) on the main path and
  `queue ! h264parse ! ndisink` on the feed path.
- **Dual output** (record + stream): the feed is an extra output of the
  existing `video_tee` (`video_tee. ! queue ! h264parse ! ndisink`).
- When the **replay buffer** is also enabled, both the replay appsink and the
  NDI sink share the same video tee — no second tee is introduced.

Audio is not part of the NDI source yet (video-only monitoring feed).

## Verification

- **Unit/contract tests** (`rivulet-core`): the NDI feed is embedded in the
  recording, streaming, and dual-output pipeline strings only when active, is
  absent by default, plays alongside the replay buffer, and invalid names are
  rejected without mutating state. `NdiOutput`'s own tests cover validation,
  escaping, group fragment, and the deterministic plugin probe.
- **Plugin probe**: `NdiOutput::plugin_available()` (via
  `ElementFactory::find("ndisink")`) reports whether the current GStreamer
  installation can publish. Settings shows the result; without the plugin,
  enabling NDI makes the session start fail with a clear `no element "ndisink"`
  error (the same behavior as configuring an SRT/RIST contribution sink whose
  plugin is missing).
- **Live LAN check (manual)**: install the NDI plugin, enable the feed in
  Settings, start a recording or stream, then discover the source on another
  machine (NDI Tools / a second Rivulet or OBS with NDI output) under the
  configured name. The feed ends when the session stops.

## Limitations / follow-ups

- Video-only feed (no NDI audio source yet).
- No standalone “publish NDI without recording/streaming” session mode.
- LAN interop is documented, not exercised in CI (CI has no NewTek NDI
  runtime); pipeline embedding and plugin probes are covered deterministically.
