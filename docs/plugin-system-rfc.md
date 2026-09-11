# RFC: Rivulet Plugin System

**Status:** Draft  
**Author:** Buffy (AI)  
**Target Milestone:** M11 – Extensible UI & Plugin Platform  
**Depends on:** M5 VST3 Host Boundary (Z96), P1–P2 (Layout Persistence, View Registry)

## Abstract

Rivulet needs a plugin system that lets third-party developers extend the
application (UI panels, audio/video effects, streaming integrations, VST3
processing) **without** compromising streaming reliability, user privacy, or
process stability. This RFC defines the manifest format, sandbox model, host
API, capability system, and lifecycle that make that possible.

The design builds directly on the VST3 host boundary (Z96) already shipped in
M5: the `Loaded`/`Skipped` result pattern, skip-on-error semantics, and
COM-sandbox isolation on Windows are the architectural precedents.

---

## 1. Design Goals

| # | Goal | Non-Goal |
|---|------|----------|
| G1 | **Crash isolation** — a plugin fault cannot terminate the GUI or pipeline | Unlimited access to host internals |
| G2 | **Capability-gated** — plugins declare what they need; host denies by default | Implicit trust or blanket permissions |
| G3 | **Deterministic loading** — manifest-first, no runtime discovery or registry | Hot-reload (future, not this RFC) |
| G4 | **API stability** — semver-gated plugin API; host can reject incompatible plugins | Supporting every historical API version |
| G5 | **Cross-platform** — WASM (primary) and native DLY (OBS-compat bridge) | Replacing VST3 with WASM (VST3 stays native) |

---

## 2. Plugin Types

Rivulet supports four plugin categories. Each category has a distinct
integration point but shares the same manifest, sandbox, and lifecycle model.

| Category | Integration Point | Description |
|----------|-------------------|-------------|
| **UI Panel** | View Registry (P2) | Adds a sidebar view, dock panel, or settings page |
| **Audio Effect** | VST3 Chain / Audio Track | Process audio samples (effect, analyzer, VST3 wrapper) |
| **Video Filter** | Engine Pipeline | Modify frames before encoding (overlay, color grading, AI upscale) |
| **Integration** | Chat / Alerts / Webhook | Connect to external services (Discord bot, StreamElements, custom alerts) |

---

## 3. Manifest Format

Every plugin ships with a `rivulet-plugin.toml` manifest at the bundle root.
The manifest is the **sole source of truth** for plugin identity, capabilities,
and compatibility. No runtime introspection is performed.

```toml
[plugin]
id = "com.example.my-plugin"        # Reverse-DNS, globally unique
version = "1.2.0"                    # Semver
api_version = "1.0"                  # Host API compatibility (major.minor)
name = "My Cool Plugin"
description = "Adds a real-time FPS counter overlay"
author = "Example Dev"
license = "MIT"
homepage = "https://example.com/my-plugin"

[plugin.type]
kind = "ui_panel"                    # ui_panel | audio_effect | video_filter | integration
entry_point = "plugin.wasm"          # WASM binary or .dly for native bridge

[plugin.api_version]
min = "1.0"                          # Minimum host API version
max = "1.2"                          # Maximum host API version (semver range)

[plugin.capabilities]
# What this plugin needs. Host denies by default; user is prompted on first load.
ui = true                            # Render to a GUI panel
audio_in = false                     # Read audio samples
audio_out = false                    # Write audio samples
video_in = false                     # Read video frames
video_out = false                    # Write video frames
network = []                         # Allowed hostnames (empty = no network)
filesystem = []                      # Allowed directories (empty = no FS)
secrets = false                      # Access to stored credentials (always denied for WASM)
capture = false                      # Access to capture sources
chat = false                         # Read/write chat messages
alerts = false                       # Trigger or modify alerts

[plugin.resources]
max_memory_mb = 128                  # WASM linear memory limit
max_cpu_ms = 50                      # Max CPU time per frame/process call
timeout_ms = 5000                    # Max wall-clock time for init/process
max_file_size_mb = 10                # Max file I/O per operation

[plugin.compatibility]
min_host_version = "0.65.0"          # Minimum Rivulet version
max_host_version = "1.0.0"           # Maximum Rivulet version (optional)
platforms = ["windows", "linux", "macos"]  # Platform filter
gstreamer_plugins = []               # Required GStreamer plugins (for video_filter)
```

### 3.1 Manifest Validation Rules

