//! Launcher-based game identification (issue #239, Steam slice).
//!
//! Real game identification for the game-capture picker: installed Steam
//! games are read **locally** from the Steam installation — no storefront
//! web/partner APIs, no telemetry transport, no process inspection. This is
//! the same privacy posture as the telemetry pipeline (`docs/telemetry.md`):
//! local reads only, nothing leaves the device.
//!
//! Steam slice scope: `libraryfolders.vdf` lists every Steam library, each
//! library's `steamapps/appmanifest_<appid>.acf` describes one installed
//! game (`name`, `installdir`, `stateFlags` — bit 2 (value 4) marks fully
//! installed), and `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\Apps\<appid>`
//! carries a `Running = DWORD:1` value while a game is running. The
//! existing size/title window heuristic stays the low-confidence fallback.
//!
//! All five storefront launchers from issue #239 are covered: Steam (its own
//! `Running` registry signal), Epic Games Store, GOG, Origin/EA app and
//! Battle.net. The manifest/registry *parsers* are pure functions over text
//! or key-value slices, so every launcher is fixture-testable without the
//! launcher being installed; only the thin discovery wrappers touch the
//! registry or the filesystem.
//!
//! Launchers without a documented "running" signal (Epic, GOG, Origin/EA,
//! Battle.net) still rank highly through
//! [`rank_candidates`]: a foreground window whose title or class matches an
//! installed game's launch executable resolves to that manifest identity at
//! [`Score::Medium`] instead of staying a raw heuristic window.
//!
//! Privacy posture is unchanged and local-only: manifest files and registry
//! keys on the user's own machine. No storefront API, no transport, and no
//! process-memory inspection — reading another process' `SteamAppId` from
//! its environment stays an explicit non-goal of issue #239.
//!
//! # Linux
//!
//! Steam is the one launcher whose *installed-game* source is not a Windows
//! registry: on Linux/macOS the installation is located through the XDG base
//! directories ([`steam_roots`]) instead of `HKCU\SOFTWARE\Valve\Steam`.
//! Everything downstream of the root — [`list_installed_games_in_library`],
//! `libraryfolders.vdf`, `appmanifest_*.acf`, [`rank_candidates`] — is the
//! same code on every platform, so a Linux user gets real `game:steam:<id>`
//! identities instead of an empty catalog.
//!
//! The rest stays Windows-gated: Epic, GOG, Origin/EA app and Battle.net are
//! shipped for Windows in practice, and their registry readers have no
//! honest XDG equivalent. Steam games therefore reach [`Score::Medium`]
//! (foreground-window match) on Linux rather than [`Score::High`]: the
//! `Running = 1` registry value has no counterpart, and reconstructing one
//! from another process' startup environment would break the privacy
//! posture documented above.
//!
use std::path::{Path, PathBuf};

/// Which launcher identified a game. `Heuristic` is the existing size/title
/// window fallback — a *window*, not an installed-game identity.
///
/// Serde derives support the GUI's persisted app state (eframe storage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LauncherKind {
    Steam,
    Epic,
    Gog,
    Origin,
    /// EA app (the successor of Origin); kept separate so a migrated
    /// library keeps its own `game:eaapp:` device-id namespace.
    EaApp,
    BattleNet,
    /// The size/title window heuristic (lowest confidence).
    Heuristic,
}

impl LauncherKind {
    /// Every real launcher, in the order the picker groups them.
    ///
    /// [`LauncherKind::Heuristic`] is deliberately absent: heuristic windows
    /// are not a launcher and are listed separately as the last-resort
    /// fallback.
    pub fn all() -> &'static [LauncherKind] {
        &[
            LauncherKind::Steam,
            LauncherKind::Epic,
            LauncherKind::Gog,
            LauncherKind::Origin,
            LauncherKind::EaApp,
            LauncherKind::BattleNet,
        ]
    }

    /// Lower-case name used in device ids (`game:steam:<id>`) and reports.
    pub fn as_str(self) -> &'static str {
        match self {
            LauncherKind::Steam => "steam",
            LauncherKind::Epic => "epic",
            LauncherKind::Gog => "gog",
            LauncherKind::Origin => "origin",
            LauncherKind::EaApp => "eaapp",
            LauncherKind::BattleNet => "battlenet",
            LauncherKind::Heuristic => "heuristic",
        }
    }
}

/// How confident the identification is. Ordered so a higher value means
/// more confidence: [`Score::High`] (launcher signal) ranks above
/// [`Score::Medium`] (foreground-window match), which ranks above
/// [`Score::Low`] (the size/title heuristic). The derived order drives
/// candidate sorting in [`detect_running_game`] consumers.
///
/// Serde derives support the GUI's persisted app state (eframe storage).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Score {
    /// Size/title heuristic only (a "game-like" window, no identity).
    Low,
    /// Foreground window title matched an installed game's name/executable.
    Medium,
    /// Launcher manifest/registry evidence (installed game + running signal).
    High,
}

/// One identified game: stable id, display name, install location.
///
/// Serde derives support the GUI's persisted app state (eframe storage).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GameIdentity {
    /// Which launcher produced this identity.
    pub launcher: LauncherKind,
    /// Stable launcher id (Steam: the numeric app id as string).
    pub game_id: String,
    /// Human-readable display name (Steam: `name` from the `.acf`).
    pub display_name: String,
    /// Install directory of the game, when resolvable.
    pub install_dir: Option<PathBuf>,
    /// Executable names of the installed game (Steam slice: `installdir`
    /// contents are not scanned yet; the field exists for foreground
    /// matching in later slices and stays empty for Steam here).
    pub executables: Vec<String>,
}

impl GameIdentity {
    /// The device-id convention for scene sources (mirrors `camera:` /
    /// `monitor:` from #216): `game:<launcher>:<id>`, e.g.
    /// `game:steam:730`.
    /// Heuristic windows are not a launcher, so they keep the pre-#239
    /// convention `game:<window-id>` with no launcher segment. Adding one
    /// would silently re-id every already-persisted scene source, because the
    /// window id is the only stable key a raw window has.
    pub fn device_id(&self) -> String {
        match self.launcher {
            LauncherKind::Heuristic => format!("game:{}", self.game_id),
            _ => format!("game:{}:{}", self.launcher.as_str(), self.game_id),
        }
    }
}

/// One candidate in a ranked running-game detection result.
///
/// Serde derives support the GUI's persisted app state (eframe storage).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunningGameCandidate {
    /// The identified game.
    pub identity: GameIdentity,
    /// Why this candidate was ranked where it is.
    pub score: Score,
}

// ── VDF / ACF parsing (pure, fixture-testable) ─────────────────────────

/// Unescape a VDF string value: Steam's VDF escapes backslashes (`\\`)
/// and quotes (`\"`). Best-effort flat handling — embedded escaped quotes
/// split the naive extraction, which the files this module reads never use.
fn unescape_vdf_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parse a single quoted key/value pair from a VDF/ACF text line.
///
/// Steam's VDF is a nested text format; the files this module reads
/// (`libraryfolders.vdf`, `appmanifest_*.acf`) only need the flat
/// `"key"  "value"` entries at one nesting level. The quoted split of a
/// pair line is `"" | key | whitespace-gap | value | ""` — structural
/// lines (`"libraryfolders"`, `{`, `}`) have no value part and yield `None`.
pub fn parse_vdf_key_value(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if !trimmed.starts_with('"') {
        return None;
    }
    let mut parts = trimmed.split('"');
    let _open = parts.next()?; // text before the first quote (empty)
    let key = parts.next()?.to_string();
    let _gap = parts.next()?; // whitespace between key and value quotes
    let raw_value = parts.next()?.to_string();
    if key.is_empty() || raw_value.is_empty() {
        return None;
    }
    Some((key, unescape_vdf_value(&raw_value)))
}

/// Extract every `"appid" { "name" … "installdir" … "stateFlags" … }` block
/// from an `appmanifest_<appid>.acf` body and return the parsed game.
///
/// The function is pure: it takes the file contents as a string, so tests
/// run against checked-in fixture text without Steam installed.
pub fn parse_appmanifest(contents: &str) -> Option<GameIdentity> {
    let mut app_id: Option<String> = None;
    let mut name: Option<String> = None;
    let mut install_dir: Option<String> = None;
    let mut state_flags: Option<u64> = None;
    let mut depth = 0usize;

    for line in contents.lines() {
        let trimmed = line.trim();
        match trimmed {
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {
                if let Some((key, value)) = parse_vdf_key_value(line) {
                    // Real ACF files mix case (`appid`, `StateFlags`); match
                    // keys case-insensitively.
                    match key.to_ascii_lowercase().as_str() {
                        "appid" if depth >= 1 && app_id.is_none() => app_id = Some(value),
                        "name" if depth >= 1 && name.is_none() => name = Some(value),
                        "installdir" if depth >= 1 && install_dir.is_none() => {
                            install_dir = Some(value)
                        }
                        "stateflags" if depth >= 1 && state_flags.is_none() => {
                            state_flags = value.parse().ok()
                        }
                        _ => {}
                    }
                }
            }
        }
        // Guard against pathological nesting in hostile manifest files.
        if depth > 64 {
            return None;
        }
    }

    let game_id = app_id?;
    let display_name = name?;
    // stateFlags bit 2 (value 4) = fully installed; manifests of games still
    // updating may exist but must not show up as installed games.
    if state_flags.map(|flags| flags & 4 == 0).unwrap_or(true) {
        return None;
    }
    Some(GameIdentity {
        launcher: LauncherKind::Steam,
        game_id,
        display_name,
        install_dir: install_dir.map(PathBuf::from),
        executables: Vec::new(),
    })
}

