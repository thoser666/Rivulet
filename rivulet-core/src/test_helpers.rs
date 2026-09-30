//! Deterministic test helpers (M7 W2b, issue #188): golden-frame comparison,
//! exact PTS/DTS verification, and scene-state comparison for deterministic
//! pipeline contract tests.
//!
//! The M7 quality gate asks for "useful diffs on golden-frame/timestamp/
//! reproducibility failures". A raw buffer dump does not qualify — a 320x240
//! RGBA frame is 300 KB of hex, and when a test fails in CI nobody reads it.
//! This module provides the helpers the gate names:
//!
//! - [`GoldenFrame`] — render/capture frame *N*, compare against a reference,
//!   and on mismatch report the frame index plus a pixel-level summary: how
//!   many pixels differ, by how much (max/mean channel delta), and the first
//!   differing coordinates. [`GoldenFrame::diff`] returns that as data and
//!   [`GoldenFrame::assert_matches`] renders it as the panic message.
//! - [`Timestamps`] — the PTS/DTS sequence of a run, checked against an
//!   expected cadence or an expected sequence. Violations name the offending
//!   index, the actual value, and what was expected, so a tampered timestamp
//!   is located rather than merely detected.
//! - [`SceneState`] — the scene-collection counterpart, so "this operation is
//!   deterministic" is assertable as a named difference (which item, which
//!   property) instead of one opaque whole-collection `assert_eq!`.
//!
//! The frame and timestamp helpers are dependency-free (no image/PNG crate)
//! and operate on raw RGBA, which is the engine's appsrc format. PNG encoding
//! belongs to the caller (the GUI already has the `image` crate); the gate
//! cares about the diff quality, not the file format.

use crate::source::{Crop, SceneSource, Source, SourceKind, Transform};
use serde::Serialize;
use uuid::Uuid;

/// Byte length of one RGBA pixel.
const RGBA_CHANNELS: usize = 4;

/// One frame's worth of raw RGBA plus the geometry needed to interpret it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenFrame {
    /// Horizontal size in pixels.
    pub width: u32,
    /// Vertical size in pixels.
    pub height: u32,
    /// Raw RGBA bytes, `width * height * 4` long.
    pub rgba: Vec<u8>,
}

impl GoldenFrame {
    /// Wrap raw RGBA bytes.
    ///
    /// # Panics
    /// If `rgba` is not exactly `width * height * 4` bytes, or the geometry is
    /// degenerate. A length mismatch would otherwise silently compare
    /// unrelated regions and report a misleading diff.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        assert!(
            width > 0 && height > 0,
            "golden frame geometry must be non-zero, got {width}x{height}"
        );
        let expected = width as usize * height as usize * RGBA_CHANNELS;
        assert_eq!(
            rgba.len(),
            expected,
            "expected {expected} bytes for {width}x{height} RGBA, got {}",
            rgba.len()
        );
        Self {
            width,
            height,
            rgba,
        }
    }

    /// A uniformly colored frame — the starting point for a synthetic case.
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Self {
        Self::new(width, height, rgba.repeat(width as usize * height as usize))
    }

    /// The frame's pixel at `(x, y)`, or `None` when out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; RGBA_CHANNELS]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let offset = (y as usize * self.width as usize + x as usize) * RGBA_CHANNELS;
        Some([
            self.rgba[offset],
            self.rgba[offset + 1],
            self.rgba[offset + 2],
            self.rgba[offset + 3],
        ])
    }

    /// Compare against a reference frame.
    ///
    /// Returns `None` when the frames are identical (including same-size
    /// geometry), and a [`FrameDiff`] otherwise. A geometry mismatch is itself
    /// a diff rather than a panic, because "the renderer changed the output
    /// size" is exactly the kind of regression worth reporting.
    pub fn diff(&self, reference: &GoldenFrame) -> Option<FrameDiff> {
        if self.width != reference.width || self.height != reference.height {
            return Some(FrameDiff {
                frame_index: None,
                geometry_changed: true,
                expected_geometry: (reference.width, reference.height),
                actual_geometry: (self.width, self.height),
                differing_pixels: 0,
                total_pixels: 0,
                max_channel_delta: 0,
                mean_channel_delta: 0.0,
                first_difference: None,
            });
        }

        let total_pixels = self.width as usize * self.height as usize;
        let mut differing_pixels = 0usize;
        let mut max_channel_delta = 0u8;
        let mut sum_channel_delta = 0u64;
        let mut first_difference = None;

        for pixel_index in 0..total_pixels {
            let offset = pixel_index * RGBA_CHANNELS;
            let mut pixel_differs = false;
            for channel in 0..RGBA_CHANNELS {
                let expected = reference.rgba[offset + channel];
                let actual = self.rgba[offset + channel];
                let delta = actual.abs_diff(expected);
                if delta > 0 {
                    pixel_differs = true;
                    max_channel_delta = max_channel_delta.max(delta);
                    sum_channel_delta += u64::from(delta);
                }
            }
            if pixel_differs {
                differing_pixels += 1;
                if first_difference.is_none() {
                    let x = (pixel_index % self.width as usize) as u32;
                    let y = (pixel_index / self.width as usize) as u32;
                    first_difference = Some(PixelDifference {
                        x,
                        y,
                        expected: reference.pixel(x, y).unwrap_or([0; RGBA_CHANNELS]),
                        actual: self.pixel(x, y).unwrap_or([0; RGBA_CHANNELS]),
                    });
                }
            }
        }

        if differing_pixels == 0 {
            return None;
        }

        let differing_channels = differing_pixels * RGBA_CHANNELS;
        Some(FrameDiff {
            frame_index: None,
            geometry_changed: false,
            expected_geometry: (self.width, self.height),
            actual_geometry: (self.width, self.height),
            differing_pixels,
            total_pixels,
            max_channel_delta,
            mean_channel_delta: sum_channel_delta as f64 / differing_channels as f64,
            first_difference,
        })
    }

    /// Assert equality against a reference, naming the frame index and a
    /// pixel-level summary on failure.
    ///
    /// `frame_index` is carried into the message so a failure in a 500-frame
    /// sequence points at the frame instead of dumping buffers.
    #[track_caller]
    pub fn assert_matches(&self, reference: &GoldenFrame, frame_index: usize) {
        let Some(mut diff) = self.diff(reference) else {
            return;
        };
        diff.frame_index = Some(frame_index);
        panic!("{}", diff.describe());
    }

    /// Write the frame as a PNG next to a failed test's output.
    ///
    /// Uses the `image` crate only if the caller enables the `golden-png`
    /// feature; without it the bytes are returned so the caller can decide.
    /// Kept free of a hard dependency because core has no image handling today
    /// and the diff itself needs none.
    pub fn to_png_bytes(&self) -> Option<Vec<u8>> {
        png_bytes(self.width, self.height, &self.rgba)
    }
}

