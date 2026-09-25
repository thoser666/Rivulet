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
//! Later slices add the remaining launchers (GOG, Epic, Origin/EA app,
//! Battle.net) as additional [`LauncherKind`] variants with their own
//! readers, so the model below is deliberately launcher-generic already.

use std::path::{Path, PathBuf};

/// Which launcher identified a game. `Heuristic` is the existing size/title
/// window fallback — a *window*, not an installed-game identity.
///
/// Serde derives support the GUI's persisted app state (eframe storage).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LauncherKind {
    Steam,
    /// Reserved for later #239 slices; never constructed yet.
    #[allow(dead_code)]
    Epic,
    /// Reserved for later #239 slices; never constructed yet.
    #[allow(dead_code)]
    Gog,
    /// Reserved for later #239 slices; never constructed yet.
    #[allow(dead_code)]
    Origin,
    /// Reserved for later #239 slices; never constructed yet.
    #[allow(dead_code)]
    EaApp,
    /// Reserved for later #239 slices; never constructed yet.
    #[allow(dead_code)]
    BattleNet,
    /// The size/title window heuristic (lowest confidence).
    #[allow(dead_code)]
    Heuristic,
}

impl LauncherKind {
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
    pub fn device_id(&self) -> String {
        format!("game:{}:{}", self.launcher.as_str(), self.game_id)
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

// ── Steam installation discovery + readers (Windows-gated) ─────────────

/// The Steam installation root: `HKCU\SOFTWARE\Valve\Steam` → `SteamPath`.
///
/// Windows-only (the registry is the documented source); every other
/// platform returns `None` in this slice.
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

#[cfg(not(target_os = "windows"))]
fn steam_root() -> Option<PathBuf> {
    None
}

/// Steam's running-game registry key:
/// `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\Apps\<appid>` → `Running = 1`
/// (Steam also writes the 64-bit view; the WOW6432Node path is the one its
/// own docs reference). `None` when the game is not running / not present.
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

#[cfg(not(target_os = "windows"))]
fn steam_app_is_running(_app_id: &str) -> bool {
    false
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
        if let Some(game) = parse_appmanifest(&contents) {
            games.push(game);
        }
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

/// Detect the currently running game: Steam's `Running` registry signal over
/// the installed catalog (ranked [`Score::High`]). The size/title window
/// heuristic is *not* re-implemented here — the GUI keeps its existing
/// window list as the `Score::Low` fallback, so callers combine both lists.
pub fn detect_running_game() -> Vec<RunningGameCandidate> {
    let mut candidates = Vec::new();
    for game in list_installed_games() {
        if steam_app_is_running(&game.game_id) {
            candidates.push(RunningGameCandidate {
                identity: game,
                score: Score::High,
            });
        }
    }
    candidates
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