1. `id` must be reverse-DNS, alphanumeric with dots/dashes only.
2. `version` and `api_version` must be valid semver.
3. `kind` must be one of the four categories.
4. `entry_point` must reference a file that exists in the bundle.
5. `capabilities.network` entries must be valid hostnames or CIDR ranges.
6. `capabilities.filesystem` entries must be absolute paths.
7. `resources.max_memory_mb` must be ≤ 512 (hard cap).
8. `resources.max_cpu_ms` must be ≤ 200 (hard cap).

Validation errors are **fatal** — the plugin is loaded as `Skipped` with the
error reason logged. No fallback or partial loading.

---

## 4. Sandbox Model

### 4.1 WASM (Primary)

All UI panel, audio effect, video filter, and integration plugins run inside a
WASI-based WASM sandbox. The host provides a restricted set of host functions
(called "imports") that the plugin can call.

```
┌─────────────────────────────────────────────────┐
│  Rivulet Host Process                            │
│  ┌───────────────────────────────────────────┐  │
│  │  Plugin Host (WASM Runtime)                │  │
│  │  ┌─────────────────────────────────────┐  │  │
│  │  │  Plugin WASM Module                 │  │  │
│  │  │  (sandboxed, no direct syscalls)    │  │  │
│  │  └─────────────────────────────────────┘  │  │
│  │  Imports: log, ui_render, audio_read,     │  │
│  │          video_read, http_get, fs_read    │  │
│  └───────────────────────────────────────────┘  │
│  Capabilities: { ui: true, audio_in: true, ... }│
│  Resource Limits: { memory: 128MB, cpu: 50ms }  │
└─────────────────────────────────────────────────┘
```

**Key properties:**
- No direct syscalls (network, filesystem, process)
- All I/O goes through host-provided imports
- Memory limited by WASM linear memory + host cap
- CPU time measured per call; exceeded → plugin is paused
- Crash in WASM does not propagate to host (trap → `Skipped`)

### 4.2 Native DLY Bridge (OBS Compat)

For plugins that cannot run in WASM (VST3, native OBS plugins), Rivulet
provides a native dynamic library bridge. This is explicitly marked as
**higher trust** and requires user confirmation.

```toml
[plugin.type]
kind = "audio_effect"
entry_point = "plugin.vst3"          # Native binary, not WASM
sandbox = "native_dly"               # Explicit opt-in
```

The native bridge uses the same lifecycle and capability model but cannot
enforce memory/CPU limits at the WASM level. Instead:
- Plugins run in a **separate thread** (not the GUI thread)
- Timeout enforcement via `std::panic::catch_unwind`
- COM isolation on Windows (same pattern as Z96-2)
- User must explicitly approve native plugins

### 4.3 Resource Limits Enforcement

| Resource | WASM Enforcement | Native Enforcement |
|----------|------------------|--------------------|
| Memory | WASM linear memory cap + host allocator | Not enforceable (documented limitation) |
| CPU | Per-call timing + `wasmtime` fuel | Thread timeout + `catch_unwind` |
| Wall-clock | Call timeout + abort | Thread join timeout |
| File I/O | Host import whitelist | Not enforceable (user-approved) |
| Network | Host import whitelist | Not enforceable (user-approved) |

---

## 5. Host API

The host API is the set of functions a plugin can call. These are exposed as
WASM imports (for WASM plugins) or as a C ABI (for native DLY plugins).

### 5.1 Core Imports

```rust
// Host function signatures (WASM imports)

/// Log a message at the given level.
fn host_log(level: u32, ptr: *const u8, len: u32);

/// Read the current plugin configuration (manifest values).
fn host_config_read(key_ptr: *const u8, key_len: u32, 
                    val_buf: *mut u8, val_buf_len: u32) -> i32;

/// Write plugin configuration (persisted by host).
fn host_config_write(key_ptr: *const u8, key_len: u32,
                     val_ptr: *const u8, val_len: u32) -> i32;

/// Get the current timestamp (nanoseconds since epoch).
fn host_time_now() -> u64;

/// Request the host to re-render the plugin's UI panel.
fn host_ui_invalidate();
```

### 5.2 Capability-Gated Imports

Each gated import is only available when the corresponding capability is
declared in the manifest. The host returns an error if the plugin calls a
gated import without the capability.

