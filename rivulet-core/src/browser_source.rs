use std::collections::VecDeque;
use std::fmt;

/// Default browser-source viewport width.
pub const DEFAULT_BROWSER_WIDTH: u32 = 1280;
/// Default browser-source viewport height.
pub const DEFAULT_BROWSER_HEIGHT: u32 = 720;
/// Default browser-source refresh rate.
pub const DEFAULT_BROWSER_FPS: u32 = 30;
/// Maximum number of input events retained until a renderer consumes them.
pub const MAX_PENDING_INPUT_EVENTS: usize = 256;

/// Errors returned while configuring a browser source or accepting a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowserSourceError {
    /// The URL uses an unsupported or malformed scheme.
    InvalidUrl(String),
    /// The viewport dimensions or frame buffer size is invalid.
    InvalidDimensions { width: u32, height: u32 },
    /// The zoom value is outside the supported range.
    InvalidZoom(f32),
    /// The frame does not contain exactly one RGBA pixel per viewport pixel.
    InvalidFrameBuffer { expected: usize, actual: usize },
}

impl fmt::Display for BrowserSourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(url) => write!(f, "invalid browser URL: {url}"),
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid browser viewport dimensions: {width}x{height}")
            }
            Self::InvalidZoom(zoom) => write!(f, "invalid browser zoom level: {zoom}"),
            Self::InvalidFrameBuffer { expected, actual } => write!(
                f,
                "invalid browser frame buffer size: expected {expected} bytes, got {actual}"
            ),
        }
    }
}

impl std::error::Error for BrowserSourceError {}

/// Input delivered to an interactive browser source.
#[derive(Debug, Clone, PartialEq)]
pub enum BrowserInput {
    /// Pointer movement in CSS pixels relative to the browser viewport.
    MouseMove { x: f32, y: f32 },
    /// Pointer button press/release in CSS pixels relative to the viewport.
    MouseButton {
        x: f32,
        y: f32,
        button: BrowserMouseButton,
        pressed: bool,
    },
    /// Wheel delta in CSS pixels (positive values scroll up/right).
    Scroll { x: f32, y: f32 },
    /// Keyboard key press/release using the platform-independent key name.
    Key { key: String, pressed: bool },
}

/// Mouse buttons understood by the browser input bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserMouseButton {
    Left,
    Middle,
    Right,
}

/// One RGBA frame produced by a browser renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Monotonic renderer frame number, if supplied by the backend.
    pub sequence: u64,
}

impl BrowserFrame {
    /// Construct a frame and validate that its buffer has RGBA layout.
    pub fn new(
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        sequence: u64,
    ) -> Result<Self, BrowserSourceError> {
        let expected = expected_rgba_len(width, height)?;
        if rgba.len() != expected {
            return Err(BrowserSourceError::InvalidFrameBuffer {
                expected,
                actual: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
            sequence,
        })
    }
}

/// Configuration and host-side state for an interactive browser source.
///
/// The actual platform webview is deliberately kept behind the
/// [`BrowserSourceBackend`] trait. This keeps the scene model serializable and
/// makes WebView2, WebKitGTK, and WKWebView adapters interchangeable without
/// coupling the core crate to a native window/event-loop implementation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BrowserSource {
    /// URL currently loaded by the source. Only http(s) and local file URLs
    /// are accepted; javascript/data URLs are intentionally rejected.
    pub url: String,
    /// Browser viewport width in CSS pixels.
    pub width: u32,
    /// Browser viewport height in CSS pixels.
    pub height: u32,
    /// Target render rate requested from the platform backend.
    pub fps: u32,
    /// Whether pointer and keyboard events are forwarded to the webview.
    pub interaction_enabled: bool,
    /// Whether the browser surface preserves its alpha channel.
    pub transparent: bool,
    /// Browser zoom factor, from 25% to 500%.
    pub zoom_level: f64,
    /// Optional CSS injected after navigation.
    pub custom_css: Option<String>,
    #[serde(skip)]
    pending_input: VecDeque<BrowserInput>,
    #[serde(skip)]
    latest_frame: Option<BrowserFrame>,
}

