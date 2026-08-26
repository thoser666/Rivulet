use crate::{Crop, Scene, SourceKind, SourceManager, Transform};

/// Virtual canvas used by the scene editor and snapshot export.
pub const DEFAULT_CANVAS_WIDTH: u32 = 1920;
pub const DEFAULT_CANVAS_HEIGHT: u32 = 1080;

/// A visible source layer captured in a scene snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotLayer {
    pub name: String,
    pub kind: SourceKind,
    pub transform: Transform,
    pub crop: Crop,
    pub z_order: i32,
}

/// A deterministic representation of the current scene composition.
///
/// The current GUI does not have a cross-platform native source renderer yet.
/// Therefore snapshots render the scene layout as colored source tiles while
/// preserving the scene's visible layers, transforms, crops, and z-order.
/// This keeps export useful and reproducible on every platform until native
/// media rendering is added.
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
                    name: source.name.clone(),
                    kind: source.kind.clone(),
                    transform: binding.effective_transform(&source.transform).clone(),
                    crop: binding.crop,
                    z_order: binding.z_order,
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

    /// Render the layout into an RGBA8 image buffer.
    ///
    /// Source kinds use stable colors to make the layer composition legible in
    /// the exported image without depending on a platform renderer.
    pub fn render_rgba(&self) -> Vec<u8> {
        let pixel_count = self.width as usize * self.height as usize;
        let mut pixels = vec![0u8; pixel_count * 4];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&[18, 22, 29, 255]);
        }

        for layer in &self.layers {
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
        pixels
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
}
