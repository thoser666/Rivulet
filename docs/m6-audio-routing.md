# M6 — Multi-Track Audio Routing (Record / Stream)

**Status:** Planned (M6)
**Date:** 2026-09-05
**Tracked in:** Milestone M6 — Creator Toolkit & Interactivity

## Problem

Today Rivulet's Mixer exposes two audio sources: **System** (all desktop audio
mixed) and **Microphone**. Both share a single filter chain and volume. OBS
studio lets streamers route individual apps (game, Spotify, Discord) to
separate tracks with independent filters and volume — essential for clean
recordings and stream mixes where e.g. music plays on stream but not in the
recording, or mic levels differ between record and stream.

## Goal

A **multi-track audio routing system** that lets the user:

1. Capture **individual application audio streams** (game, Spotify, Discord,
   browser, etc.) as separate named sources — not just one "System" mix.
2. Apply **independent filters and volume per source** (per-source filter chain:
   noise gate, compressor, expander, limiter, gain, EQ — same elements as
   today, but owned by each source).
3. Route each source **independently to the Record and Stream outputs**
   (checkbox matrix: Source × Output). A source can go to record only, stream
   only, both, or neither.
4. Persist the full routing configuration (source names, filter settings,
   volume, routing matrix) across restarts.
5. Expose everything through a **localised Mixer UI** (EN + DE i18n parity).

## Scope per platform

| Platform | App-specific capture mechanism | Status |
| --- | --- | --- |
| Windows | WASAPI loopback per window (`AudioEndpointVolume` or `IAudioClient` per process) | Research needed |
| Linux | PipeWire/PulseAudio per-app streams (`pw-record --target=<node>` or `PULSE_SINK` routing) | Research needed |
| macOS | Core Audio per-app capture (requires `kAudioHardwareServiceDeviceProperty_VirtualDevice` or third-party driver) | Research needed |

> **Note:** per-app capture on macOS is limited. A fallback is to capture
> the system loopback and let the user split via routing in OS audio settings.
> The feature must degrade gracefully when per-app capture is unavailable.

## Engine changes

### New types

```rust
/// A named audio source — replaces the hardcoded System/Microphone pair.
pub struct AudioSource {
    pub id: AudioSourceId,       // unique, persisted
    pub name: String,            // user-editable, e.g. "Spotify"
    pub capture_backend: AudioCaptureBackend, // platform-specific
    pub volume: f64,             // 0.0 – 2.0, default 1.0
    pub muted: bool,
    pub filters: AudioFilterChain, // per-source filters
    pub routing: AudioRouting,     // record/stream matrix
}

/// Which outputs receive this source.
pub struct AudioRouting {
    pub record: bool,
    pub stream: bool,
}

/// Filter chain — same GStreamer elements as today, owned per-source.
pub struct AudioFilterChain {
    pub noise_gate: Option<NoiseGateConfig>,
    pub compressor: Option<CompressorConfig>,
    pub expander: Option<ExpanderConfig>,
    pub limiter: Option<LimiterConfig>,
    pub gain_db: f64,
    pub eq: Option<EqConfig>, // 10-band, same as today
}
```

### Engine API additions

```rust
impl RivuletEngine {
    // Source management
    pub fn add_audio_source(&mut self, source: AudioSource) -> AudioSourceId;
    pub fn remove_audio_source(&mut self, id: AudioSourceId);
    pub fn audio_sources(&self) -> &[AudioSource];
    pub fn set_audio_source_volume(&mut self, id: AudioSourceId, volume: f64);
    pub fn set_audio_source_muted(&mut self, id: AudioSourceId, muted: bool);
    pub fn set_audio_source_routing(&mut self, id: AudioSourceId, routing: AudioRouting);
    pub fn set_audio_source_filters(&mut self, id: AudioSourceId, filters: AudioFilterChain);
}
```

### Pipeline changes

The GStreamer pipeline gains per-source filter chains. Each audio source gets:

```
[app capture] ! queue ! audioconvert ! audioresample !
  [per-source filters: gate ! compressor ! expander ! gain ! EQ] !
  [routing tee] ──┬── record_tee → record mux
                  └── stream_tee → stream mux
```

