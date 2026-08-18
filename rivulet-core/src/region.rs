//! Rectangular capture regions (subsets of a monitor) for region capture.
//!
//! A [`CaptureRegion`] is expressed in monitor-local pixels: `(x, y)` is the
//! top-left corner of the region relative to the top-left corner of the
//! captured monitor, and `width`/`height` describe the region size. Regions
//! are always clamped to the monitor bounds at crop time, so a stale region
//! (e.g. after a monitor was re-plugged with different dimensions) can never
//! panic or read out of bounds.

/// A rectangular region of a monitor, in monitor-local pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRegion {
    /// Horizontal offset from the monitor's left edge.
    pub x: u32,
    /// Vertical offset from the monitor's top edge.
    pub y: u32,
    /// Region width in pixels.
    pub width: u32,
    /// Region height in pixels.
    pub height: u32,
}

impl CaptureRegion {
    /// A region covering an entire `width` × `height` surface.
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// Whether the region is empty (zero width or height).
    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Whether the region covers the full `width` × `height` surface.
    pub fn is_full_for(self, width: u32, height: u32) -> bool {
        self.x == 0 && self.y == 0 && self.width == width && self.height == height
    }

    /// Snap the width and height down to even pixel values.
    ///
    /// Common video encoders (H.264, H.265, VP9) require even frame
    /// dimensions, so a region with an odd width or height must be shrunk
    /// before its frames can be encoded.
    pub fn even_aligned(self) -> Self {
        Self {
            x: self.x,
            y: self.y,
            width: self.width & !1,
            height: self.height & !1,
        }
    }

    /// Clamp the region so it lies fully inside a `max_width` × `max_height`
    /// surface, keeping the top-left corner fixed when possible. An empty or
    /// degenerate surface yields an empty region.
    pub fn clamped_to(self, max_width: u32, max_height: u32) -> Self {
        if max_width == 0 || max_height == 0 {
            return Self {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            };
        }
        let x = self.x.min(max_width - 1);
        let y = self.y.min(max_height - 1);
        Self {
            x,
            y,
            width: self.width.min(max_width - x),
            height: self.height.min(max_height - y),
        }
    }

    /// Crop an RGBA8 frame (row-major, `stride` bytes per row) to this region.
    ///
    /// The region is clamped to the frame bounds and even-aligned first, so
    /// callers can pass stale or odd-sized regions safely. Returns `None`
    /// when the clamped region is empty or the buffer is too small for the
    /// declared frame size. On success, returns the cropped pixel data plus
    /// the cropped dimensions (which may differ from the region's own
    /// width/height when clamping or even alignment applied).
    pub fn crop_rgba(
        &self,
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) -> Option<(Vec<u8>, u32, u32)> {
        let region = self.clamped_to(width, height).even_aligned();
        if region.is_empty() {
            return None;
        }
        let needed = (height.saturating_sub(1)) as usize * stride as usize + width as usize * 4;
        if data.len() < needed {
            return None;
        }
        let mut out = Vec::with_capacity(region.width as usize * region.height as usize * 4);
        for row in region.y..region.y + region.height {
            let start = row as usize * stride as usize + region.x as usize * 4;
            out.extend_from_slice(&data[start..start + region.width as usize * 4]);
        }
        Some((out, region.width, region.height))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_region_covers_the_surface() {
        let region = CaptureRegion::full(1920, 1080);
        assert!(region.is_full_for(1920, 1080));
        assert!(!region.is_empty());
        assert!(!CaptureRegion::full(1920, 1080).is_full_for(1921, 1080));
    }

    #[test]
    fn empty_region_is_empty() {
        assert!(CaptureRegion::full(0, 1080).is_empty());
        assert!(CaptureRegion::full(1920, 0).is_empty());
        assert!(!CaptureRegion {
            x: 10,
            y: 20,
            width: 8,
            height: 4
        }
        .is_empty());
    }

    #[test]
    fn clamped_to_shrinks_overflowing_regions() {
        let region = CaptureRegion {
            x: 1900,
            y: 1050,
            width: 200,
            height: 100,
        };
        let clamped = region.clamped_to(1920, 1080);
        assert_eq!(clamped.x, 1900);
        assert_eq!(clamped.y, 1050);
        assert_eq!(clamped.width, 20);
        assert_eq!(clamped.height, 30);
    }

    #[test]
    fn clamped_to_keeps_valid_regions_unchanged() {
        let region = CaptureRegion {
            x: 10,
            y: 20,
            width: 100,
            height: 50,
        };
        assert_eq!(region.clamped_to(1920, 1080), region);
    }

    #[test]
    fn clamped_to_clamps_offset_beyond_the_surface() {
        let region = CaptureRegion {
            x: 5000,
            y: 3000,
            width: 100,
            height: 100,
        };
        let clamped = region.clamped_to(1920, 1080);
        assert_eq!(clamped.x, 1919);
        assert_eq!(clamped.y, 1079);
        assert_eq!(clamped.width, 1);
        assert_eq!(clamped.height, 1);
    }

    #[test]
    fn clamped_to_empty_surface_is_empty() {
        let region = CaptureRegion {
            x: 5,
            y: 5,
            width: 50,
            height: 50,
        };
        assert!(region.clamped_to(0, 0).is_empty());
    }

    #[test]
    fn even_aligned_snaps_odd_dimensions_down() {
        let region = CaptureRegion {
            x: 3,
            y: 5,
            width: 1919,
            height: 1081,
        }
        .even_aligned();
        assert_eq!(region.width, 1918);
        assert_eq!(region.height, 1080);
        assert_eq!(region.x, 3);
        assert_eq!(region.y, 5);
        assert!(CaptureRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }
        .even_aligned()
        .is_empty());
    }