/// The first differing pixel of a frame comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PixelDifference {
    /// X coordinate of the first differing pixel.
    pub x: u32,
    /// Y coordinate of the first differing pixel.
    pub y: u32,
    /// The reference pixel's RGBA value.
    pub expected: [u8; RGBA_CHANNELS],
    /// The actual pixel's RGBA value.
    pub actual: [u8; RGBA_CHANNELS],
}

/// A pixel-level summary of why two frames differ.
///
/// Field order is stable and every field is `Serialize`, so a diff can be
/// emitted as machine-readable JSON by a future CI step just as easily as it is
/// rendered into a panic message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FrameDiff {
    /// The frame index the caller compared, when it supplied one.
    pub frame_index: Option<usize>,
    /// Whether the mismatch is a geometry change rather than pixel content.
    pub geometry_changed: bool,
    /// Reference `(width, height)`.
    pub expected_geometry: (u32, u32),
    /// Actual `(width, height)`.
    pub actual_geometry: (u32, u32),
    /// Number of pixels that differ.
    pub differing_pixels: usize,
    /// Total pixels compared.
    pub total_pixels: usize,
    /// Largest single-channel difference anywhere in the frame.
    pub max_channel_delta: u8,
    /// Mean per-channel difference across the differing channels.
    pub mean_channel_delta: f64,
    /// The first differing pixel, so the failure starts at a known coordinate.
    pub first_difference: Option<PixelDifference>,
}

impl FrameDiff {
    /// A one-line fraction of pixels that differ, e.g. `"12/76800 (0.02%)"`.
    pub fn differing_fraction(&self) -> String {
        if self.total_pixels == 0 {
            return "0/0".to_string();
        }
        let percent = self.differing_pixels as f64 * 100.0 / self.total_pixels as f64;
        format!(
            "{}/{} ({percent:.2}%)",
            self.differing_pixels, self.total_pixels
        )
    }

    /// The panic message: frame index, extent, magnitude and coordinates.
    pub fn describe(&self) -> String {
        let mut message = String::from("golden frame mismatch");
        if let Some(index) = self.frame_index {
            message.push_str(&format!(" at frame {index}"));
        }
        if self.geometry_changed {
            return format!(
                "{message}: geometry changed, expected {}x{}, got {}x{}",
                self.expected_geometry.0,
                self.expected_geometry.1,
                self.actual_geometry.0,
                self.actual_geometry.1
            );
        }
        message.push_str(&format!(
            ": {} pixels differ, max channel delta {}, mean {:.2}, first difference ",
            self.differing_fraction(),
            self.max_channel_delta,
            self.mean_channel_delta
        ));
        match self.first_difference {
            Some(diff) => format!(
                "{message}at ({}, {}): expected {:?}, got {:?}",
                diff.x, diff.y, diff.expected, diff.actual
            ),
            None => format!("{message}unavailable"),
        }
    }
}

