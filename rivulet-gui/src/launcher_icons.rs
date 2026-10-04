//! Self-designed launcher marks for the scene-device picker (issue #239).
//!
//! These glyphs are Rivulet's own: plain geometry painted with egui's
//! [`Painter`](egui::Painter), not reproductions of any storefront logo.
//!
//! ## Why they are not the vendors' logos
//!
//! Using the real marks is not a free choice:
//!
//! * **Epic** — its *Trademark Usage Guidelines (Non-Licensee)* state that
//!   Epic's logos must not be used "without express written permission".
//! * **Valve** — the Steam Branding Guidelines require the logo to "stand
//!   alone and may not be combined with any object, including but not
//!   limited to other logos", which a row of launcher logos would violate.
//! * **Blizzard**, **EA** and **CD PROJEKT** each gate logo use behind their
//!   own brand-guideline or media-contact process.
//!
//! Drawing our own marks keeps the picker recognisable while shipping zero
//! third-party trademarks, zero binary assets, and no licensing obligation.
//! Colours are brand-*adjacent* — dark, desaturated, so each store stays
//! tellable apart without imitating the mark itself.
//!
//! Every glyph is described by a pure [`LauncherGlyph`] value so a test can
//! assert that each launcher is actually distinct and that a newly added
//! launcher cannot silently ship without an icon.

// `egui` is re-exported through `eframe` in this crate, not a direct dependency.
use eframe::egui::{self, Color32, Rect, Stroke, Vec2};

/// The primitive a launcher mark is built from.
///
/// Each variant is a distinct, generic geometric form. None of them traces a
/// storefront's logo: no gear, shield, ribbon or wordmark is reproduced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphShape {
    /// Three ascending bars — a library/track motif.
    Bars,
    /// A chevron pair pointing forward.
    Chevrons,
    /// A ring with a solid centre dot.
    Ring,
    /// A solid diamond.
    Diamond,
    /// A six-sided outline.
    Hexagon,
    /// Four rounded petals around a centre.
    Petals,
    /// A generic window frame — the heuristic fallback, not a launcher.
    Window,
}

/// The geometry and colours of one launcher's mark.
///
/// Pure data, so [`launcher_glyph`] is testable without a running UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LauncherGlyph {
    pub shape: GlyphShape,
    /// Fill of the mark, brand-adjacent but darkened/desaturated.
    pub color: Color32,
    /// Slightly brighter tone used for the accent parts.
    pub accent: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// The mark for `launcher`.
///
/// `None` for [`rivulet_core::LauncherKind::Heuristic`]: a heuristic window
/// has no storefront, so it gets [`heuristic_glyph`] instead. Returning
/// `None` rather than guessing means a future launcher that forgets its
/// entry is caught by the "every launcher has a distinct mark" test.
pub fn launcher_glyph(launcher: rivulet_core::LauncherKind) -> Option<LauncherGlyph> {
    use rivulet_core::LauncherKind;
    Some(match launcher {
        // Steam's mark is a gear-and-piston; ours is an ascending bar group.
        LauncherKind::Steam => LauncherGlyph {
            shape: GlyphShape::Bars,
            color: rgb(0x2a, 0x4d, 0x69),
            accent: rgb(0x66, 0xc0, 0xf4),
        },
        // Epic's mark is a shield; ours is a chevron pair.
        LauncherKind::Epic => LauncherGlyph {
            shape: GlyphShape::Chevrons,
            color: rgb(0x2d, 0x2d, 0x3a),
            accent: rgb(0xb0, 0xb6, 0xc4),
        },
        LauncherKind::Gog => LauncherGlyph {
            shape: GlyphShape::Ring,
            color: rgb(0x3d, 0x24, 0x4d),
            accent: rgb(0xc9, 0x6b, 0xd8),
        },
        // Origin and the EA app must look different from each other.
        LauncherKind::Origin => LauncherGlyph {
            shape: GlyphShape::Diamond,
            color: rgb(0x33, 0x3d, 0x2c),
            accent: rgb(0x9b, 0xc8, 0x6b),
        },
        LauncherKind::EaApp => LauncherGlyph {
            shape: GlyphShape::Hexagon,
            color: rgb(0x3d, 0x2d, 0x1e),
            accent: rgb(0xe8, 0x9c, 0x4a),
        },
        LauncherKind::BattleNet => LauncherGlyph {
            shape: GlyphShape::Petals,
            color: rgb(0x1e, 0x33, 0x40),
            accent: rgb(0x5c, 0xb8, 0xd8),
        },
        LauncherKind::Heuristic => return None,
    })
}

