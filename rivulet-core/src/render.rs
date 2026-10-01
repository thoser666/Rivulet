//! Headless deterministic rendering from code (M7 W3, issue #189).
//!
//! The GUI can export a scene snapshot and `rivulet record` can write a video,
//! but neither can be driven from a config file on a runner with no display,
//! no capture hardware and no fixed wall clock. W3 closes that gap: a scene
//! composition is described declaratively, rendered to PNG or to a video
//! without any user interaction, and — the point of the workstream — renders
//! the same bytes on every run.
//!
//! Determinism has three sources, and each one is a deliberate choice rather
//! than a happy accident:
//!
//! - **Pixels** come from [`SceneSnapshot`], a pure CPU compositor over an
//!   integer-exact frame buffer, plus [`TestVideoSource`] for animated source
//!   content. Nothing samples a wall clock or a hardware surface.
//! - **Timestamps** come from an injected [`VirtualClock`]. The engine stamps
//!   each buffer at `session base + clock.now_ns()` (see
//!   `RivuletEngine::process_raw_frame`), so frame *N* always lands on
//!   `N * frame_interval` — the same property M7 W2a's reproducible-run
//!   contract relies on.
//! - **Iteration count** is an explicit argument, never "until the file looks
//!   big enough".
//!
//! The composition model ([`SceneRenderConfig`]) is intentionally the same
//! one the editor already persists (`Scene` + `SourceManager`), so a rendered
//! frame is the same composition the user sees rather than a parallel
//! reimplementation that can drift from the editor.

use crate::clock::{frame_interval_ns, VirtualClock};
use crate::inspect::NondeterminismReport;
use crate::source::{Source, SourceKind, SourceManager, TestVideoSource};
use crate::test_helpers::GoldenFrame;
use crate::{
    RecordingContainer, RivuletEngine, Scene, SceneSnapshot, SnapshotFrame, Transform, VideoEncoder,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

/// Output canvas of a render job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanvasConfig {
    pub width: u32,
    pub height: u32,
}

impl Default for CanvasConfig {
    /// Matches `rivulet record`'s video default (640x360) rather than the
    /// editor canvas: a CI artifact that is 1920x1080 is 8 MB per PNG.
    fn default() -> Self {
        Self {
            width: 640,
            height: 360,
        }
    }
}

/// One layer of the composition to render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderLayerConfig {
    /// Display name; also the source name, so reports are readable.
    pub name: String,
    /// Which stable color tile to use, and which generator drives animated
    /// content.
    pub kind: SourceKind,
    /// Placement on the canvas.
    #[serde(default)]
    pub transform: Transform,
    /// Paint order; higher is on top. Ties break on layer index, so a config
    /// that leaves `z_order` at its default is still fully determined.
    #[serde(default)]
    pub z_order: i32,
    /// Defaults to visible; a hidden layer is excluded from the render rather
    /// than blended at zero opacity.
    #[serde(default = "default_true")]
    pub visible: bool,
    /// When set, the layer is fed animated per-frame content instead of a
    /// flat color tile. This is what makes consecutive frames differ, so a
    /// rendered video is a real motion test rather than N copies of one image.
    #[serde(default = "default_true")]
    pub animated: bool,
}

fn default_true() -> bool {
    true
}

/// A declarative scene composition that can be rendered headlessly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneRenderConfig {
    /// Scene name, echoed into reports and into the run's diagnostics.
    #[serde(default = "default_scene_name")]
    pub name: String,
    /// Collection/profile labels, carried through to the snapshot so a report
    /// can name what was rendered.
    #[serde(default)]
    pub collection: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub canvas: CanvasConfig,
    /// Frame rate. Drives both the virtual clock's cadence and the animated
    /// source content, so the two can never disagree.
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default)]
    pub layers: Vec<RenderLayerConfig>,
}

fn default_scene_name() -> String {
    "Render".to_string()
}

fn default_fps() -> u32 {
    30
}

/// A render configuration error, phrased for a CLI that must name the
/// offending key (exit code 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderConfigError(pub String);

impl fmt::Display for RenderConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RenderConfigError {}

