use crate::{Crop, Scene, SourceKind, SourceManager, Transform};
use uuid::Uuid;

/// Virtual canvas used by the scene editor and snapshot export.
pub const DEFAULT_CANVAS_WIDTH: u32 = 1920;
pub const DEFAULT_CANVAS_HEIGHT: u32 = 1080;

/// One RGBA8 pixel buffer produced by a native source renderer.
///
/// Frames are attached to [`SnapshotLayer`]s so the compositor can blend real
/// source content instead of a synthetic color tile.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl SnapshotFrame {
    /// Bound-checked constructor for an RGBA8 frame.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        if rgba.len() != (width as usize) * (height as usize) * 4 {
            return None;
        }
        Some(Self {
            width,
            height,
            rgba,
        })
    }
}

/// A visible source layer captured in a scene snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotLayer {
    pub source_id: Uuid,
    pub name: String,
    pub kind: SourceKind,
    pub transform: Transform,
    pub crop: Crop,
    pub z_order: i32,
    /// Native source pixels supplied by a renderer, if available.
    pub frame: Option<SnapshotFrame>,
}

/// A deterministic representation of the current scene composition.
///
/// Layers carry either native source [`SnapshotFrame`]s (supplied by a
/// platform renderer via [`SceneSnapshot::attach_frames`]) or a stable color
/// tile. The compositor blends both kinds in z-order while honoring
/// transforms, rotation, crops, and opacity, so export stays deterministic
/// until every source kind has a native renderer.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSnapshot {
    pub scene_name: String,
    pub collection: String,
    pub profile: String,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<SnapshotLayer>,
}

impl SceneSnapshot {
    /// Build a snapshot from one scene and its scene-local source bindings.
    pub fn from_scene(
        scene: &Scene,
        sources: &SourceManager,
        collection: impl Into<String>,
        profile: impl Into<String>,
    ) -> Self {
        let mut layers: Vec<_> = sources
            .scene_sources(scene.id)
            .into_iter()
            .filter_map(|binding| {
                let source = sources.get_source(binding.source_id)?;
                if !binding.visible || !source.visible {
                    return None;
                }
                Some(SnapshotLayer {
                    source_id: binding.source_id,
                    name: source.name.clone(),
                    kind: source.kind.clone(),
                    transform: binding.effective_transform(&source.transform).clone(),
                    crop: binding.crop,
                    z_order: binding.z_order,
                    frame: None,
                })
            })
            .collect();
        layers.sort_by_key(|layer| layer.z_order);

        Self {
            scene_name: scene.name.clone(),
            collection: collection.into(),
            profile: profile.into(),
            width: DEFAULT_CANVAS_WIDTH,
            height: DEFAULT_CANVAS_HEIGHT,
            layers,
        }
    }

    /// Attach a native frame to the layer for the given source, if present.
    ///
    /// The frame is validated against the layer's crop: `crop` dims reference
    /// source pixels, so an out-of-bounds crop yields a frame that samples
    /// only the visible region during rendering.
    pub fn attach_frames(&mut self, frame_of: &dyn Fn(Uuid) -> Option<SnapshotFrame>) {
        for layer in &mut self.layers {
            let Some(mut frame) = frame_of(layer.source_id) else {
                continue;
            };
            clamp_frame_to_crop(&mut frame, layer.crop);
            if frame.width == 0 || frame.height == 0 {
                continue;
            }
            layer.frame = Some(frame);
        }
    }