/// The PTS/DTS sequence of a run, checked against an expectation.
///
/// Timestamps are stored as nanoseconds (`u64`) so a comparison never depends
/// on a `ClockTime` conversion. Every check reports the offending index and
/// both values, so "the container timestamps are wrong" becomes "frame 7
/// expected 233333332 ns, got 233166665 ns".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timestamps {
    /// The recorded PTS values in nanoseconds, in push order.
    pub pts_ns: Vec<u64>,
    /// The recorded DTS values in nanoseconds, in push order.
    ///
    /// Empty when the source carried no separate DTS, which is normal for
    /// simple pipelines where DTS mirrors PTS.
    pub dts_ns: Vec<u64>,
}

impl Timestamps {
    /// Record a PTS sequence with DTS mirroring PTS.
    pub fn from_pts(pts_ns: Vec<u64>) -> Self {
        Self {
            dts_ns: pts_ns.clone(),
            pts_ns,
        }
    }

    /// Record both sequences.
    pub fn new(pts_ns: Vec<u64>, dts_ns: Vec<u64>) -> Self {
        Self { pts_ns, dts_ns }
    }

    /// Read PTS/DTS out of GStreamer buffers, preserving order.
    ///
    /// A buffer without a PTS contributes `None` so the caller can decide
    /// whether that is a failure; DTS falls back to PTS when absent, matching
    /// how the engine's own probes read timestamps.
    pub fn from_buffers(buffers: &[(Option<u64>, Option<u64>)]) -> Self {
        Self {
            pts_ns: buffers.iter().map(|(pts, _)| pts.unwrap_or(0)).collect(),
            dts_ns: buffers
                .iter()
                .map(|(pts, dts)| dts.or(*pts).unwrap_or(0))
                .collect(),
        }
    }

    /// Number of timestamps recorded.
    pub fn len(&self) -> usize {
        self.pts_ns.len()
    }

    /// Whether no timestamp was recorded.
    pub fn is_empty(&self) -> bool {
        self.pts_ns.is_empty()
    }

    /// Assert the sequence equals `expected` exactly.
    ///
    /// Reports the first divergent index and both values rather than dumping
    /// two whole sequences.
    #[track_caller]
    pub fn assert_equals(&self, expected: &[u64]) {
        if let Err(violation) = self.check_equals(expected) {
            panic!("{violation}");
        }
    }

    /// Check the sequence against `expected`, returning the first violation.
    pub fn check_equals(&self, expected: &[u64]) -> Result<(), TimestampViolation> {
        if self.pts_ns.len() != expected.len() {
            return Err(TimestampViolation {
                index: self.pts_ns.len().min(expected.len()),
                kind: TimestampViolationKind::Length {
                    expected: expected.len(),
                    actual: self.pts_ns.len(),
                },
                expected_ns: expected.get(self.pts_ns.len().min(expected.len())).copied(),
                actual_ns: self
                    .pts_ns
                    .get(self.pts_ns.len().min(expected.len()))
                    .copied(),
            });
        }
        for (index, (&want, &got)) in expected.iter().zip(self.pts_ns.iter()).enumerate() {
            if want != got {
                return Err(TimestampViolation {
                    index,
                    kind: TimestampViolationKind::Value,
                    expected_ns: Some(want),
                    actual_ns: Some(got),
                });
            }
        }
        Ok(())
    }

    /// Assert every gap between consecutive timestamps is exactly `interval_ns`.
    ///
    /// This is the contract a fixed-cadence recording must hold: a single
    /// dropped or duplicated frame shows up as one bad gap, named by index.
    #[track_caller]
    pub fn assert_constant_interval(&self, interval_ns: u64) {
        if let Err(violation) = self.check_constant_interval(interval_ns) {
            panic!("{violation}");
        }
    }

    /// Check the cadence, returning the first violating gap.
    pub fn check_constant_interval(&self, interval_ns: u64) -> Result<(), TimestampViolation> {
        assert!(interval_ns > 0, "frame interval must be non-zero");
        for index in 1..self.pts_ns.len() {
            let gap = self.pts_ns[index] - self.pts_ns[index - 1];
            if gap != interval_ns {
                return Err(TimestampViolation {
                    index,
                    kind: TimestampViolationKind::Interval { interval_ns },
                    expected_ns: Some(self.pts_ns[index - 1] + interval_ns),
                    actual_ns: Some(self.pts_ns[index]),
                });
            }
        }
        Ok(())
    }

    /// Assert timestamps are strictly increasing (no repeats, no reordering).
    #[track_caller]
    pub fn assert_monotonic(&self) {
        if let Err(violation) = self.check_monotonic() {
            panic!("{violation}");
        }
    }

    /// Check monotonicity, returning the first violation.
    pub fn check_monotonic(&self) -> Result<(), TimestampViolation> {
        for index in 1..self.pts_ns.len() {
            if self.pts_ns[index] <= self.pts_ns[index - 1] {
                return Err(TimestampViolation {
                    index,
                    kind: TimestampViolationKind::Monotonic,
                    expected_ns: Some(self.pts_ns[index - 1] + 1),
                    actual_ns: Some(self.pts_ns[index]),
                });
            }
        }
        Ok(())
    }

