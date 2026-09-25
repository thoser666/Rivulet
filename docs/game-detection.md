# Game detection (launcher-based)

**Status:** Steam slice shipped (2026-09-25) — issue [#239](https://github.com/thoser666/Rivulet/issues/239).
**Privacy posture:** local reads only. No storefront web/partner APIs, no
telemetry transport, no process-memory inspection. Same contract as
[`telemetry.md`](telemetry.md): detection reads manifest files and registry
keys on the user's machine, and nothing leaves the device.

## Why

Rivulet's game-capture picker listed "games" purely by a window-size/title
heuristic (non-empty title, >640×480) — it cannot tell a game apart from a
large browser window. Launcher-based identification resolves *installed*
games and the *currently running* game from the storefront launchers the
user actually owns.

## Launcher matrix

| Launcher | Installed games (catalog) | Running game | Status |
| --- | --- | --- | --- |
| **Steam** | `HKCU\SOFTWARE\Valve\Steam` → `SteamPath`; `steamapps\libraryfolders.vdf` lists every library; per game `steamapps\appmanifest_<appid>.acf` (`name`, `installdir`, `StateFlags` bit 2 = installed) | `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\Apps\<appid>` → `Running = DWORD:1` | ✅ shipped (`rivulet-core/src/game_detection.rs`) |
| **Epic Games Store** | `HKLM\SOFTWARE\WOW6432Node\Epic Games\EpicGamesLauncher` → `AppDataPath`; JSON manifests `...\Data\Manifests\*.item` | — (foreground-window scoring) | planned slice |
| **GOG** | `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games` per-game keys | — (foreground-window scoring) | planned slice |
| **Origin / EA app** | `C:\ProgramData\Origin\LocalContent\<Game>\*.mfst` (`&id=` game id); EA app config under `%ProgramData%\EA Desktop` | — (foreground-window scoring) | planned slice |
| **Battle.net** | `HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment` per-title keys | — (foreground-window scoring) | planned slice |

Launchers without a running signal will still contribute to the
high-confidence ranking through foreground-window matching: when the
foreground window title/class matches an installed game's launch executable
or display name, the window resolves to the manifest identity
(`Score::Medium`) instead of staying a raw heuristic window.

## Confidence model

| Score | Evidence |
| --- | --- |
| `High` | Launcher signal (Steam: `Running = 1` registry value) |
| `Medium` | Foreground window matches an installed game's executable/name (later slices) |
| `Low` | Size/title window heuristic (existing `enumerate_game_windows` list) |

`detect_running_game()` returns ranked candidates; the scene-item picker
offers the running game first, followed by the installed catalog, followed
by the raw heuristic windows ("Other windows" fallback in later slices).

## Device-id convention

Scene sources persist the chosen identity as the source `device_id`,
mirroring the `camera:`/`monitor:` conventions from #216:

```
game:steam:<appid>     e.g. game:steam:730
game:<launcher>:<id>   later launchers (game:epic:..., game:gog:...)
game:<window-id>       heuristic windows (unchanged, no launcher segment)
```

## Platform scope

Windows-only in this slice (`winreg` is a Windows-only dependency; the
module is `cfg`-gated). Linux/macOS keep the existing window heuristic
until their launcher paths (e.g. `~/.steam/steam` on Linux) are added as a
follow-up slice.

## Privacy invariants (ci_pinning-guarded)

- Detection reads only local registry keys and manifest files.
- No HTTP/transport in the detection module.
- No `ReadProcessMemory` (the `SteamAppId` env-var route is explicitly a
  non-goal in issue #239).
- Missing launchers/keys degrade to an empty list — never an error, never a
  network fallback.