    /// Render the composition into an RGBA8 image buffer.
    ///
    /// Layers with an attached [`SnapshotFrame`] are rasterized as real source
    /// content (scaled and rotated into their transform rect, cropped, then
    /// alpha-blended); layers without a frame fall back to a stable source-kind
    /// color so the layout stays legible.
    pub fn render_rgba(&self) -> Vec<u8> {
        let pixel_count = self.width as usize * self.height as usize;
        let mut pixels = vec![0u8; pixel_count * 4];
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[18, 22, 29, 255]);
        }

        for layer in &self.layers {
            match &layer.frame {
                Some(frame) => {
                    rasterize_frame(self.width, self.height, &mut pixels, layer, frame);
                }
                None => {
                    let color = layer_color(&layer.kind);
                    let alpha = layer.transform.opacity.clamp(0.0, 1.0);
                    let x0 = layer.transform.x.max(0.0) as u32;
                    let y0 = layer.transform.y.max(0.0) as u32;
                    let x1 = (layer.transform.x + layer.transform.width)
                        .max(0.0)
                        .min(self.width as f32) as u32;
                    let y1 = (layer.transform.y + layer.transform.height)
                        .max(0.0)
                        .min(self.height as f32) as u32;
                    if x0 >= x1 || y0 >= y1 {
                        continue;
                    }
                    for y in y0..y1 {
                        for x in x0..x1 {
                            let offset = ((y * self.width + x) * 4) as usize;
                            blend_pixel(&mut pixels[offset..offset + 4], color, alpha);
                        }
                    }
                }
            }
        }
        pixels
    }
}

/// Clamp a source frame to the layer crop so the compositor only samples the
/// visible region. `crop` dimensions reference source pixels.
fn clamp_frame_to_crop(frame: &mut SnapshotFrame, crop: Crop) {
    let left = crop.left.min(frame.width);
    let right = crop.right.min(frame.width.saturating_sub(left));
    let top = crop.top.min(frame.height);
    let bottom = crop.bottom.min(frame.height.saturating_sub(top));
    if frame.width == left + right || frame.height == top + bottom {
        frame.width = 0;
        frame.height = 0;
        frame.rgba.clear();
        return;
    }
    let out_width = frame.width - left - right;
    let out_height = frame.height - top - bottom;
    let mut cropped = vec![0u8; (out_width * out_height * 4) as usize];
    for y in 0..out_height {
        for x in 0..out_width {
            let src = ((y + top) * frame.width + (x + left)) as usize * 4;
            let dst = (y * out_width + x) as usize * 4;
            cropped[dst..dst + 4].copy_from_slice(&frame.rgba[src..src + 4]);
        }
    }
    frame.width = out_width;
    frame.height = out_height;
    frame.rgba = cropped;
}

/// Rasterize a cropped source frame into its transform rect, honoring scale,
/// rotation, and opacity via nearest-neighbor sampling.
fn rasterize_frame(
    canvas_width: u32,
    canvas_height: u32,
    pixels: &mut [u8],
    layer: &SnapshotLayer,
    frame: &SnapshotFrame,
) {
    let t = &layer.transform;
    let alpha = t.opacity.clamp(0.0, 1.0);
    let center_x = t.x + t.width / 2.0;
    let center_y = t.y + t.height / 2.0;
    let rotation_rad = t.rotation.to_radians();
    let (sin, cos) = rotation_rad.sin_cos();

    let x0 = t.x.max(0.0) as u32;
    let y0 = t.y.max(0.0) as u32;
    let x1 = (t.x + t.width).max(0.0).min(canvas_width as f32) as u32;
    let y1 = (t.y + t.height).max(0.0).min(canvas_height as f32) as u32;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let scale_x = t.width.max(0.0001) / frame.width as f32;
    let scale_y = t.height.max(0.0001) / frame.height as f32;
    let frame_center_x = frame.width as f32 / 2.0;
    let frame_center_y = frame.height as f32 / 2.0;

    for y in y0..y1 {
        for x in x0..x1 {
            let d_x = x as f32 - center_x;
            let d_y = y as f32 - center_y;
            // Inverse rotation into the source's (unrotated) space.
            let s_x = d_x * cos + d_y * sin;
            let s_y = -d_x * sin + d_y * cos;
            let src_x = s_x / scale_x + frame_center_x;
            let src_y = s_y / scale_y + frame_center_y;
            let sx = src_x.floor() as i64;
            let sy = src_y.floor() as i64;
            if sx < 0 || sy < 0 || sx >= frame.width as i64 || sy >= frame.height as i64 {
                continue;
            }
            let offset = (sy * frame.width as i64 + sx) as usize * 4;
            let source_pixel: [u8; 4] = frame.rgba[offset..offset + 4]
                .try_into()
                .unwrap_or([0, 0, 0, 0]);
            let source_alpha = source_pixel[3] as f32 / 255.0 * alpha;
            blend_rgba(
                &mut pixels[(y as usize * canvas_width as usize + x as usize) * 4..][..4],
                &source_pixel,
                source_alpha,
            );
        }
    }
}