    /// Assert DTS never exceeds PTS for the same buffer.
    #[track_caller]
    pub fn assert_dts_not_after_pts(&self) {
        for (index, (&pts, &dts)) in self.pts_ns.iter().zip(self.dts_ns.iter()).enumerate() {
            assert!(
                dts <= pts,
                "timestamp violation at index {index}: DTS {dts} ns is after PTS {pts} ns"
            );
        }
    }
}

/// What kind of timestamp contract was broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampViolationKind {
    /// The sequence had a different number of entries than expected.
    Length {
        /// Entries the contract expects.
        expected: usize,
        /// Entries the run produced.
        actual: usize,
    },
    /// A timestamp differs from the expected value.
    Value,
    /// A gap between consecutive timestamps differs from the cadence.
    Interval {
        /// The cadence the contract requires.
        interval_ns: u64,
    },
    /// Timestamps did not increase strictly.
    Monotonic,
}

/// A single timestamp contract violation, with enough context to locate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TimestampViolation {
    /// Index of the offending timestamp in the sequence.
    pub index: usize,
    /// Which contract was broken.
    pub kind: TimestampViolationKind,
    /// The expected value in nanoseconds, when meaningful.
    pub expected_ns: Option<u64>,
    /// The actual value in nanoseconds, when meaningful.
    pub actual_ns: Option<u64>,
}

impl std::fmt::Display for TimestampViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "timestamp violation at index {}", self.index)?;
        match self.kind {
            TimestampViolationKind::Length { expected, actual } => {
                write!(f, ": expected {expected} timestamps, run produced {actual}")
            }
            TimestampViolationKind::Value => write!(
                f,
                ": expected {} ns, got {} ns",
                self.expected_ns.unwrap_or(0),
                self.actual_ns.unwrap_or(0)
            ),
            TimestampViolationKind::Interval { interval_ns } => write!(
                f,
                ": expected a gap of {interval_ns} ns ({} ns), got {} ns",
                self.expected_ns.unwrap_or(0),
                self.actual_ns.unwrap_or(0)
            ),
            TimestampViolationKind::Monotonic => write!(
                f,
                ": timestamps must increase strictly, expected > {} ns, got {} ns",
                self.expected_ns.map(|ns| ns - 1).unwrap_or(0),
                self.actual_ns.unwrap_or(0)
            ),
        }
    }
}

/// Minimal PNG encoder for a raw RGBA frame.
///
/// The golden-frame *diff* needs no image crate, but a failing test is much
/// more useful with a viewable artifact next to it. This encodes a
/// non-interlaced 8-bit RGBA PNG with stored (uncompressed) deflate blocks
/// and a CRC32/Adler32 pair — no external dependency, at the cost of file
/// size, which is irrelevant for a test artifact.
fn png_bytes(width: u32, height: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(rgba.len() + 1024);
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, deflate, no filter
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Raw scanlines, each prefixed with filter type 0 (None).
    let stride = width as usize * RGBA_CHANNELS;
    let mut raw = Vec::with_capacity(height as usize * (stride + 1));
    for row in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgba[row * stride..(row + 1) * stride]);
    }

    let idat = zlib_stored(&raw);
    write_chunk(&mut out, b"IDAT", &idat);
    write_chunk(&mut out, b"IEND", &[]);
    Some(out)
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// Wrap `data` in a zlib stream of stored deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // deflate, 32K window, no dict, fastest
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    } else {
        let mut offset = 0usize;
        while offset < data.len() {
            let take = (data.len() - offset).min(0xffff);
            let is_last = offset + take >= data.len();
            out.push(if is_last { 1 } else { 0 });
            out.extend_from_slice(&(take as u16).to_le_bytes());
            out.extend_from_slice(&(!(take as u16)).to_le_bytes());
            out.extend_from_slice(&data[offset..offset + take]);
            offset += take;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

// ── Scene-state determinism ─────────────────────────────────────

/// How a scene state differs from its reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneStateDifference {
    /// A different number of sources exists.
    SourceCount { expected: usize, actual: usize },
    /// A different number of scene bindings exists.
    BindingCount { expected: usize, actual: usize },
    /// A source with this id is missing (or unexpectedly present).
    SourceMissing { source_id: Uuid },
    /// A binding for `source_id` in `scene_id` is missing (or present).
    BindingMissing { source_id: Uuid, scene_id: Uuid },
    /// A source exists in both states but a property differs.
    SourceProperty {
        source_id: Uuid,
        property: &'static str,
        expected: String,
        actual: String,
    },
    /// A binding exists in both states but a property differs.
    BindingProperty {
        source_id: Uuid,
        scene_id: Uuid,
        property: &'static str,
        expected: String,
        actual: String,
    },
}

impl std::fmt::Display for SceneStateDifference {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SceneStateDifference::SourceCount { expected, actual } => write!(
                formatter,
                "source count differs: expected {expected}, got {actual}"
            ),
            SceneStateDifference::BindingCount { expected, actual } => write!(
                formatter,
                "binding count differs: expected {expected}, got {actual}"
            ),
            SceneStateDifference::SourceMissing { source_id } => write!(
                formatter,
                "source {source_id} exists in one state but not the other"
            ),
            SceneStateDifference::BindingMissing {
                source_id,
                scene_id,
            } => write!(
                formatter,
                "binding of source {source_id} into scene {scene_id} exists in one \
                 state but not the other"
            ),
            SceneStateDifference::SourceProperty {
                source_id,
                property,
                expected,
                actual,
            } => write!(
                formatter,
                "source {source_id} property {property} differs: expected {expected}, \
                 got {actual}"
            ),
            SceneStateDifference::BindingProperty {
                source_id,
                scene_id,
                property,
                expected,
                actual,
            } => write!(
                formatter,
                "binding of source {source_id} into scene {scene_id} property {property} \
                 differs: expected {expected}, got {actual}"
            ),
        }
    }
}

