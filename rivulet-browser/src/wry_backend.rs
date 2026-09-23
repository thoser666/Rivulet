//! Windows wry spike backend (issue #215, M6.9).
//!
//! This is a **proof of concept**, not production code. It proves the
//! remaining #215 pieces are achievable with the stack already chosen in
//! `docs/browser-source-spike.md`:
//!
//! 1. embed a real WebView2 viewport via [`wry`] inside a dedicated
//!    [`winit`] event loop thread,
//! 2. capture each rendered frame as RGBA pixels and ship it through the
//!    existing core contract ([`BrowserSourceBackend`], [`BrowserFrame`]),
//! 3. keep the GUI thread fully decoupled (commands one way, frames the
//!    other) exactly like the GUI's own capture threads already do.
//!
//! Frame capture uses the oficial WebView2 [`CapturePreview`] API
//! (`ICoreWebView2::CapturePreview`) which yields composited pixels even when
//! the window is off-screen. GDI (`PrintWindow` / `BitBlt`) was tried first
//! but only ever produced white frames, because WebView2 composites through
//! DirectComposition rather than the window DC — that experiment is kept in
//! the git history of this file. `CapturePreview` is also what a production
//! adapter would use, so the spike exercises the real API instead of a
//! shortcut. The PNG is decoded back to RGBA on the calling (GUI) thread, so
//! the COM stream stays confined to the webview thread.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc, Arc};

use rivulet_core::browser_source::{
    BrowserFrame, BrowserInput, BrowserMouseButton, BrowserSource, BrowserSourceBackend,
    BrowserSourceError,
};
use webview2_com::CapturePreviewCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Controller, ICoreWebView2Controller2,
    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG, COREWEBVIEW2_COLOR,
};
use windows::core::Interface;
use windows::Win32::Foundation::{HGLOBAL, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::System::Com::IStream;
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindow, GetWindowRect, PostMessageW, GW_CHILD, WM_CHAR, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes};

#[cfg(windows)]
use winit::platform::windows::EventLoopBuilderExtWindows;

/// Size of the "browser surface" the spike creates.
const SPIKE_WIDTH: u32 = 320;
const SPIKE_HEIGHT: u32 = 180;

/// Commands the GUI thread sends to the webview event-loop thread.
#[derive(Debug)]
enum Command {
    Navigate(String),
    Resize {
        width: u32,
        height: u32,
    },
    SetTransparent(bool),
    SetInteractionEnabled(bool),
    SetZoom(f64),
    EvaluateJavaScript(String),
    /// Forward a [`BrowserInput`] event to the WebView2 viewport by posting
    /// the equivalent Windows-message sequence to its shared HWND.
    SendInput(BrowserInput),
    /// Capture a frame with `ICoreWebView2::CapturePreview` and publish the
    /// PNG bytes on the thread's persistent frame channel.
    Capture,
    Shutdown,
}

/// Event-loop side of the spike: owns the window + webview.
struct WryWindowState {
    window: Window,
    webview: wry::WebView,
}

struct WryApp {
    hwnd: Arc<AtomicIsize>,
    state: Option<WryWindowState>,
    /// Mirrors `BrowserSource::interaction_enabled`; while disabled, posted
    /// input events are dropped instead of forwarded.
    interaction_enabled: bool,
    /// Set while a `CapturePreview` request is in flight; additional capture
    /// commands are dropped so the engine never runs concurrent captures.
    /// Shared with the completion handler so it can be cleared on finish.
    capture_busy: Arc<AtomicBool>,
    /// Persistent sink for captured PNGs, consumed by the backend's poll loop.
    frame_tx: mpsc::Sender<Result<Vec<u8>, String>>,
}

impl WryApp {
    fn webview_hwnd(&self) -> Option<HWND> {
        self.state.as_ref().map(|state| {
            // wry's `hwnd()` is the container window; the real WebView2
            // surface is its first child (Chrome_WidgetWin). Input messages
            // must reach that child window.
            let container = WebViewExtWindows::hwnd(&state.webview);
            unsafe { GetWindow(container, GW_CHILD) }
                .ok()
                .unwrap_or(container)
        })
    }
}