    #[test]
    fn crop_rgba_extracts_the_sub_region() {
        // 4x3 frame with distinct per-pixel values: RGBA = [x*4+y*4..].
        let width = 4u32;
        let height = 3u32;
        let mut data = Vec::new();
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[x as u8, y as u8, 7, 9]);
            }
        }
        let region = CaptureRegion {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let (cropped, cw, ch) = region
            .crop_rgba(&data, width, height, width * 4)
            .expect("crop should succeed");
        assert_eq!((cw, ch), (2, 2));
        // Pixel (1,1): RGBA (1,1,7,9); (2,1): (2,1,7,9); (1,2): (1,2,7,9); (2,2): (2,2,7,9)
        let expected = [
            1, 1, 7, 9, 2, 1, 7, 9, //
            1, 2, 7, 9, 2, 2, 7, 9,
        ];
        assert_eq!(cropped, expected);
    }

    #[test]
    fn crop_rgba_full_frame_is_identical_to_input() {
        let width = 4u32;
        let height = 4u32;
        let data: Vec<u8> = (0..width * height * 4).map(|i| i as u8).collect();
        let (cropped, cw, ch) = CaptureRegion::full(width, height)
            .crop_rgba(&data, width, height, width * 4)
            .expect("full crop should succeed");
        assert_eq!((cw, ch), (width, height));
        assert_eq!(cropped, data);
    }

    #[test]
    fn crop_rgba_clamps_overflowing_regions() {
        let width = 4u32;
        let height = 3u32;
        let data: Vec<u8> = (0..width * height * 4).map(|i| i as u8).collect();
        let region = CaptureRegion {
            x: 2,
            y: 1,
            width: 10,
            height: 10,
        };
        let (cropped, cw, ch) = region
            .crop_rgba(&data, width, height, width * 4)
            .expect("clamped crop should succeed");
        assert_eq!((cw, ch), (2, 2));
        // With data[i] = i as u8: pixel (x,y) is bytes [4*(y*4+x)..].
        // Pixel (2,1): 24..28; (3,1): 28..32; (2,2): 40..44; (3,2): 44..48.
        assert_eq!(
            cropped,
            vec![
                24, 25, 26, 27, 28, 29, 30, 31, //
                40, 41, 42, 43, 44, 45, 46, 47
            ]
        );
    }

    #[test]
    fn crop_rgba_returns_none_when_clamping_leaves_one_pixel() {
        // A region that clamps down to a single odd pixel cannot be
        // even-aligned, so the crop is rejected (the caller falls back to
        // the uncropped frame).
        let width = 4u32;
        let height = 3u32;
        let data: Vec<u8> = (0..width * height * 4).map(|i| i as u8).collect();
        let region = CaptureRegion {
            x: 3,
            y: 2,
            width: 10,
            height: 10,
        };
        assert!(region.crop_rgba(&data, width, height, width * 4).is_none());
    }

    #[test]
    fn crop_rgba_odd_region_is_even_aligned() {
        let width = 4u32;
        let height = 4u32;
        let data: Vec<u8> = (0..width * height * 4).map(|i| i as u8).collect();
        let region = CaptureRegion {
            x: 1,
            y: 1,
            width: 3,
            height: 3,
        };
        let (cropped, cw, ch) = region
            .crop_rgba(&data, width, height, width * 4)
            .expect("crop should succeed");
        assert_eq!((cw, ch), (2, 2));
        assert_eq!(cropped.len(), 2 * 2 * 4);
    }

    #[test]
    fn crop_rgba_returns_none_for_empty_region() {
        let data = vec![0u8; 4 * 4 * 4];
        let empty = CaptureRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 4,
        };
        assert!(empty.crop_rgba(&data, 4, 4, 16).is_none());
        let odd = CaptureRegion {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        assert!(odd.crop_rgba(&data, 4, 4, 16).is_none());
    }

    #[test]
    fn crop_rgba_returns_none_for_short_buffer() {
        let region = CaptureRegion {
            x: 0,
            y: 0,
            width: 4,
            height: 4,
        };
        assert!(region.crop_rgba(&[0u8; 10], 4, 4, 16).is_none());
    }
}