/// The mark for the heuristic "Other windows" fallback.
pub fn heuristic_glyph() -> LauncherGlyph {
    LauncherGlyph {
        shape: GlyphShape::Window,
        color: rgb(0x3a, 0x3a, 0x42),
        accent: rgb(0x9a, 0x9a, 0xa6),
    }
}

/// Paint `glyph` into `rect`.
///
/// Drawn rather than rasterised so it stays crisp at any DPI and follows the
/// UI's own scaling; no image assets are involved at all.
pub fn paint_glyph(painter: &egui::Painter, rect: Rect, glyph: LauncherGlyph) {
    let stroke_width = (rect.width() * 0.11).max(1.0);
    let stroke = Stroke::new(stroke_width, glyph.color);
    let accent = Stroke::new(stroke_width.max(1.0), glyph.accent);
    let centre = rect.center();

    match glyph.shape {
        GlyphShape::Bars => {
            // Ascending bars: a library/track motif, deliberately not
            // Steam's gear-and-piston mark.
            const BAR_COUNT: usize = 3;
            let heights = [0.40, 0.62, 0.84];
            let gap = rect.width() * 0.10;
            let bar_width = (rect.width() - gap * 2.0) / BAR_COUNT as f32;
            for (index, height) in heights.iter().enumerate() {
                let x = rect.min.x + gap + index as f32 * (bar_width + gap / 2.0);
                let bar = Rect::from_min_max(
                    egui::pos2(
                        x,
                        rect.center().y + rect.height() * 0.42 - rect.height() * height,
                    ),
                    egui::pos2(x + bar_width, rect.center().y + rect.height() * 0.42),
                );
                let color = if index == 2 {
                    glyph.accent
                } else {
                    glyph.color
                };
                painter.rect_filled(bar, 2.0, color);
            }
        }
        GlyphShape::Chevrons => {
            let unit = rect.width() * 0.34;
            for index in 0..2 {
                let base = Rect::from_min_size(
                    egui::pos2(
                        rect.min.x + rect.width() * 0.18 + index as f32 * unit * 0.82,
                        rect.min.y + rect.height() * 0.20,
                    ),
                    Vec2::new(unit, rect.height() * 0.60),
                );
                let stroke_for = if index == 1 { accent } else { stroke };
                painter.line(
                    vec![
                        egui::pos2(base.min.x, base.min.y),
                        egui::pos2(base.max.x, base.center().y),
                        egui::pos2(base.min.x, base.max.y),
                    ],
                    stroke_for,
                );
            }
        }
        GlyphShape::Ring => {
            painter.circle_stroke(centre, rect.width() * 0.36, stroke);
            painter.circle_filled(centre, rect.width() * 0.14, glyph.accent);
        }
        GlyphShape::Diamond => {
            let r = rect.width() * 0.40;
            let points = vec![
                egui::pos2(centre.x, centre.y - r),
                egui::pos2(centre.x + r, centre.y),
                egui::pos2(centre.x, centre.y + r),
                egui::pos2(centre.x - r, centre.y),
                egui::pos2(centre.x, centre.y - r),
            ];
            painter.line(points, stroke);
            painter.circle_filled(centre, rect.width() * 0.11, glyph.accent);
        }
        GlyphShape::Hexagon => {
            let r = rect.width() * 0.42;
            let mut points = Vec::with_capacity(7);
            for index in 0..=6 {
                let angle = index as f32 * std::f32::consts::TAU / 6.0;
                points.push(egui::pos2(
                    centre.x + r * angle.cos(),
                    centre.y + r * angle.sin(),
                ));
            }
            let inner: Vec<egui::Pos2> = points
                .iter()
                .map(|p| {
                    egui::pos2(
                        centre.x + (p.x - centre.x) * 0.45,
                        centre.y + (p.y - centre.y) * 0.45,
                    )
                })
                .collect();
            painter.line(points, stroke);
            painter.line(inner, accent);
        }
        GlyphShape::Petals => {
            let radius = rect.width() * 0.20;
            let distance = rect.width() * 0.24;
            for index in 0..4 {
                let angle =
                    index as f32 * std::f32::consts::TAU / 4.0 + std::f32::consts::FRAC_PI_4;
                let offset = Vec2::new(angle.cos() * distance, angle.sin() * distance);
                let petal = Rect::from_center_size(centre + offset, Vec2::splat(radius * 2.0));
                painter.circle_filled(
                    petal.center(),
                    radius,
                    if index % 2 == 0 {
                        glyph.accent
                    } else {
                        glyph.color
                    },
                );
            }
            painter.circle_filled(centre, radius * 0.55, glyph.accent);
        }
        GlyphShape::Window => {
            painter.rect_stroke(
                rect.shrink(rect.width() * 0.10),
                egui::CornerRadius::same(2),
                stroke,
                egui::StrokeKind::Middle,
            );
            let bar = Rect::from_min_size(
                egui::pos2(rect.min.x, rect.min.y + rect.height() * 0.10),
                Vec2::new(rect.width(), rect.height() * 0.10),
            );
            painter.rect_filled(bar, 0.0, glyph.color);
            // Two generic content lines, so the mark reads as "window".
            for index in 0..2 {
                let y = rect.min.y + rect.height() * (0.45 + index as f32 * 0.20);
                painter.rect_filled(
                    Rect::from_min_size(
                        egui::pos2(rect.min.x + rect.width() * 0.18, y),
                        Vec2::new(rect.width() * 0.64, rect.height() * 0.08),
                    ),
                    1.0,
                    if index == 0 {
                        glyph.accent
                    } else {
                        glyph.color
                    },
                );
            }
        }
    }
}

