use std::time::Duration;

/// Supported pixel formats for webcam capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PixelFormat {
    /// YUYV 4:2:2 (most common).
    Yuyv,
    /// MJPEG (hardware-encoded, common on USB webcams).
    #[default]
    Mjpeg,
    /// NV12 (YUV 4:2:0 semi-planar).
    Nv12,
    /// RGB24 (raw, rarely used).
    Rgb24,
}

impl PixelFormat {
    pub fn label(&self) -> &'static str {
        match self {
            PixelFormat::Yuyv => "YUYV",
            PixelFormat::Mjpeg => "MJPEG",
            PixelFormat::Nv12 => "NV12",
            PixelFormat::Rgb24 => "RGB24",
        }
    }
}

/// A common webcam resolution with a human-readable label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const HD: Resolution = Resolution {
        width: 1280,
        height: 720,
    };
    pub const FHD: Resolution = Resolution {
        width: 1920,
        height: 1080,
    };
    pub const VGA: Resolution = Resolution {
        width: 640,
        height: 480,
    };

    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// A label like "1920×1080" or "720p".
    pub fn label(&self) -> String {
        match (self.width, self.height) {
            (640, 480) => "480p (VGA)".to_string(),
            (1280, 720) => "720p (HD)".to_string(),
            (1920, 1080) => "1080p (FHD)".to_string(),
            (3840, 2160) => "2160p (4K)".to_string(),
            _ => format!("{}×{}", self.width, self.height),
        }
    }

    /// Total pixel count.
    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Aspect ratio as (width, height) simplified.
    pub fn aspect_ratio(&self) -> (u32, u32) {
        let g = gcd(self.width, self.height);
        (self.width / g, self.height / g)
    }
}

impl Default for Resolution {
    fn default() -> Self {
        Resolution::HD
    }
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// A webcam source — captures video from a camera device.
///
/// Stores device identity, resolution/framerate preferences, and pixel
/// format. The actual capture is handled by the platform-specific capture
/// layer (xcap on Linux, MF on Windows, AVFoundation on macOS).
#[derive(Debug, Clone)]
pub struct WebcamSource {
    /// User-visible device name (e.g. "Logitech C920").
    pub device_name: String,
    /// Platform-specific device identifier (e.g. `/dev/video0`, device path).
    pub device_id: String,
    /// Desired capture resolution.
    pub resolution: Resolution,
    /// Desired frames per second.
    pub fps: u32,
    /// Preferred pixel format.
    pub pixel_format: PixelFormat,
    /// Whether the camera is currently active (streaming).
    pub active: bool,
    /// Whether to mirror the video horizontally (selfie mode).
    pub mirror: bool,
    /// Capture timeout — if no frames arrive within this duration,
    /// the source reports an error.
    pub timeout: Duration,
}

impl WebcamSource {
    /// Create a new webcam source with sensible defaults.
    pub fn new(device_name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self {
            device_name: device_name.into(),
            device_id: device_id.into(),
            resolution: Resolution::HD,
            fps: 30,
            pixel_format: PixelFormat::default(),
            active: false,
            mirror: false,
            timeout: Duration::from_secs(5),
        }
    }

    /// Returns true if the device_id is set and non-empty.
    pub fn has_device(&self) -> bool {
        !self.device_id.is_empty()
    }

    /// Returns a summary string for UI display.
    pub fn summary(&self) -> String {
        let status = if self.active { "active" } else { "inactive" };
        format!(
            "{} @ {} {}fps [{}]",
            self.device_name,
            self.resolution.label(),
            self.fps,
            status,
        )
    }