impl ApplicationHandler<Command> for WryApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_inner_size(winit::dpi::PhysicalSize::new(SPIKE_WIDTH, SPIKE_HEIGHT))
            .with_position(winit::dpi::PhysicalPosition::new(100, 100))
            .with_visible(true);
        let window = event_loop.create_window(attributes).expect("create window");
        let builder = wry::WebViewBuilder::new()
            .with_url("about:blank")
            .with_visible(true);
        let webview = builder.build(&window).expect("build webview");
        let hwnd = WebViewExtWindows::hwnd(&webview).0 as isize;
        self.hwnd.store(hwnd, Ordering::SeqCst);
        self.state = Some(WryWindowState { window, webview });
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: WindowEvent,
    ) {
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Command) {
        match event {
            Command::Navigate(url) => {
                if let Some(state) = self.state.as_ref() {
                    let _ = state.webview.load_url(&url);
                }
            }
            Command::Resize { width, height } => {
                if let Some(state) = self.state.as_ref() {
                    let _ = state
                        .window
                        .request_inner_size(winit::dpi::PhysicalSize::new(width, height));
                }
            }
            Command::SetTransparent(transparent) => {
                if let Some(state) = self.state.as_ref() {
                    let _ = set_webview_transparent(&state.webview, transparent);
                }
            }
            Command::SetInteractionEnabled(enabled) => {
                self.interaction_enabled = enabled;
            }
            Command::SetZoom(zoom) => {
                if let Some(state) = self.state.as_ref() {
                    let _ = state.webview.zoom(zoom);
                }
            }
            Command::EvaluateJavaScript(script) => {
                if let Some(state) = self.state.as_ref() {
                    let _ = state.webview.evaluate_script(&script);
                }
            }
            Command::SendInput(input) => {
                if self.interaction_enabled {
                    if let Some(hwnd) = self.webview_hwnd() {
                        let _ = forward_browser_input(hwnd, &input);
                    }
                }
            }
            Command::Capture => {
                if let Some(state) = self.state.as_ref() {
                    // Never stack concurrent CapturePreview calls; the
                    // completion handler clears the flag.
                    if self.capture_busy.swap(true, Ordering::SeqCst) {
                        return;
                    }
                    let busy = Arc::clone(&self.capture_busy);
                    let tx = self.frame_tx.clone();
                    capture_preview(&state.webview, busy, tx);
                }
            }
            Command::Shutdown => event_loop.exit(),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
}

/// Handle handed to the GUI thread after [`WryBrowserBackend::spawn`].
/// Implements the core [`BrowserSourceBackend`] contract on top of the
/// event-loop thread.
#[derive(Debug)]
pub struct WryBrowserBackend {
    proxy: EventLoopProxy<Command>,
    width: u32,
    height: u32,
    sequence: u64,
    /// PNG frames come back over this channel; the GUI poll loop only ever
    /// does `try_recv`, so it never blocks on a slow webview thread.
    frame_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
}

impl WryBrowserBackend {
    /// Spawn the webview event-loop thread and the WebView2 viewport.
    /// Waits until the webview HWND is published (or ~5s elapse).
    pub fn spawn() -> Result<Self, BrowserSourceError> {
        let hwnd = Arc::new(AtomicIsize::new(0));
        let (proxy_tx, proxy_rx) = std::sync::mpsc::channel::<EventLoopProxy<Command>>();
        // Frames flow from the webview thread to the poll loop over their own
        // bounded-free channel; `poll_frame` only ever does `try_recv`.
        let (frame_tx, frame_rx) = mpsc::channel::<Result<Vec<u8>, String>>();
        let spawned_hwnd = Arc::clone(&hwnd);
        let spawned_frame_tx = frame_tx;
        std::thread::spawn(move || {
            // `EventLoop` and `WebView` are `!Send`, so the whole loop must
            // be created and owned inside this thread.
            let mut builder = EventLoop::<Command>::with_user_event();
            builder.with_any_thread(true);
            let event_loop = match builder.build() {
                Ok(event_loop) => event_loop,
                Err(_) => return,
            };
            let proxy = event_loop.create_proxy();
            let _ = proxy_tx.send(proxy);
            let mut app = WryApp {
                hwnd: spawned_hwnd,
                state: None,
                interaction_enabled: true,
                capture_busy: Arc::new(AtomicBool::new(false)),
                frame_tx: spawned_frame_tx,
            };
            let _ = event_loop.run_app(&mut app);
        });
        let proxy = proxy_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| BrowserSourceError::InvalidDimensions {
                width: SPIKE_WIDTH,
                height: SPIKE_HEIGHT,
            })?;
        for _ in 0..500 {
            if hwnd.load(Ordering::SeqCst) != 0 {
                return Ok(Self {
                    proxy,
                    width: SPIKE_WIDTH,
                    height: SPIKE_HEIGHT,
                    sequence: 0,
                    frame_rx,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err(BrowserSourceError::InvalidDimensions {
            width: SPIKE_WIDTH,
            height: SPIKE_HEIGHT,
        })
    }

    fn send(&self, command: Command) -> Result<(), BrowserSourceError> {
        self.proxy
            .send_event(command)
            .map_err(|_| BrowserSourceError::InvalidUrl("webview event loop closed".to_owned()))
    }

    /// Capture the current WebView2 pixels via `CapturePreview` (PNG) and
    /// decode back to RGBA for the core [`BrowserFrame`]. The capture is
    /// requested asynchronous; PNGs arrive on `frame_rx` and are consumed
    /// with `try_recv`, so this never blocks the GUI thread. Returns `None`
    /// when no frame is ready yet (the poll loop retries later).
    fn capture_frame(&mut self) -> Result<Option<BrowserFrame>, BrowserSourceError> {
        self.send(Command::Capture)?;
        let png = match self.frame_rx.try_recv() {
            Ok(Ok(png)) => png,
            _ => return Ok(None),
        };
        // Decode back to RGBA. The dimensions are authoritative from the PNG,
        // so we do not claim a client-rect size that might disagree.
        let image = image::load_from_memory(&png)
            .map_err(|_| BrowserSourceError::InvalidDimensions {
                width: SPIKE_WIDTH,
                height: SPIKE_HEIGHT,
            })?
            .to_rgba8();
        let width = image.width();
        let height = image.height();
        self.sequence += 1;
        Ok(Some(BrowserFrame::new(
            width,
            height,
            image.into_raw(),
            self.sequence,
        )?))
    }
}

impl Drop for WryBrowserBackend {
    fn drop(&mut self) {
        let _ = self.proxy.send_event(Command::Shutdown);
    }
}

impl BrowserSourceBackend for WryBrowserBackend {
    type Error = BrowserSourceError;

    fn navigate(&mut self, url: &str) -> Result<(), Self::Error> {
        BrowserSource::validate_url(url)?;
        self.send(Command::Navigate(url.to_owned()))
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<(), Self::Error> {
        if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
            return Err(BrowserSourceError::InvalidDimensions { width, height });
        }
        self.width = width;
        self.height = height;
        self.send(Command::Resize { width, height })
    }

    fn set_transparent(&mut self, transparent: bool) -> Result<(), Self::Error> {
        self.send(Command::SetTransparent(transparent))
    }

    fn set_interaction_enabled(&mut self, enabled: bool) -> Result<(), Self::Error> {
        self.send(Command::SetInteractionEnabled(enabled))
    }

    fn set_zoom_level(&mut self, zoom_level: f64) -> Result<(), Self::Error> {
        if !zoom_level.is_finite() || !(0.25..=5.0).contains(&zoom_level) {
            return Err(BrowserSourceError::InvalidZoom(zoom_level as f32));
        }
        self.send(Command::SetZoom(zoom_level))
    }

    fn evaluate_javascript(&mut self, script: &str) -> Result<(), Self::Error> {
        self.send(Command::EvaluateJavaScript(script.to_owned()))
    }

    fn send_input(&mut self, input: BrowserInput) -> Result<(), Self::Error> {
        self.send(Command::SendInput(input))
    }

    fn poll_frame(&mut self) -> Result<Option<BrowserFrame>, Self::Error> {
        self.capture_frame()
    }
}

/// Run `ICoreWebView2::CapturePreview` on the webview thread and forward the
/// resulting PNG bytes over `tx`.
///
/// The completion handler runs on the same UI thread the webview was created
/// on, so this must be called from inside the event-loop thread (see
/// [`Command::Capture`]).
fn capture_preview(
    webview: &wry::WebView,
    busy: Arc<AtomicBool>,
    tx: mpsc::Sender<Result<Vec<u8>, String>>,
) {
    let core: ICoreWebView2 = wry::WebViewExtWindows::webview(webview);
    let stream = match unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) } {
        Ok(stream) => stream,
        Err(e) => {
            busy.store(false, Ordering::SeqCst);
            let _ = tx.send(Err(e.to_string()));
            return;
        }
    };
    let stream_for_result = stream.clone();
    let tx_for_handler = tx.clone();
    let busy_for_handler = Arc::clone(&busy);
    // Webview writes the PNG into `stream`, then invokes the handler.
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
        busy_for_handler.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                let png = read_istream(&stream_for_result).map_err(|e| e.to_string());
                let _ = tx_for_handler.send(png);
            }
            Err(e) => {
                let _ = tx_for_handler.send(Err(e.to_string()));
            }
        }
        Ok(())
    }));
    if let Err(e) = unsafe {
        core.CapturePreview(
            COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
            &stream,
            &handler,
        )
    } {
        busy.store(false, Ordering::SeqCst);
        let _ = tx.send(Err(e.to_string()));
    }
}

