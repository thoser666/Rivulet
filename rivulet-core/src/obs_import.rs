//! Import an OBS Studio scene collection into Rivulet's scene model
//! ([#367](https://github.com/thoser666/Rivulet/issues/367)).
//!
//! The mapping is the one `docs/scene-management.md` describes: an OBS scene
//! becomes a [`crate::Scene`], an OBS group becomes a scene other scenes point
//! at through `parent`, and a group inside a group points at the enclosing
//! group. What this module adds is the part a user actually needs: reading the
//! collection file, binding the OBS scene items as sources with their
//! transform, crop, visibility and lock state, and *reporting* everything that
//! has no counterpart instead of dropping it silently.
//!
//! # What is carried, and what is not
//!
//! | OBS | Rivulet |
//! | --- | --- |
//! | Scene | [`crate::Scene`] (`kind = Primary`) |
//! | Group | [`crate::Scene`] that its members point at via `parent` |
//! | Scene / group item | [`crate::SourceManager`] binding in that scene |
//! | `pos`, `rot`, `bounds`, `crop_*`, `visible`, `locked` | [`crate::Transform`], [`crate::Crop`], binding flags |
//! | Source type | [`crate::SourceKind`] for the types in [`source_kind`] |
//! | `device_id` / `window` / `monitor` | [`crate::Source::device_id`] |
//!
//! Deliberately **not** carried, each reported as an [`ObsImportNote`]:
//!
//! - the payload of a source (`url`, `file`, `text`, ...): [`crate::Source`]
//!   has no field for it, the payload lives in the per-kind source modules;
//! - an OBS source type without a Rivulet counterpart: it is skipped, because
//!   a placeholder would need a new [`crate::SourceKind`] variant - a breaking
//!   change under `docs/api-migration.md`, not something an importer may do;
//! - the collection's global transition: Rivulet's transition overrides are
//!   per folder ([#368](https://github.com/thoser666/Rivulet/issues/368)), not
//!   a collection-wide default;
//! - OBS's `scale`: it is relative to the source's native size, which the
//!   collection file does not contain. Items without `bounds` keep Rivulet's
//!   default frame size and the report names them.
//!
//! Nothing here touches GStreamer, hardware or the filesystem beyond reading
//! the collection file, so the mapper is testable in milliseconds.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::scene::{Scene, SceneManager};
use crate::source::{Crop, Source, SourceKind, SourceManager, Transform};

/// The OBS source id of a scene container.
const OBS_SCENE: &str = "scene";

/// The OBS source id of a group container.
const OBS_GROUP: &str = "group";

/// One thing the import could not carry one-to-one.
///
/// Every variant names the OBS object it is about, so a user can find it in
/// OBS and decide what to do, instead of discovering a gap months later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObsImportNote {
    /// An OBS source type Rivulet has no counterpart for. The source is
    /// skipped; its name and OBS type are both reported.
    UnsupportedSource {
        /// The OBS source name.
        name: String,
        /// The OBS source type (`obs_nvidia_denoiser`, ...).
        obs_id: String,
    },
    /// A mapped source whose payload has no field in [`crate::Source`].
    PayloadNotCarried {
        /// The OBS source name.
        source: String,
        /// The settings key that held the payload (`url`, `file`, `text`, ...).
        field: String,
    },
    /// An item whose OBS `scale` could not be converted because the collection
    /// does not contain the source's native size and the item has no `bounds`.
    ScaleNotConverted {
        /// The scene or group that holds the item.
        scene: String,
        /// The item (source) name.
        source: String,
    },
    /// The collection had no usable `scene_order`, so the order was derived
    /// from the source list (which OBS writes in creation order).
    SceneOrderDerived,
    /// `current_scene` named a scene the collection does not contain.
    ActiveSceneUnknown {
        /// The name `current_scene` held.
        name: String,
    },
    /// A scene item referenced a source the collection does not define.
    DanglingItem {
        /// The scene or group that holds the dangling item.
        scene: String,
        /// The item name that did not resolve.
        source: String,
    },
    /// A `groups[]` entry had no matching group source, so its members were
    /// imported at the top level instead of disappearing.
    DanglingGroup {
        /// The group name from the `groups` list.
        name: String,
    },
    /// A group appeared inside itself; the nesting was cut at that point.
    GroupCycle {
        /// The group whose membership closed the cycle.
        name: String,
    },
    /// The collection's global transition has no Rivulet counterpart.
    TransitionNotCarried {
        /// The OBS transition name (`Fade`, `Cut`, ...).
        name: String,
        /// The OBS transition duration in milliseconds.
        duration_ms: Option<u64>,
    },
}