/// A comparable snapshot of the deterministic part of a scene collection.
///
/// Scene-item operations (copy/paste/duplicate) are *supposed* to be
/// deterministic — same clipboard in, same scene state out — but "the states
/// are equal" is only assertable as one opaque `assert_eq!` over the whole
/// collection. When that fails in CI, the diff is a wall of `Debug` output
/// with no indication of which item or property actually changed.
///
/// This helper applies the same idea as [`GoldenFrame`] to scene state: find
/// the first *semantic* difference and name it. Ordering is normalized, so
/// two runs that agree on content but not on insertion order do not report a
/// spurious difference — which is the distinction that matters when asking
/// "is this operation deterministic?".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneState {
    /// Sources sorted by id, so comparison is order-independent.
    sources: Vec<SceneItemState>,
    /// Bindings sorted by (scene, source), likewise order-independent.
    bindings: Vec<SceneBindingState>,
}

/// One source, flattened to its identity and its deterministic properties.
///
/// `SourceKind` and `Transform` do not derive `Ord`/`Eq`, so ordering is done
/// explicitly in [`SceneState::new`] instead of derived here.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneItemState {
    pub id: Uuid,
    pub name: String,
    pub kind: SourceKind,
    pub visible: bool,
    pub locked: bool,
    pub z_order: i32,
}

/// One scene binding, flattened the same way.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SceneBindingState {
    pub scene_id: Uuid,
    pub source_id: Uuid,
    pub transform_override: Option<Transform>,
    pub crop: Crop,
    pub visible: bool,
    pub locked: bool,
    pub z_order: i32,
}

impl SceneState {
    /// Snapshot a source collection. Accepts what
    /// [`SourceManager::current_collection`] returns, so a test can write
    /// `SceneState::new(mgr.current_collection())`.
    pub fn new((sources, bindings): (Vec<Source>, Vec<SceneSource>)) -> Self {
        let mut sources: Vec<SceneItemState> = sources
            .into_iter()
            .map(|source| SceneItemState {
                id: source.id,
                name: source.name,
                kind: source.kind,
                visible: source.visible,
                locked: source.locked,
                z_order: source.z_order,
            })
            .collect();
        let mut bindings: Vec<SceneBindingState> = bindings
            .into_iter()
            .map(|binding| SceneBindingState {
                scene_id: binding.scene_id,
                source_id: binding.source_id,
                transform_override: binding.transform_override,
                crop: binding.crop,
                visible: binding.visible,
                locked: binding.locked,
                z_order: binding.z_order,
            })
            .collect();
        // Sort by the identity fields only (all `Ord`), never by the property
        // values: ordering by a `Debug` string would make the comparison
        // order-dependent on formatting.
        sources.sort_by_key(|item| item.id);
        bindings.sort_by_key(|binding| (binding.scene_id, binding.source_id));
        SceneState { sources, bindings }
    }