impl SceneRenderConfig {
    /// Validate the configuration, naming the offending key on failure.
    ///
    /// This runs before any pipeline work so a bad config fails as a usage
    /// error rather than as an opaque GStreamer error 500 frames later.
    pub fn validate(&self) -> Result<(), RenderConfigError> {
        if self.canvas.width == 0 || self.canvas.height == 0 {
            return Err(RenderConfigError(format!(
                "canvas: width and height must be non-zero (got {}x{})",
                self.canvas.width, self.canvas.height
            )));
        }
        // The engine's RGBA caps path and x264 both require even dimensions;
        // rejecting here keeps the failure a usage error.
        if !self.canvas.width.is_multiple_of(2) || !self.canvas.height.is_multiple_of(2) {
            return Err(RenderConfigError(format!(
                "canvas: width and height must be even (got {}x{})",
                self.canvas.width, self.canvas.height
            )));
        }
        if self.fps == 0 {
            return Err(RenderConfigError("fps: must be non-zero".to_string()));
        }
        if self.layers.is_empty() {
            return Err(RenderConfigError(
                "layers: at least one layer is required".to_string(),
            ));
        }
        if self.layers.len() > MAX_LAYERS {
            return Err(RenderConfigError(format!(
                "layers: at most {MAX_LAYERS} layers are supported (got {})",
                self.layers.len()
            )));
        }
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.name.trim().is_empty() {
                return Err(RenderConfigError(format!(
                    "layers[{index}].name: must not be empty"
                )));
            }
            // Names identify layers in reports and pin them to a source id, so
            // a duplicate would make the binding ambiguous.
            let duplicates = self
                .layers
                .iter()
                .filter(|other| other.name == layer.name)
                .count();
            if duplicates > 1 {
                return Err(RenderConfigError(format!(
                    "layers[{index}].name: \"{}\" is used by more than one layer",
                    layer.name
                )));
            }
            let transform = &layer.transform;
            if transform.width < 0.0 || transform.height < 0.0 {
                return Err(RenderConfigError(format!(
                    "layers[{index}].transform: width and height must not be negative"
                )));
            }
            if !(0.0..=1.0).contains(&transform.opacity) {
                return Err(RenderConfigError(format!(
                    "layers[{index}].transform.opacity: must be within 0.0..=1.0 (got {})",
                    transform.opacity
                )));
            }
        }
        Ok(())
    }

    /// Build the editor scene model this config describes.
    ///
    /// Going through `Scene`/`SourceManager` rather than rendering the layers
    /// directly is the point: the snapshot compositor then sees exactly the
    /// model the editor persists, so a render cannot disagree with what the
    /// user configured.
    pub fn build_scene(&self) -> (Scene, SourceManager) {
        let (scene, sources, _) = self.build_scene_with_placements();
        (scene, sources)
    }

    /// Build the scene plus, for each configured layer, the source id it
    /// became and its stable motion index.
    ///
    /// `bind_source` assigns each new binding `max_z_order + 1`, so binding
    /// in paint order is what produces the configured z-order — sorting first
    /// avoids a delta-based reorder and keeps ties resolved by config index,
    /// which is deterministic.
    fn build_scene_with_placements(&self) -> (Scene, SourceManager, Vec<PlacedLayer<'_>>) {
        let scene = Scene::new(self.name.clone());
        let mut sources = SourceManager::new();
        let mut placed = Vec::with_capacity(self.layers.len());

        let mut ordered: Vec<(usize, &RenderLayerConfig)> =
            self.layers.iter().enumerate().collect();
        ordered.sort_by_key(|(index, layer)| (layer.z_order, *index));

        for (_, layer) in ordered {
            let mut source = Source::new(layer.name.clone(), layer.kind.clone());
            source.visible = layer.visible;
            source.z_order = layer.z_order;
            source.transform = layer.transform.clone();
            let id = sources.add_source(source);
            sources.bind_source(id, scene.id, Some(layer.transform.clone()));
            sources.set_visibility(id, scene.id, layer.visible);
            placed.push(PlacedLayer {
                source_id: id,
                layer,
                // A stable per-layer offset, so two animated layers do not
                // replay identical content and a moving backdrop does not
                // look frozen while the foreground moves.
                motion_index: placed.len() as u64,
            });
        }
        (scene, sources, placed)
    }

    /// Compose frame `frame_index` of this scene.
    ///
    /// Animated layers are fed [`TestVideoSource`] output generated for that
    /// exact frame index at the layer's own resolution, which is what makes
    /// frame *N* reproducible regardless of how many frames were rendered
    /// before it.
    pub fn snapshot(&self, frame_index: u64) -> SceneSnapshot {
        let (scene, sources, placed) = self.build_scene_with_placements();
        let mut snapshot =
            SceneSnapshot::from_scene(&scene, &sources, &self.collection, &self.profile)
                .with_size(self.canvas.width, self.canvas.height);

        // Pre-generate each animated layer's content once: the compositor asks per
        // layer, and regenerating inside the callback would redo the work for
        // every lookup.
        let generated: Vec<(uuid::Uuid, Option<SnapshotFrame>)> = placed
            .iter()
            .map(|entry| {
                if !entry.layer.animated || !entry.layer.visible {
                    return (entry.source_id, None);
                }
                // Clamped to at least 1px because `SnapshotFrame` rejects zero
                // geometry, and capped to the canvas so a wildly oversized
                // layer cannot allocate unbounded memory from a config file.
                let width = entry
                    .layer
                    .transform
                    .width
                    .max(1.0)
                    .min(self.canvas.width as f32) as u32;
                let height = entry
                    .layer
                    .transform
                    .height
                    .max(1.0)
                    .min(self.canvas.height as f32) as u32;
                let frame = TestVideoSource::frame_at(
                    frame_index + entry.motion_index,
                    width,
                    height,
                    self.fps,
                );
                (
                    entry.source_id,
                    SnapshotFrame::new(frame.width, frame.height, frame.rgba),
                )
            })
            .collect();

        snapshot.attach_frames(&|source_id| {
            generated
                .iter()
                .find(|(id, _)| *id == source_id)
                .and_then(|(_, frame)| frame.clone())
        });
        snapshot
    }

    /// Render frame `frame_index` to a comparable frame buffer.
    pub fn render_frame(&self, frame_index: u64) -> GoldenFrame {
        let snapshot = self.snapshot(frame_index);
        GoldenFrame::new(
            self.canvas.width,
            self.canvas.height,
            snapshot.render_rgba(),
        )
    }

    /// Render frame `frame_index` and write it as a PNG.
    ///
    /// Byte-identical for the same `frame_index`: the encoder is the
    /// deterministic one from [`GoldenFrame`], so two renders of the same
    /// frame produce identical files and `diff` decides.
    pub fn write_frame_png(&self, frame_index: u64, path: &Path) -> Result<PathBuf, RenderError> {
        let frame = self.render_frame(frame_index);
        let bytes = frame.to_png_bytes().ok_or_else(|| {
            RenderError::Io(format!(
                "encoding frame {frame_index} as PNG failed ({}x{})",
                frame.width, frame.height
            ))
        })?;
        write_file(path, &bytes)?;
        Ok(path.to_path_buf())
    }

    /// The source kind that decides this run's reproducibility claim.
    ///
    /// The renderer itself substitutes [`TestVideoSource`] for every layer, so
    /// pixel output is deterministic regardless of the declared `kind`. But the
    /// declared kind still documents intent, and a config naming a *live* kind
    /// (webcam, game capture, …) must not be reported as byte-reproducible:
    /// that promise only holds once real capture is wired in. So a live kind
    /// wins over a recorded one, and the first match in config order breaks the
    /// tie, keeping the choice stable for a given config.
    pub fn dominant_source_kind(&self) -> SourceKind {
        if let Some(layer) = self.layers.iter().find(|layer| layer.kind.is_live()) {
            return layer.kind.clone();
        }
        self.layers
            .first()
            .map(|layer| layer.kind.clone())
            // An empty layer list is rejected by `validate`, but a report must
            // never be built from an unvalidated config.
            .unwrap_or(SourceKind::Color)
    }
}

