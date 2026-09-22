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

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, Arc};

use rivulet_core::browser_source::{
    BrowserFrame, BrowserInput, BrowserSource, BrowserSourceBackend, BrowserSourceError,
};
use webview2_com::CapturePreviewCompletedHandler;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
};
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::Com::IStream;
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
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
    /// No-op placeholder: full input forwarding needs the WebView2
    /// `MouseInput`/`KeyboardInput` API and is out of scope for the spike.
    SendInput(BrowserInput),
    /// Capture a frame with `ICoreWebView2::CapturePreview` and return the
    /// PNG bytes over `tx`.
    Capture(mpsc::Sender<Result<Vec<u8>, String>>),
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
            Command::SetTransparent(_transparent) => {}
            Command::SetInteractionEnabled(_enabled) => {}
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
            Command::SendInput(_input) => {}
            Command::Capture(tx) => {
                if let Some(state) = self.state.as_ref() {
                    capture_preview(&state.webview, tx);
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
}

impl WryBrowserBackend {
    /// Spawn the webview event-loop thread and the WebView2 viewport.
    /// Waits until the webview HWND is published (or ~5s elapse).
    pub fn spawn() -> Result<Self, BrowserSourceError> {
        let hwnd = Arc::new(AtomicIsize::new(0));
        let (proxy_tx, proxy_rx) = std::sync::mpsc::channel::<EventLoopProxy<Command>>();
        let spawned_hwnd = Arc::clone(&hwnd);
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
    /// decode back to RGBA for the core [`BrowserFrame`]. Returns `None` when
    /// the webview is not ready to produce a frame yet
    /// (`ERROR_WRONG_STATE`/timeout), so the poll loop just retries later.
    fn capture_frame(&self) -> Result<Option<BrowserFrame>, BrowserSourceError> {
        let (tx, rx) = mpsc::channel();
        self.send(Command::Capture(tx))?;
        // The PNG is produced on the webview thread; wait for it here.
        let png = match rx.recv_timeout(std::time::Duration::from_secs(5)) {
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
        Ok(Some(BrowserFrame::new(width, height, image.into_raw(), 1)?))
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
fn capture_preview(webview: &wry::WebView, tx: mpsc::Sender<Result<Vec<u8>, String>>) {
    let core: ICoreWebView2 = wry::WebViewExtWindows::webview(webview);
    let stream = match unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true) } {
        Ok(stream) => stream,
        Err(e) => {
            let _ = tx.send(Err(e.to_string()));
            return;
        }
    };
    let stream_for_result = stream.clone();
    let tx_for_handler = tx.clone();
    // Webview writes the PNG into `stream`, then invokes the handler.
    let handler = CapturePreviewCompletedHandler::create(Box::new(move |result| {
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
