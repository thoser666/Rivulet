# MIDI controller mapping

Rivulet can map **MIDI input** (controllers like the Korg NanoKontrol, AKAI
MIDImix, or any device that sends Note/CC messages) to actions: scene
switches, master-volume faders, mute, and chroma-key toggles. This is aimed at
live production and music streams, where a hardware controller is faster than
reaching for the mouse.

- Roadmap: M5 (README), *frequently requested for live production and music
  streams*
- Core logic: `rivulet-core/src/midi.rs` (hardware-free: parsing + mapping)
- Device bridge: `rivulet-gui/src/midi_io.rs` (`midir` input thread)

## How it works

The GUI enumerates the system's MIDI **input** ports. When enabled, a listener
thread connects to the selected device, parses every incoming message
(Note On, Note Off, Control Change) with `rivulet_core::parse_midi`, and the
app applies whatever action is bound to that (channel, kind, number) triple —
exactly like a hotkey, but driven by the controller.

MIDI is **opt-in**: nothing listens until you enable it in Settings, and no
device is touched until you pick one.

## Configuration (Settings → MIDI controller)

1. **Enable MIDI input**.
2. Pick your **device** from the dropdown (hot-plugged devices appear when you
   reopen the list).
3. Add bindings: **message kind** (NoteOn / NoteOff / CC), **channel** (1-16),
   **number** (note or CC number 0-127), and the **action**:
   - **Toggle recording** — start/stop the active capture path
   - **Toggle streaming** — start/stop the configured ingest
   - **Toggle mute** — mute/unmute the active output
   - **Master volume** — fader: the CC value 0-127 scales to 0.0-1.0
   - **Toggle chroma key** — enable/disable chroma key on the currently
     selected source
   - **Switch scene** — pick the target scene (bindings keep the scene id, so
     renaming a scene does not break them)

Each binding row shows a **Remove** button. Bindings, the enabled toggle, the
selected device index, and the per-device preset library are persisted across
restarts.

### Learn mode

Instead of typing channel/number by hand, click **Learn** and then move the
fader/button you want to map: the next incoming MIDI message is captured into
the "add binding" row (kind, channel, number are pre-filled) and **not**
dispatched — so the control you are identifying never triggers an action by
accident. Pick the action (or scene) and press **Add binding** to confirm;
learn mode turns itself off after the first capture.

### Per-device presets

Once a setup works, save it as a **preset**: type a name (e.g. "Streaming") and
press **Save as preset**. The whole binding set is stored under the currently
selected device (by its stable port name — not by index, which shifts when
devices are hot-plugged). Later you can **Apply** a saved preset to restore
that device's bindings or **Delete** it. Because presets are keyed by device
name, the same controller plugged into another machine can carry its profile
along.

## Example: Korg NanoKontrol

A common setup for a NanoKontrol (channel 1):

| Control | Message | Binding |
|---|---|---|
| Top fader 1 | CC 0 | Master volume (fader) |
| Solo button A | Note 32 | Toggle mute |
| Record button | Note 44 | Toggle recording |
| Track button 1 | Note 48 | Switch scene (e.g. "Game") |

## Honest limitations

- **Running status** is not supported by the parser: only complete 3-byte
  messages are handled. Almost all controllers send complete messages, but a
  few devices stream running-status CCs.
- **Master volume** is applied to the Linux audio mixer (where Rivulet owns the
  audio graph). On Windows/macOS the value is stored but not applied — Rivulet
  does not control the OS-level volume there (out of scope for M5).
- **Note Off** (including Note On with velocity 0) and **CC** are the supported
  message kinds. Pitch bend, aftertouch, and SysEx are ignored.
- Bindings match the exact (channel, kind, number); there is no wildcard
  matching. Per-device presets cover common setups, but a given device has one
  active mapping at a time.

## Verification

- **Unit tests** in `rivulet-core/src/midi.rs`: byte parsing (Note On/Off/CC,
  velocity-0 normalization, malformed/unknown frames), exact dispatch matching,
  CC→volume scaling, serde round-trip, and the preset library (per-device
  save/load/list, replacement, deletion, deterministic serialization).
- **GUI tests** in `rivulet-gui/src/app.rs`: opt-in defaults, scene-switch
  application, fader scaling, settings/mapping persistence across restarts,
  the raw-bytes → dispatch → action path, learn-mode capture (including that
  capture does not dispatch), and preset save/load/delete + persistence.
- **CI wiring test** in `rivulet-core/tests/ci_pinning.rs`: the mapping core,
  preset library, the `midir` GUI dependency, learn mode + presets in the
  Settings section, both i18n locales, and the Linux `libasound2-dev` build
  dependency (test CI **and** release packaging) must stay in place.