/// One configured layer after it became a source: which id it got, and its
/// stable motion index (see [`SceneRenderConfig::snapshot`]).
struct PlacedLayer<'a> {
    source_id: uuid::Uuid,
    layer: &'a RenderLayerConfig,
    motion_index: u64,
}

/// Upper bound on layer count.
///
/// Not an aesthetic choice: the compositor is O(canvas x layers) per pixel, so
/// an unbounded list is a way for a config file to hang a CI runner. A real
/// composition is a handful of layers.
pub const MAX_LAYERS: usize = 64;

/// How a render job encodes its video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderVideoTarget {
    Mp4,
    Mkv,
}

impl RenderVideoTarget {
    /// Resolve the engine container for this target.
    pub fn container(self) -> RecordingContainer {
        match self {
            RenderVideoTarget::Mp4 => RecordingContainer::Mp4,
            RenderVideoTarget::Mkv => RecordingContainer::Mkv,
        }
    }
}

/// What a completed video render produced.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderVideoReport {
    /// Number of frames pushed through the encoder.
    pub frames: u32,
    /// Container duration in seconds, derived from the frame count and fps
    /// rather than measured from a wall clock.
    pub duration_secs: f64,
    pub fps: u32,
    pub width: u32,
    pub height: u32,
    /// Where the container was written.
    pub output: PathBuf,
    /// Byte size of the written file.
    pub file_size_bytes: u64,
    /// True when the run was stamped by the virtual clock. Always true here;
    /// the field exists so a report states the guarantee instead of implying
    /// it.
    pub virtual_clock: bool,
    /// The nondeterminism inventory of this run, using the same report the
    /// recording path emits (M7 W2a) so both surfaces agree.
    pub nondeterminism: NondeterminismReport,
}