impl Default for BrowserSource {
    fn default() -> Self {
        Self {
            url: "https://example.com".to_owned(),
            width: DEFAULT_BROWSER_WIDTH,
            height: DEFAULT_BROWSER_HEIGHT,
            fps: DEFAULT_BROWSER_FPS,
            interaction_enabled: true,
            transparent: false,
            zoom_level: 1.0,
            custom_css: None,
            pending_input: VecDeque::new(),
            latest_frame: None,
        }
    }
}

impl BrowserSource {
    /// Create a browser source after validating its initial URL and viewport.
    pub fn new(
        url: impl Into<String>,
        width: u32,
        height: u32,
    ) -> Result<Self, BrowserSourceError> {
        let mut source = Self {
            width,
            height,
            ..Self::default()
        };
        source.set_viewport(width, height)?;
        source.navigate(url)?;
        Ok(source)
    }

    /// Validate a URL without changing a source.
    pub fn validate_url(url: &str) -> Result<(), BrowserSourceError> {
        let Some((scheme, rest)) = url.split_once("://") else {
            return Err(BrowserSourceError::InvalidUrl(url.to_owned()));
        };
        let scheme = scheme.to_ascii_lowercase();
        if url.trim() != url || url.chars().any(char::is_whitespace) {
            return Err(BrowserSourceError::InvalidUrl(url.to_owned()));
        }
        match scheme.as_str() {
            "http" | "https" if !rest.is_empty() && !rest.starts_with('/') => Ok(()),
            "file" if !rest.is_empty() => Ok(()),
            _ => Err(BrowserSourceError::InvalidUrl(url.to_owned())),
        }
    }

    /// Navigate to a new URL. The platform backend should observe the updated
    /// value on its next synchronization pass.
    pub fn navigate(&mut self, url: impl Into<String>) -> Result<(), BrowserSourceError> {
        let url = url.into();
        Self::validate_url(&url)?;
        self.url = url;
        self.latest_frame = None;
        Ok(())
    }

    /// Set the webview viewport in CSS pixels.
    pub fn set_viewport(&mut self, width: u32, height: u32) -> Result<(), BrowserSourceError> {
        if !valid_dimensions(width, height) {
            return Err(BrowserSourceError::InvalidDimensions { width, height });
        }
        self.width = width;
        self.height = height;
        self.latest_frame = None;
        Ok(())
    }

    /// Set the target render rate. Zero is rejected because a browser source
    /// must produce at least one frame when visible.
    pub fn set_fps(&mut self, fps: u32) -> Result<(), BrowserSourceError> {
        if fps == 0 || fps > 240 {
            return Err(BrowserSourceError::InvalidDimensions {
                width: self.width,
                height: self.height,
            });
        }
        self.fps = fps;
        Ok(())
    }

    /// Set the zoom factor. Values are limited to the range supported by all
    /// three native webview families.
    pub fn set_zoom_level(&mut self, zoom_level: f64) -> Result<(), BrowserSourceError> {
        if !zoom_level.is_finite() || !(0.25..=5.0).contains(&zoom_level) {
            return Err(BrowserSourceError::InvalidZoom(zoom_level as f32));
        }
        self.zoom_level = zoom_level;
        Ok(())
    }

    /// Queue an input event for the platform backend. Returns `false` when
    /// interaction is disabled; otherwise the oldest event is evicted if the
    /// bounded queue is full.
    pub fn enqueue_input(&mut self, input: BrowserInput) -> bool {
        if !self.interaction_enabled {
            return false;
        }
        if self.pending_input.len() == MAX_PENDING_INPUT_EVENTS {
            self.pending_input.pop_front();
        }
        self.pending_input.push_back(input);
        true
    }

    /// Drain all queued interaction events for a webview backend.
    pub fn take_input_events(&mut self) -> Vec<BrowserInput> {
        self.pending_input.drain(..).collect()
    }

    /// Number of input events waiting for a backend.
    pub fn pending_input_count(&self) -> usize {
        self.pending_input.len()
    }

