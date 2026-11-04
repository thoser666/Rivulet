//! Live scene composition for the recording and streaming output path.
//!
//! [`SceneSnapshot`](crate::SceneSnapshot) already turns a scene layout into
//! RGBA pixels; this module decides **which live picture fills which layer** and
//! keeps the result on one output canvas, so the encoder receives a stable
//! resolution while the scene dock decides what is on air.
//!
//! The GUI calls [`compose_scene`] once per captured frame, before the frame
//! reaches the recording/stream engine (both outputs share that pipeline).

use crate::{Scene, SceneSnapshot, SnapshotFrame, SourceKind, SourceManager};
use uuid::Uuid;

/// Output canvas of a composed live frame.
///
/// This is the scene editor's coordinate space, so a layer transform means the
/// same thing on screen and in the encoded picture. Recording and streaming
/// therefore run at 1920x1080 while composition is enabled, whatever the
/// capture source's native size is; turning composition off restores the raw
/// capture resolution.
pub const LIVE_CANVAS_WIDTH: u32 = 1920;
pub const LIVE_CANVAS_HEIGHT: u32 = 1080;

/// One composed RGBA8 frame, ready for `process_raw_frame`.
#[derive(Debug, Clone, PartialEq)]
pub struct ComposedFrame {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8 pixels, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Whether `kind` is filled from the live capture picture.
///
/// Screen, game and webcam layers all show the picture the capture pipeline
/// just produced: they are the only live surface the engine has, so whichever
/// capture is running feeds them instead of leaving them as a layout tile.
pub fn accepts_live_capture(kind: &SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::ScreenCapture | SourceKind::GameCapture | SourceKind::Webcam
    )
}

/// Whether `kind` can carry a real pixel frame in the live path at all.
///
/// Image, text, media, colour and audio layers keep their stable layout tile
/// until their platform renderers exist
/// ([#217](https://github.com/thoser666/Rivulet/issues/217)); returning `false`
/// here is what keeps them from being silently blanked.
pub fn accepts_live_frame(kind: &SourceKind) -> bool {
    accepts_live_capture(kind) || matches!(kind, SourceKind::Browser)
}