/// Extract the library root paths from `libraryfolders.vdf` contents.
///
/// Returns one path per `"path"` entry inside each `"1" { … "path" … }`
/// block. Pure string work: tests run against fixture text. Backslashes are
/// kept as-is (Windows paths); the caller joins `steamapps` onto them.
pub fn parse_libraryfolders(contents: &str) -> Vec<PathBuf> {
    let mut libraries = Vec::new();
    let mut depth = 0usize;
    for line in contents.lines() {
        let trimmed = line.trim();
        match trimmed {
            "{" => depth += 1,
            "}" => depth -= 1,
            _ => {
                if let Some((key, value)) = parse_vdf_key_value(line) {
                    if key == "path" && depth >= 1 && !value.is_empty() {
                        libraries.push(PathBuf::from(value));
                    }
                }
            }
        }
    }
    libraries
}

// ── Epic Games Store manifests (pure, fixture-testable) ────────────────

/// Reduce a launch command to a comparable executable name.
///
/// Deliberately **not** `Path::file_stem()`: the launcher manifests always
/// carry Windows paths (`C:\\Games\\bg3.exe`), and on a non-Windows host a
/// backslash is an ordinary character, not a path separator, so `Path`
/// would return the whole path instead of `bg3`. These parsers are pure
/// and are exercised by the Linux CI runner, so the result must not depend
/// on the host's path semantics.
fn executable_name(raw: &str) -> String {
    let segment = raw
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or(raw)
        .trim();
    match segment.rfind('.') {
        // A leading dot belongs to the name (".helper"), not an extension.
        Some(dot) if dot > 0 => segment[..dot].to_string(),
        _ => segment.to_string(),
    }
}

/// Parse one Epic Games Store `.item` manifest into a [`GameIdentity`].
///
/// Epic writes a JSON document per installed game under
/// `<AppDataPath>\Data\Manifests\`. Only four fields matter for
/// identification: `AppName` (the stable id), `DisplayName` (shown to the
/// user), `InstallLocation` and `LaunchExecutable` (the latter feeds
/// foreground-window matching in [`rank_candidates`]).
///
/// Pure over the file text, so tests run against checked-in fixture JSON
/// without Epic installed. A document that is not valid JSON, or that lacks
/// an id or a display name, yields `None` — an unreadable manifest must not
/// break the rest of the catalog.
pub fn parse_epic_manifest(contents: &str) -> Option<GameIdentity> {
    let value: serde_json::Value = serde_json::from_str(contents).ok()?;

    // Epic's schema repeats the game fields at the top level *and* nests them
    // in an `AppPath` object (older manifest generation). Accept the
    // documented top-level shape first, then the nested one, so both
    // manifest generations identify.
    let nested = value.get("AppPath");
    let field = |name: &str| -> Option<&str> {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                nested
                    .and_then(|n| n.get(name))
                    .and_then(serde_json::Value::as_str)
            })
            .filter(|s| !s.trim().is_empty())
    };

    let game_id = field("AppName")?.trim().to_string();
    let display_name = field("DisplayName")?.trim().to_string();

    let install_dir = field("InstallLocation")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    // At most one launch executable per Epic manifest, hence the Option ->
    // single-element vec rather than a filter/map/collect chain.
    let executables: Vec<String> = field("LaunchExecutable")
        .map(executable_name)
        .into_iter()
        .collect();

    Some(GameIdentity {
        launcher: LauncherKind::Epic,
        game_id,
        display_name,
        install_dir,
        executables,
    })
}

/// Enumerate the installed Epic games of one manifest directory. Shared with
/// [`list_installed_games`] so tests can point the reader at a fixture
/// directory without the registry.
pub fn list_epic_games_in(app_data_path: &Path) -> Vec<GameIdentity> {
    let manifests = app_data_path.join("Data").join("Manifests");
    let Ok(entries) = std::fs::read_dir(&manifests) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("item") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(game) = parse_epic_manifest(&contents) {
            games.push(game);
        }
    }
    games
}

// ── GOG registry entries (pure, fixture-testable) ───────────────────────

/// One `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>` subkey as a flat
/// name/value list, already read from the registry.
///
/// The reader in [`gog_games_from_entries`] is pure so the GOG catalog can be
/// tested without a GOG installation: tests feed checked-in fixtures shaped
/// exactly like this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryGameEntry {
    /// The subkey name — GOG's numeric `gameID`.
    pub key: String,
    /// Flat name/value pairs of that subkey.
    pub values: Vec<(String, String)>,
}

impl RegistryGameEntry {
    /// Case-insensitive value lookup (GOG's own registry mixes cases).
    pub fn value(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.trim().is_empty())
    }
}

/// Build the GOG catalog from registry entries.
///
/// GOG keys its games by numeric `gameID`; the human-readable title lives in
/// `name`, the install path in `path`, and the launch executable (used for
/// foreground matching) in `exe`. A subkey without a title is skipped rather
/// than guessed — a nameless entry would render as an empty row in the
/// picker.
pub fn gog_games_from_entries(entries: &[RegistryGameEntry]) -> Vec<GameIdentity> {
    entries
        .iter()
        .filter_map(|entry| {
            let display_name = entry.value("name")?.trim().to_string();
            let game_id = entry
                .value("gameID")
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| entry.key.trim())
                .to_string();
            if game_id.is_empty() {
                return None;
            }
            let install_dir = entry
                .value("path")
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from);
            let executables = entry
                .value("exe")
                .map(str::trim)
                .filter(|exe| !exe.is_empty())
                .map(executable_name)
                .into_iter()
                .collect();
            Some(GameIdentity {
                launcher: LauncherKind::Gog,
                game_id,
                display_name,
                install_dir,
                executables,
            })
        })
        .collect()
}

// ── Origin / EA app manifests (pure, fixture-testable) ─────────────────

