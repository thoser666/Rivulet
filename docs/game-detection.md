# Game detection (launcher-based)

**Status:** all five launchers shipped (Steam 2026-09-25, Epic/GOG/Origin/
EA app/Battle.net + foreground ranking 2026-10-04) — issue
[#239](https://github.com/thoser666/Rivulet/issues/239).
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
| **Epic Games Store** | `HKLM\SOFTWARE\WOW6432Node\Epic Games\EpicGamesLauncher` → `AppDataPath`; JSON manifests `...\Data\Manifests\*.item` (`AppName`, `DisplayName`, `InstallLocation`, `LaunchExecutable`) | — (foreground-window scoring) | ✅ shipped |
| **GOG** | `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games` per-game keys (`gameID`, `name`, `path`, `exe`) | — (foreground-window scoring) | ✅ shipped |
| **Origin / EA app** | `%ProgramData%\Origin\LocalContent\<Game>\*.mfst` (`&id=` / `origin2://game/<id>`); EA app under `%ProgramData%\EA Desktop\LocalContent` | — (foreground-window scoring) | ✅ shipped |
| **Battle.net** | `HKLM\SOFTWARE\WOW6432Node\Blizzard Entertainment` per-title keys (`InstallPath`, `ExecutablePath`) | — (foreground-window scoring) | ✅ shipped |

Launchers without a running signal (Epic, GOG, Origin/EA app, Battle.net)
contribute through **foreground-window matching**: when a heuristic window's
title matches an installed game's launch executable or display name, the
window resolves to that manifest identity (`Score::Medium`) instead of
staying a raw window title. The comparison is exact after normalization
(case, ` - Steam`, ` :: …` decorations, punctuation are stripped), so
`Diablo II` never matches `Diablo IV`.

## Confidence model

| Score | Evidence |
| --- | --- |
| `High` | Launcher signal (Steam: `Running = 1` registry value) |
| `Medium` | Foreground window matches an installed game's executable/name |
| `Low` | Size/title window heuristic (existing `enumerate_game_windows` list) |

`detect_running_game()` returns the ranked list: Steam's running signal
(`High`), then foreground matches (`Medium`), then every heuristic window
(`Low`). The scene-item picker renders it grouped by launcher — Steam, Epic,
GOG, Origin, EA app, Battle.net — with the raw heuristic windows last under
an **"Other windows"** header. The ranking itself is the pure function
`rank_candidates(installed, running_app_ids, windows)`, so the confidence
order is unit-tested without any launcher or game installed.

## Device-id convention

Scene sources persist the chosen identity as the source `device_id`,
mirroring the `camera:`/`monitor:` conventions from #216:

```
game:steam:<appid>        e.g. game:steam:730
game:epic:<AppName>       e.g. game:epic:Fortnite
game:gog:<gameID>         e.g. game:gog:1207658924
game:origin:<id>          e.g. game:origin:1172470
game:eaapp:<id>           EA app, its own namespace after the Origin migration
game:battlenet:<title>    e.g. game:battlenet:Diablo IV
game:<window-id>          heuristic windows (unchanged, no launcher segment)
```

Heuristic windows deliberately keep the pre-#239 `game:<window-id>` form
without a launcher segment: a raw window's id is the only stable key it has,
so adding a segment would silently re-id every already-persisted scene
source.

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