/// Compose `scene` into one encoded-frame candidate.
///
/// `capture` is the picture the capture pipeline just produced, `browser` the
/// newest browser-source frame. Frames are attached to the visible layers whose
/// kind accepts them; layers without a frame keep their layout tile.
///
/// Returns `None` when the scene has no visible layer at all. The caller then
/// keeps the raw capture frame, so a misconfigured scene can never black out a
/// recording or a live stream.
pub fn compose_scene(
    scene: &Scene,
    sources: &SourceManager,
    collection: &str,
    profile: &str,
    capture: Option<&SnapshotFrame>,
    browser: Option<&SnapshotFrame>,
) -> Option<ComposedFrame> {
    let mut snapshot = SceneSnapshot::from_scene(scene, sources, collection, profile)
        .with_size(LIVE_CANVAS_WIDTH, LIVE_CANVAS_HEIGHT);
    if snapshot.layers.is_empty() {
        return None;
    }
    snapshot.attach_frames(&|source_id: Uuid| {
        let source = sources.get_source(source_id)?;
        match &source.kind {
            SourceKind::Browser => browser.cloned(),
            kind if accepts_live_capture(kind) => capture.cloned(),
            _ => None,
        }
    });
    Some(ComposedFrame {
        width: snapshot.width,
        height: snapshot.height,
        rgba: snapshot.render_rgba(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Source, Transform};

    fn scene_with(kind: SourceKind) -> (Scene, SourceManager, Uuid) {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Source".to_string(), kind));
        sources.bind_source(id, scene.id, None);
        (scene, sources, id)
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> SnapshotFrame {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            pixels.extend_from_slice(&rgba);
        }
        SnapshotFrame::new(width, height, pixels).expect("valid frame")
    }

    #[test]
    fn composes_the_capture_picture_into_the_output_canvas() {
        let (scene, sources, _) = scene_with(SourceKind::ScreenCapture);
        let capture = solid(2, 2, [200, 10, 10, 255]);
        let composed = compose_scene(&scene, &sources, "Default", "Default", Some(&capture), None)
            .expect("scene has a layer");
        assert_eq!(composed.width, LIVE_CANVAS_WIDTH);
        assert_eq!(composed.height, LIVE_CANVAS_HEIGHT);
        assert_eq!(
            composed.rgba.len(),
            (LIVE_CANVAS_WIDTH * LIVE_CANVAS_HEIGHT * 4) as usize
        );
        assert_eq!(&composed.rgba[..4], &[200, 10, 10, 255]);
        // The default transform covers the whole canvas, so the last pixel is
        // the capture colour too.
        let last = composed.rgba.len() - 4;
        assert_eq!(&composed.rgba[last..], &[200, 10, 10, 255]);
    }

    #[test]
    fn every_capture_kind_is_filled_by_the_live_picture() {
        for kind in [
            SourceKind::ScreenCapture,
            SourceKind::GameCapture,
            SourceKind::Webcam,
        ] {
            assert!(accepts_live_capture(&kind), "{kind:?} must accept capture");
        }
        for kind in [
            SourceKind::Image,
            SourceKind::Text,
            SourceKind::Media,
            SourceKind::Color,
            SourceKind::Audio,
        ] {
            assert!(!accepts_live_capture(&kind), "{kind:?} has no live picture");
            assert!(!accepts_live_frame(&kind));
        }
        assert!(accepts_live_frame(&SourceKind::Browser));
    }

    #[test]
    fn scene_without_visible_layers_returns_none() {
        let scene = Scene::new("Empty".to_string());
        let sources = SourceManager::new();
        let capture = solid(1, 1, [1, 2, 3, 255]);
        assert!(
            compose_scene(&scene, &sources, "Default", "Default", Some(&capture), None).is_none()
        );
    }

    #[test]
    fn hidden_layer_is_not_composed() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Hidden".to_string(), SourceKind::ScreenCapture));
        sources.bind_source(id, scene.id, None);
        sources.set_visibility(id, scene.id, false);
        let capture = solid(1, 1, [9, 9, 9, 255]);
        assert!(
            compose_scene(&scene, &sources, "Default", "Default", Some(&capture), None).is_none()
        );
    }

    #[test]
    fn unsupported_kind_keeps_its_layout_tile() {
        let (scene, sources, _) = scene_with(SourceKind::Text);
        let capture = solid(1, 1, [200, 10, 10, 255]);
        let composed = compose_scene(&scene, &sources, "Default", "Default", Some(&capture), None)
            .expect("scene has a layer");
        // Text keeps its stable tile colour instead of borrowing the capture.
        assert_eq!(&composed.rgba[..4], &[46, 204, 113, 255]);
    }

    #[test]
    fn browser_layer_uses_the_browser_frame_not_the_capture() {
        let (scene, sources, _) = scene_with(SourceKind::Browser);
        let capture = solid(1, 1, [200, 10, 10, 255]);
        let browser = solid(1, 1, [7, 8, 9, 255]);
        let composed = compose_scene(
            &scene,
            &sources,
            "Default",
            "Default",
            Some(&capture),
            Some(&browser),
        )
        .expect("scene has a layer");
        assert_eq!(&composed.rgba[..4], &[7, 8, 9, 255]);
    }

    #[test]
    fn capture_layer_transform_places_the_picture() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Game".to_string(), SourceKind::GameCapture));
        sources.bind_source(id, scene.id, Some(Transform::new(0.0, 0.0, 2.0, 2.0)));
        let capture = solid(1, 1, [50, 60, 70, 255]);
        let composed = compose_scene(&scene, &sources, "Default", "Default", Some(&capture), None)
            .expect("scene has a layer");
        assert_eq!(&composed.rgba[..4], &[50, 60, 70, 255]);
        // Pixel (2, 2) is outside the 2x2 rect, so it keeps the canvas
        // background instead of the capture colour.
        let index = ((2 * LIVE_CANVAS_WIDTH + 2) * 4) as usize;
        assert_eq!(&composed.rgba[index..index + 4], &[18, 22, 29, 255]);
    }

    #[test]
    fn missing_capture_keeps_capture_layers_as_tiles() {
        let (scene, sources, _) = scene_with(SourceKind::ScreenCapture);
        let composed = compose_scene(&scene, &sources, "Default", "Default", None, None)
            .expect("scene has a layer");
        assert_eq!(&composed.rgba[..4], &[149, 165, 166, 255]);
    }
}