/// Read the whole content of a COM `IStream` into a buffer. Works for the
/// HGLOBAL-backed stream `CreateStreamOnHGlobal` gives us.
fn read_istream(stream: &IStream) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let mut statstg = windows::Win32::System::Com::STATSTG::default();
        stream.Stat(&mut statstg, windows::Win32::System::Com::STATFLAG_NONAME)?;
        let size = statstg.cbSize as usize;
        if size == 0 {
            return Ok(Vec::new());
        }
        // WebView2 wrote the PNG without resetting the stream position; seek
        // back to the start before reading so we capture all bytes.
        stream.Seek(0, windows::Win32::System::Com::STREAM_SEEK_SET, None)?;
        let mut buffer = vec![0u8; size];
        let hresult = stream.Read(
            buffer.as_mut_ptr() as *mut core::ffi::c_void,
            size as u32,
            None,
        );
        hresult.ok()?;
        Ok(buffer)
    }
}

/// Toggle WebView2 transparency at runtime via
/// `ICoreWebView2Controller2::SetDefaultBackgroundColor`. A background alpha
/// of `0` yields transparent pixels (alpha 0) in `CapturePreview` PNGs.
fn set_webview_transparent(webview: &wry::WebView, transparent: bool) -> windows::core::Result<()> {
    let color = if transparent {
        COREWEBVIEW2_COLOR {
            A: 0,
            R: 0,
            G: 0,
            B: 0,
        }
    } else {
        COREWEBVIEW2_COLOR {
            A: 255,
            R: 255,
            G: 255,
            B: 255,
        }
    };
    let controller: ICoreWebView2Controller = wry::WebViewExtWindows::controller(webview);
    let controller2: ICoreWebView2Controller2 = controller.cast()?;
    unsafe { controller2.SetDefaultBackgroundColor(color) }
}

