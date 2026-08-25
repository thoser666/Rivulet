# S5a – Browser Source Spike

**Status:** ✅ S5a spike done; S5b portable contract and configuration UI implemented  
**Date:** 2026-08-23  
**Author:** Buffy (Codebuff)

## Goal

Evaluate platform webviews for embedding interactive web content as a scene source in Rivulet.
The browser source should render a URL into a GPU texture, support user interaction
(mouse/keyboard), and work on Windows, Linux, and macOS.

## Platform Webview Matrix

| Platform | WebView | Rust Crate | GPU Rendering | Interaction | Transparency | Maturity |
|----------|---------|-------------|---------------|-------------|--------------|----------|
| Windows  | WebView2 (Chromium) | `wry` / `tao` | ✅ via ICoreWebView2 Composition | ✅ Full | ✅ Supported | Production-ready |
| Linux    | WebKitGTK (WebKit) | `wry` / `webkit2gtk` | ⚠️ Requires offscreen → texture copy | ✅ Full | ⚠️ Limited | Production-ready |
| macOS    | WKWebView (WebKit) | `wry` / `cocoa` | ✅ CALayer-based | ✅ Full | ✅ Supported | Production-ready |

## Recommendation: Use `wry`

[`wry`](https://github.com/nicedoc/wry) is the most mature cross-platform WebView crate for Rust.
It abstracts WebView2 (Windows), WebKitGTK (Linux), and WKWebView (macOS) behind a unified API.

### Key Features
- URL loading, HTML content, custom protocols
- JavaScript evaluation and IPC (JavaScript → Rust)
- User input handling (mouse, keyboard)
- Offscreen rendering (headless mode)
- Window/layer embedding

### GPU Texture Integration

The challenge is getting the webview's rendered output into a GStreamer pipeline as a video frame.

**Approach 1: Offscreen Rendering + Pixel Readback**
```rust
// wry supports offscreen rendering
let webview = WebViewBuilder::new()
    .with_offscreen(true)
    .build()?;

// On each frame, read pixels from the webview surface
let pixels = webview.read_pixels()?; // RGBA buffer
// Feed into GStreamer appsrc
```

**Approach 2: Shared Texture (Windows only)**
- WebView2 supports ICoreWebView2CompositionController for direct composition
- Can share the texture with OBS/Rivulet's render pipeline
- Best performance, but platform-specific

**Approach 3: Screenshot Timer**
- Periodically screenshot the webview surface
- Convert to RGBA and push into pipeline
- Simpler but less performant

### Recommended Approach

**Offscreen Rendering + Pixel Readback** for cross-platform compatibility.
Platform-specific optimizations (shared texture on Windows) can be added later.

## Implementation Sketch

```rust
// rivulet-core/src/browser_source.rs

pub struct BrowserSource {
    url: String,
    width: u32,
    height: u32,
    fps: u32,
    interaction_enabled: bool,
    zoom_level: f64,
    custom_css: Option<String>,
    // Internal wry WebView
    webview: Option<WryWebView>,
}

impl BrowserSource {
    pub fn new(url: &str, width: u32, height: u32) -> Self { ... }
    pub fn navigate(&mut self, url: &str) -> Result<()> { ... }
    pub fn execute_js(&self, script: &str) -> Result<String> { ... }
    pub fn read_pixels(&self) -> Option<Vec<u8>> { ... } // RGBA
    pub fn resize(&mut self, width: u32, height: u32) { ... }
}
```

## Dependencies

```toml
[dependencies]
wry = "0.35"
tao = "0.28"   # Event loop (required by wry)
```

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| WebKitGTK pixel readback is slow | Medium | Throttle updates, use shared memory |
| WebView2 not available on older Windows | Low | WebView2 runtime installer ships with Rivulet |
| CSS/JS injection security | Medium | Sandbox mode, CSP headers |
| Memory usage (Chromium) | High | Offscreen rendering, lazy loading |

## Conclusion

**Go/No-Go:** ✅ Go

Browser source is feasible on all platforms using `wry`. The main challenge is
GPU texture integration, which can be solved with offscreen rendering + pixel readback.

## S5b implementation status

The first S5b slice is now implemented without coupling `rivulet-core` to a
native window/event loop:

- `rivulet-core/src/browser_source.rs` owns the validated URL, viewport, FPS,
  zoom, transparency, custom CSS, bounded interaction queue, and latest RGBA
  frame contract.
- `BrowserSourceBackend` is the adapter boundary for WebView2 on Windows,
  WebKitGTK on Linux, and WKWebView on macOS. An adapter consumes queued
  `BrowserInput` events and submits `BrowserFrame` values to the source.
- The Scenes view exposes URL, viewport, interaction, transparency, zoom, and
  custom-CSS configuration in both supported UI locales.
- Unit tests cover URL policy, viewport/frame validation, serialization,
  interaction back-pressure, navigation invalidation, and settings bounds.

The native webview adapter and GPU texture upload remain the next implementation
slice. Until that adapter is present, the UI deliberately reports that it is
waiting for the webview renderer instead of pretending a browser frame exists.

### Adapter acceptance criteria

A platform adapter can complete S5b when it:

1. creates an off-screen WebView and applies the serialized configuration;
2. forwards pointer/keyboard input only when interaction is enabled;
3. submits correctly sized RGBA frames at or below the configured FPS;
4. preserves alpha when transparency is enabled;
5. runs the same adapter contract tests on Windows, Linux, and macOS.


## References

- [wry GitHub](https://github.com/nicedoc/wry)
- [WebView2 Docs](https://learn.microsoft.com/en-us/web-platform/webview2)
- [WebKitGTK Docs](https://webkitgtk.org/)
- [WKWebView Docs](https://developer.apple.com/documentation/webkit/wkwebview)
- [OBS Browser Source](https://github.com/obsproject/obs-studio/tree/master/plugins/obs-browser)