fn layer_color(kind: &SourceKind) -> [u8; 3] {
    match kind {
        SourceKind::Image => [52, 152, 219],
        SourceKind::Text => [46, 204, 113],
        SourceKind::Webcam => [155, 89, 182],
        SourceKind::Browser => [26, 188, 156],
        SourceKind::Media => [241, 196, 15],
        SourceKind::Color => [230, 126, 34],
        SourceKind::GameCapture => [231, 76, 60],
        SourceKind::ScreenCapture => [149, 165, 166],
        SourceKind::Audio => [52, 73, 94],
    }
}

fn blend_pixel(pixel: &mut [u8], color: [u8; 3], alpha: f32) {
    let source_alpha = alpha.clamp(0.0, 1.0);
    for (channel, source) in pixel[..3].iter_mut().zip(color) {
        *channel = ((*channel as f32 * (1.0 - source_alpha)) + (source as f32 * source_alpha))
            .round() as u8;
    }
}

/// Blend a full RGBA source pixel over the destination (straight alpha).
fn blend_rgba(pixel: &mut [u8], source: &[u8; 4], source_alpha: f32) {
    let source_alpha = source_alpha.clamp(0.0, 1.0);
    let dest_alpha = pixel[3] as f32 / 255.0;
    let out_alpha = source_alpha + dest_alpha * (1.0 - source_alpha);
    if out_alpha <= 0.0 {
        return;
    }
    for channel in 0..3 {
        let s = source[channel] as f32 * source_alpha
            + pixel[channel] as f32 * dest_alpha * (1.0 - source_alpha);
        pixel[channel] = (s / out_alpha).round().clamp(0.0, 255.0) as u8;
    }
    pixel[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Source;

    #[test]
    fn snapshot_includes_visible_layers_in_z_order() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let back = sources.add_source(Source::new("Background".to_string(), SourceKind::Color));
        let front = sources.add_source(Source::new("Game".to_string(), SourceKind::GameCapture));
        sources.bind_source(back, scene.id, None);
        sources.bind_source(
            front,
            scene.id,
            Some(Transform::new(10.0, 20.0, 100.0, 80.0)),
        );

        let snapshot = SceneSnapshot::from_scene(&scene, &sources, "Gaming", "Default");
        assert_eq!(snapshot.scene_name, "Live");
        assert_eq!(snapshot.collection, "Gaming");
        assert_eq!(snapshot.layers.len(), 2);
        assert!(snapshot.layers[0].z_order < snapshot.layers[1].z_order);
        assert_eq!(snapshot.layers[1].name, "Game");
    }

    #[test]
    fn snapshot_omits_hidden_source_and_binding() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let hidden_source =
            sources.add_source(Source::new("Hidden".to_string(), SourceKind::Text).visible(false));
        let hidden_binding =
            sources.add_source(Source::new("Hidden binding".to_string(), SourceKind::Image));
        sources.bind_source(hidden_source, scene.id, None);
        sources.bind_source(hidden_binding, scene.id, None);
        sources.set_visibility(hidden_binding, scene.id, false);

        let snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        assert!(snapshot.layers.is_empty());
    }

    #[test]
    fn snapshot_render_has_stable_size_and_layer_pixels() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Game".to_string(), SourceKind::GameCapture));
        sources.bind_source(id, scene.id, Some(Transform::new(0.0, 0.0, 2.0, 2.0)));
        let snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        let pixels = snapshot.render_rgba();
        assert_eq!(
            pixels.len(),
            (DEFAULT_CANVAS_WIDTH * DEFAULT_CANVAS_HEIGHT * 4) as usize
        );
        assert_eq!(&pixels[..4], &[231, 76, 60, 255]);
        assert_eq!(
            &pixels[((DEFAULT_CANVAS_WIDTH + 1) * 4) as usize
                ..((DEFAULT_CANVAS_WIDTH + 2) * 4) as usize],
            &[231, 76, 60, 255]
        );
    }

    #[test]
    fn snapshot_opacity_blends_with_background() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Game".to_string(), SourceKind::GameCapture));
        let mut transform = Transform::new(0.0, 0.0, 1.0, 1.0);
        transform.opacity = 0.5;
        sources.bind_source(id, scene.id, Some(transform));
        let snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        assert_eq!(&snapshot.render_rgba()[..4], &[125, 49, 45, 255]);
    }

    #[test]
    fn attach_frames_replaces_tile_with_native_pixels() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Cam".to_string(), SourceKind::Webcam));
        sources.bind_source(id, scene.id, Some(Transform::new(0.0, 0.0, 1.0, 1.0)));
        let mut snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        let frame = SnapshotFrame::new(1, 1, vec![200, 30, 10, 255]).unwrap();
        snapshot.attach_frames(&|_| Some(frame.clone()));
        assert_eq!(&snapshot.render_rgba()[..4], &[200, 30, 10, 255]);
    }

    #[test]
    fn attach_frames_scales_source_into_transform_rect() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Cam".to_string(), SourceKind::Webcam));
        sources.bind_source(id, scene.id, Some(Transform::new(0.0, 0.0, 2.0, 2.0)));
        let mut snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        // 2x2 image: top-left red, others green.
        let frame = SnapshotFrame::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, //
                0, 255, 0, 255, 0, 255, 0, 255,
            ],
        )
        .unwrap();
        snapshot.attach_frames(&|_| Some(frame.clone()));
        let pixels = snapshot.render_rgba();
        assert_eq!(&pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&pixels[4..8], &[0, 255, 0, 255]);
    }

    #[test]
    fn attach_frames_respects_layer_crop() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Cam".to_string(), SourceKind::Webcam));
        sources.bind_source(id, scene.id, Some(Transform::new(0.0, 0.0, 1.0, 1.0)));
        // Left column of the 2x2 frame is cropped away, leaving the right
        // column (green) visible.
        sources.set_crop(id, scene.id, crate::Crop::new(1, 0, 0, 0));
        let mut snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        let frame = SnapshotFrame::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
        snapshot.attach_frames(&|_| Some(frame.clone()));
        assert_eq!(&snapshot.render_rgba()[..4], &[0, 255, 0, 255]);
    }

    #[test]
    fn attach_frames_blends_opacity_over_browser_pixels() {
        let scene = Scene::new("Live".to_string());
        let mut sources = SourceManager::new();
        let id = sources.add_source(Source::new("Browser".to_string(), SourceKind::Browser));
        let mut transform = Transform::new(0.0, 0.0, 1.0, 1.0);
        transform.opacity = 0.5;
        sources.bind_source(id, scene.id, Some(transform));
        let mut snapshot = SceneSnapshot::from_scene(&scene, &sources, "Default", "Default");
        let frame = SnapshotFrame::new(1, 1, vec![200, 30, 10, 255]).unwrap();
        snapshot.attach_frames(&|_| Some(frame.clone()));
        // background (18,22,29) blended 50% with (200,30,10) via straight alpha
        assert_eq!(&snapshot.render_rgba()[..4], &[109, 26, 20, 255]);
    }
}
