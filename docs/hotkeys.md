# Hotkeys & Remapping

M5 issue **#80**. OBS-style hotkey handling: bind each action to a key plus
optional modifiers, rebind in the GUI, and — on Windows — have the bindings keep
working while the app is unfocused.

## Actions

| Action            | Default | Meaning                         |
| ----------------- | ------- | ------------------------------- |
| `record`          | `F9`    | Start / stop recording          |
| `pause`           | `F10`   | Pause / resume recording        |
| `mute`            | `F11`   | Mute / unmute capture audio     |
| `save_replay`     | `F12`   | Save the replay buffer as clip  |
| Scene hotkeys     | —       | Switch to a scene (assigned per scene) |

## Rebinding

The **Settings → Hotkeys** screen lists every action with a key ComboBox and
`Ctrl` / `Alt` / `Shift` toggles. Changes are written straight back into the
persisted `HotkeyConfig` and flagged for OS-level re-registration, so they apply
from the next frame and survive a restart.

## In-app vs. global

- **In-app (all platforms):** the focused window dispatches the bindings itself
  (`HotkeyBinding::pressed_in`). This works identically on Windows, Linux and
  macOS and is what powers the binding even without OS registration.
- **Global (Windows, while unfocused):** the bindings are registered with the OS
  via `RegisterHotKey` behind a dedicated message-pump thread
  (`rivulet-core::GlobalHotkey`). A `WM_HOTKEY` fires the same action dispatch
  used by the in-app path, so recording toggles, replay save and scene switching
  keep working while Rivulet is not the foreground window. Bindings are
  re-registered in place whenever the config changes.

## Platform matrix (honest)

| Platform | In-app rebinding | OS-level while unfocused |
| -------- | ---------------- | ------------------------ |
| Windows  | ✅               | ✅ (`RegisterHotKey`)    |
| Linux    | ✅               | ⏳ not yet (needs X11 grab / Wayland shortcuts portal) |
| macOS    | ✅               | ⏳ not yet (needs Carbon `RegisterEventHotKey`)         |

The GUI falls back cleanly: on platforms without OS registration the bindings
still fire whenever the app is focused, and there is no visual or functional
regression.

## Implementation notes

- `rivulet-core/src/global_hotkey.rs` is UI-agnostic: it takes action names plus
  `KeyCode` (Windows virtual-key code) and a `ModMask` bitfield (Ctrl/Alt/Shift/
  Super). The GUI translates egui keys to VK codes via `vk_code`.
- Modifier-only or unmapped keys are never registered at the OS level (a lone
  modifier is not a usable global hotkey).
- Actions are delivered over a non-blocking channel and dispatched on the next
  UI frame, so a fired hotkey never blocks the message pump.