// ── Windows-message input bridge ────────────────────────────────────
//
// WebView2's *windowed* hosting mode receives input as plain Windows
// messages. `forward_browser_input` translates a [`BrowserInput`] into the
// matching message(s) and posts them to the WebView2 surface, so the real
// engine handles hit-testing, focus and scroll targets.

/// Compose a `MAKELPARAM`-style client coordinate from CSS-pixel floats.
fn pack_client_coords(x: f32, y: f32) -> isize {
    let x = (x as i64).clamp(0, 0x7FFF) as u16;
    let y = (y as i64).clamp(0, 0x7FFF) as u16;
    (u32::from(y) << 16 | u32::from(x)) as i32 as isize
}

/// Post a plain mouse message with client coordinates.
fn post_mouse(hwnd: HWND, msg: u32, modifiers: usize, x: f32, y: f32) {
    let _ = unsafe {
        PostMessageW(
            Some(hwnd),
            msg,
            WPARAM(modifiers),
            LPARAM(pack_client_coords(x, y)),
        )
    };
}

/// Post a wheel message; `delta` is in 120ths of a wheel notch and travels in
/// the high word of `wParam`, per `WM_MOUSEWHEEL`/`WM_MOUSEHWHEEL`. The
/// pointer position is taken from the centre of the surface's screen rect so
/// the event always lands on the viewport.
fn post_wheel(hwnd: HWND, msg: u32, delta: i32) {
    let mut rect = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut rect) };
    let (cx, cy) = (
        rect.left + (rect.right - rect.left) / 2,
        rect.top + (rect.bottom - rect.top) / 2,
    );
    let _ = unsafe {
        PostMessageW(
            Some(hwnd),
            msg,
            WPARAM((delta as usize & 0xFFFF) << 16),
            LPARAM(pack_client_coords(cx as f32, cy as f32)),
        )
    };
}