/// A render failure, kept separate from [`RenderConfigError`] because the two
/// map to different exit codes (usage vs. runtime).
#[derive(Debug)]
pub enum RenderError {
    /// The configuration is invalid.
    Config(RenderConfigError),
    /// The encoder pipeline refused to run.
    Engine(String),
    /// Writing the output failed.
    Io(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::Config(error) => write!(formatter, "{error}"),
            RenderError::Engine(message) => write!(formatter, "engine error: {message}"),
            RenderError::Io(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<RenderConfigError> for RenderError {
    fn from(error: RenderConfigError) -> Self {
        RenderError::Config(error)
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), RenderError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RenderError::Io(format!(
                    "creating output directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
    }
    std::fs::write(path, bytes)
        .map_err(|error| RenderError::Io(format!("writing {}: {error}", path.display())))
}

/// Render `frame_count` frames of a scene into a container file.
///
/// The whole run is driven by an injected [`VirtualClock`]: the clock is
/// stepped exactly one frame interval before each buffer is pushed, so the
/// encoder receives PTS `0, interval, 2*interval, ...` regardless of how fast
/// the machine actually renders. That is the difference between "CI produced a
/// video" and "CI produced *the* video".
pub fn render_video(
    config: &SceneRenderConfig,
    output: &Path,
    frame_count: u32,
    target: RenderVideoTarget,
    encoder: VideoEncoder,
) -> Result<RenderVideoReport, RenderError> {
    config.validate()?;
    if frame_count == 0 {
        return Err(RenderError::Config(RenderConfigError(
            "frames: must be non-zero".to_string(),
        )));
    }

    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| {
                RenderError::Io(format!(
                    "creating output directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
    }

    let virtual_clock = VirtualClock::new();
    let mut engine = RivuletEngine::new();
    engine.set_video_encoder(encoder);
    engine.set_recording_container(target.container());
    // Must precede `start_local_recording`: the engine rejects a clock swap
    // once a session is running.
    engine.set_clock(virtual_clock.clone());
    engine.start_local_recording(output.to_path_buf());

    for index in 0..u64::from(frame_count) {
        if let Some(error) = engine.take_error() {
            return Err(RenderError::Engine(error));
        }
        let frame = config.render_frame(index);
        // Step *before* the push so frame 0 is stamped at the session base
        // rather than one interval in.
        virtual_clock.step_frames(1, (config.fps, 1));
        engine.process_raw_frame(&frame.rgba, frame.width, frame.height);
    }

    engine.stop_recording();
    if let Some(error) = engine.take_error() {
        return Err(RenderError::Engine(error));
    }

    let file_size_bytes = std::fs::metadata(output)
        .map(|meta| meta.len())
        .unwrap_or(0);
    let report = RenderVideoReport {
        frames: frame_count,
        duration_secs: f64::from(frame_count) / f64::from(config.fps),
        fps: config.fps,
        width: config.canvas.width,
        height: config.canvas.height,
        output: output.to_path_buf(),
        file_size_bytes,
        virtual_clock: true,
        nondeterminism: NondeterminismReport::for_run(
            engine.clock_mode(),
            engine.video_encoder(),
            config.dominant_source_kind(),
        ),
    };

    if report.file_size_bytes == 0 {
        return Err(RenderError::Io(format!(
            "render produced no output at {} (pipeline finalized an empty file)",
            output.display()
        )));
    }
    Ok(report)
}

/// The nominal PTS of frame `index` at `fps`, in nanoseconds.
///
/// Exposed so a caller can assert a rendered run's cadence without reaching
/// into the clock: this is the sequence
/// [`render_video`] is contracted to produce.
pub fn frame_pts_ns(index: u64, fps: u32) -> u64 {
    index.saturating_mul(frame_interval_ns((fps, 1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_config() -> SceneRenderConfig {
        SceneRenderConfig {
            name: "Demo".to_string(),
            collection: "Tests".to_string(),
            profile: "Default".to_string(),
            canvas: CanvasConfig {
                width: 64,
                height: 48,
            },
            fps: 30,
            layers: vec![
                RenderLayerConfig {
                    name: "Backdrop".to_string(),
                    kind: SourceKind::Color,
                    transform: Transform::new(0.0, 0.0, 64.0, 48.0),
                    z_order: 0,
                    visible: true,
                    animated: false,
                },
                RenderLayerConfig {
                    name: "Motion".to_string(),
                    kind: SourceKind::GameCapture,
                    transform: Transform::new(8.0, 8.0, 32.0, 24.0),
                    z_order: 1,
                    visible: true,
                    animated: true,
                },
            ],
        }
    }

    #[test]
    fn a_valid_config_builds_and_renders() {
        let config = demo_config();
        config.validate().expect("config is valid");
        let frame = config.render_frame(0);
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert_eq!(frame.rgba.len(), 64 * 48 * 4);
    }

    #[test]
    fn rendering_the_same_frame_twice_is_byte_identical() {
        // The core determinism guarantee of W3: same frame index, same bytes,
        // with no wall clock involved anywhere.
        let config = demo_config();
        let first = config.render_frame(7);
        let second = config.render_frame(7);
        first.assert_matches(&second, 7);
        assert_eq!(first.rgba, second.rgba);
        assert_eq!(
            first.to_png_bytes(),
            second.to_png_bytes(),
            "PNG encoding must be deterministic too, not just the pixels"
        );
    }

    #[test]
    fn a_live_layer_kind_downgrades_the_reproducibility_claim() {
        // `demo_config` has a GameCapture layer. The renderer substitutes a
        // synthetic source for it, so today's bytes happen to be stable — but
        // reporting the run as byte-reproducible would be a promise the render
        // only keeps by accident. The claim follows the declared kind.
        let config = demo_config();
        assert_eq!(
            config.dominant_source_kind(),
            SourceKind::GameCapture,
            "a live kind must win over a recorded one"
        );

        let mut recorded = demo_config();
        recorded.layers[1].kind = SourceKind::Image;
        assert_eq!(
            recorded.dominant_source_kind(),
            SourceKind::Color,
            "with no live layer left, the first declared kind decides"
        );
    }

    #[test]
    fn animated_layers_make_consecutive_frames_differ() {
        let config = demo_config();
        let frame_n = config.render_frame(3);
        let frame_n_plus_one = config.render_frame(4);
        let difference = frame_n_plus_one
            .diff(&frame_n)
            .expect("an animated layer must change between frames");
        assert!(
            difference.differing_pixels > 0,
            "got: {}",
            difference.describe()
        );
    }

    #[test]
    fn a_static_composition_is_frame_invariant() {
        // The converse check: without animation, every frame must be
        // identical, otherwise "same frame twice" would be vacuous.
        let mut config = demo_config();
        for layer in &mut config.layers {
            layer.animated = false;
        }
        let first = config.render_frame(0);
        let second = config.render_frame(9);
        first.assert_matches(&second, 0);
    }

    #[test]
    fn hidden_layers_do_not_contribute() {
        let mut config = demo_config();
        config.layers[1].visible = false;
        let without_hidden = config.render_frame(2);

        let mut only_backdrop = config.clone();
        only_backdrop.layers.truncate(1);
        let backdrop_only = only_backdrop.render_frame(2);

        without_hidden.assert_matches(&backdrop_only, 2);
    }

    #[test]
    fn validation_names_the_offending_key() {
        let mut config = demo_config();
        config.canvas.width = 0;
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "canvas: width and height must be non-zero (got 0x48)"
        );

        let mut config = demo_config();
        config.canvas.width = 63;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .starts_with("canvas: width and height must be even"));

        let mut config = demo_config();
        config.layers.clear();
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "layers: at least one layer is required"
        );

        let mut config = demo_config();
        config.layers.push(RenderLayerConfig {
            name: "  ".to_string(),
            kind: SourceKind::Color,
            transform: Transform::default(),
            z_order: 0,
            visible: true,
            animated: true,
        });
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "layers[2].name: must not be empty"
        );

        let mut config = demo_config();
        config.layers[1].name = config.layers[0].name.clone();
        // The conflict is reported at the first layer carrying the name, so a
        // report names one offending key rather than every participant.
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "layers[0].name: \"Backdrop\" is used by more than one layer"
        );

        let mut config = demo_config();
        config.fps = 0;
        assert_eq!(
            config.validate().unwrap_err().to_string(),
            "fps: must be non-zero"
        );

        let mut config = demo_config();
        config.layers = vec![config.layers[0].clone(), config.layers[0].clone()];
        config.layers[1].name = "Second".to_string();
        config.layers[0].transform.opacity = 4.0;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("layers[0].transform.opacity"));
    }

    #[test]
    fn opacity_outside_the_unit_range_is_rejected() {
        let mut config = demo_config();
        config.layers[0].transform.opacity = 1.5;
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("layers[0].transform.opacity"));
    }

    #[test]
    fn more_layers_than_the_cap_is_rejected() {
        let mut config = demo_config();
        config.layers = (0..=MAX_LAYERS)
            .map(|index| RenderLayerConfig {
                name: format!("Layer{index}"),
                kind: SourceKind::Color,
                transform: Transform::default(),
                z_order: 0,
                visible: true,
                animated: false,
            })
            .collect();
        assert!(config
            .validate()
            .unwrap_err()
            .to_string()
            .contains(&format!("at most {MAX_LAYERS} layers")));
    }

    #[test]
    fn the_config_round_trips_through_serde() {
        let config = demo_config();
        let json = serde_json::to_string(&config).expect("serialize");
        let parsed: SceneRenderConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, config);
        assert_eq!(parsed.render_frame(1).rgba, config.render_frame(1).rgba);
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_ignored() {
        // A typo in a CI config must fail loudly; silently rendering the
        // default is the worse outcome.
        let error =
            serde_json::from_str::<SceneRenderConfig>(r#"{"name":"X","fps":30,"layerz":[]}"#)
                .expect_err("unknown key must be rejected");
        assert!(error.to_string().contains("layerz"), "got {error}");
    }

    #[test]
    fn frame_pts_ns_follows_the_contract_sequence() {
        let interval = frame_interval_ns((30, 1));
        assert_eq!(frame_pts_ns(0, 30), 0);
        assert_eq!(frame_pts_ns(1, 30), interval);
        assert_eq!(frame_pts_ns(10, 30), 10 * interval);
    }
}