When `separate_audio_tracks` is enabled for recording, each routed source
becomes its own AAC-encoded track in the MP4/MKV container. For streaming
(FLV), routed sources are mixed into the single FLV audio track (FLV only
supports one).

### Backward compatibility

- The existing `AudioTrack::System` / `AudioTrack::Microphone` pair becomes
  the default two sources when no user-configured sources exist.
- `push_audio_track()` continues to work for external callers; the engine
  maps it to the legacy System source.
- The `separate_audio_tracks` toggle remains but now controls per-source
  AAC encoding rather than a fixed two-track layout.

## GUI changes

### Mixer view

- **Source list:** each audio source appears as a row with name, volume
  slider, mute toggle, and filter button (opens filter panel).
- **Routing matrix:** two checkboxes per source row — "Record" and "Stream".
  Headers are column labels. Grayed-out when the output is not active.
- **Add / Remove:** "+" button to add a new source (platform-dependent:
  WASAPI app picker on Windows, PipeWire node picker on Linux, loopback
  driver hint on macOS); trash icon to remove.
- **Filter panel:** same filters as today (noise gate, compressor, expander,
  limiter, gain, 10-band EQ), but scoped to the selected source.

### Inline mixers in Record / Stream views

The same per-source controls (volume, mute, filter) must be reachable without
leaving the active workflow — the Mixer sidebar alone is not enough:

- **Record view:** a compact mixer strip under the preview (volume + mute per
  source, routing badges "R"/"S", filter button) so recording start/stop never
  requires a view switch.
- **Stream view:** extend the existing audio section into the same inline
  mixer (volume + mute per source + live level meters), consistent with the
  Record strip. The full routing matrix stays in the Mixer view.
- Both inline mixers share the same rendered controls as the full Mixer view
  (single implementation, three placements) to avoid UI drift.

This also fixes the macOS gap: with per-source volume/filters becoming part of
the M6 routing feature, the macOS mixer must ship the same controls as
Windows/Linux (closing the M5 mixer follow-up from `docs/macos-recording.md`).

### Settings

- Persisted in `eframe::Storage` under a versioned JSON schema:
  `audio_routing_v1: { sources: [...], routing: [...] }`
- Secrets / tokens are never stored here (same rule as M11 P1).

## i18n keys (new)

| Key | EN | DE |
| --- | --- | --- |
| `audio_source_add` | Add Source | Quelle hinzufügen |
| `audio_source_remove` | Remove | Entfernen |
| `audio_source_name` | Source name | Quellenname |
| `audio_routing_record` | Record | Aufnahme |
| `audio_routing_stream` | Stream | Stream |
| `audio_routing_hint` | Choose which outputs receive this source | Wähle, welche Ausgaben diese Quelle erhalten |
| `audio_filter_per_source` | Filters for {name} | Filter für {name} |
| `audio_capture_app_select` | Select application audio | Anwendungs-Audio auswählen |
| `audio_capture_loopback_hint` | System loopback required — install BlackHole (macOS) or configure PipeWire (Linux) | System-Loopback erforderlich — installiere BlackHole (macOS) oder konfiguriere PipeWire (Linux) |

## Quality gate (M6-specific)

The following M6-specific checks must pass before this feature ships:

- [ ] Per-source volume, mute, and filter settings survive a serialize →
  deserialize round-trip (unit test).
- [ ] Routing matrix: a source routed to record only does NOT appear in the
  stream output, and vice versa (integration test with mock pipeline).
- [ ] At least one source routed to record: recording starts without error.
  Zero sources routed: recording starts with a warning and no audio tracks.
- [ ] Stream mix contains exactly the sources with `stream = true` (FLV
  single-track verification).
- [ ] Platform fallback: on macOS where per-app capture is unavailable, the
  UI shows the loopback hint and the system source acts as the single
  "System" capture (behaviour test).
- [ ] CPU/memory/frame-time stays within the M6 resource budget when
  5+ sources with full filter chains are active simultaneously.
- [ ] i18n parity: every new key exists in EN and DE locale files
  (`ci_pinning` guard).

## Out of scope (this feature)

- VST3 plugin hosting (tracked separately in `docs/vst3.md`).
- Cloud/streaming remote audio routing.
- MIDI-triggered audio source switching.
- ASIO backend (Windows pro-audio niche; follow-up if requested).