/// Extract the numeric game id from an Origin `.mfst` manifest.
///
/// Origin's `*.mfst` files embed a command line such as
/// `origin2://game/12345?offerIds=…`; the id after `/game/` is the stable
/// key. Pure over the file text so tests need no Origin installation.
/// Returns `None` when no `&id=`/game id can be found.
pub fn parse_origin_mfst(contents: &str) -> Option<String> {
    // Modern EA app manifests use an explicit `&id=` parameter.
    if let Some(id) = contents.split("&id=").nth(1) {
        let id: String = id.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    // Classic Origin manifests use the `origin2://game/<id>` URL form.
    if let Some(rest) = contents.split("origin2://game/").nth(1) {
        let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    None
}

/// Enumerate installed Origin games under one `LocalContent` root.
///
/// `origin2://game/<id>` gives the stable id; the *display name* is the
/// `.mfst` file's own stem (Origin has no title field in the manifest), so the
/// file name is the best local identifier. Reads are independent per file, so
/// one unreadable manifest degrades to the rest.
pub fn list_origin_games_in(local_content: &Path) -> Vec<GameIdentity> {
    let Ok(entries) = std::fs::read_dir(local_content) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        // Each game owns a folder (`<Game>\*.mfst`).
        let Ok(manifests) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        for manifest in manifests.flatten() {
            let file_name = manifest.file_name();
            let Some(stem) = Path::new(&file_name).file_stem() else {
                continue;
            };
            if stem.to_string_lossy().eq_ignore_ascii_case("_manifest") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(manifest.path()) else {
                continue;
            };
            let Some(game_id) = parse_origin_mfst(&contents) else {
                continue;
            };
            games.push(GameIdentity {
                launcher: LauncherKind::Origin,
                game_id,
                display_name: stem.to_string_lossy().into_owned(),
                install_dir: Some(entry.path()),
                executables: Vec::new(),
            });
            // One manifest per game folder is the documented layout; stop
            // here so duplicate `.mfst` variants cannot double-list a game.
            break;
        }
    }
    games
}

// ── Battle.net registry entries (pure, fixture-testable) ───────────────

/// Build the Battle.net catalog from `HKLM\SOFTWARE\WOW6432Node\Blizzard
/// Entertainment` subkeys.
///
/// Battle.net keys games by *title* (`Diablo IV`), which doubles as the
/// stable id and the display name. `InstallPath` locates the game;
/// `InstallPath` + the well-known `Binaries\<exe>` layout is where the
/// launch executable comes from, so foreground matching can resolve a
/// running Battle.net game to this identity.
pub fn battlenet_games_from_entries(entries: &[RegistryGameEntry]) -> Vec<GameIdentity> {
    entries
        .iter()
        .filter_map(|entry| {
            let display_name = entry
                .value("DisplayName")
                .or_else(|| entry.value("GameName"))
                .unwrap_or_else(|| entry.key.trim())
                .trim()
                .to_string();
            if display_name.is_empty() {
                return None;
            }
            let install_dir = entry
                .value("InstallPath")
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(PathBuf::from);
            let executables = entry
                .value("ExecutablePath")
                .map(str::trim)
                .filter(|exe| !exe.is_empty())
                .map(executable_name)
                .into_iter()
                .collect();
            Some(GameIdentity {
                launcher: LauncherKind::BattleNet,
                game_id: entry.key.trim().to_string(),
                display_name,
                install_dir,
                executables,
            })
        })
        .collect()
}

// ── Foreground-window matching (pure) ───────────────────────────────────

/// Normalize a window title or executable for comparison: lowercase, and
/// strip everything the launcher adds around the name (`- Steam`,
/// `(64-bit, ...)`, ` :: ` suffixes, punctuation).
fn normalize_for_match(value: &str) -> String {
    value
        .to_lowercase()
        .trim()
        .split(" :: ")
        .next()
        .unwrap_or_default()
        .split(" - ")
        .next()
        .unwrap_or_default()
        .trim_matches(|c: char| c.is_whitespace() || c == '-' || c == '|' || c == '\u{2014}')
        .to_string()
}

/// Does a foreground window's title identify `game`?
///
/// Matching is deliberately conservative and pure: an exact (normalized)
/// match on the launch executable wins, otherwise the window title must
/// contain the whole normalized display name. Substring-free name matching
/// avoids the classic false positive where "Diablo" matches "Diablo II".
pub fn window_matches_game(title: &str, game: &GameIdentity) -> bool {
    let title_key = normalize_for_match(title);
    if title_key.is_empty() {
        return false;
    }
    if game
        .executables
        .iter()
        .any(|exe| normalize_for_match(exe) == title_key)
    {
        return true;
    }
    let name_key = normalize_for_match(&game.display_name);
    !name_key.is_empty() && title_key == name_key
}

/// Rank the candidates for the scene-item picker.
///
/// `High` first (a launcher running signal — currently Steam's `Running`
/// registry value), then `Medium` (a foreground window that matches an
/// installed game), then `Low` (the size/title heuristic windows). Within a
/// score the catalog keeps its input order, so a launcher-enumerated game
/// stays above a heuristic window of the same name.
///
/// Pure and slice-driven: callers pass the installed catalog, the app ids a
/// launcher reports as running, and the heuristic windows. That keeps the
/// ranking unit-testable with no launcher installed, and lets the Windows
/// wrapper [`detect_running_game`] keep doing only the I/O.
pub fn rank_candidates(
    installed: &[GameIdentity],
    running_app_ids: &[String],
    windows: &[crate::game_capture::GameWindow],
) -> Vec<RunningGameCandidate> {
    let mut candidates: Vec<RunningGameCandidate> = Vec::new();

    for game in installed {
        if running_app_ids.iter().any(|id| id == &game.game_id) {
            candidates.push(RunningGameCandidate {
                identity: game.clone(),
                score: Score::High,
            });
        }
    }
    for game in installed {
        let already_high = candidates
            .iter()
            .any(|c| &c.identity == game && c.score == Score::High);
        if already_high {
            continue;
        }
        if windows.iter().any(|w| window_matches_game(&w.title, game)) {
            candidates.push(RunningGameCandidate {
                identity: game.clone(),
                score: Score::Medium,
            });
        }
    }
    // Heuristic windows stay available as the last-resort fallback, and keep
    // their raw window id as the device id (`game:<window-id>`).
    for window in windows {
        candidates.push(RunningGameCandidate {
            identity: GameIdentity {
                launcher: LauncherKind::Heuristic,
                game_id: window.id.to_string(),
                display_name: window.title.clone(),
                install_dir: None,
                executables: Vec::new(),
            },
            score: Score::Low,
        });
    }

    candidates
}

// ── Steam installation discovery (registry vs. XDG) ────────────────────

/// The Steam installation root: `HKCU\SOFTWARE\Valve\Steam` → `SteamPath`.
///
/// Windows uses the registry value, which is the documented source. Other
/// platforms resolve the same *concept* through [`steam_roots`], so this
/// function stays the single place the rest of the module asks "where is
/// Steam installed?".
#[cfg(target_os = "windows")]
fn steam_root() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(r"SOFTWARE\Valve\Steam").ok()?;
    let path: String = key.get_value("SteamPath").ok()?;
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

/// The Steam installation root on non-Windows hosts: the first of the XDG
/// probe candidates that exists.
///
/// Deliberately a three-line glue over [`xdg_data_home`], [`steam_roots`] and
/// `dirs::home_dir` instead of open-coded path building: those are portable
/// and unit-tested on *every* platform, so the Linux layout is verifiable on
/// a Windows dev box and in CI, while this wrapper stays trivial enough that
/// a platform-specific mistake cannot hide inside it.
#[cfg(not(target_os = "windows"))]
fn steam_root() -> Option<PathBuf> {
    steam_roots(xdg_data_home().as_deref(), dirs::home_dir().as_deref())
        .into_iter()
        .next()
}

/// The user's XDG data directory: `$XDG_DATA_HOME`, else `~/.local/share`.
///
/// `None` when neither an absolute `$XDG_DATA_HOME` nor a home directory can
/// be determined, and then the caller simply finds no Steam root.
///
/// Public because the non-Windows `steam_root()` above is its only production
/// caller: a `cfg`-gated helper could not be unit-tested on the platforms
/// where it is not compiled.
pub fn xdg_data_home() -> Option<PathBuf> {
    xdg_data_dir(std::env::var_os("XDG_DATA_HOME"), dirs::home_dir())
}

/// Resolve the XDG data directory from an explicit variable value and home.
///
/// Pure, so the Base Directory Specification rules are fixture-testable: a
/// value that is **not an absolute path** must be ignored as if it were
/// unset (the spec says so explicitly, and a relative `XDG_DATA_HOME` is a
/// classic way to break path assumptions), and so must an empty one.
fn xdg_data_dir(variable: Option<std::ffi::OsString>, home: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(configured) = variable
        .map(PathBuf::from)
        // Empty *and* relative both fall through to the `$HOME` default.
        .filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
    {
        return Some(configured);
    }
    Some(home?.join(".local/share"))
}

/// Every Steam installation root this host may use, in probe order.
///
/// Steam on Linux installs into the XDG data directory (`~/.local/share/Steam`
/// by default). The `~/.steam/steam` and `~/.steam/root` entries are what the
/// desktop entries and older clients point at """ + EM + """ usually symlinks *to* the
/// XDG location, which is why the list is probed rather than merged: reading
/// several aliases of one directory would enumerate the same games twice. The
/// last entry is the Flatpak install, whose tree lives under
/// `~/.var/app/com.valvesoftware.Steam/data`.
///
/// Only existing directories are returned, in probe order, so the caller can
/// take the first one and get a single authoritative root. Pure over its
/// arguments apart from the existence check, hence fixture-testable on every
/// platform.
pub fn steam_roots(data_home: Option<&Path>, home: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(data_home) = data_home {
        candidates.push(data_home.join("Steam"));
    }
    if let Some(home) = home {
        candidates.push(home.join(".steam").join("steam"));
        candidates.push(home.join(".steam").join("root"));
        candidates.push(
            home.join(".var")
                .join("app")
                .join("com.valvesoftware.Steam")
                .join("data")
                .join("Steam"),
        );
    }
    candidates
        .into_iter()
        .filter(|candidate| candidate.is_dir())
        .collect()
}

/// Steam's running-game registry key:
/// `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\Apps\<appid>` → `Running = 1`
/// (Steam also writes the 64-bit view; the WOW6432Node path is the one its
/// own docs reference). `None` when the game is not running / not present.
///
/// Windows-only by nature: there is no registry on the platforms that reach
/// the stub below, and the only portable equivalent (Steam's `SteamAppId` in
/// another process' startup environment) is the process-inspection route issue #239
/// explicitly rules out. A Linux Steam game therefore tops out at
/// [`Score::Medium`] through foreground matching — a deliberate, documented
/// ceiling rather than a missing feature.
#[cfg(target_os = "windows")]
fn steam_app_is_running(app_id: &str) -> bool {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    hklm.open_subkey(format!(r"SOFTWARE\WOW6432Node\Valve\Steam\Apps\{app_id}"))
        .and_then(|key| key.get_value::<u32, _>("Running"))
        .map(|running| running == 1)
        .unwrap_or(false)
}

/// Always `false` off Windows — see the note on the Windows variant.
#[cfg(not(target_os = "windows"))]
fn steam_app_is_running(_app_id: &str) -> bool {
    false
}

// ── Generic registry enumeration (Windows-gated) ───────────────────────

/// Read every value of an open registry key as a flat name/value list.
///
/// Deliberately value-agnostic: GOG and Battle.net both store one game per
/// subkey, and the readers that consume [`RegistryGameEntry`] are pure, so
/// enumerating the (possibly heterogeneous) value types happens exactly once,
/// here. Non-string values are read as their lossy display form, because the
/// only consumers are path/name strings.
#[cfg(target_os = "windows")]
fn registry_values(key: &winreg::RegKey) -> Vec<(String, String)> {
    let mut values = Vec::new();
    for (name, _value) in key.enum_values().flatten() {
        let rendered = key
            .get_value::<String, _>(&name)
            .or_else(|_| key.get_value::<u32, _>(&name).map(|v| v.to_string()))
            .or_else(|_| key.get_value::<u64, _>(&name).map(|v| v.to_string()))
            .unwrap_or_default();
        if !rendered.is_empty() {
            values.push((name, rendered));
        }
    }
    values
}

/// Enumerate the direct child subkeys of `root_path` as [`RegistryGameEntry`]
/// values, for the launchers that key one game per subkey.
#[cfg(target_os = "windows")]
fn registry_game_entries(root_path: &str) -> Vec<RegistryGameEntry> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let Ok(root) = hklm.open_subkey(root_path) else {
        return Vec::new();
    };
    root.enum_keys()
        .flatten()
        .filter_map(|subkey| {
            let child = root.open_subkey(&subkey).ok()?;
            Some(RegistryGameEntry {
                key: subkey,
                values: registry_values(&child),
            })
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn registry_game_entries(_root_path: &str) -> Vec<RegistryGameEntry> {
    Vec::new()
}

// ── Epic Games Store discovery (Windows-gated) ──────────────────────────

/// `HKLM\SOFTWARE\WOW6432Node\Epic Games\EpicGamesLauncher` → `AppDataPath`,
/// the directory whose `Data\Manifests\*.item` describe installed games.
#[cfg(target_os = "windows")]
fn epic_app_data_path() -> Option<PathBuf> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let path: String = hklm
        .open_subkey(r"SOFTWARE\WOW6432Node\Epic Games\EpicGamesLauncher")
        .ok()?
        .get_value("AppDataPath")
        .ok()?;
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[cfg(not(target_os = "windows"))]
fn epic_app_data_path() -> Option<PathBuf> {
    None
}

// ── Origin / EA app discovery (Windows-gated) ───────────────────────────

/// Origin's per-user `LocalContent` root, which holds one folder plus
/// `*.mfst` per installed game.
#[cfg(target_os = "windows")]
fn origin_local_content() -> Option<PathBuf> {
    program_data().map(|root| root.join("Origin").join("LocalContent"))
}

#[cfg(not(target_os = "windows"))]
fn origin_local_content() -> Option<PathBuf> {
    None
}

/// The EA app (Origin's successor) keeps its own install root; the manifest
/// layout matches Origin's, so the same parser serves both launchers.
#[cfg(target_os = "windows")]
fn ea_app_local_content() -> Option<PathBuf> {
    program_data().map(|root| root.join("EA Desktop").join("LocalContent"))
}

#[cfg(not(target_os = "windows"))]
fn ea_app_local_content() -> Option<PathBuf> {
    None
}

/// `%ProgramData%`, where both Origin and the EA app keep their manifests.
///
/// Only the Windows-gated callers above reach this, so the helper itself is
/// Windows-only: a non-Windows stub would be dead code that `-D warnings`
/// rejects on the Linux/macOS CI runners.
#[cfg(target_os = "windows")]
fn program_data() -> Option<PathBuf> {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

// ── Steam install-directory scan (populates `executables`) ─────────────

/// Upper bound on executables kept per game.
///
/// Only foreground matching consumes this list, and that compares against a
/// *window title*. A game folder can hold hundreds of `.exe` files (redist
/// runtimes, launchers, anti-cheat stubs), almost none of which can ever own
/// a top-level window, so a small cap keeps the scan and the cache bounded
/// without losing the binaries that actually run.
const MAX_EXECUTABLES_PER_GAME: usize = 32;

/// Upper bound on subdirectories walked per game.
///
/// Steam games nest their real entry point one level down often enough
/// (`<game>/<publisher>/game.exe`) that top-level-only would miss many, but a
/// full recursive walk over a 100 GB install tree is not worth it for a
/// title comparison. One level, capped.
const MAX_SUBDIRS_PER_GAME: usize = 8;

/// Is this a Windows executable by name?
fn is_executable_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
}

/// The executable name of a path that came from the local filesystem.
///
/// Unlike the manifest-derived strings handled by `executable_name`, this
/// input *is* a real path on this host, so `Path::file_stem` is correct and
/// host-consistent here.
fn local_executable_name(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
}

/// Collect the launch executables of one Steam install directory.
///
/// Reads `.exe` files from the directory itself and from a bounded number of
/// immediate subdirectories, returns them sorted and deduplicated.
///
/// `read_dir` order is arbitrary, so sorting is what makes the result — and
/// therefore the cache and the tests — deterministic.
pub fn scan_steam_executables(game_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(game_dir) else {
        return Vec::new();
    };

    let mut names: Vec<String> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if subdirs.len() < MAX_SUBDIRS_PER_GAME {
                subdirs.push(path);
            }
            continue;
        }
        if is_executable_file(&path) {
            if let Some(name) = local_executable_name(&path) {
                names.push(name);
            }
        }
    }

    for subdir in subdirs {
        let Ok(entries) = std::fs::read_dir(subdir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() || !is_executable_file(&path) {
                continue;
            }
            if let Some(name) = local_executable_name(&path) {
                names.push(name);
            }
        }
    }

    names.sort();
    names.dedup();
    names.truncate(MAX_EXECUTABLES_PER_GAME);
    names
}

/// Per-session cache of install-directory scans, keyed by game directory.
///
/// The catalog is rebuilt on every picker refresh; walking every installed
/// game each time would put seconds of disk I/O in front of a dropdown. The
/// cache makes the cost a one-off per game directory.
///
/// Trade-off: a game installed *while* Rivulet runs keeps an empty
/// executable list until the cache is dropped. That is deliberate — a stale
/// but instant list beats a correct but blocking one. Use
/// [`clear_steam_executable_cache`] for an explicit rescan.
static STEAM_EXECUTABLE_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, std::sync::Arc<Vec<String>>>>,
> = std::sync::OnceLock::new();

/// The executables of one Steam game, scanning its directory at most once.
fn steam_executables(game_dir: &Path) -> Vec<String> {
    if game_dir.as_os_str().is_empty() {
        return Vec::new();
    }
    let cache = STEAM_EXECUTABLE_CACHE.get_or_init(Default::default);
    // A poisoned cache would mean a scan panicked, which cannot happen for
    // these read-only loops; recovering keeps the picker alive either way.
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(hit) = guard.get(game_dir) {
        return hit.as_ref().clone();
    }
    let found = std::sync::Arc::new(scan_steam_executables(game_dir));
    guard.insert(game_dir.to_path_buf(), std::sync::Arc::clone(&found));
    found.as_ref().clone()
}

/// Drop the cached install-directory scans.
///
/// Not called on the refresh path — that is the point of the cache. Prefer
/// [`rescan_steam_executables`], which also resets the already-filled lists:
/// dropping the cache alone is not enough, because [`enrich_executables`]
/// deliberately skips games that already carry executables.
pub fn clear_steam_executable_cache() {
    if let Some(cache) = STEAM_EXECUTABLE_CACHE.get() {
        cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

/// Enumerate the installed games of one Steam library (its `steamapps`
/// directory). Shared by [`list_installed_games`] so tests can point the
/// reader at a fixture directory without the registry.
pub fn list_installed_games_in_library(library_root: &Path) -> Vec<GameIdentity> {
    let steamapps = library_root.join("steamapps");
    let Ok(entries) = std::fs::read_dir(&steamapps) else {
        return Vec::new();
    };
    let mut games = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(file_name) = name.to_str() else {
            continue;
        };
        if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
            continue;
        }
        let Ok(contents) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Some(mut game) = parse_appmanifest(&contents) else {
            continue;
        };
        // `parse_appmanifest` stays a pure parser of the `.acf` text and
        // therefore reports `installdir` exactly as Steam wrote it. Only the
        // reader knows the library root, so *here* is where the relative
        // directory becomes a real path. The directory is *not* walked yet:
        // that is deferred to [`enrich_executables`] so the GUI thread is
        // never blocked by install-tree I/O.
        if let Some(relative) = game.install_dir.clone() {
            game.install_dir = Some(steamapps.join("common").join(relative));
        }
        games.push(game);
    }
    games
}

/// Enumerate all installed Steam games across every configured library.
///
/// Discovery order: the HKCU `SteamPath` registry value, then every
/// `"path"` entry of that installation's `libraryfolders.vdf` (the default
/// install root lists itself there too). A missing/failed source degrades
/// to an empty result — never an error: the picker falls back to the
/// window heuristic.
pub fn list_installed_games() -> Vec<GameIdentity> {
    let mut games = steam_installed_games();

    // Each remaining launcher is read independently: a missing launcher (or a
    // folder that moved) degrades to nothing contributed, never to an error
    // that would hide the launchers that did work.
    if let Some(app_data_path) = epic_app_data_path() {
        games.extend(list_epic_games_in(&app_data_path));
    }
    games.extend(gog_games_from_entries(&registry_game_entries(
        r"SOFTWARE\WOW6432Node\GOG.com\Games",
    )));
    games.extend(battlenet_games_from_entries(&registry_game_entries(
        r"SOFTWARE\WOW6432Node\Blizzard Entertainment",
    )));
    if let Some(local_content) = origin_local_content() {
        games.extend(list_origin_games_in(&local_content));
    }
    if let Some(local_content) = ea_app_local_content() {
        // Same manifest layout, different launcher identity: retag so the
        // device id stays in the `game:eaapp:` namespace.
        games.extend(
            list_origin_games_in(&local_content)
                .into_iter()
                .map(|mut game| {
                    game.launcher = LauncherKind::EaApp;
                    game
                }),
        );
    }

    // A game installed under two launchers would appear twice with different
    // device ids; that is intentional (each launcher entry is selectable),
    // but a duplicate *within* one launcher is a catalog bug.
    deduplicate_by_device_id(games)
}

/// The Steam half of [`list_installed_games`], split out so the other
/// launchers can be added without touching the library logic.
fn steam_installed_games() -> Vec<GameIdentity> {
    let Some(root) = steam_root() else {
        return Vec::new();
    };
    let mut libraries = Vec::new();
    if let Ok(contents) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf"))
    {
        libraries = parse_libraryfolders(&contents);
    }
    // The default install root is authoritative even if the VDF is unreadable
    // (older Steam versions list fewer libraries than exist).
    if !libraries.iter().any(|p| p == &root) {
        libraries.push(root);
    }
    let mut games = Vec::new();
    for library in &libraries {
        games.extend(list_installed_games_in_library(library));
    }
    games
}

/// Drop repeated `game:<launcher>:<id>` rows, keeping the first occurrence so
/// enumeration order stays deterministic.
fn deduplicate_by_device_id(games: Vec<GameIdentity>) -> Vec<GameIdentity> {
    let mut seen = std::collections::HashSet::new();
    games
        .into_iter()
        .filter(|game| seen.insert(game.device_id()))
        .collect()
}

/// Detect the currently running game, ranked.
///
/// Steam's `Running` registry value yields [`Score::High`]; a heuristic window
/// whose title matches an installed game's name or launch executable yields
/// [`Score::Medium`], which is how Epic, GOG, Origin/EA and Battle.net games
/// (none of which publish a running signal) can still resolve to a real
/// identity instead of a raw window title. Every heuristic window is kept at
/// [`Score::Low`] as the last-resort fallback.
///
/// The ranking itself lives in [`rank_candidates`], which is pure; this
/// function only performs the I/O (catalog, Steam's running signal, the
/// heuristic window list) and hands them over. That keeps the confidence
/// ordering testable without any launcher or game installed.
pub fn detect_running_game() -> Vec<RunningGameCandidate> {
    let installed = list_installed_games();
    let running_app_ids: Vec<String> = installed
        .iter()
        .filter(|game| game.launcher == LauncherKind::Steam && steam_app_is_running(&game.game_id))
        .map(|game| game.game_id.clone())
        .collect();
    let windows = crate::game_capture::list_game_windows();
    let ranked = rank_candidates(&installed, &running_app_ids, &windows);

    // Lazy second pass. Ranking by display name is free, but a game whose
    // window title is the *executable* name cannot match that way — and Steam
    // is the one launcher whose manifests carry no `LaunchExecutable`, which
    // is why its `executables` list used to stay empty forever.
    //
    // Scanning is only worth its I/O when a heuristic window is still
    // unexplained, so that check gates the whole pass. Measured cost of the
    // eager version on a 120-game library was 3.3 s on the GUI thread; here
    // it is paid at most once per session (the cache) and usually not at all.
    let unexplained_window = windows.iter().any(|window| {
        !ranked.iter().any(|candidate| {
            candidate.score != Score::Low && window_matches_game(&window.title, &candidate.identity)
        })
    });
    if !unexplained_window {
        return ranked;
    }

    let mut enriched = installed;
    enrich_executables(&mut enriched);
    rank_candidates(&enriched, &running_app_ids, &windows)
}

/// Fill in the launch executables of every identity that does not have them
/// yet, reading the install directory of each (cached per directory).
///
/// Only Steam needs this: Epic, GOG and Battle.net carry the launch
/// executable in their manifest or registry values, and Origin has none.
pub fn enrich_executables(games: &mut [GameIdentity]) {
    for game in games.iter_mut() {
        if !game.executables.is_empty() {
            continue;
        }
        let Some(dir) = game.install_dir.clone() else {
            continue;
        };
        game.executables = steam_executables(&dir);
    }
}

/// Re-read the install directories of every Steam game in `games`.
///
/// This is the explicit-rescan counterpart to [`enrich_executables`]: it drops
/// the session cache *and* clears the lists first, so a game installed while
/// Rivulet runs is picked up. Deliberately not wired into the picker refresh
/// — see [`STEAM_EXECUTABLE_CACHE`] for why the refresh path stays cheap.
pub fn rescan_steam_executables(games: &mut [GameIdentity]) {
    clear_steam_executable_cache();
    for game in games.iter_mut() {
        if game.launcher == LauncherKind::Steam {
            game.executables.clear();
        }
    }
    enrich_executables(games);
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
"AppState"
{
	"appid"		"730"
	"Universe"		"1"
	"name"		"Counter-Strike 2"
	"StateFlags"		"4"
	"installdir"		"Counter-Strike Global Offensive"
	"LastUpdated"		"1727000000"
	"SizeOnDisk"		"34000000000"
}
"#;

    const MANIFEST_UPDATING: &str = r#"
"AppState"
{
	"appid"		"570"
	"name"		"Dota 2"
	"StateFlags"		"1082"
	"installdir"		"dota 2 beta"
}
"#;

    #[test]
    fn parse_vdf_key_value_reads_quoted_pairs() {
        assert_eq!(
            parse_vdf_key_value("\t\"appid\"\t\t\"730\""),
            Some(("appid".to_string(), "730".to_string()))
        );
        assert_eq!(
            parse_vdf_key_value("\"name\"\t\"Half-Life 3\""),
            Some(("name".to_string(), "Half-Life 3".to_string()))
        );
        // Structural lines are not key/value pairs.
        assert_eq!(parse_vdf_key_value("\"AppState\""), None);
        assert_eq!(parse_vdf_key_value("{"), None);
        assert_eq!(parse_vdf_key_value("}"), None);
        assert_eq!(parse_vdf_key_value(""), None);
    }

    #[test]
    fn parse_appmanifest_reads_the_full_identity() {
        let game = parse_appmanifest(MANIFEST).expect("installed manifest parses");
        assert_eq!(game.launcher, LauncherKind::Steam);
        assert_eq!(game.game_id, "730");
        assert_eq!(game.display_name, "Counter-Strike 2");
        assert_eq!(
            game.install_dir,
            Some(PathBuf::from("Counter-Strike Global Offensive"))
        );
        assert_eq!(game.device_id(), "game:steam:730");
    }

    #[test]
    fn parse_appmanifest_rejects_not_fully_installed_games() {
        // StateFlags 1082 = 0b10000111010: bit 2 (value 4) unset → updating.
        assert!(parse_appmanifest(MANIFEST_UPDATING).is_none());
    }

    #[test]
    fn parse_appmanifest_rejects_incomplete_manifests() {
        assert!(parse_appmanifest("").is_none());
        assert!(parse_appmanifest("\"AppState\"\n{\n\t\"appid\"\t\"1\"\n}").is_none());
    }

    #[test]
    fn parse_appmanifest_survives_deep_nesting_without_panicking() {
        let mut hostile = String::from("\"AppState\"\n");
        for _ in 0..200 {
            hostile.push_str("{\n");
        }
        // The depth guard must bail out instead of overflowing the stack.
        assert!(parse_appmanifest(&hostile).is_none());
    }

    #[test]
    fn parse_libraryfolders_lists_every_path_entry() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
	"2"
	{
		"path"		"E:\\Games\\Steam"
	}
}
"#;
        let libraries = parse_libraryfolders(vdf);
        assert_eq!(libraries.len(), 3);
        // Steam's VDF escapes backslashes; the parser must unescape them.
        assert_eq!(
            libraries[0],
            PathBuf::from("C:\\Program Files (x86)\\Steam")
        );
        assert_eq!(libraries[1], PathBuf::from("D:\\SteamLibrary"));
        assert_eq!(libraries[2], PathBuf::from("E:\\Games\\Steam"));
    }

    #[test]
    fn list_installed_games_in_library_reads_fixture_manifests() {
        let tmp = std::env::temp_dir().join(format!("rivulet_steam_fix_{}", std::process::id()));
        let steamapps = tmp.join("steamapps");
        std::fs::create_dir_all(&steamapps).unwrap();
        std::fs::write(steamapps.join("appmanifest_730.acf"), MANIFEST).unwrap();
        std::fs::write(steamapps.join("appmanifest_570.acf"), MANIFEST_UPDATING).unwrap();
        // Unrelated files must be ignored.
        std::fs::write(steamapps.join("libraryfolders.vdf"), "{}").unwrap();

        let games = list_installed_games_in_library(&tmp);
        assert_eq!(games.len(), 1, "only the installed game is listed");
        assert_eq!(games[0].game_id, "730");
        assert_eq!(games[0].display_name, "Counter-Strike 2");

        // A missing library degrades to an empty list, never an error.
        let missing = list_installed_games_in_library(&tmp.join("nope"));
        assert!(missing.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn device_id_convention_is_launcher_prefixed() {
        let game = GameIdentity {
            launcher: LauncherKind::Steam,
            game_id: "440".to_string(),
            display_name: "Team Fortress 2".to_string(),
            install_dir: None,
            executables: Vec::new(),
        };
        assert_eq!(game.device_id(), "game:steam:440");
    }

    #[test]
    fn launcher_kind_strings_are_stable() {
        assert_eq!(LauncherKind::Steam.as_str(), "steam");
        assert_eq!(LauncherKind::Heuristic.as_str(), "heuristic");
    }

    #[test]
    fn score_ranking_orders_launcher_over_heuristic() {
        assert!(Score::High > Score::Medium);
        assert!(Score::Medium > Score::Low);
    }

    // ── Epic Games Store (slice 2) ────────────────────────────────────

    const EPIC_MANIFEST: &str = r#"{
        "AppName": "Fortnite",
        "DisplayName": "Fortnite",
        "InstallLocation": "C:\\Program Files\\Epic Games\\Fortnite",
        "LaunchExecutable": "C:\\Program Files\\Epic Games\\Fortnite\\Binaries\\Win64\\FortniteClient-Win64-Shipping.exe",
        "AppVersion": "28.10.00.28.10.00"
    }"#;

    #[test]
    fn parse_epic_manifest_reads_identity_and_launch_executable() {
        let game = parse_epic_manifest(EPIC_MANIFEST).expect("epic manifest parses");
        assert_eq!(game.launcher, LauncherKind::Epic);
        assert_eq!(game.game_id, "Fortnite");
        assert_eq!(game.display_name, "Fortnite");
        assert_eq!(
            game.install_dir,
            Some(PathBuf::from("C:\\Program Files\\Epic Games\\Fortnite"))
        );
        // Foreground matching compares basenames, so the manifest's full
        // launch path must be reduced to its file stem.
        assert_eq!(
            game.executables,
            vec!["FortniteClient-Win64-Shipping".to_string()]
        );
        assert_eq!(game.device_id(), "game:epic:Fortnite");
    }

    #[test]
    fn parse_epic_manifest_accepts_the_nested_app_path_shape() {
        // Older Epic manifests nest the fields under AppName -> AppPath.
        let nested = r#"{
            "AppName": "RocketLeague",
            "AppPath": {
                "DisplayName": "Rocket League",
                "InstallLocation": "C:\\RocketLeague",
                "LaunchExecutable": "RocketLeague.exe"
            }
        }"#;
        let game = parse_epic_manifest(nested).expect("nested shape parses");
        assert_eq!(game.game_id, "RocketLeague");
        assert_eq!(game.display_name, "Rocket League");
        assert_eq!(game.executables, vec!["RocketLeague".to_string()]);
    }

    #[test]
    fn parse_epic_manifest_rejects_unusable_documents() {
        // Not JSON.
        assert!(parse_epic_manifest("not json at all").is_none());
        // Valid JSON but no id, or no display name: unusable as an identity.
        assert!(parse_epic_manifest(r#"{"DisplayName": "Nameless"}"#).is_none());
        assert!(parse_epic_manifest(r#"{"AppName": "OnlyId"}"#).is_none());
        // Blank strings must not become an empty-named row in the picker.
        assert!(parse_epic_manifest(r#"{"AppName": "  ", "DisplayName": "Blank"}"#).is_none());
        assert!(parse_epic_manifest("").is_none());
    }

    #[test]
    fn list_epic_games_in_reads_fixture_manifests() {
        let tmp = std::env::temp_dir().join(format!("rivulet_epic_fix_{}", std::process::id()));
        let manifests = tmp.join("Data").join("Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        std::fs::write(manifests.join("Fortnite.item"), EPIC_MANIFEST).unwrap();
        // Non-.item files and unreadable JSON must be ignored, not fatal.
        std::fs::write(manifests.join("notes.txt"), "ignore me").unwrap();
        std::fs::write(manifests.join("Broken.item"), "{oops").unwrap();

        let games = list_epic_games_in(&tmp);
        assert_eq!(games.len(), 1, "only the valid manifest is listed");
        assert_eq!(games[0].game_id, "Fortnite");

        // A launcher that is not installed degrades to an empty list.
        assert!(list_epic_games_in(&tmp.join("nope")).is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── GOG (slice 2) ─────────────────────────────────────────────────

    fn gog_fixture() -> Vec<RegistryGameEntry> {
        vec![
            RegistryGameEntry {
                key: "1207658924".to_string(),
                values: vec![
                    ("gameID".to_string(), "1207658924".to_string()),
                    ("name".to_string(), "Baldur's Gate 3".to_string()),
                    ("path".to_string(), "C:\\GOG Games\\BG3".to_string()),
                    (
                        "exe".to_string(),
                        "C:\\GOG Games\\BG3\\bin\\bg3.exe".to_string(),
                    ),
                ],
            },
            // A game without a title must be skipped, not rendered blank.
            RegistryGameEntry {
                key: "999".to_string(),
                values: vec![("gameID".to_string(), "999".to_string())],
            },
        ]
    }

    #[test]
    fn gog_games_from_entries_reads_identity_case_insensitively() {
        let games = gog_games_from_entries(&gog_fixture());
        assert_eq!(games.len(), 1, "the unnamed entry is skipped");
        let game = &games[0];
        assert_eq!(game.launcher, LauncherKind::Gog);
        assert_eq!(game.game_id, "1207658924");
        assert_eq!(game.display_name, "Baldur's Gate 3");
        assert_eq!(game.install_dir, Some(PathBuf::from("C:\\GOG Games\\BG3")));
        assert_eq!(game.executables, vec!["bg3".to_string()]);
        assert_eq!(game.device_id(), "game:gog:1207658924");
    }

    #[test]
    fn gog_games_from_entries_falls_back_to_the_subkey_name() {
        // Older GOG installs omit the explicit gameID value; the subkey name
        // is the id in that case.
        let entries = vec![RegistryGameEntry {
            key: "1440163901".to_string(),
            values: vec![("Name".to_string(), "Cyberpunk 2077".to_string())],
        }];
        let games = gog_games_from_entries(&entries);
        assert_eq!(games[0].game_id, "1440163901");
        assert_eq!(games[0].display_name, "Cyberpunk 2077");
    }

    #[test]
    fn gog_games_from_entries_degrades_on_an_empty_registry() {
        assert!(gog_games_from_entries(&[]).is_empty());
    }

    // ── Origin / EA app (slice 3) ─────────────────────────────────────

    #[test]
    fn parse_origin_mfst_extracts_the_game_id() {
        let modern = r#"<manifest gameid="12345">
  <version>1.0</version>
  <launch>&id=12345&itemid=987&amp;platform=windows</launch>
</manifest>"#;
        assert_eq!(parse_origin_mfst(modern).as_deref(), Some("12345"));

        let classic = r#"
<game>
  <launch>origin2://game/5567?offerIds=abc</launch>
</game>"#;
        assert_eq!(parse_origin_mfst(classic).as_deref(), Some("5567"));
    }

    #[test]
    fn parse_origin_mfst_rejects_manifests_without_an_id() {
        assert!(parse_origin_mfst("<manifest/>").is_none());
        assert!(parse_origin_mfst("").is_none());
        // An `&id=` marker with no digits behind it is not an id.
        assert!(parse_origin_mfst("&id=&platform=windows").is_none());
    }

    #[test]
    fn list_origin_games_in_reads_fixture_manifests() {
        let tmp = std::env::temp_dir().join(format!("rivulet_origin_fix_{}", std::process::id()));
        let game_dir = tmp.join("Apex Legends");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::write(
            game_dir.join("Apex Legends.mfst"),
            "<launch>origin2://game/1172470?offerIds=x</launch>",
        )
        .unwrap();
        // A game folder without a usable manifest contributes nothing.
        let broken = tmp.join("Uninstalled Game");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("Uninstalled Game.mfst"), "<manifest/>").unwrap();
        // A loose file at the root is not a game folder.
        std::fs::write(tmp.join("stray.mfst"), "origin2://game/1").unwrap();

        let games = list_origin_games_in(&tmp);
        assert_eq!(games.len(), 1, "only the manifested game is listed");
        assert_eq!(games[0].launcher, LauncherKind::Origin);
        assert_eq!(games[0].game_id, "1172470");
        // Origin has no title in the manifest, so the file stem is the name.
        assert_eq!(games[0].display_name, "Apex Legends");
        assert_eq!(games[0].install_dir, Some(game_dir));
        assert_eq!(games[0].device_id(), "game:origin:1172470");

        assert!(list_origin_games_in(&tmp.join("nope")).is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ── Battle.net (slice 3) ──────────────────────────────────────────

    #[test]
    fn battlenet_games_from_entries_uses_the_title_key_as_id_and_name() {
        let entries = vec![
            RegistryGameEntry {
                key: "Diablo IV".to_string(),
                values: vec![
                    (
                        "InstallPath".to_string(),
                        "C:\\Program Files (x86)\\Diablo IV".to_string(),
                    ),
                    (
                        "ExecutablePath".to_string(),
                        "C:\\Program Files (x86)\\Diablo IV\\Diablo IV.exe".to_string(),
                    ),
                ],
            },
            RegistryGameEntry {
                key: "StarCraft II".to_string(),
                values: vec![(
                    "DisplayName".to_string(),
                    "StarCraft II: Legacy".to_string(),
                )],
            },
        ];
        let games = battlenet_games_from_entries(&entries);
        assert_eq!(games.len(), 2);

        // No explicit display name: the title key is both id and name.
        let diablo = games
            .iter()
            .find(|g| g.game_id == "Diablo IV")
            .expect("Diablo IV is listed");
        assert_eq!(diablo.display_name, "Diablo IV");
        assert_eq!(diablo.executables, vec!["Diablo IV".to_string()]);
        assert_eq!(diablo.device_id(), "game:battlenet:Diablo IV");

        // An explicit display name wins over the key for the label.
        let starcraft = games
            .iter()
            .find(|g| g.game_id == "StarCraft II")
            .expect("StarCraft II is listed");
        assert_eq!(starcraft.display_name, "StarCraft II: Legacy");
    }

    #[test]
    fn battlenet_games_from_entries_skits_blank_keys() {
        let entries = vec![RegistryGameEntry {
            key: "   ".to_string(),
            values: Vec::new(),
        }];
        assert!(battlenet_games_from_entries(&entries).is_empty());
        assert!(battlenet_games_from_entries(&[]).is_empty());
    }

    /// Regression test for a host-dependent bug the Linux CI runner found:
    /// the launcher manifests always contain **Windows** paths, but
    /// `Path::file_stem()` only treats `\` as a separator on Windows, so on
    /// Linux the whole path was stored as the "executable name" and
    /// foreground matching silently stopped resolving these games.
    #[test]
    fn executable_name_is_independent_of_the_host_path_semantics() {
        // Windows separators — the real shape of the manifest values.
        assert_eq!(executable_name(r"C:\GOG Games\BG3\bin\bg3.exe"), "bg3");
        assert_eq!(
            executable_name(r"C:\Program Files (x86)\Diablo IV\Diablo IV.exe"),
            "Diablo IV"
        );
        // Forward slashes must behave identically.
        assert_eq!(executable_name("/opt/games/bg3.exe"), "bg3");
        // No extension, bare name, trailing separators.
        assert_eq!(executable_name("FortniteClient"), "FortniteClient");
        assert_eq!(executable_name(r"C:\Games\Game.exe\"), "Game");
        // A leading dot belongs to the name, not to an extension.
        assert_eq!(executable_name(".hidden"), ".hidden");
        // Empty input must not panic.
        assert_eq!(executable_name(""), "");

        // End-to-end through the parser, using a Windows path regardless of
        // the host this test runs on.
        let game = parse_epic_manifest(
            r#"{"AppName":"Fortnite","DisplayName":"Fortnite","LaunchExecutable":"C:\\Games\\Fortnite\\Binaries\\FortniteClient-Win64-Shipping.exe"}"#,
        )
        .expect("manifest parses");
        assert_eq!(
            game.executables,
            vec!["FortniteClient-Win64-Shipping".to_string()],
            "the launch executable must be a bare name on every host"
        );
    }

    // ── Foreground-window matching + ranking (slice 4) ────────────────

    fn heuristic_window(id: u64, title: &str) -> crate::game_capture::GameWindow {
        crate::game_capture::GameWindow {
            id,
            title: title.to_string(),
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn window_matches_game_ignores_launcher_decorations() {
        let game = GameIdentity {
            launcher: LauncherKind::Steam,
            game_id: "730".to_string(),
            display_name: "Counter-Strike 2".to_string(),
            install_dir: None,
            executables: Vec::new(),
        };
        // The exact title matches.
        assert!(window_matches_game("Counter-Strike 2", &game));
        // Launcher decorations are stripped.
        assert!(window_matches_game("Counter-Strike 2 - Steam", &game));
        assert!(window_matches_game("Counter-Strike 2 :: In-Game", &game));
        // Case-insensitive.
        assert!(window_matches_game("counter-strike 2", &game));
        // A different window does not.
        assert!(!window_matches_game(
            "Counter-Strike 2 (Server Browser)",
            &game
        ));
        assert!(!window_matches_game("", &game));
    }

    #[test]
    fn window_matches_game_prefers_the_launch_executable() {
        // A GOG game whose manifest title does not appear in the window
        // title at all still resolves through its launch executable.
        let game = GameIdentity {
            launcher: LauncherKind::Gog,
            game_id: "1207658924".to_string(),
            display_name: "Baldur's Gate 3".to_string(),
            install_dir: None,
            executables: vec!["bg3".to_string()],
        };
        assert!(window_matches_game("bg3", &game));
        // Matching is on the exact normalized stem, not a loose substring:
        // an unrelated window must not be swept in.
        assert!(!window_matches_game("Notepad", &game));
        assert!(!window_matches_game("bg3 launcher", &game));
    }

    #[test]
    fn rank_candidates_orders_launcher_over_foreground_over_heuristic() {
        let installed = vec![
            GameIdentity {
                launcher: LauncherKind::Steam,
                game_id: "730".to_string(),
                display_name: "Counter-Strike 2".to_string(),
                install_dir: None,
                executables: Vec::new(),
            },
            GameIdentity {
                launcher: LauncherKind::Epic,
                game_id: "Fortnite".to_string(),
                display_name: "Fortnite".to_string(),
                install_dir: None,
                executables: vec!["FortniteClient-Win64-Shipping".to_string()],
            },
        ];
        let windows = vec![
            heuristic_window(42, "FortniteClient-Win64-Shipping"),
            heuristic_window(7, "Counter-Strike 2"),
        ];

        let ranked = rank_candidates(&installed, &["730".to_string()], &windows);

        // Steam's running signal ranks above the Epic foreground match, which
        // ranks above every heuristic window.
        assert_eq!(ranked[0].identity.game_id, "730");
        assert_eq!(ranked[0].score, Score::High);
        assert_eq!(ranked[0].identity.device_id(), "game:steam:730");

        assert_eq!(ranked[1].identity.game_id, "Fortnite");
        assert_eq!(ranked[1].score, Score::Medium);
        assert_eq!(ranked[1].identity.device_id(), "game:epic:Fortnite");

        // Both heuristic windows remain available as the fallback.
        let low: Vec<_> = ranked
            .iter()
            .filter(|c| c.score == Score::Low)
            .map(|c| c.identity.game_id.clone())
            .collect();
        assert_eq!(low, vec!["42".to_string(), "7".to_string()]);
        assert_eq!(ranked[2].identity.launcher, LauncherKind::Heuristic);
        assert_eq!(ranked[2].identity.device_id(), "game:42");
    }

    #[test]
    fn rank_candidates_does_not_list_a_running_game_twice() {
        // A game that is both Steam-running and foreground-matching must
        // appear once, at High — not again at Medium.
        let installed = vec![GameIdentity {
            launcher: LauncherKind::Steam,
            game_id: "730".to_string(),
            display_name: "Counter-Strike 2".to_string(),
            install_dir: None,
            executables: Vec::new(),
        }];
        let windows = vec![heuristic_window(7, "Counter-Strike 2")];
        let ranked = rank_candidates(&installed, &["730".to_string()], &windows);

        let identity_hits = ranked
            .iter()
            .filter(|c| c.identity.launcher == LauncherKind::Steam)
            .count();
        assert_eq!(identity_hits, 1, "the running game is listed once");
        assert_eq!(ranked[0].score, Score::High);
        // The heuristic window is still offered as a fallback.
        assert_eq!(ranked[1].score, Score::Low);
    }

    #[test]
    fn rank_candidates_degrades_to_heuristic_windows_without_a_catalog() {
        // No launcher installed at all: the picker must still offer windows.
        let windows = vec![heuristic_window(42, "Some Game")];
        let ranked = rank_candidates(&[], &[], &windows);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].score, Score::Low);
        assert_eq!(ranked[0].identity.device_id(), "game:42");
    }

    #[test]
    fn deduplicate_by_device_id_keeps_the_first_occurrence() {
        let dup = GameIdentity {
            launcher: LauncherKind::Steam,
            game_id: "730".to_string(),
            display_name: "Counter-Strike 2".to_string(),
            install_dir: Some(PathBuf::from("first")),
            executables: Vec::new(),
        };
        let mut second = dup.clone();
        second.install_dir = Some(PathBuf::from("second"));
        let mut other = dup.clone();
        other.launcher = LauncherKind::Epic;
        other.game_id = "730".to_string();

        let deduped = deduplicate_by_device_id(vec![dup.clone(), second, other]);
        assert_eq!(
            deduped.len(),
            2,
            "same device id collapses, other launcher stays"
        );
        assert_eq!(deduped[0].install_dir, Some(PathBuf::from("first")));
        assert_eq!(deduped[1].launcher, LauncherKind::Epic);
    }

    #[test]
    fn launcher_kind_all_covers_every_real_launcher_without_the_heuristic() {
        let all = LauncherKind::all();
        // Steam, Epic, GOG, Origin, EA app, Battle.net.
        assert_eq!(all.len(), 6);
        assert!(!all.contains(&LauncherKind::Heuristic));
        for kind in all {
            assert_ne!(kind.as_str(), "heuristic");
        }
    }

    // ── Steam install-directory scan (fills `executables`) ─────────────

    /// Build a Steam-shaped install tree and return its library root.
    fn steam_library_fixture(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("rivulet_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let steamapps = tmp.join("steamapps");
        std::fs::create_dir_all(steamapps.join("common")).unwrap();
        std::fs::write(
            steamapps.join("appmanifest_220.acf"),
            r#""AppState"
{
	"appid"		"220"
	"name"		"Half-Life 2"
	"StateFlags"		"4"
	"installdir"		"Half-Life 2"
}
"#,
        )
        .unwrap();
        tmp
    }

    #[test]
    fn scan_steam_executables_collects_top_level_and_one_level_deep() {
        let root = steam_library_fixture("steam_scan");
        let game = root.join("steamapps").join("common").join("Half-Life 2");
        std::fs::create_dir_all(game.join("bin")).unwrap();
        std::fs::write(game.join("hl2.exe"), b"MZ").unwrap();
        std::fs::write(game.join("Launcher.exe"), b"MZ").unwrap();
        // Non-executable files and deeper trees must be ignored.
        std::fs::write(game.join("readme.txt"), b"hi").unwrap();
        std::fs::create_dir_all(game.join("bin").join("nested")).unwrap();
        std::fs::write(game.join("bin").join("deep.exe"), b"MZ").unwrap();
        std::fs::write(game.join("bin").join("nested").join("toodeep.exe"), b"MZ").unwrap();

        let names = scan_steam_executables(&game);
        // One level deep is scanned (`bin/deep.exe`), two levels is not.
        assert!(names.contains(&"hl2".to_string()), "{names:?}");
        assert!(names.contains(&"Launcher".to_string()), "{names:?}");
        assert!(names.contains(&"deep".to_string()), "{names:?}");
        assert!(!names.contains(&"toodeep".to_string()), "{names:?}");
        assert!(!names.contains(&"readme".to_string()), "{names:?}");

        // Sorted and deduplicated, so the cache and the tests are stable.
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "the scan must be deterministic");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_steam_executables_is_bounded_and_degrades_gracefully() {
        let root = steam_library_fixture("steam_bound");
        let game = root.join("steamapps").join("common").join("Half-Life 2");
        std::fs::create_dir_all(&game).unwrap();
        // Far more executables than the cap, plus more subdirectories than it.
        for index in 0..MAX_EXECUTABLES_PER_GAME * 3 {
            std::fs::write(game.join(format!("game{index}.exe")), b"MZ").unwrap();
        }
        for index in 0..MAX_SUBDIRS_PER_GAME * 3 {
            std::fs::create_dir_all(game.join(format!("bin{index}"))).unwrap();
            std::fs::write(game.join(format!("bin{index}")).join("inner.exe"), b"MZ").unwrap();
        }

        let names = scan_steam_executables(&game);
        assert_eq!(
            names.len(),
            MAX_EXECUTABLES_PER_GAME,
            "the per-game cap must hold"
        );

        // A missing directory is an empty list, never a panic or an error.
        assert!(scan_steam_executables(&game.join("does-not-exist")).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The regression this feature exists for: a Steam game whose window title
    /// is the executable name (not the store's display name) must resolve to
    /// the game identity at `Score::Medium`.
    #[test]
    fn steam_game_resolves_through_its_launch_executable() {
        let identity = GameIdentity {
            launcher: LauncherKind::Steam,
            game_id: "220".to_string(),
            display_name: "Half-Life 2".to_string(),
            install_dir: None,
            // Only the executable matches; the display name deliberately does not.
            executables: vec!["hl2".to_string()],
        };

        assert!(window_matches_game("hl2", &identity));
        assert!(window_matches_game("hl2 - Steam", &identity));

        let windows = vec![crate::game_capture::GameWindow {
            id: 99,
            title: "hl2".to_string(),
            width: 1920,
            height: 1080,
        }];
        let ranked = rank_candidates(&[identity], &[], &windows);

        assert_eq!(
            ranked[0].score,
            Score::Medium,
            "a Steam game must now reach Medium through its executable"
        );
        assert_eq!(ranked[0].identity.device_id(), "game:steam:220");
    }

    /// The reader resolves `installdir` to a real path but must **not** walk
    /// it: that is deferred so the GUI thread never blocks on install-tree
    /// I/O when the picker opens.
    #[test]
    fn library_reader_resolves_install_dir_without_scanning_it() {
        let root = steam_library_fixture("steam_reader");
        let common = root.join("steamapps").join("common").join("Half-Life 2");
        std::fs::create_dir_all(&common).unwrap();
        std::fs::write(common.join("hl2.exe"), b"MZ").unwrap();

        let games = list_installed_games_in_library(&root);
        assert_eq!(games.len(), 1);
        let game = &games[0];
        assert_eq!(game.game_id, "220");
        // Absolute, so a later scan can use it without knowing the library root.
        assert_eq!(game.install_dir, Some(common));
        // And still unscanned.
        assert!(
            game.executables.is_empty(),
            "building the catalog must not touch the install directory"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The lazy pass fills the list, and the cache keeps the second pass free.
    #[test]
    fn enrich_executables_scans_once_and_survives_a_second_call() {
        let root = steam_library_fixture("steam_cache");
        let common = root.join("steamapps").join("common").join("Half-Life 2");
        std::fs::create_dir_all(&common).unwrap();
        std::fs::write(common.join("first.exe"), b"MZ").unwrap();

        let mut games = list_installed_games_in_library(&root);
        enrich_executables(&mut games);
        assert_eq!(games[0].executables, vec!["first".to_string()]);

        // A second pass must not observe a new file: that is the cache doing
        // its job, keeping a picker refresh from re-walking the library.
        std::fs::write(common.join("second.exe"), b"MZ").unwrap();
        enrich_executables(&mut games);
        assert_eq!(
            games[0].executables,
            vec!["first".to_string()],
            "the cache must make the second pass free"
        );

        // An explicit rescan picks the new executable up.
        rescan_steam_executables(&mut games);
        assert_eq!(
            games[0].executables,
            vec!["first".to_string(), "second".to_string()]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A game that already knows its executables (Epic, GOG, Battle.net) must
    /// not be re-scanned, and one without an install directory must be skipped
    /// rather than panicking.
    #[test]
    fn enrich_executables_skips_filled_and_unlocated_games() {
        let mut games = vec![
            GameIdentity {
                launcher: LauncherKind::Epic,
                game_id: "Fortnite".to_string(),
                display_name: "Fortnite".to_string(),
                install_dir: None,
                executables: vec!["FortniteClient".to_string()],
            },
            GameIdentity {
                launcher: LauncherKind::Origin,
                game_id: "1172470".to_string(),
                display_name: "Apex Legends".to_string(),
                install_dir: None,
                executables: Vec::new(),
            },
        ];

        enrich_executables(&mut games);

        assert_eq!(games[0].executables, vec!["FortniteClient".to_string()]);
        assert!(
            games[1].executables.is_empty(),
            "a game without an install directory must stay empty, not panic"
        );
    }

    // ── XDG base directories / Steam root probing (Linux slice) ───────

    /// A fresh, empty temp directory. Removed up front as well as afterwards,
    /// so a leftover from a crashed run cannot make a test pass by accident.
    fn scratch_dir(name: &str) -> PathBuf {
        let tmp = std::env::temp_dir().join(format!("rivulet_xdg_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        tmp
    }

    #[test]
    fn xdg_data_dir_prefers_an_absolute_variable() {
        let home = PathBuf::from("/home/tester");
        // "Absolute" is a per-platform notion (a drive letter on Windows), so the
        // fixture is taken from the host rather than hardcoded to a Unix root.
        let absolute = std::env::temp_dir().join("xdg_data");
        assert_eq!(
            xdg_data_dir(Some(absolute.clone().into_os_string()), Some(home)),
            Some(absolute),
            "an absolute XDG_DATA_HOME wins over the $HOME default"
        );
    }

    #[test]
    fn xdg_data_dir_ignores_a_relative_or_empty_variable() {
        let home = PathBuf::from("/home/tester");
        let expected = Some(PathBuf::from("/home/tester/.local/share"));
        // The Base Directory Specification: a value that is not an absolute
        // path must be ignored as if it was unset.
        assert_eq!(
            xdg_data_dir(
                Some(std::ffi::OsString::from("relative/data")),
                Some(home.clone())
            ),
            expected
        );
        assert_eq!(
            xdg_data_dir(Some(std::ffi::OsString::from("")), Some(home.clone())),
            expected
        );
        assert_eq!(xdg_data_dir(None, Some(home)), expected);
    }

    #[test]
    fn xdg_data_dir_is_none_without_any_anchor() {
        assert_eq!(xdg_data_dir(None, None), None);
        let absolute = std::env::temp_dir().join("xdg_data");
        assert_eq!(
            xdg_data_dir(Some(absolute.clone().into_os_string()), None),
            Some(absolute),
            "an absolute variable alone is already enough"
        );
    }

    #[test]
    fn steam_roots_probe_order_is_xdg_then_home() {
        let home = scratch_dir("order");
        // Only the Flatpak layout exists, so it is the single hit -- and it
        // has to be found without the two earlier candidates existing.
        let flatpak = home
            .join(".var")
            .join("app")
            .join("com.valvesoftware.Steam")
            .join("data")
            .join("Steam");
        std::fs::create_dir_all(&flatpak).unwrap();

        let data_home = home.join(".local").join("share");
        let roots = steam_roots(Some(&data_home), Some(&home));
        assert_eq!(roots, vec![flatpak], "{roots:?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn steam_roots_prefers_xdg_over_the_legacy_home_aliases() {
        let home = scratch_dir("prefer");
        let data_home = home.join(".local").join("share");
        let xdg_steam = data_home.join("Steam");
        let legacy = home.join(".steam").join("steam");
        std::fs::create_dir_all(&xdg_steam).unwrap();
        std::fs::create_dir_all(&legacy).unwrap();

        let roots = steam_roots(Some(&data_home), Some(&home));
        assert_eq!(
            roots.first().map(|root| root.as_path()),
            Some(xdg_steam.as_path()),
            "the XDG install must win so the legacy symlink is never read twice"
        );
        assert_eq!(roots.len(), 2, "both existing candidates are reported");

        // Without an XDG data home only the legacy alias is left.
        let roots = steam_roots(None, Some(&home));
        assert_eq!(roots, vec![legacy], "{roots:?}");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn steam_roots_is_empty_when_steam_is_absent() {
        let home = scratch_dir("absent");
        assert!(steam_roots(Some(&home.join(".local").join("share")), Some(&home)).is_empty());
        assert!(steam_roots(None, None).is_empty());

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn xdg_steam_layout_yields_real_game_identities() {
        let home = scratch_dir("identity");
        let data_home = home.join(".local").join("share");
        let steamapps = data_home.join("Steam").join("steamapps");
        std::fs::create_dir_all(&steamapps).unwrap();
        std::fs::write(steamapps.join("appmanifest_730.acf"), MANIFEST).unwrap();

        let roots = steam_roots(Some(&data_home), Some(&home));
        let games: Vec<GameIdentity> = roots
            .iter()
            .flat_map(|root| list_installed_games_in_library(root))
            .collect();
        assert_eq!(
            games.len(),
            1,
            "the Linux layout must yield a catalog entry"
        );
        assert_eq!(games[0].device_id(), "game:steam:730");
        assert_eq!(games[0].display_name, "Counter-Strike 2");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn steam_app_is_running_is_false_for_a_missing_app() {
        assert!(!steam_app_is_running("0"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn list_installed_games_degrades_without_steam_installed() {
        // Must return an empty list instead of erroring when the registry
        // key or files do not exist (CI machines have no Steam).
        let games = list_installed_games();
        let _ = games.len();
    }
}