/// Size of a launcher mark, in points.
pub const ICON_SIZE: f32 = 14.0;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every storefront launcher must have its own mark.
    ///
    /// This is the guard that makes the icon feature fail loudly rather than
    /// silently: adding a `LauncherKind` without a glyph leaves `None` here,
    /// and the picker would then fall back to the heuristic window frame and
    /// mislabel a storefront as "Other windows".
    #[test]
    fn every_launcher_has_a_distinct_mark() {
        let mut shapes = Vec::new();
        for launcher in rivulet_core::LauncherKind::all() {
            let glyph = launcher_glyph(*launcher)
                .unwrap_or_else(|| panic!("{launcher:?} has no launcher mark"));
            assert!(
                !shapes.contains(&glyph.shape),
                "{launcher:?} reuses {:?}, so its mark is not distinguishable",
                glyph.shape
            );
            shapes.push(glyph.shape);
        }
        assert_eq!(shapes.len(), 6, "six storefront launchers need six marks");
    }

    /// Colours are brand-adjacent but deliberately darker and less saturated
    /// than the marks themselves, so a store stays tellable apart without the
    /// icon imitating its logo.
    #[test]
    fn marks_are_dark_and_distinguishable_in_colour() {
        let mut colours = Vec::new();
        for launcher in rivulet_core::LauncherKind::all() {
            let glyph = launcher_glyph(*launcher).expect("mark exists");
            for channel in [glyph.color.r(), glyph.color.g(), glyph.color.b()] {
                assert!(
                    channel <= 0xE8,
                    "{launcher:?} mark is too bright: channel {channel:#04x}"
                );
            }
            assert!(
                !colours.contains(&glyph.color),
                "{launcher:?} reuses another launcher's fill colour"
            );
            colours.push(glyph.color);
        }
    }

    /// A heuristic window is not a storefront and must not be given one.
    #[test]
    fn heuristic_fallback_has_no_storefront_mark() {
        assert_eq!(launcher_glyph(rivulet_core::LauncherKind::Heuristic), None);
        let fallback = heuristic_glyph();
        assert_eq!(fallback.shape, GlyphShape::Window);
        // And it must not collide with any storefront's shape.
        for launcher in rivulet_core::LauncherKind::all() {
            assert_ne!(
                launcher_glyph(*launcher).map(|g| g.shape),
                Some(fallback.shape)
            );
        }
    }

    /// The colour helper is `const fn`, so the palette is deterministic and
    /// reviewable in the source rather than produced at runtime.
    #[test]
    fn palette_is_explicit_and_deterministic() {
        assert_eq!(rgb(0x2a, 0x4d, 0x69), Color32::from_rgb(0x2a, 0x4d, 0x69));
        let steam = launcher_glyph(rivulet_core::LauncherKind::Steam).expect("mark");
        assert_eq!(steam.color, rgb(0x2a, 0x4d, 0x69));
    }

    /// Painting must not panic for any launcher at any sane icon size: the
    /// geometry is computed from the rect, so a degenerate size is the risky
    /// input, not a typical one.
    #[test]
    fn paint_glyph_survives_degenerate_rects() {
        let ctx = egui::Context::default();
        let painter = ctx.debug_painter();
        let sizes = [0.0, 0.5, 1.0, ICON_SIZE, 64.0];
        for launcher in rivulet_core::LauncherKind::all() {
            let glyph = launcher_glyph(*launcher).expect("mark");
            for size in sizes {
                let rect = Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::splat(size));
                paint_glyph(&painter, rect, glyph);
            }
        }
        paint_glyph(
            &painter,
            Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::splat(0.0)),
            heuristic_glyph(),
        );
    }
}