    /// Accept a rendered RGBA frame from a platform backend.
    pub fn submit_frame(&mut self, frame: BrowserFrame) -> Result<(), BrowserSourceError> {
        if frame.width != self.width || frame.height != self.height {
            return Err(BrowserSourceError::InvalidDimensions {
                width: frame.width,
                height: frame.height,
            });
        }
        // BrowserFrame already validates its own buffer; retaining this check
        // makes the invariant explicit if the type gains more constructors.
        let expected = expected_rgba_len(frame.width, frame.height)?;
        if frame.rgba.len() != expected {
            return Err(BrowserSourceError::InvalidFrameBuffer {
                expected,
                actual: frame.rgba.len(),
            });
        }
        self.latest_frame = Some(frame);
        Ok(())
    }

    /// Borrow the most recently rendered frame, if a backend has produced one.
    pub fn latest_frame(&self) -> Option<&BrowserFrame> {
        self.latest_frame.as_ref()
    }

    /// Remove the cached frame, for example after the webview is closed.
    pub fn clear_frame(&mut self) {
        self.latest_frame = None;
    }

    /// A concise status string suitable for a scene/source list.
    pub fn summary(&self) -> String {
        format!(
            "Browser · {} · {}x{} @ {} FPS",
            self.url, self.width, self.height, self.fps
        )
    }
}

/// Platform adapter contract for WebView2, WebKitGTK, or WKWebView.
///
/// Adapters own the native webview and event loop. They synchronize the
/// serializable [`BrowserSource`] configuration, consume input events, and
/// return RGBA frames that can be uploaded to a GPU texture by the GUI.
pub trait BrowserSourceBackend {
    type Error;

    fn navigate(&mut self, url: &str) -> Result<(), Self::Error>;
    fn resize(&mut self, width: u32, height: u32) -> Result<(), Self::Error>;
    fn set_transparent(&mut self, transparent: bool) -> Result<(), Self::Error>;
    fn set_interaction_enabled(&mut self, enabled: bool) -> Result<(), Self::Error>;
    fn set_zoom_level(&mut self, zoom_level: f64) -> Result<(), Self::Error>;
    fn evaluate_javascript(&mut self, script: &str) -> Result<(), Self::Error>;
    fn send_input(&mut self, input: BrowserInput) -> Result<(), Self::Error>;
    fn poll_frame(&mut self) -> Result<Option<BrowserFrame>, Self::Error>;
}

fn valid_dimensions(width: u32, height: u32) -> bool {
    (1..=8192).contains(&width) && (1..=8192).contains(&height)
}