    /// Returns the effective resolution as (width, height).
    pub fn dimensions(&self) -> (u32, u32) {
        (self.resolution.width, self.resolution.height)
    }
}

impl Default for WebcamSource {
    fn default() -> Self {
        Self::new("", "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PixelFormat ───────────────────────────────────────────────

    #[test]
    fn pixel_format_labels() {
        assert_eq!(PixelFormat::Yuyv.label(), "YUYV");
        assert_eq!(PixelFormat::Mjpeg.label(), "MJPEG");
        assert_eq!(PixelFormat::Nv12.label(), "NV12");
        assert_eq!(PixelFormat::Rgb24.label(), "RGB24");
    }

    #[test]
    fn pixel_format_default_is_mjpeg() {
        assert_eq!(PixelFormat::default(), PixelFormat::Mjpeg);
    }

    // ── Resolution ────────────────────────────────────────────────

    #[test]
    fn resolution_constants() {
        assert_eq!(Resolution::HD.width, 1280);
        assert_eq!(Resolution::HD.height, 720);
        assert_eq!(Resolution::FHD.width, 1920);
        assert_eq!(Resolution::FHD.height, 1080);
        assert_eq!(Resolution::VGA.width, 640);
        assert_eq!(Resolution::VGA.height, 480);
    }

    #[test]
    fn resolution_new() {
        let r = Resolution::new(2560, 1440);
        assert_eq!(r.width, 2560);
        assert_eq!(r.height, 1440);
    }

    #[test]
    fn resolution_label_known() {
        assert_eq!(Resolution::HD.label(), "720p (HD)");
        assert_eq!(Resolution::FHD.label(), "1080p (FHD)");
        assert_eq!(Resolution::VGA.label(), "480p (VGA)");
    }

    #[test]
    fn resolution_label_custom() {
        let r = Resolution::new(800, 600);
        assert_eq!(r.label(), "800×600");
    }

    #[test]
    fn resolution_pixel_count() {
        assert_eq!(Resolution::FHD.pixel_count(), 1920 * 1080);
        assert_eq!(Resolution::VGA.pixel_count(), 640 * 480);
    }

    #[test]
    fn resolution_aspect_ratio_16_9() {
        let (w, h) = Resolution::FHD.aspect_ratio();
        assert_eq!((w, h), (16, 9));
    }

    #[test]
    fn resolution_aspect_ratio_4_3() {
        let (w, h) = Resolution::VGA.aspect_ratio();
        assert_eq!((w, h), (4, 3));
    }

    #[test]
    fn resolution_default_is_hd() {
        let r = Resolution::default();
        assert_eq!(r, Resolution::HD);
    }

    // ── WebcamSource ──────────────────────────────────────────────

    #[test]
    fn webcam_source_new_defaults() {
        let ws = WebcamSource::new("C920", "/dev/video0");
        assert_eq!(ws.device_name, "C920");
        assert_eq!(ws.device_id, "/dev/video0");
        assert_eq!(ws.resolution, Resolution::HD);
        assert_eq!(ws.fps, 30);
        assert_eq!(ws.pixel_format, PixelFormat::Mjpeg);
        assert!(!ws.active);
        assert!(!ws.mirror);
        assert_eq!(ws.timeout, Duration::from_secs(5));
    }

    #[test]
    fn webcam_source_default_is_empty() {
        let ws = WebcamSource::default();
        assert!(ws.device_name.is_empty());
        assert!(ws.device_id.is_empty());
    }

    #[test]
    fn webcam_source_has_device() {
        let ws = WebcamSource::new("Cam", "/dev/video0");
        assert!(ws.has_device());

        let ws2 = WebcamSource::new("Cam", "");
        assert!(!ws2.has_device());
    }

    #[test]
    fn webcam_source_summary_active() {
        let mut ws = WebcamSource::new("C920", "video0");
        ws.active = true;
        let s = ws.summary();
        assert!(s.contains("C920"));
        assert!(s.contains("active"));
        assert!(s.contains("720p"));
        assert!(s.contains("30fps"));
    }

    #[test]
    fn webcam_source_summary_inactive() {
        let ws = WebcamSource::new("C920", "video0");
        let s = ws.summary();
        assert!(s.contains("inactive"));
    }

    #[test]
    fn webcam_source_dimensions() {
        let mut ws = WebcamSource::new("Cam", "id");
        ws.resolution = Resolution::FHD;
        assert_eq!(ws.dimensions(), (1920, 1080));
    }

    #[test]
    fn webcam_source_custom_config() {
        let mut ws = WebcamSource::new("C920", "video0");
        ws.resolution = Resolution::FHD;
        ws.fps = 60;
        ws.pixel_format = PixelFormat::Yuyv;
        ws.mirror = true;
        ws.timeout = Duration::from_secs(10);

        assert_eq!(ws.resolution, Resolution::FHD);
        assert_eq!(ws.fps, 60);
        assert_eq!(ws.pixel_format, PixelFormat::Yuyv);
        assert!(ws.mirror);
        assert_eq!(ws.timeout, Duration::from_secs(10));
    }

    #[test]
    fn webcam_source_clone_preserves_data() {
        let mut ws = WebcamSource::new("Clone", "dev");
        ws.resolution = Resolution::FHD;
        ws.fps = 60;
        ws.active = true;

        let ws2 = ws.clone();
        assert_eq!(ws2.device_name, "Clone");
        assert_eq!(ws2.resolution, Resolution::FHD);
        assert_eq!(ws2.fps, 60);
        assert!(ws2.active);
    }

    #[test]
    fn webcam_source_debug_impl() {
        let ws = WebcamSource::new("Debug", "id");
        let dbg = format!("{ws:?}");
        assert!(dbg.contains("WebcamSource"));
        assert!(dbg.contains("Debug"));
    }
}