```rust
// Audio capability
fn host_audio_read_sample(track_id: u32, frame: u32) -> f32;
fn host_audio_write_sample(track_id: u32, frame: u32, sample: f32);

// Video capability
fn host_video_read_frame(frame_id: u32, buf: *mut u8, buf_len: u32) -> i32;
fn host_video_write_frame(frame_id: u32, buf: *const u8, buf_len: u32) -> i32;

// Network capability (URL must match manifest whitelist)
fn host_http_get(url_ptr: *const u8, url_len: u32,
                 resp_buf: *mut u8, resp_buf_len: u32) -> i32;

// Filesystem capability (path must match manifest whitelist)
fn host_fs_read(path_ptr: *const u8, path_len: u32,
                buf: *mut u8, buf_len: u32) -> i32;
fn host_fs_write(path_ptr: *const u8, path_len: u32,
                 data: *const u8, data_len: u32) -> i32;

// Chat capability
fn host_chat_send(platform: u32, msg_ptr: *const u8, msg_len: u32) -> i32;
fn host_chat_read(platform: u32, buf: *mut u8, buf_len: u32) -> i32;

// Alert capability
fn host_alert_trigger(kind: u32, data_ptr: *const u8, data_len: u32) -> i32;
```

### 5.3 Plugin Exports

Every plugin must export these functions (WASM) or implement this trait (native):

```rust
/// Plugin lifecycle exports.

/// Initialize the plugin. Called once after loading.
/// Returns 0 on success, negative on error.
fn plugin_init(host_api_version: u32) -> i32;

/// Activate the plugin (start processing).
fn plugin_activate() -> i32;

/// Process one unit of work (audio samples, video frame, or event).
/// Called repeatedly while active.
fn plugin_process(input_ptr: *const u8, input_len: u32,
                  output_buf: *mut u8, output_buf_len: u32) -> i32;

/// Deactivate the plugin (stop processing).
fn plugin_deactivate() -> i32;

/// Unload the plugin. Called once before memory is freed.
fn plugin_unload();

/// Query plugin metadata (name, version, capabilities).
fn plugin_metadata(buf: *mut u8, buf_len: u32) -> i32;

/// Get the plugin's current UI state (for persistence).
fn plugin_ui_state(buf: *mut u8, buf_len: u32) -> i32;

/// Restore UI state from persisted data.
fn plugin_ui_restore(data_ptr: *const u8, data_len: u32) -> i32;
```

### 5.4 API Version Negotiation

When the host calls `plugin_init`, it passes its API version. The plugin
compares this against its `[plugin.api_version]` range. If incompatible, the
plugin returns a negative code and is loaded as `Skipped`.

The host also checks the plugin's declared `api_version` against the host's
supported range before calling `plugin_init`. Double-check prevents loading
an incompatible binary.

---

## 6. Capability Model

### 6.1 Default Denial

All capabilities default to `false` / empty. Plugins must explicitly declare
what they need. The host enforces this at load time, not runtime.

### 6.2 User Approval

When a plugin requests capabilities, the user sees a confirmation dialog:

```
┌─────────────────────────────────────────────┐
│  Plugin: My Cool Plugin v1.2.0              │
│                                             │
│  This plugin requests:                      │
│  ☑ UI Panel (render custom views)           │
│  ☑ Audio In (read audio samples)            │
│  ☐ Video Out (modify video frames)          │
│  ☐ Network (connect to external services)   │
│  ☐ Filesystem (read/write local files)      │
│  ☐ Secrets (access stored credentials)      │
│                                             │
│  [Deny]  [Approve Selected]  [Approve All]  │
└─────────────────────────────────────────────┘
```

### 6.3 Sensitive Capabilities

The following capabilities are **always denied for WASM plugins** and require
explicit user approval for native plugins:

- `secrets` — access to stored credentials
- `capture` — access to capture sources
- `chat` — ability to send chat messages (read is lower-privilege)

### 6.4 Runtime Enforcement

Even after approval, the host enforces limits:

| Check | Trigger | Action |
|-------|---------|--------|
| Memory exceeded | WASM alloc > `max_memory_mb` | Plugin paused, error logged |
| CPU exceeded | Single call > `max_cpu_ms` | Call aborted, plugin paused |
| Timeout | Single call > `timeout_ms` | Plugin paused, error logged |
| Invalid import | Call to non-declared capability | Immediate abort, plugin skipped |
| Network violation | URL not in manifest whitelist | Request denied |
| FS violation | Path not in manifest whitelist | Request denied |

---

## 7. Lifecycle

### 7.1 State Machine