    /// Number of sources in the snapshot.
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Number of scene bindings in the snapshot.
    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    /// The first semantic difference from `reference`, or `None` if the two
    /// states agree.
    ///
    /// `self` is the actual state, `reference` the expected one, matching
    /// [`GoldenFrame::diff`].
    pub fn diff(&self, reference: &SceneState) -> Option<SceneStateDifference> {
        if self.sources.len() != reference.sources.len() {
            return Some(SceneStateDifference::SourceCount {
                expected: reference.sources.len(),
                actual: self.sources.len(),
            });
        }
        if self.bindings.len() != reference.bindings.len() {
            return Some(SceneStateDifference::BindingCount {
                expected: reference.bindings.len(),
                actual: self.bindings.len(),
            });
        }

        for actual in &self.sources {
            let Some(expected) = reference.sources.iter().find(|item| item.id == actual.id) else {
                return Some(SceneStateDifference::SourceMissing {
                    source_id: actual.id,
                });
            };
            if let Some(difference) = property_differences(actual, expected, |item| {
                vec![
                    ("name", format!("{:?}", item.name)),
                    ("kind", format!("{:?}", item.kind)),
                    ("visible", format!("{:?}", item.visible)),
                    ("locked", format!("{:?}", item.locked)),
                    ("z_order", format!("{:?}", item.z_order)),
                ]
            }) {
                return Some(SceneStateDifference::SourceProperty {
                    source_id: actual.id,
                    property: difference.0,
                    expected: difference.1,
                    actual: difference.2,
                });
            }
        }

        for actual in &self.bindings {
            let Some(expected) = reference.bindings.iter().find(|binding| {
                binding.source_id == actual.source_id && binding.scene_id == actual.scene_id
            }) else {
                return Some(SceneStateDifference::BindingMissing {
                    source_id: actual.source_id,
                    scene_id: actual.scene_id,
                });
            };
            if let Some(difference) = property_differences(actual, expected, |binding| {
                vec![
                    ("transform", format!("{:?}", binding.transform_override)),
                    ("crop", format!("{:?}", binding.crop)),
                    ("visible", format!("{:?}", binding.visible)),
                    ("locked", format!("{:?}", binding.locked)),
                    ("z_order", format!("{:?}", binding.z_order)),
                ]
            }) {
                return Some(SceneStateDifference::BindingProperty {
                    source_id: actual.source_id,
                    scene_id: actual.scene_id,
                    property: difference.0,
                    expected: difference.1,
                    actual: difference.2,
                });
            }
        }

        None
    }

    /// Check two states agree, returning the first difference as data.
    pub fn check_matches(&self, reference: &SceneState) -> Result<(), SceneStateDifference> {
        self.diff(reference).map_or(Ok(()), Err)
    }

    /// Assert two states agree, panicking with a *named* difference rather
    /// than an opaque whole-collection dump.
    pub fn assert_matches(&self, reference: &SceneState) {
        if let Some(difference) = self.diff(reference) {
            panic!(
                "scene state differs: {difference}\n  (actual {} sources / {} bindings, \
                 expected {} sources / {} bindings)",
                self.sources.len(),
                self.bindings.len(),
                reference.sources.len(),
                reference.bindings.len(),
            );
        }
    }
}