/// `WM_MOUSEWHEEL`/`WM_MOUSEHWHEEL` deltas live in units of `WHEEL_DELTA`
/// (120 = one notch). winit's scroll events arrive in physical pixels, so
/// scale one notch to 120 shares (a pixel-per-notch mapping is the browser
/// convention).
fn wheel_delta_from_scroll(scroll: f32) -> i32 {
    (scroll * 120.0)
        .round()
        .clamp(i32::MIN as f32, i32::MAX as f32) as i32
}

/// Map a platform-independent key name (as used by the core contract) to a
/// Windows virtual-key code. Returns `None` for keys the bridge does not
/// know; those are simply dropped.
fn key_to_vk(key: &str) -> Option<u16> {
    let upper = key.to_ascii_uppercase();
    match upper.as_str() {
        "ENTER" | "RETURN" => Some(0x0D),
        "TAB" => Some(0x09),
        "ESCAPE" | "ESC" => Some(0x1B),
        "SPACE" => Some(0x20),
        "BACKSPACE" => Some(0x08),
        "DELETE" | "DEL" => Some(0x2E),
        "INSERT" => Some(0x2D),
        "HOME" => Some(0x24),
        "END" => Some(0x23),
        "PAGEUP" => Some(0x21),
        "PAGEDOWN" => Some(0x22),
        "ARROWUP" | "UP" => Some(0x26),
        "ARROWDOWN" | "DOWN" => Some(0x28),
        "ARROWLEFT" | "LEFT" => Some(0x25),
        "ARROWRIGHT" | "RIGHT" => Some(0x27),
        "SHIFT" => Some(0x10),
        "CONTROL" | "CTRL" => Some(0x11),
        "ALT" => Some(0x12),
        _ => {
            if let Some(ascii) = single_vk_ascii(&upper) {
                return Some(ascii);
            }
            if let Some(num) = upper.strip_prefix('F').and_then(|s| s.parse::<u8>().ok()) {
                if (1..=24).contains(&num) {
                    return Some(0x70 + num as u16 - 1);
                }
            }
            None
        }
    }
}

/// Single letters/digits map to their ASCII virtual-key code.
fn single_vk_ascii(upper: &str) -> Option<u16> {
    let b = upper.as_bytes();
    if b.len() != 1 {
        return None;
    }
    let byte = b[0];
    match byte {
        b'A'..=b'Z' => Some(byte as u16),
        b'0'..=b'9' => Some(byte as u16),
        _ => None,
    }
}