fn expected_rgba_len(width: u32, height: u32) -> Result<usize, BrowserSourceError> {
    if !valid_dimensions(width, height) {
        return Err(BrowserSourceError::InvalidDimensions { width, height });
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(BrowserSourceError::InvalidDimensions { width, height })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32, height: u32, sequence: u64) -> BrowserFrame {
        BrowserFrame::new(
            width,
            height,
            vec![0; (width * height * 4) as usize],
            sequence,
        )
        .unwrap()
    }

    #[test]
    fn defaults_are_interactive_and_opaque() {
        let source = BrowserSource::default();
        assert_eq!(source.url, "https://example.com");
        assert_eq!((source.width, source.height, source.fps), (1280, 720, 30));
        assert!(source.interaction_enabled);
        assert!(!source.transparent);
        assert_eq!(source.zoom_level, 1.0);
    }

    #[test]
    fn constructor_validates_url_and_viewport() {
        let source = BrowserSource::new("https://stream.example/live", 1920, 1080).unwrap();
        assert_eq!(source.url, "https://stream.example/live");
        assert!(BrowserSource::new("javascript:alert(1)", 1280, 720).is_err());
        assert!(BrowserSource::new("https://example.com", 0, 720).is_err());
    }

    #[test]
    fn url_validation_allows_web_and_local_content() {
        assert!(BrowserSource::validate_url("http://localhost:3000").is_ok());
        assert!(BrowserSource::validate_url("https://example.com/dashboard").is_ok());
        assert!(BrowserSource::validate_url("file:///tmp/overlay.html").is_ok());
        assert!(BrowserSource::validate_url("data:text/html,hello").is_err());
        assert!(BrowserSource::validate_url("https://").is_err());
        assert!(BrowserSource::validate_url(" https://example.com").is_err());
    }

    #[test]
    fn navigate_clears_stale_frame() {
        let mut source = BrowserSource::default();
        source
            .submit_frame(frame(source.width, source.height, 1))
            .unwrap();
        assert!(source.latest_frame().is_some());
        source.navigate("https://example.org").unwrap();
        assert_eq!(source.url, "https://example.org");
        assert!(source.latest_frame().is_none());
    }

    #[test]
    fn viewport_change_clears_stale_frame() {
        let mut source = BrowserSource::default();
        source
            .submit_frame(frame(source.width, source.height, 1))
            .unwrap();
        source.set_viewport(640, 360).unwrap();
        assert_eq!((source.width, source.height), (640, 360));
        assert!(source.latest_frame().is_none());
    }

    #[test]
    fn zoom_and_fps_are_bounded() {
        let mut source = BrowserSource::default();
        source.set_zoom_level(1.5).unwrap();
        assert_eq!(source.zoom_level, 1.5);
        assert!(source.set_zoom_level(0.1).is_err());
        assert!(source.set_zoom_level(f64::NAN).is_err());
        source.set_fps(60).unwrap();
        assert_eq!(source.fps, 60);
        assert!(source.set_fps(0).is_err());
        assert!(source.set_fps(241).is_err());
    }

    #[test]
    fn input_is_ignored_when_interaction_is_disabled() {
        let mut source = BrowserSource {
            interaction_enabled: false,
            ..BrowserSource::default()
        };
        assert!(!source.enqueue_input(BrowserInput::Scroll { x: 0.0, y: 1.0 }));
        assert_eq!(source.pending_input_count(), 0);
    }

    #[test]
    fn input_queue_preserves_order_and_drains() {
        let mut source = BrowserSource::default();
        assert!(source.enqueue_input(BrowserInput::MouseMove { x: 10.0, y: 20.0 }));
        assert!(source.enqueue_input(BrowserInput::Key {
            key: "Enter".to_owned(),
            pressed: true,
        }));
        assert_eq!(source.pending_input_count(), 2);
        let events = source.take_input_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], BrowserInput::MouseMove { .. }));
        assert!(matches!(events[1], BrowserInput::Key { .. }));
        assert_eq!(source.pending_input_count(), 0);
    }

    #[test]
    fn input_queue_is_bounded_by_dropping_oldest() {
        let mut source = BrowserSource::default();
        for i in 0..=MAX_PENDING_INPUT_EVENTS {
            source.enqueue_input(BrowserInput::MouseMove {
                x: i as f32,
                y: 0.0,
            });
        }
        assert_eq!(source.pending_input_count(), MAX_PENDING_INPUT_EVENTS);
        let events = source.take_input_events();
        assert_eq!(events[0], BrowserInput::MouseMove { x: 1.0, y: 0.0 });
    }

    #[test]
    fn frames_require_matching_dimensions_and_rgba_size() {
        let mut source = BrowserSource::default();
        assert!(source.submit_frame(frame(1280, 720, 7)).is_ok());
        assert_eq!(source.latest_frame().unwrap().sequence, 7);
        assert!(source.submit_frame(frame(640, 360, 8)).is_err());
        assert!(BrowserFrame::new(2, 2, vec![0; 3], 1).is_err());
    }

    #[test]
    fn transparent_and_css_settings_are_serializable() {
        let source = BrowserSource {
            transparent: true,
            custom_css: Some("body { background: transparent; }".to_owned()),
            ..BrowserSource::default()
        };
        let json = serde_json::to_string(&source).unwrap();
        let restored: BrowserSource = serde_json::from_str(&json).unwrap();
        assert!(restored.transparent);
        assert_eq!(
            restored.custom_css.as_deref(),
            Some("body { background: transparent; }")
        );
        assert_eq!(restored.pending_input_count(), 0);
        assert!(restored.latest_frame().is_none());
    }

    #[test]
    fn summary_contains_url_viewport_and_fps() {
        let source = BrowserSource::new("https://example.com/chat", 800, 600).unwrap();
        assert_eq!(
            source.summary(),
            "Browser · https://example.com/chat · 800x600 @ 30 FPS"
        );
    }
}