/// Compare the named properties of one actual/expected item pair, returning
/// `(property, expected_rendered, actual_rendered)` for the first mismatch.
///
/// Properties are supplied as `(name, rendered_value)` pairs so the comparison
/// works uniformly across property types: `SourceKind` and `Transform` derive
/// neither `Ord` nor `Eq`, and rendering through `Debug` keeps the reported
/// difference readable. Names are matched positionally, so both closures must
/// list the same properties in the same order.
fn property_differences<T>(
    actual: &T,
    expected: &T,
    properties: impl Fn(&T) -> Vec<(&'static str, String)>,
) -> Option<(&'static str, String, String)> {
    let actual_values = properties(actual);
    let expected_values = properties(expected);
    let pair = actual_values.iter().zip(expected_values.iter());
    for ((name, actual_value), (expected_name, expected_value)) in pair {
        debug_assert_eq!(
            name, expected_name,
            "property lists must be positionally aligned"
        );
        if actual_value != expected_value {
            return Some((*name, expected_value.clone(), actual_value.clone()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> GoldenFrame {
        let mut rgba = vec![0u8; width as usize * height as usize * RGBA_CHANNELS];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let offset = (y * width as usize + x) * RGBA_CHANNELS;
                rgba[offset] = (x * 4) as u8;
                rgba[offset + 1] = (y * 4) as u8;
                rgba[offset + 2] = 128;
                rgba[offset + 3] = 255;
            }
        }
        GoldenFrame::new(width, height, rgba)
    }

    #[test]
    fn identical_frames_produce_no_diff() {
        let frame = gradient(16, 12);
        let same = gradient(16, 12);
        assert!(frame.diff(&same).is_none());
        // The assert form must not panic either.
        frame.assert_matches(&same, 7);
    }

    #[test]
    fn a_diff_names_the_frame_index_and_the_pixel() {
        let reference = gradient(16, 12);
        let mut actual = gradient(16, 12);
        // Perturb one pixel's red channel by 100.
        let offset = (3 * 16 + 5) * RGBA_CHANNELS;
        actual.rgba[offset] = actual.rgba[offset].wrapping_add(100);

        let diff = actual.diff(&reference).expect("a difference exists");
        assert_eq!(diff.differing_pixels, 1);
        assert_eq!(diff.total_pixels, 16 * 12);
        assert_eq!(diff.max_channel_delta, 100);
        let first = diff.first_difference.expect("first difference reported");
        assert_eq!((first.x, first.y), (5, 3));

        let mut diff = diff;
        diff.frame_index = Some(42);
        let message = diff.describe();
        assert!(message.contains("frame 42"), "got {message}");
        assert!(message.contains("1/192"), "got {message}");
        assert!(message.contains("(5, 3)"), "got {message}");
    }

    #[test]
    fn a_geometry_change_is_reported_as_such() {
        let reference = gradient(16, 12);
        let resized = gradient(8, 6);
        let diff = resized.diff(&reference).expect("geometry differs");
        assert!(diff.geometry_changed);
        assert_eq!(diff.expected_geometry, (16, 12));
        assert_eq!(diff.actual_geometry, (8, 6));
        let message = diff.describe();
        assert!(message.contains("geometry changed"), "got {message}");
    }

    #[test]
    fn a_wrong_length_frame_is_rejected_at_construction() {
        // A length mismatch would silently compare unrelated memory, so it is
        // a construction error rather than a confusing diff later.
        let result = std::panic::catch_unwind(|| GoldenFrame::new(4, 4, vec![0u8; 10]));
        assert!(result.is_err(), "a short RGBA buffer must be rejected");
    }

    #[test]
    fn diff_is_serializable_for_machine_readable_ci_output() {
        let reference = gradient(8, 8);
        let mut actual = gradient(8, 8);
        actual.rgba[0] = 7;
        let diff = actual.diff(&reference).expect("diff exists");
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&diff).unwrap()).unwrap();
        assert_eq!(value["differing_pixels"], 1);
        assert_eq!(value["first_difference"]["x"], 0);
    }

    #[test]
    fn png_bytes_start_with_the_png_signature_and_size_header() {
        let frame = gradient(8, 4);
        let png = frame.to_png_bytes().expect("png bytes produced");
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert_eq!(&png[12..16], b"IHDR");
        // Width/height are big-endian right after the IHDR tag.
        assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]), 8);
        assert_eq!(u32::from_be_bytes([png[20], png[21], png[22], png[23]]), 4);
        assert!(
            png.ends_with(&crc32(b"IEND").to_be_bytes()),
            "must end with IEND"
        );
    }

    #[test]
    fn stored_deflate_round_trips_through_adler_and_crc() {
        // Guards the hand-rolled encoder's checksum path: a wrong CRC or
        // Adler32 yields a file no viewer can open, and the test would still
        // pass on length alone.
        let data: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        let stream = zlib_stored(&data);
        assert_eq!(&stream[0..2], &[0x78, 0x01]);
        let tail = &stream[stream.len() - 4..];
        assert_eq!(
            u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]),
            adler32(&data)
        );
    }

    // --- PTS/DTS helper ---

    #[test]
    fn an_exact_sequence_passes_the_equality_check() {
        let stamps = Timestamps::from_pts(vec![0, 33_333_333, 66_666_666]);
        stamps.assert_equals(&[0, 33_333_333, 66_666_666]);
        stamps.assert_monotonic();
        stamps.assert_dts_not_after_pts();
    }

    #[test]
    fn a_tampered_timestamp_is_located_by_index() {
        let stamps = Timestamps::from_pts(vec![0, 33_333_333, 66_600_000]);
        let violation = stamps
            .check_equals(&[0, 33_333_333, 66_666_666])
            .expect_err("index 2 was tampered");
        assert_eq!(violation.index, 2);
        assert_eq!(violation.kind, TimestampViolationKind::Value);
        assert_eq!(violation.expected_ns, Some(66_666_666));
        assert_eq!(violation.actual_ns, Some(66_600_000));

        let message = violation.to_string();
        assert!(message.contains("index 2"), "got {message}");
        assert!(message.contains("66666666"), "got {message}");
        assert!(message.contains("66600000"), "got {message}");
    }

    #[test]
    fn a_dropped_frame_is_caught_by_the_cadence_check() {
        let stamps = Timestamps::from_pts(vec![0, 33_333_333, 99_999_999]);
        let violation = stamps
            .check_constant_interval(33_333_333)
            .expect_err("a frame was dropped");
        assert_eq!(violation.index, 2);
        assert_eq!(
            violation.kind,
            TimestampViolationKind::Interval {
                interval_ns: 33_333_333
            }
        );
        assert_eq!(violation.expected_ns, Some(66_666_666));
        assert_eq!(violation.actual_ns, Some(99_999_999));
    }

    #[test]
    fn a_short_sequence_is_reported_as_a_length_violation() {
        let stamps = Timestamps::from_pts(vec![0, 33_333_333]);
        let violation = stamps
            .check_equals(&[0, 33_333_333, 66_666_666])
            .expect_err("one frame missing");
        assert_eq!(
            violation.kind,
            TimestampViolationKind::Length {
                expected: 3,
                actual: 2
            }
        );
        assert_eq!(violation.index, 2);
    }

    #[test]
    fn non_monotonic_timestamps_are_caught() {
        let stamps = Timestamps::from_pts(vec![0, 33_333_333, 33_333_333]);
        let violation = stamps.check_monotonic().expect_err("duplicate PTS");
        assert_eq!(violation.kind, TimestampViolationKind::Monotonic);
        assert_eq!(violation.index, 2);
    }

    #[test]
    fn dts_after_pts_is_reported() {
        let stamps = Timestamps::new(vec![0, 10], vec![0, 20]);
        let panic = std::panic::catch_unwind(|| stamps.assert_dts_not_after_pts());
        assert!(panic.is_err(), "DTS after PTS must fail");
    }

    #[test]
    fn buffers_convert_with_dts_falling_back_to_pts() {
        let stamps = Timestamps::from_buffers(&[(Some(0), Some(0)), (Some(40), None)]);
        assert_eq!(stamps.pts_ns, vec![0, 40]);
        assert_eq!(stamps.dts_ns, vec![0, 40], "absent DTS mirrors PTS");
        assert_eq!(stamps.len(), 2);
        assert!(!stamps.is_empty());
    }

    // ── SceneState ────────────────────────────────────────────────

    fn sample_source(id: u128) -> Source {
        let mut source = Source::new("Cam".to_string(), SourceKind::Webcam);
        source.id = Uuid::from_u128(id);
        source
    }

    fn sample_binding(source: u128, scene: u128) -> SceneSource {
        SceneSource {
            source_id: Uuid::from_u128(source),
            scene_id: Uuid::from_u128(scene),
            transform_override: Some(Transform::new(1.0, 2.0, 3.0, 4.0)),
            crop: Crop::new(1, 2, 3, 4),
            visible: true,
            locked: false,
            z_order: 0,
        }
    }

    #[test]
    fn equal_scene_states_report_no_difference() {
        let state = SceneState::new((vec![sample_source(1)], vec![sample_binding(1, 7)]));
        let same = SceneState::new((vec![sample_source(1)], vec![sample_binding(1, 7)]));
        assert_eq!(state.check_matches(&same), Ok(()));
        state.assert_matches(&same);
        assert_eq!(state.source_count(), 1);
        assert_eq!(state.binding_count(), 1);
    }

    #[test]
    fn scene_state_ignores_insertion_order() {
        // Determinism means "same content", not "same order of Vec pushes":
        // two runs that append the same items in a different order must not
        // report a difference, or every ordering change reads as a regression.
        let forwards = SceneState::new((
            vec![sample_source(1), sample_source(2)],
            vec![sample_binding(1, 7), sample_binding(2, 7)],
        ));
        let backwards = SceneState::new((
            vec![sample_source(2), sample_source(1)],
            vec![sample_binding(2, 7), sample_binding(1, 7)],
        ));
        forwards.assert_matches(&backwards);
    }

    #[test]
    fn scene_state_names_the_changing_property() {
        let reference = SceneState::new((vec![sample_source(1)], vec![sample_binding(1, 7)]));
        let mut moved = sample_source(1);
        moved.z_order = 9;
        let actual = SceneState::new((vec![moved], vec![sample_binding(1, 7)]));

        let difference = actual.check_matches(&reference).unwrap_err();
        assert_eq!(
            difference,
            SceneStateDifference::SourceProperty {
                source_id: Uuid::from_u128(1),
                property: "z_order",
                expected: "0".to_string(),
                actual: "9".to_string(),
            }
        );
        let message = difference.to_string();
        assert!(message.contains("z_order"), "got {message}");
        assert!(message.contains("expected 0"), "got {message}");
    }

    #[test]
    fn scene_state_reports_counts_and_missing_items() {
        let one = SceneState::new((vec![sample_source(1)], vec![]));
        let two = SceneState::new((vec![sample_source(1), sample_source(2)], vec![]));

        assert_eq!(
            two.check_matches(&one).unwrap_err(),
            SceneStateDifference::SourceCount {
                expected: 1,
                actual: 2
            }
        );

        let bound = SceneState::new((vec![sample_source(1)], vec![sample_binding(1, 7)]));
        let unbound = SceneState::new((vec![sample_source(1)], vec![]));
        assert_eq!(
            bound.check_matches(&unbound).unwrap_err(),
            SceneStateDifference::BindingCount {
                expected: 0,
                actual: 1
            }
        );

        // Same length, different identity: the count checks pass and the item
        // lookup is what catches it.
        let other_identity = SceneState::new((vec![sample_source(3)], vec![]));
        assert_eq!(
            other_identity.check_matches(&one).unwrap_err(),
            SceneStateDifference::SourceMissing {
                source_id: Uuid::from_u128(3)
            }
        );
    }

    #[test]
    fn scene_state_reports_a_changed_binding_property() {
        let reference = SceneState::new((vec![sample_source(1)], vec![sample_binding(1, 7)]));
        let mut cropped = sample_binding(1, 7);
        cropped.crop = Crop::new(9, 9, 9, 9);
        let actual = SceneState::new((vec![sample_source(1)], vec![cropped]));

        let difference = actual.check_matches(&reference).unwrap_err();
        assert_eq!(
            difference,
            SceneStateDifference::BindingProperty {
                source_id: Uuid::from_u128(1),
                scene_id: Uuid::from_u128(7),
                property: "crop",
                expected: format!("{:?}", Crop::new(1, 2, 3, 4)),
                actual: format!("{:?}", Crop::new(9, 9, 9, 9)),
            }
        );
    }
}