/// Forward a [`BrowserInput`] to a WebView2 surface by posting the matching
/// Windows messages.
fn forward_browser_input(hwnd: HWND, input: &BrowserInput) -> Result<(), BrowserSourceError> {
    match input {
        BrowserInput::MouseMove { x, y } => {
            post_mouse(hwnd, WM_MOUSEMOVE, 0, *x, *y);
        }
        BrowserInput::MouseButton {
            x,
            y,
            button,
            pressed,
        } => {
            post_mouse(hwnd, WM_MOUSEMOVE, 0, *x, *y);
            let msg = match (button, pressed) {
                (BrowserMouseButton::Left, true) => WM_LBUTTONDOWN,
                (BrowserMouseButton::Left, false) => WM_LBUTTONUP,
                (BrowserMouseButton::Right, true) => WM_RBUTTONDOWN,
                (BrowserMouseButton::Right, false) => WM_RBUTTONUP,
                (BrowserMouseButton::Middle, true) => WM_MBUTTONDOWN,
                (BrowserMouseButton::Middle, false) => WM_MBUTTONUP,
            };
            post_mouse(hwnd, msg, 0, *x, *y);
        }
        BrowserInput::Scroll { x, y } => {
            let dx = wheel_delta_from_scroll(*x);
            let dy = wheel_delta_from_scroll(*y);
            if dy != 0 {
                post_wheel(hwnd, WM_MOUSEWHEEL, dy);
            }
            if dx != 0 {
                post_wheel(hwnd, WM_MOUSEHWHEEL, dx);
            }
        }
        BrowserInput::Key { key, pressed } => {
            let Some(vk) = key_to_vk(key) else {
                return Ok(());
            };
            let wparam = WPARAM(vk as usize);
            let lparam = if *pressed {
                LPARAM(1)
            } else {
                // Bit 31 set marks a key-up transition.
                LPARAM(0xC000_0001usize as isize)
            };
            let _ = unsafe { PostMessageW(Some(hwnd), WM_KEYDOWN, wparam, lparam) };
            if *pressed {
                if let Some(ch) = key_to_char(key) {
                    let _ = unsafe {
                        PostMessageW(Some(hwnd), WM_CHAR, WPARAM(ch as usize), LPARAM(1))
                    };
                }
            } else {
                let _ = unsafe { PostMessageW(Some(hwnd), WM_KEYUP, wparam, lparam) };
            }
        }
    }
    Ok(())
}

/// A printable character for `WM_CHAR`, if the key name is a single glyph.
fn key_to_char(key: &str) -> Option<u16> {
    let upper = key.to_ascii_uppercase();
    let b = upper.as_bytes();
    match b {
        [b'A'..=b'Z'] | [b'0'..=b'9'] => Some(upper.chars().next()? as u16),
        _ => None,
    }
}

#[cfg(test)]
mod input_bridge_tests {
    use super::*;

    #[test]
    fn key_names_map_to_virtual_key_codes() {
        assert_eq!(key_to_vk("Enter"), Some(0x0D));
        assert_eq!(key_to_vk("tab"), Some(0x09));
        assert_eq!(key_to_vk("Esc"), Some(0x1B));
        assert_eq!(key_to_vk("Space"), Some(0x20));
        assert_eq!(key_to_vk("Backspace"), Some(0x08));
        assert_eq!(key_to_vk("ArrowUp"), Some(0x26));
        assert_eq!(key_to_vk("ArrowDown"), Some(0x28));
        assert_eq!(key_to_vk("ArrowLeft"), Some(0x25));
        assert_eq!(key_to_vk("ArrowRight"), Some(0x27));
        assert_eq!(key_to_vk("Delete"), Some(0x2E));
        assert_eq!(key_to_vk("Home"), Some(0x24));
        assert_eq!(key_to_vk("End"), Some(0x23));
        assert_eq!(key_to_vk("Shift"), Some(0x10));
        assert_eq!(key_to_vk("Control"), Some(0x11));
        assert_eq!(key_to_vk("Alt"), Some(0x12));
        assert_eq!(key_to_vk("F5"), Some(0x74));
        assert_eq!(key_to_vk("f12"), Some(0x7B));
        assert_eq!(key_to_vk("a"), Some(0x41));
        assert_eq!(key_to_vk("z"), Some(0x5A));
        assert_eq!(key_to_vk("7"), Some(0x37));
        assert_eq!(key_to_vk("???unknown"), None);
        assert_eq!(key_to_vk(""), None);
    }

    #[test]
    fn printable_keys_yield_wm_char() {
        assert_eq!(key_to_char("a"), Some(0x41));
        assert_eq!(key_to_char("Enter"), None);
        assert_eq!(key_to_char("F5"), None);
    }