impl ObsImportNote {
    /// One human-readable line for the CLI report and the GUI list.
    pub fn describe(&self) -> String {
        match self {
            Self::UnsupportedSource { name, obs_id } => {
                format!("skipped source {name:?}: no Rivulet counterpart for OBS type {obs_id:?}")
            }
            Self::PayloadNotCarried { source, field } => format!(
                "{source:?}: OBS setting {field:?} has no counterpart in a Rivulet source (not carried)"
            ),
            Self::ScaleNotConverted { scene, source } => format!(
                "{scene:?}/{source:?}: OBS scale is relative to the source size, which the collection \
                 does not contain; the item keeps the default frame size"
            ),
            Self::SceneOrderDerived => {
                "no usable `scene_order`: the order follows the source list".to_string()
            }
            Self::ActiveSceneUnknown { name } => {
                format!("`current_scene` {name:?} does not exist; the first scene stays active")
            }
            Self::DanglingItem { scene, source } => format!(
                "{scene:?}: item {source:?} references a source the collection does not define"
            ),
            Self::DanglingGroup { name } => format!(
                "the `groups` entry {name:?} has no group source; its members were imported at the \
                 top level"
            ),
            Self::GroupCycle { name } => {
                format!("group {name:?} contains itself; the nesting stops there")
            }
            Self::TransitionNotCarried {
                name,
                duration_ms,
            } => match duration_ms {
                Some(ms) => format!(
                    "transition {name:?} ({ms} ms) is not carried: Rivulet overrides transitions per folder"
                ),
                None => format!(
                    "transition {name:?} is not carried: Rivulet overrides transitions per folder"
                ),
            },
        }
    }
}

/// What an import produced, beyond the two managers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObsImportReport {
    /// The OBS collection name (`"Untitled"` in a fresh OBS install).
    pub collection: String,
    /// Primary scenes created (OBS scenes, excluding groups).
    pub scenes: usize,
    /// Folder scenes created (one per OBS group).
    pub folders: usize,
    /// Sources mapped into the [`SourceManager`].
    pub sources: usize,
    /// Scene-item bindings created.
    pub bindings: usize,
    /// OBS sources skipped because their type has no counterpart.
    pub skipped: usize,
    /// Everything that did not map one-to-one, in generation order.
    pub notes: Vec<ObsImportNote>,
}

impl ObsImportReport {
    /// Whether the import lost or approximated anything.
    pub fn has_notes(&self) -> bool {
        !self.notes.is_empty()
    }

    /// The human-readable report the CLI prints and the GUI shows before
    /// applying an import.
    pub fn describe(&self) -> String {
        let mut out = format!(
            "OBS import: {}\n  scenes: {} primary, {} folder(s)\n  sources: {} mapped, {} skipped\n  items: {} bound\n",
            self.collection,
            self.scenes,
            self.folders,
            self.sources,
            self.skipped,
            self.bindings
        );
        if self.notes.is_empty() {
            out.push_str("  every source and item mapped without a difference\n");
        } else {
            out.push_str(&format!("  differences ({}):\n", self.notes.len()));
            for note in &self.notes {
                out.push_str(&format!("  - {}\n", note.describe()));
            }
        }
        out
    }
}

/// Why a collection could not be imported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObsImportError {
    /// The file could not be read.
    Io(String),
    /// The file is not valid OBS collection JSON.
    InvalidJson(String),
    /// The file parses but contains no scene and no group.
    NoScenes,
}

impl std::fmt::Display for ObsImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not read the OBS collection: {error}"),
            Self::InvalidJson(error) => {
                write!(f, "not a valid OBS scene collection: {error}")
            }
            Self::NoScenes => write!(
                f,
                "the OBS collection contains no scenes or groups (nothing to import)"
            ),
        }
    }
}

impl std::error::Error for ObsImportError {}

/// A successful import: the collection's two managers plus the report.
#[derive(Debug, Clone)]
pub struct ObsImport {
    /// The imported collection name (also set on [`Self::scenes`]).
    pub collection: String,
    /// Scenes and folders, active scene included.
    pub scenes: SceneManager,
    /// Sources and their scene bindings.
    pub sources: SourceManager,
    /// Counts and every difference the import had to make.
    pub report: ObsImportReport,
}