```
                    ┌──────────┐
                    │ Discovered│
                    └─────┬────┘
                          │ load()
                          ▼
                    ┌──────────┐
            ┌──────│  Loaded   │──────┐
            │      └──────────┘      │
            │ activate()             │ unload()
            ▼                        ▼
      ┌──────────┐             ┌──────────┐
      │  Active   │             │  Unloaded │
      └─────┬────┘             └──────────┘
            │ deactivate()
            ▼
      ┌──────────┐
      │ Inactive │── activate() ──▶ Active
      └──────────┘
            │ unload()
            ▼
      ┌──────────┐
      │ Unloaded  │
      └──────────┘
```

### 7.2 Lifecycle Functions

| State | Host Action | Plugin Export | Failure |
|-------|-------------|---------------|---------|
| **Discovered** | Parse manifest, validate | — | `Skipped` (invalid manifest) |
| **Loaded** | Create WASM instance, call `plugin_init` | `plugin_init(api_version)` | `Skipped` (init failed) |
| **Active** | Call `plugin_activate`, then `plugin_process` in loop | `plugin_activate()` | `Inactive` (timeout/crash) |
| **Inactive** | Call `plugin_deactivate` | `plugin_deactivate()` | `Unloaded` (crash) |
| **Unloaded** | Call `plugin_unload`, free memory | `plugin_unload()` | — |

### 7.3 Error Recovery

When a plugin fails in any state:
1. The error is logged with the plugin ID, state, and error message.
2. The plugin transitions to `Skipped` (permanent) or `Inactive` (retryable).
3. The host continues operating normally — **no other plugins or host features are affected**.
4. The user can retry or disable the plugin from the UI.

This mirrors the VST3 host boundary pattern (Z96-1): `Loaded`/`Skipped` result,
skip-on-error semantics, error isolation.

---

## 8. Plugin Bundle Layout

```
com.example.my-plugin/
├── rivulet-plugin.toml          # Manifest (required)
├── plugin.wasm                  # WASM binary (for WASM plugins)
│   OR
├── plugin.vst3                  # VST3 bundle (for native audio effects)
│   OR
├── plugin.dly                   # Native dynamic library (for OBS-compat)
├── icon.png                     # Plugin icon (optional, 512×512)
├── README.md                    # Human-readable description (optional)
└── LICENSE                      # License file (optional)
```

---

## 9. Integration Points

### 9.1 Engine Pipeline (Video Filter)

Video filter plugins are inserted into the GStreamer pipeline between
`videoconvert` and the encoder:

```
appsrc → videoconvert → [plugin_effect] → encoder → muxer
```

The host wraps the plugin in a GStreamer wrapper element that:
1. Calls `plugin_process` with each frame's raw pixels
2. Returns the (possibly modified) frame
3. Handles timeouts and crashes without dropping frames

### 9.2 Audio Graph (Audio Effect)

Audio effect plugins are inserted into the audio processing chain:

```
audio_src → [plugin_effect] → audioconvert → encoder
```

The host maintains a per-plugin audio buffer and calls `plugin_process` with
blocks of samples.

### 9.3 View Registry (UI Panel)

UI panel plugins register a view ID with the View Registry (P2). The host
creates an eframe/egui panel and calls `plugin_ui_render` to draw its contents.

The plugin receives a restricted drawing API (no direct egui access) that maps
to host-provided drawing primitives.

### 9.4 Chat / Alerts (Integration)

Integration plugins receive events (chat messages, alerts) via
`plugin_process` and can trigger responses via host imports. The chat
capability governs read/write access per platform.

---

## 10. Migration Path

### 10.1 From VST3 Host Boundary (Z96)

The existing VST3 host boundary (`VstPlugin`, `VstChain`, discovery) remains
unchanged. VST3 plugins continue to work as native DLY plugins with the
higher-trust sandbox. The new plugin system wraps VST3 in the same lifecycle
and capability model for consistency.

### 10.2 From OBS Compat Mode

OBS plugins (native DLY) are loaded via the same native bridge. The host
provides a compatibility layer that maps OBS API calls to Rivulet host imports.
This is explicitly marked as "unstable" and may be removed in future versions.

### 10.3 Breaking Changes

The plugin API version (`api_version` in manifest) governs compatibility.
When the host API changes:
- **Major version bump:** Old plugins are incompatible (loaded as `Skipped`).
- **Minor version bump:** Old plugins continue to work (deprecated imports
  still available).
- **Patch version bump:** No API change; old plugins unaffected.

---

## 11. Quality Gates

Before any plugin ships to users, it must pass:

| Gate | Description |
|------|-------------|
| **Manifest validation** | All required fields present and valid |
| **Capability audit** | Requested capabilities match actual imports used |
| **Resource limits** | Memory, CPU, and timeout limits respected |
| **Crash isolation** | Plugin crash does not terminate host |
| **Timeout handling** | Plugin timeout does not block host |
| **Security review** | No capability escalation, no host function abuse |
| **API compatibility** | Plugin API version matches host range |
| **Platform test** | Plugin runs on all declared platforms |

---

## 12. Security Considerations

1. **WASM sandbox is mandatory** for untrusted code. Native DLY is opt-in
   and requires user approval.
2. **No runtime discovery** — all plugins must have a manifest. No loading
   from arbitrary paths.
3. **No hot-reload** — plugins are loaded once at startup. Changes require
   restart. This eliminates race conditions and state corruption.
4. **Capability escalation is impossible** — the host enforces capabilities
   at the import level, not the plugin level.
5. **Resource limits are enforced by the WASM runtime**, not the plugin.
   A malicious plugin cannot exceed its declared limits.
6. **Crash isolation** — WASM traps are caught and converted to `Skipped`
   state. Native DLY uses `catch_unwind` and thread isolation.

---

## 13. Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| WASM runtime? | wasmtime, wasmer, wasm3 | wasmtime (Mozilla-backed, WASI-first) |
| UI rendering for WASM? | Restricted drawing API, WebView | Restricted drawing API (native feel) |
| Plugin marketplace? | Official registry, community repos | Phase 2 — not in this RFC |
| Plugin signing? | Sigstore, key-based | Phase 2 — not in this RFC |
| Multi-threaded plugins? | Single-thread, worker pool | Single-thread per plugin (simplicity) |

---

## 14. Implementation Phases

| Phase | Scope | Milestone |
|-------|-------|-----------|
| **Phase 1** | Manifest format + validation + VST3 wrapper | M11-P3 |
| **Phase 2** | WASM runtime + sandbox + core imports | M11-P5 |
| **Phase 3** | Capability system + user approval UI | M11-P4 |
| **Phase 4** | UI panel integration + View Registry | M11-P2 |
| **Phase 5** | Audio/Video filter integration | M11-P6 |
| **Phase 6** | Integration plugins (chat/alerts) | Post-M11 |

---

## Appendix A: Example Manifest

```toml
[plugin]
id = "dev.rivulet.fps-overlay"
version = "1.0.0"
api_version = "1.0"
name = "FPS Overlay"
description = "Displays a real-time FPS counter and frame-time graph"
author = "Rivulet Team"
license = "MIT"

[plugin.type]
kind = "video_filter"
entry_point = "plugin.wasm"

[plugin.api_version]
min = "1.0"
max = "1.0"

[plugin.capabilities]
ui = false
audio_in = false
audio_out = false
video_in = true                      # Read frames to measure FPS
video_out = true                     # Draw overlay onto frames
network = []
filesystem = []
secrets = false
capture = false
chat = false
alerts = false

[plugin.resources]
max_memory_mb = 32
max_cpu_ms = 10
timeout_ms = 1000
max_file_size_mb = 0

[plugin.compatibility]
min_host_version = "0.65.0"
platforms = ["windows", "linux", "macos"]
```

---

## Appendix B: Host Import Table

| Import ID | Name | Capability | Description |
|-----------|------|------------|-------------|
| 0x01 | `host_log` | — | Log a message |
| 0x02 | `host_config_read` | — | Read plugin config |
| 0x03 | `host_config_write` | — | Write plugin config |
| 0x04 | `host_time_now` | — | Get current timestamp |
| 0x05 | `host_ui_invalidate` | ui | Request UI re-render |
| 0x10 | `host_audio_read_sample` | audio_in | Read audio sample |
| 0x11 | `host_audio_write_sample` | audio_out | Write audio sample |
| 0x20 | `host_video_read_frame` | video_in | Read video frame |
| 0x21 | `host_video_write_frame` | video_out | Write video frame |
| 0x30 | `host_http_get` | network | HTTP GET request |
| 0x31 | `host_http_post` | network | HTTP POST request |
| 0x40 | `host_fs_read` | filesystem | Read file |
| 0x41 | `host_fs_write` | filesystem | Write file |
| 0x50 | `host_chat_send` | chat | Send chat message |
| 0x51 | `host_chat_read` | chat | Read chat message |
| 0x60 | `host_alert_trigger` | alerts | Trigger alert |

---

*This RFC is a living document. Updates are tracked in the repository's
`docs/` directory and discussed in GitHub Issues.*