    #[test]
    fn client_coords_pack_into_lparam_like_makelparam() {
        // MAKELPARAM(10, 4) → low word 10, high word 4.
        assert_eq!(pack_client_coords(10.0, 4.0), (4i32 << 16 | 10) as isize);
        assert_eq!(pack_client_coords(0.0, 0.0), 0);
        // Negative coords clamp to 0; the low word is the x coordinate.
        require_positive_order();
        fn require_positive_order() {
            let packed = pack_client_coords(1.0, 0.0);
            assert_eq!(packed, 1);
        }
    }

    #[test]
    fn wheel_deltas_scale_to_wheel_units() {
        assert_eq!(wheel_delta_from_scroll(1.0), 120);
        assert_eq!(wheel_delta_from_scroll(-0.5), -60);
        assert_eq!(wheel_delta_from_scroll(0.25), 30);
    }
}

/// Platform extension for [`wry::WebView`] that exposes the child HWND.
use wry::WebViewExtWindows;

#[cfg(test)]
mod tests {
    use super::*;

    /// Skip the spike in CI/headless runs: it needs a desktop session with
    /// WebView2 Runtime. Run locally with `cargo test -p rivulet-browser
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn spike_wry_renders_offscreen_and_yields_browser_frames() {
        // Deterministic, offline page with a recognisable background so we can
        // tell a real frame from the initial blank/white paint. Served over
        // plain HTTP so the core URL validation accepts it.
        let (addr, _server) =
            local_http_server(r#"<html><body style="background:#ca3b7f;margin:0"></body></html>"#);
        let url = format!("http://{addr}");

        let mut backend = WryBrowserBackend::spawn().expect("spawn webview thread");
        let mut source = BrowserSource::new(&url, SPIKE_WIDTH, SPIKE_HEIGHT).expect("source");
        backend
            .navigate(&url)
            .expect("navigate to local spike page");

        let mut seen = None;
        let t0 = std::time::Instant::now();
        let deadline = t0 + std::time::Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            if let Some(frame) = backend.poll_frame().expect("poll") {
                source.submit_frame(frame).expect("core accepts frame");
                let frame = source.latest_frame().expect("latest frame");
                // Look for the pink background colour in the middle rows.
                let cy = (frame.height / 2) as usize;
                let row = &frame.rgba[(cy * frame.width as usize) * 4..];
                let centre = &row[(frame.width as usize / 2) * 4..];
                if centre[0] > 0x90 && centre[1] < 0x80 && centre[2] > 0x20 {
                    seen = Some((frame.width, frame.height, (centre[0], centre[1], centre[2])));
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        let (width, height, (r, g, b)) = seen.expect("got a pink-ish frame within 30s");
        println!("captured frame {width}x{height} centre pixel #{r:02X}{g:02X}{b:02X}");
        assert_eq!(width, SPIKE_WIDTH);
        assert_eq!(height, SPIKE_HEIGHT);
        // The page background is #ca3b7f → red 0xca, no tint from the browser
        // chrome defaults.
        assert!((r as i32 - 0xca).abs() <= 40, "expected pink, got {r:02X}");
        assert!(g < 0x80, "expected pink, got green {g:02X}");
    }

    /// Minimal single-request HTTP server that serves `html` once and then
    /// parks the listener (no threads beyond the caller's poll loop).
    /// Returns the bound `addr` and the (detached) server thread.
    fn local_http_server(
        html: &'static str,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let handle = std::thread::spawn(move || {
            // Serve a handful of requests (page + potential subresources) and
            // then park; WebView2 may open more than one connection.
            for mut stream in listener.incoming().take(8).flatten() {
                // Drain the request line/headers so the browser's request
                // is fully buffered before we answer and close. Otherwise
                // the client may race our `Connection: close`.
                let _ = {
                    use std::io::{BufRead, BufReader};
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut line = String::new();
                    let mut result = Ok(());
                    while line != "\r\n" {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) => break,
                            Ok(_) => {}
                            Err(e) => {
                                result = Err(e);
                                break;
                            }
                        }
                    }
                    result
                };
                let request = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = std::io::Write::write_all(&mut stream, request.as_bytes());
            }
        });
        (addr, handle)
    }
}