/// Read and map an OBS collection file.
pub fn import_file(path: &Path) -> Result<ObsImport, ObsImportError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| ObsImportError::Io(format!("{}: {error}", path.display())))?;
    import_collection(&raw)
}

/// Map an OBS collection document.
///
/// The document is the JSON OBS writes to
/// `basic/scenes/<collection>.json`. Unknown top-level keys are ignored on
/// purpose: OBS adds fields between releases and a newer collection must stay
/// importable. Only the shapes this mapper reads are deserialized.
pub fn import_collection(json: &str) -> Result<ObsImport, ObsImportError> {
    let raw: RawCollection = serde_json::from_str(json)
        .map_err(|error| ObsImportError::InvalidJson(error.to_string()))?;
    let has_container = raw
        .sources
        .iter()
        .any(|source| source.id == OBS_SCENE || source.id == OBS_GROUP);
    if !has_container {
        return Err(ObsImportError::NoScenes);
    }
    Ok(build(raw))
}

// ---------------------------------------------------------------------------
// OBS document shape (only the fields this mapper reads)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RawCollection {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    current_scene: Option<String>,
    #[serde(default)]
    scene_order: Vec<RawName>,
    #[serde(default)]
    sources: Vec<RawSource>,
    #[serde(default)]
    groups: Vec<RawGroup>,
    #[serde(default)]
    transition: Option<String>,
    #[serde(default)]
    transition_duration: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawName {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawSource {
    name: String,
    /// OBS source type (`scene`, `group`, `monitor_capture`, ...).
    #[serde(default)]
    id: String,
    #[serde(default)]
    settings: RawSettings,
}

#[derive(Debug, Default, Deserialize)]
struct RawSettings {
    #[serde(default)]
    items: Vec<RawItem>,
    #[serde(default)]
    device_id: Option<String>,
    #[serde(default)]
    video_device_id: Option<String>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    window: Option<String>,
    /// OBS writes a monitor index in older collections and a device path in
    /// newer ones, so both shapes are accepted.
    #[serde(default)]
    monitor: Option<RawMonitor>,
    // Payload keys that have no counterpart in `Source`; only their presence
    // is reported.
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    local_file: Option<String>,
    #[serde(default)]
    input_file: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawMonitor {
    Index(i64),
    Id(String),
}

#[derive(Debug, Deserialize)]
struct RawGroup {
    name: String,
    #[serde(default)]
    items: Vec<RawName>,
}

#[derive(Debug, Default, Deserialize)]
struct RawItem {
    #[serde(default)]
    name: String,
    #[serde(default = "default_true")]
    visible: bool,
    #[serde(default)]
    locked: bool,
    #[serde(default)]
    rot: f32,
    #[serde(default)]
    pos: RawPos,
    #[serde(default)]
    bounds_type: u32,
    #[serde(default)]
    bounds: RawPos,
    #[serde(default)]
    crop_left: i64,
    #[serde(default)]
    crop_top: i64,
    #[serde(default)]
    crop_right: i64,
    #[serde(default)]
    crop_bottom: i64,
}

#[derive(Debug, Default, Deserialize)]
struct RawPos {
    #[serde(default)]
    x: f32,
    #[serde(default)]
    y: f32,
    /// OBS alignment: row-major from 0 (top-left) to 8 (bottom-right), 4 is
    /// the centre.
    #[serde(default)]
    alignment: u32,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

/// The Rivulet kind an OBS source type maps to, or `None` when there is no
/// counterpart.
///
/// The ids are the ones OBS writes into `sources[].id`; a type is listed only
/// when the Rivulet renderer can draw it, so a new OBS plugin source is
/// reported as unsupported instead of being imported as the wrong thing.
pub fn source_kind(obs_id: &str) -> Option<SourceKind> {
    match obs_id {
        // Compositor capture: monitors, windows and desktop streams.
        "monitor_capture"
        | "display_capture"
        | "screen_capture"
        | "xshm_input"
        | "window_capture"
        | "window_capture_v2"
        | "pipewire_desktop_capture"
        | "pipewire_window_capture" => Some(SourceKind::ScreenCapture),
        "game_capture" | "game_capture_v2" => Some(SourceKind::GameCapture),
        // Cameras and capture devices.
        "v4l2_input" | "av_capture_input" | "dshow_input" | "decklink_input" => {
            Some(SourceKind::Webcam)
        }
        // Audio devices, application audio and outputs.
        "audio_input_capture"
        | "wasapi_input_capture"
        | "wasapi_process_output_capture"
        | "pulse_input_capture"
        | "coreaudio_input_capture"
        | "alsa_input_capture"
        | "jack_input_capture"
        | "sndio_input_capture"
        | "pipewire_input_capture"
        | "audio_output_capture"
        | "wasapi_output_capture"
        | "pulse_output_capture"
        | "coreaudio_output_capture"
        | "alsa_output_capture"
        | "jack_output_capture"
        | "sndio_output_capture"
        | "pipewire_output_capture" => Some(SourceKind::Audio),
        // File-backed media.
        "ffmpeg_source" | "vlc_source" | "slideshow" => Some(SourceKind::Media),
        "image_source" => Some(SourceKind::Image),
        "browser_source" | "browser_source_v2" => Some(SourceKind::Browser),
        // Text overlays (Windows GDI+, FreeType on Linux/macOS).
        "text_gdiplus" | "text_gdiplus_v2" | "text_gdiplus_v3" | "text_ft2_source"
        | "text_ft2_source_v2" | "text-freetype2" => Some(SourceKind::Text),
        // Solid colours.
        "color_source" | "color_source_v2" | "color_source_v3" => Some(SourceKind::Color),
        _ => None,
    }
}

/// Everything the mapper needs while it walks the collection.
struct Mapper<'a> {
    scene_names: &'a [String],
    folder_names: &'a [String],
    members: &'a HashMap<String, Vec<String>>,
    ids: HashMap<String, Uuid>,
    created: HashSet<String>,
    report: &'a mut ObsImportReport,
}

impl Mapper<'_> {
    fn is_container(&self, name: &str) -> bool {
        self.scene_names.iter().any(|scene| scene == name)
            || self.folder_names.iter().any(|folder| folder == name)
    }

    /// Create `name` and, depth-first, its nested containers.
    ///
    /// A folder is created before its members, so every `parent` is a real id
    /// and the collection's own order reads like the dock's tree. A group that
    /// appears inside itself is cut off and reported instead of recursing.
    fn create_subtree(&mut self, name: &str, parent: Option<Uuid>, scenes: &mut SceneManager) {
        if !self.created.insert(name.to_string()) {
            self.report.notes.push(ObsImportNote::GroupCycle {
                name: name.to_string(),
            });
            return;
        }
        let scene = match parent {
            Some(parent) => Scene::with_parent(name.to_string(), parent),
            None => Scene::new(name.to_string()),
        };
        let id = scenes.add(scene);
        self.ids.insert(name.to_string(), id);
        if self.folder_names.iter().any(|folder| folder == name) {
            self.report.folders += 1;
        } else {
            self.report.scenes += 1;
        }
        let children = self.members.get(name).cloned().unwrap_or_default();
        for child in children {
            if self.is_container(&child) {
                self.create_subtree(&child, Some(id), scenes);
            }
        }
    }
}

fn build(raw: RawCollection) -> ObsImport {
    let collection = raw
        .name
        .clone()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Default".to_string());
    let mut report = ObsImportReport {
        collection: collection.clone(),
        scenes: 0,
        folders: 0,
        sources: 0,
        bindings: 0,
        skipped: 0,
        notes: Vec::new(),
    };

    // Containers (scenes + groups) and their membership, as OBS names them.
    let scene_names: Vec<String> = raw
        .sources
        .iter()
        .filter(|source| source.id == OBS_SCENE)
        .map(|source| source.name.clone())
        .collect();
    let folder_names: Vec<String> = raw
        .sources
        .iter()
        .filter(|source| source.id == OBS_GROUP)
        .map(|source| source.name.clone())
        .collect();

    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for group in &raw.groups {
        if !folder_names.iter().any(|folder| folder == &group.name) {
            // A `groups` entry without a group source cannot become a folder.
            // Its members must not vanish, so they fall back to the top level
            // and the report says why.
            report.notes.push(ObsImportNote::DanglingGroup {
                name: group.name.clone(),
            });
        }
        members.insert(
            group.name.clone(),
            group.items.iter().map(|item| item.name.clone()).collect(),
        );
    }

    let is_container = |name: &str| {
        scene_names.iter().any(|scene| scene == name)
            || folder_names.iter().any(|folder| folder == name)
    };

    // Display order: `scene_order` is what the Scene list shows, groups
    // included. Without it OBS's source list order (creation order) is all we
    // have, and the report says so.
    let mut top_level: Vec<String> = raw
        .scene_order
        .iter()
        .map(|entry| entry.name.clone())
        .filter(|name| is_container(name))
        .collect();
    if top_level.is_empty() {
        top_level.extend(scene_names.iter().cloned());
        top_level.extend(folder_names.iter().cloned());
        report.notes.push(ObsImportNote::SceneOrderDerived);
    }
    // A container that is a member of a group is not top level, even when the
    // order list also mentions it (nested scenes in newer OBS releases).
    let nested: HashSet<&String> = members
        .values()
        .flatten()
        .filter(|name| is_container(name))
        .collect();
    top_level.retain(|name| !nested.contains(name));

    let mut scenes = SceneManager::new();
    scenes.set_collection(collection.clone());
    let mut mapper = Mapper {
        scene_names: &scene_names,
        folder_names: &folder_names,
        members: &members,
        ids: HashMap::new(),
        created: HashSet::new(),
        report: &mut report,
    };
    for name in &top_level {
        mapper.create_subtree(name, None, &mut scenes);
    }
    // Containers the order did not mention (or that were cut off by the cycle
    // guard) still get a home rather than vanishing.
    for name in scene_names.iter().chain(folder_names.iter()) {
        if !mapper.created.contains(name) {
            mapper.create_subtree(name, None, &mut scenes);
        }
    }
    let ids = mapper.ids;

    // Sources: everything that is not a container.
    let mut sources = SourceManager::new();
    let mut source_ids: HashMap<String, Uuid> = HashMap::new();
    let mut skipped: HashSet<String> = HashSet::new();
    for raw_source in raw
        .sources
        .iter()
        .filter(|source| source.id != OBS_SCENE && source.id != OBS_GROUP)
    {
        let Some(kind) = source_kind(&raw_source.id) else {
            report.skipped += 1;
            skipped.insert(raw_source.name.clone());
            report.notes.push(ObsImportNote::UnsupportedSource {
                name: raw_source.name.clone(),
                obs_id: raw_source.id.clone(),
            });
            continue;
        };
        if let Some(field) = payload_field(&raw_source.settings) {
            report.notes.push(ObsImportNote::PayloadNotCarried {
                source: raw_source.name.clone(),
                field: field.to_string(),
            });
        }
        let mut source = Source::new(raw_source.name.clone(), kind);
        if let Some(device) = device_id(&raw_source.settings) {
            source.device_id = device;
        }
        let id = sources.add_source(source);
        source_ids.insert(raw_source.name.clone(), id);
        report.sources += 1;
    }

    // Bindings: a scene's `settings.items`, plus a group's members (which OBS
    // stores in the collection's `groups` list, without per-item transforms).
    for raw_source in raw
        .sources
        .iter()
        .filter(|source| source.id == OBS_SCENE || source.id == OBS_GROUP)
    {
        let Some(&scene_id) = ids.get(&raw_source.name) else {
            continue;
        };
        // OBS lists items top-of-the-list first and the top of the list is
        // drawn last; Rivulet's z-order runs the other way, so the reversed
        // iteration makes the first JSON item the front-most one.
        for item in raw_source.settings.items.iter().rev() {
            let Some(&source_id) = source_ids.get(&item.name) else {
                if ids.contains_key(&item.name) {
                    // A nested scene/group is expressed through `parent`, not
                    // as a binding.
                    continue;
                }
                if skipped.contains(&item.name) {
                    // Already reported as unsupported; a second note would
                    // suggest the collection were broken rather than unsupported.
                    continue;
                }
                report.notes.push(ObsImportNote::DanglingItem {
                    scene: raw_source.name.clone(),
                    source: item.name.clone(),
                });
                continue;
            };
            let (transform, scale_known) = transform_for(item);
            if !scale_known {
                report.notes.push(ObsImportNote::ScaleNotConverted {
                    scene: raw_source.name.clone(),
                    source: item.name.clone(),
                });
            }
            sources.bind_source(source_id, scene_id, Some(transform));
            sources.set_crop(source_id, scene_id, crop_for(item));
            sources.set_visibility(source_id, scene_id, item.visible);
            sources.set_locked(source_id, scene_id, item.locked);
            report.bindings += 1;
        }
        if raw_source.id == OBS_GROUP {
            if let Some(children) = members.get(&raw_source.name) {
                for child in children.iter().rev() {
                    let Some(&source_id) = source_ids.get(child) else {
                        continue;
                    };
                    sources.bind_source(source_id, scene_id, None);
                    report.bindings += 1;
                }
            }
        }
    }

    // Active scene. `add` already activated the first scene; an unknown name
    // is reported rather than silently ignored.
    if let Some(name) = raw.current_scene.as_deref() {
        match ids.get(name) {
            Some(&id) => {
                scenes.switch_to(id);
            }
            None => report.notes.push(ObsImportNote::ActiveSceneUnknown {
                name: name.to_string(),
            }),
        }
    }

    if let Some(name) = raw.transition.clone() {
        report.notes.push(ObsImportNote::TransitionNotCarried {
            name,
            duration_ms: raw.transition_duration,
        });
    }

    ObsImport {
        collection,
        scenes,
        sources,
        report,
    }
}

/// The first payload setting a mapped source carries, if any.
fn payload_field(settings: &RawSettings) -> Option<&'static str> {
    if settings
        .url
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Some("url");
    }
    for (field, value) in [
        ("file", &settings.file),
        ("local_file", &settings.local_file),
        ("input_file", &settings.input_file),
        ("text", &settings.text),
    ] {
        if value.as_deref().is_some_and(|value| !value.is_empty()) {
            return Some(field);
        }
    }
    None
}

