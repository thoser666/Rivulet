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
| **Steam** | `HKCU\SOFTWARE\Valve\Steam` → `SteamPath`; `steamapps\libraryfolders.vdf` lists every library; per game `steamapps\appmanifest_<appid>.acf` (`name`, `installdir`, `StateFlags` bit 2 = installed); `installdir` resolves to `steamapps\common\<installdir>` and is scanned for launch executables on demand (see below); the root itself comes from `HKCU\SOFTWARE\Valve\Steam` on Windows and from the XDG data directory elsewhere (see below) | `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\Apps\<appid>` → `Running = DWORD:1` | ✅ shipped (`rivulet-core/src/game_detection.rs`) |
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

## Steam launch executables

Steam's `appmanifest_*.acf` carries no `LaunchExecutable` — the field
Epic and Battle.net publish and Steam does not. Without an executable list,
a Steam game could only ever be matched by its display name, so a window
titled after the binary (`hl2.exe`, `Cyberpunk2077.exe`) stayed a raw
heuristic window.

`scan_steam_executables(game_dir)` therefore reads the `.exe` files of the
resolved install directory plus a bounded number of immediate
subdirectories, sorted and deduplicated. Two hard bounds keep it cheap and
predictable:

- at most **32** executables per game (`MAX_EXECUTABLES_PER_GAME`) — a
  folder can hold hundreds of `.exe` files (redist runtimes, launchers,
  anti-cheat stubs) that can never own a top-level window;
- at most **8** subdirectories per game (`MAX_SUBDIRS_PER_GAME`) —
  Steam games nest their entry point one level down often enough
  (`<game>/<publisher>/game.exe`), but a full recursive walk of a 100 GB
  install tree is not worth a title comparison.

The scan is **lazy, not eager**. `list_installed_games_in_library()` only
resolves `installdir` to a real path and never touches the install tree,
because it runs on the GUI thread. `detect_running_game()` first ranks by
the free signals (Steam's registry `Running = 1`, display names) and only
runs `enrich_executables()` if a heuristic window is still unexplained.
The measured cost of the eager variant was **3.3 s for a 120-game
library** on the GUI thread; lazy plus the per-session
`STEAM_EXECUTABLE_CACHE` pays it at most once per game directory, and
usually not at all.

Trade-off: a game installed *while* Rivulet runs keeps an empty executable
list until the cache is dropped. `rescan_steam_executables()` is the
explicit counterpart — it drops the cache *and* clears the lists, because
`enrich_executables()` deliberately skips games that already have
executables. It is not wired into the picker refresh; a stale-but-instant
list beats a correct-but-blocking one.

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

## Linux: XDG instead of the registry

Steam is the one launcher whose *installed-game* source is not a Windows
registry, and it is the one that works on Linux today. Instead of
`HKCU\SOFTWARE\Valve\Steam` -> `SteamPath`, the root is resolved through the
XDG base directories, in this probe order (first existing wins):

1. `$XDG_DATA_HOME/Steam` (default `~/.local/share/Steam`) — where Steam
   actually installs
2. `~/.steam/steam` and `~/.steam/root` — the desktop-entry aliases, usually
   symlinks *to* (1). Probing rather than merging is deliberate: reading
   several aliases of one directory would enumerate the same games twice
3. `~/.var/app/com.valvesoftware.Steam/data/Steam` — the Flatpak install

A missing candidate is skipped and a completely absent Steam degrades to an
empty catalog, never to an error. `$XDG_DATA_HOME` is honoured only when it is
an **absolute** path, as the Base Directory Specification requires; empty and
relative values fall through to `~/.local/share`.

Everything downstream of the root — `libraryfolders.vdf`,
`appmanifest_<appid>.acf`, `rank_candidates` — is the same code as on Windows,
so a Linux user gets real `game:steam:<appid>` identities in the picker. The
window list itself already comes from `xcap` on Linux.

### Confidence ceiling on Linux

Steam games reach **`Score::Medium`** there, not `Score::High`: the
`Running = 1` registry value has no counterpart. The only portable equivalent
is Steam's `SteamAppId` in another process' startup environment, and reading
that is the process-inspection route issue #239 explicitly rules out — the
`ci_pinning` guard bans the procfs entry points so the ceiling cannot be
quietly lifted. Epic, GOG, Origin/EA app and Battle.net stay Windows-gated:
their manifests are registry-backed and I did not find layouts for their Linux
ports that I could verify rather than guess.

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