/// The capture device an OBS source names, in OBS's own vocabulary.
///
/// `monitor` is a monitor index; `window` is a window handle on Windows; the
/// rest are device ids. Empty means "let the user pick later", which is what
/// [`crate::Source::device_id`] documents.
fn device_id(settings: &RawSettings) -> Option<String> {
    for candidate in [
        &settings.device_id,
        &settings.video_device_id,
        &settings.device,
        &settings.window,
    ] {
        if let Some(value) = candidate.as_deref() {
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    settings.monitor.as_ref().map(|monitor| match monitor {
        RawMonitor::Index(index) => index.to_string(),
        RawMonitor::Id(id) => id.clone(),
    })
}

/// The per-item transform, and whether its size is known rather than assumed.
///
/// OBS stores `scale` relative to the source's native size, which the
/// collection does not contain. `bounds` (when `bounds_type` is set) is absolute
/// pixels, so it is the one size we can carry; otherwise the item keeps
/// Rivulet's default frame size and `false` tells the caller to report it.
fn transform_for(item: &RawItem) -> (Transform, bool) {
    let mut transform = Transform::default();
    let bounds_usable = item.bounds_type != 0 && item.bounds.x > 0.0 && item.bounds.y > 0.0;
    if bounds_usable {
        transform.width = item.bounds.x;
        transform.height = item.bounds.y;
    }
    transform.rotation = item.rot;
    // OBS positions an item by its alignment anchor; Rivulet's transform is
    // the top-left corner, so the anchor offset is subtracted. An anchor of 0
    // (top-left) leaves the position untouched.
    let (fx, fy) = alignment_factors(item.pos.alignment);
    transform.x = item.pos.x - fx * transform.width;
    transform.y = item.pos.y - fy * transform.height;
    (transform, bounds_usable)
}

/// Fraction of an item's width/height to subtract for an OBS alignment value.
///
/// OBS alignments run row-major from 0 (top-left) to 8 (bottom-right), so the
/// column is `alignment % 3` and the row `alignment / 3`, each worth half the
/// item's size. Values above 8 are treated as top-left, which is also what a
/// missing `alignment` means.
fn alignment_factors(alignment: u32) -> (f32, f32) {
    let alignment = alignment.min(8);
    let column = (alignment % 3) as f32 * 0.5;
    let row = (alignment / 3) as f32 * 0.5;
    (column, row)
}

fn crop_for(item: &RawItem) -> Crop {
    Crop::new(
        item.crop_left.max(0) as u32,
        item.crop_top.max(0) as u32,
        item.crop_right.max(0) as u32,
        item.crop_bottom.max(0) as u32,
    )
}
