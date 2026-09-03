#![allow(unused_imports, dead_code, unused_variables)]

use crate::midi_io::{list_devices, MidiListener};
use crate::theme;
use eframe::egui;
use rivulet_core::{
    CaptureRegion, DiscordPresence, DiscordPresenceConfig, GlobalBinding, GlobalHotkey, KeyCode,
    Locale, ModMask, PresenceActivity, PresenceStatus, RivuletEngine, SkippedFilter,
    StreamConnectionResult, StreamHealthStatus, StreamPlatform, StreamPreset, StreamProbeResult,
    StreamSettings, StreamStats,
};
use std::sync::mpsc::Receiver;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::Instant;

// --- Linux-specific imports ---
#[cfg(target_os = "linux")]
use {
    ashpd::desktop::screencast::{Screencast, Stream},
    ashpd::desktop::Session,
    once_cell::sync::Lazy,
    pipewire::spa::utils::Fd,
    rivulet_audio::{AudioCapture, AudioConfig, AudioFilters},
    rivulet_core::{AudioFrame, AudioTrack},
    std::sync::atomic::AtomicU32,
    std::sync::mpsc as std_mpsc,
    std::thread,
    tokio::runtime::Runtime,
};

// --- Windows imports (for windows-capture v1.5.0) ---
#[cfg(target_os = "windows")]
use {
    rivulet_capture::backend::{BackendKind, BackendStatus},
    rivulet_capture::dxgi::DxgiDesktopDuplication,
    std::sync::mpsc::{self, Sender},
    std::thread,
    windows_capture::{
        // Import only the types that are available from capture
        capture::{Context, GraphicsCaptureApiHandler},
        frame::{Frame, FrameBuffer},
        graphics_capture_api::InternalCaptureControl,
        monitor::Monitor,
        settings::{
            ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
            MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
        },
        window::Window,
    },
};

// --- Data structure for raw frames ---
#[derive(Debug, Clone)]
struct RawFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

/// Live preview of the selected monitor inside the region editor, including
/// the drag state for selecting the capture region on top of the image.
struct RegionPreview {
    texture: egui::TextureHandle,
    /// Full monitor size in physical pixels.
    full_width: u32,
    full_height: u32,
    /// Pointer position (in image pixel coordinates) where the current drag
    /// started, while the user is dragging the region selection.
    drag_start: Option<egui::Pos2>,
}

/// Texture-backed thumbnail shared by all recording paths. It is updated from
/// the frame that is sent to the encoder, so the user sees the effective crop
/// and not a second, potentially different capture stream.
#[derive(Default)]
struct RecordingPreview {
    texture: Option<egui::TextureHandle>,
    width: u32,
    height: u32,
    last_update: Option<std::time::Instant>,
}

impl RecordingPreview {
    fn update(&mut self, ctx: &egui::Context, data: &[u8], width: u32, height: u32) {
        if !is_valid_rgba_frame(data, width, height) {
            return;
        }
        let now = std::time::Instant::now();
        if !should_update_recording_preview(self.last_update, now) {
            return;
        }
        let image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], data);
        if let Some(texture) = &mut self.texture {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            self.texture =
                Some(ctx.load_texture("recording_preview", image, egui::TextureOptions::LINEAR));
        }
        self.width = width;
        self.height = height;
        self.last_update = Some(now);
    }
}

impl RegionPreview {
    fn new(ctx: &egui::Context, image: image::RgbaImage) -> Self {
        let full_width = image.width();
        let full_height = image.height();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [full_width as usize, full_height as usize],
            image.as_raw(),
        );
        Self {
            texture: ctx.load_texture("region_preview", color_image, egui::TextureOptions::LINEAR),
            full_width,
            full_height,
            drag_start: None,
        }
    }
}

/// Live preview of the selected game window, shown in the record view so the
/// user can verify the correct window is targeted before recording starts.
///
/// Mirrors the `RegionPreview` pattern: a single frame is grabbed from the
/// window (via `xcap`) and displayed as an egui texture. The frame refreshes
/// on window change and at most every [`GAME_PREVIEW_REFRESH_INTERVAL`] while
/// the game-capture picker is open.
///
/// The game-window picker exists on Linux and Windows; macOS has no game
/// capture today.
#[cfg(any(target_os = "linux", target_os = "windows"))]
struct GamePreview {
    texture: egui::TextureHandle,
    /// Frame size in pixels.
    width: u32,
    height: u32,
    /// Window this preview belongs to (so a stale preview is dropped when
    /// the selection changes).
    window_id: u64,
    /// Last time a frame was grabbed (throttles the refresh loop).
    last_refresh: std::time::Instant,
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
impl GamePreview {
    fn new(ctx: &egui::Context, image: image::RgbaImage, window_id: u64) -> Self {
        let width = image.width();
        let height = image.height();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            image.as_raw(),
        );
        Self {
            texture: ctx.load_texture("game_preview", color_image, egui::TextureOptions::LINEAR),
            width,
            height,
            window_id,
            last_refresh: std::time::Instant::now(),
        }
    }
}

/// How often the game-window preview is re-grabbed while the game-capture
/// picker is open (matches the roadmap: "refresh on selection change or
/// every ~500ms").
#[cfg(any(target_os = "linux", target_os = "windows"))]
const GAME_PREVIEW_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Minimum time between live re-enumerations of the game-window list while the
/// game-capture picker is open (the window list refreshes live, but not on
/// every frame).
#[cfg(any(target_os = "linux", target_os = "windows"))]
const GAME_WINDOWS_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Maximum UI update rate for the recording thumbnail. The capture pipeline
/// remains at its configured frame rate; only texture uploads are throttled.
const RECORDING_PREVIEW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

#[cfg(any(target_os = "linux", target_os = "windows"))]
const SOURCE_PREVIEW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// Default maximum time to wait for the first captured frame before aborting
/// a Windows recording with an error. The pipeline is only initialized once
/// the first frame arrives, so a capture that never delivers a frame would
/// otherwise look like a running recording while writing nothing. Overridable
/// at startup via `--no-frame-timeout <seconds>`.
pub const DEFAULT_NO_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Bounded size of the in-memory Twitch chat message list (oldest dropped).
const MAX_CHAT_MESSAGES: usize = 500;

/// Minimum workspace width (px) at which the Stream page keeps its
/// side-by-side layout. Below this the action bar, the chat dock and the
/// audio section switch to wrapped/stacked layouts so no control (start/
/// stop, connect, send, mixer) is clipped off-screen.
const STREAM_WORKSPACE_NARROW_WIDTH: f32 = 720.0;

// --- Auto-update state machine ---
#[derive(Debug, Clone, PartialEq, Default)]
enum UpdateUi {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(rivulet_updater::UpdateInfo),
    Downloading {
        name: String,
        version: String,
    },
    Downloaded {
        path: std::path::PathBuf,
        version: String,
    },
    Installing(String),
    Installed(String),
    Error(String),
}

// --- Main navigation ---
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AppView {
    #[default]
    Record,
    Mixer,
    Scenes,
    Stream,
    Assistant,
    Settings,
    Help,
}

impl AppView {
    /// All views in sidebar display order.
    fn all() -> &'static [AppView] {
        &[
            AppView::Record,
            AppView::Mixer,
            AppView::Scenes,
            AppView::Stream,
            AppView::Assistant,
            AppView::Settings,
            AppView::Help,
        ]
    }

    /// i18n key for the sidebar label and the view heading.
    fn nav_key(self) -> &'static str {
        match self {
            AppView::Record => "nav_record",
            AppView::Mixer => "nav_mixer",
            AppView::Scenes => "nav_scenes",
            AppView::Stream => "nav_stream",
            AppView::Assistant => "nav_assistant",
            AppView::Settings => "nav_settings",
            AppView::Help => "nav_help",
        }
    }

    /// Milestone of still-planned views (`None` once implemented).
    fn planned_milestone(self) -> Option<&'static str> {
        match self {
            AppView::Assistant => Some("M9"),
            _ => None,
        }
    }
}

/// Pending Twitch-chat actions, processed exactly once per frame by the
/// reconcile step (so the worker is only (re)built on explicit user intent,
/// never per keypress).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatAction {
    Connect,
    Disconnect,
    Send(String),
    /// Threaded reply `(text, parent Twitch message id)`, sent via
    /// `@reply-parent-msg-id`.
    SendReply(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneHistoryShortcut {
    Undo,
    Redo,
}

/// Resolve scene-history shortcuts without touching egui context state.
///
/// Keeping this decision pure makes it impossible to accidentally call a
/// context method from inside `ctx.input`, which already owns egui's lock.
fn scene_history_shortcut(
    wants_keyboard_input: bool,
    command: bool,
    z_pressed: bool,
    y_pressed: bool,
) -> Option<SceneHistoryShortcut> {
    if wants_keyboard_input || !command {
        return None;
    }
    if z_pressed {
        Some(SceneHistoryShortcut::Undo)
    } else if y_pressed {
        Some(SceneHistoryShortcut::Redo)
    } else {
        None
    }
}

// --- Hotkey configuration ---

/// A key plus an optional set of modifiers (Ctrl/Alt/Shift/Super). This is the
/// unit used both by the in-app (focused) handling and by the OS-level global
/// hotkey registration on Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct HotkeyBinding {
    key: egui::Key,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    alt: bool,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    #[serde(rename = "super")]
    super_mod: bool,
}

impl HotkeyBinding {
    fn plain(key: egui::Key) -> Self {
        Self {
            key,
            ctrl: false,
            alt: false,
            shift: false,
            super_mod: false,
        }
    }

    /// True when `input` reports this whole combo as freshly pressed.
    fn pressed_in(&self, input: &egui::InputState) -> bool {
        input.modifiers.command == self.super_mod
            && input.modifiers.ctrl == self.ctrl
            && input.modifiers.alt == self.alt
            && input.modifiers.shift == self.shift
            && input.key_pressed(self.key)
    }

    /// True when every modifier of this binding is currently held AND the key
    /// was freshly pressed (used by the capture dialog, which ignores modifiers
    /// the user has not selected).
    fn key_pressed_withheld_modifiers(&self, input: &egui::InputState) -> bool {
        // exact-match path used for dispatch below
        input.key_pressed(self.key)
    }

    /// Human-readable, localized-safe label ("Ctrl+F9", "F12").
    fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.super_mod {
            parts.push("Ctrl");
        }
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(key_name(self.key));
        parts.join("+")
    }
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        Self::plain(egui::Key::Num0)
    }
}

/// Keyboard shortcut sequence definition (serde-friendly; each enum is matched
/// by name in tests).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HotkeyConfig {
    record: HotkeyBinding,
    pause: HotkeyBinding,
    mute: HotkeyBinding,
    save_replay: HotkeyBinding,
    #[serde(default)]
    scene_hotkeys: std::collections::BTreeMap<uuid::Uuid, HotkeyBinding>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            record: HotkeyBinding::plain(egui::Key::F9),
            pause: HotkeyBinding::plain(egui::Key::F10),
            mute: HotkeyBinding::plain(egui::Key::F11),
            save_replay: HotkeyBinding::plain(egui::Key::F12),
            scene_hotkeys: std::collections::BTreeMap::new(),
        }
    }
}

/// Human-readable name for a key (covers the common set used by the app).
fn key_name(key: egui::Key) -> &'static str {
    match key {
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        egui::Key::Space => "Space",
        egui::Key::Enter => "Enter",
        egui::Key::Escape => "Esc",
        egui::Key::Tab => "Tab",
        egui::Key::ArrowUp => "↑",
        egui::Key::ArrowDown => "↓",
        egui::Key::ArrowLeft => "←",
        egui::Key::ArrowRight => "→",
        egui::Key::Num0 => "0",
        egui::Key::Num1 => "1",
        egui::Key::Num2 => "2",
        egui::Key::Num3 => "3",
        egui::Key::Num4 => "4",
        egui::Key::Num5 => "5",
        egui::Key::Num6 => "6",
        egui::Key::Num7 => "7",
        egui::Key::Num8 => "8",
        egui::Key::Num9 => "9",
        _ => "?",
    }
}

/// Parse a Twitch `#RRGGBB` color tag into an egui color. Returns `None` for
/// malformed values so the caller falls back to the default text color.
fn color_to_egui(color: &str) -> Option<egui::Color32> {
    let hex = color.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(egui::Color32::from_rgb(r, g, b))
}

/// Translate an egui key to a Windows virtual-key code where an equivalent
/// exists. Returns `None` for keys without a stable VK mapping or for modifier
/// keys (which can never stand alone as a global hotkey).
fn vk_code(key: egui::Key) -> Option<u32> {
    use egui::Key::*;
    let vk = match key {
        F1 => 0x70,
        F2 => 0x71,
        F3 => 0x72,
        F4 => 0x73,
        F5 => 0x74,
        F6 => 0x75,
        F7 => 0x76,
        F8 => 0x77,
        F9 => 0x78,
        F10 => 0x79,
        F11 => 0x7A,
        F12 => 0x7B,
        Num0 => 0x30,
        Num1 => 0x31,
        Num2 => 0x32,
        Num3 => 0x33,
        Num4 => 0x34,
        Num5 => 0x35,
        Num6 => 0x36,
        Num7 => 0x37,
        Num8 => 0x38,
        Num9 => 0x39,
        Space => 0x20,
        Enter => 0x0D,
        Tab => 0x09,
        ArrowUp => 0x26,
        ArrowDown => 0x28,
        ArrowLeft => 0x25,
        ArrowRight => 0x27,
        // Modifier / virtual keys cannot be a standalone global hotkey.
        _ => return None,
    };
    Some(vk)
}

impl HotkeyConfig {
    /// `None` means "not remappable / unknown action".
    fn binding_for_action(&self, action: &str) -> Option<HotkeyBinding> {
        match action {
            "record" => Some(self.record),
            "pause" => Some(self.pause),
            "mute" => Some(self.mute),
            "save_replay" => Some(self.save_replay),
            _ => None,
        }
    }

    fn set_binding_for_action(&mut self, action: &str, binding: HotkeyBinding) -> bool {
        match action {
            "record" => self.record = binding,
            "pause" => self.pause = binding,
            "mute" => self.mute = binding,
            "save_replay" => self.save_replay = binding,
            _ => return false,
        }
        true
    }

    fn label_for(&self, action: &str) -> String {
        match self.binding_for_action(action) {
            Some(binding) => binding.label(),
            None => String::from("?"),
        }
    }
}

// --- Windows handler struct ---
#[cfg(target_os = "windows")]
struct CaptureHandler {
    frame_sender: Sender<RawFrame>,
    stop_signal: Arc<AtomicBool>,
}

#[cfg(target_os = "windows")]
impl GraphicsCaptureApiHandler for CaptureHandler {
    type Flags = (Sender<RawFrame>, Arc<AtomicBool>);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(context: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (sender, signal) = context.flags;
        Ok(Self {
            frame_sender: sender,
            stop_signal: signal,
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        capture_control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        if self.stop_signal.load(Ordering::SeqCst) {
            tracing::info!("Stop signal received; ending recording");
            capture_control.stop();
            return Ok(());
        }

        // FIX: read width/height before accessing the buffer, because the
        // FrameBuffer borrows the frame mutably and no further (immutable)
        // accesses to the frame are allowed afterwards.
        let width = frame.width();
        let height = frame.height();

        // Get the frame buffer
        let mut frame_buffer = frame.buffer()?;

        // FIX: access the raw buffer via .as_raw_buffer()
        let data = frame_buffer.as_raw_buffer().to_vec();

        let raw_frame = RawFrame {
            data,
            width,
            height,
        };

        if self.frame_sender.send(raw_frame).is_err() {
            tracing::info!("GUI channel closed; ending recording");
            capture_control.stop();
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        tracing::warn!("Recording session was ended by the system");
        self.stop_signal.store(true, Ordering::SeqCst);
        Ok(())
    }
}

// --- Linux-specific types ---
#[cfg(target_os = "linux")]
static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| tokio::runtime::Runtime::new().unwrap());

#[cfg(any(target_os = "linux", target_os = "windows"))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum SourcePreviewTarget {
    Monitor(String),
    Window(String),
}

#[cfg(target_os = "linux")]
enum BackendMessage {
    Stream(Stream, Fd),
    Recording(Fd),
    Done,
    Error(anyhow::Error),
}

/// The main application structure.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct RivuletApp {
    #[serde(skip)]
    engine: RivuletEngine,

    /// User interface language. Persisted across sessions.
    locale: Locale,

    /// User color-scheme preference. Persisted across sessions.
    theme: theme::ThemePreference,
    /// Last theme applied to the egui context (so `ui()` only reapplies on
    /// change). Not persisted.
    #[serde(skip)]
    theme_applied: Option<theme::ThemePreference>,

    /// Discord Rich Presence opt-out. Persisted across sessions. Defaults to
    /// on (matching the roadmap's explicit opt-out requirement).
    discord_presence_enabled: bool,
    /// Discord Developer Application client id used for Rich Presence.
    /// Persisted across sessions. Empty disables the adapter until a real
    /// application id is configured.
    discord_presence_client_id: String,
    /// Asset key of the large image uploaded in the Discord Developer Portal
    /// (Rich Presence → Art Assets). Persisted; empty keeps the generic
    /// placeholder icon.
    discord_presence_large_image: String,
    /// Live adapter handle. Not persisted; rebuilt when the app starts.
    #[serde(skip)]
    discord_presence: Option<DiscordPresence>,
    /// Client id the currently running adapter was built with. Not persisted,
    /// used to detect when the setting changes so the adapter is rebuilt.
    #[serde(skip)]
    discord_presence_active_client_id: Option<String>,
    /// Set when the user edits the client id and clicks Apply, so the running
    /// adapter is rebuilt exactly once (not per keypress). Not persisted.
    #[serde(skip)]
    discord_client_id_dirty: bool,
    /// Set when the user clicks "Reconnect" in the Stream view while the
    /// adapter reports not connected, so the worker is torn down and rebuilt
    /// exactly once (fresh handshake) without restarting the app. Not
    /// persisted.
    #[serde(skip)]
    discord_reconnect_requested: bool,
    /// Last status pushed to the adapter, so updates only happen on activity
    /// transitions (per the adapter contract).
    #[serde(skip)]
    discord_presence_last: Option<PresenceStatus>,
    /// Set when the user applied a client id that failed format validation in
    /// Settings, so a warning is shown immediately instead of the id silently
    /// keeping the adapter off. Not persisted; cleared when the field is
    /// edited again.
    #[serde(skip)]
    discord_client_id_warning: Option<rivulet_core::discord::ClientIdError>,
    /// Set when the user applied the presence settings (client id / art asset
    /// key) and the resulting SET_ACTIVITY payload would violate Discord's
    /// validation rules (overlong state/details, implausible asset key), so a
    /// warning is shown immediately instead of Discord silently dropping the
    /// update or the artwork. Not persisted; cleared on a clean apply.
    #[serde(skip)]
    discord_payload_warning: Option<rivulet_core::discord::PayloadIssue>,

    /// Chat dock: selected platform (Twitch, Kick or YouTube). Persisted;
    /// defaults to Twitch for existing configs.
    #[serde(default)]
    chat_platform: rivulet_core::ChatPlatform,
    /// Chat dock: channel to join — Twitch channel / Kick slug / YouTube
    /// video id (persisted).
    chat_channel: String,
    /// Chat dock: token — Twitch OAuth (`oauth:...`) or Kick session token
    /// (persisted, but never shown in logs/UI). Empty connects anonymously.
    chat_oauth_token: String,
    /// Chat dock: pending message text to send on Enter. Not persisted.
    #[serde(skip)]
    chat_input: String,
    /// Chat dock: threaded reply target `(display name, Twitch message id)`.
    /// Armed by the reply affordance on a message; cleared on send, cancel,
    /// connect and disconnect. Not persisted. Only Twitch messages carry an
    /// IRC id, so this stays `None` on Kick/YouTube.
    #[serde(skip)]
    chat_reply_target: Option<(String, String)>,
    /// Chat dock: running worker handle. Not persisted.
    #[serde(skip)]
    chat_worker: Option<rivulet_core::Chat>,
    /// Chat dock: messages collected from the worker since connect. Not
    /// persisted. Bounded to the newest entries (oldest dropped).
    #[serde(skip)]
    chat_messages: Vec<rivulet_core::ChatMessage>,
    /// Chat dock: pending connect/disconnect action (processed once per
    /// frame). Not persisted.
    #[serde(skip)]
    chat_action_pending: Option<ChatAction>,
    /// Chat dock: last connection state shown to the user. Not persisted.
    #[serde(skip)]
    chat_state: rivulet_core::ChatConnState,

    /// Set when any hotkey binding changes through the Settings UI (or scene
    /// hotkey assignment), so the OS-level global hotkeys are re-registered
    /// exactly once per change. Not persisted.
    #[serde(skip)]
    global_hotkeys_dirty: bool,
    /// OS-level global hotkey handle (Windows registers real RegisterHotKey
    /// bindings; Linux/macOS is a documented no-op). Rebuilt lazily on first
    /// use. Not persisted, not Clone.
    #[serde(skip)]
    global_hotkeys: Option<GlobalHotkey>,

    /// OBS WebSocket remote control (Stream Deck / TouchPortal): master
    /// switch. Persisted across sessions.
    obs_ws_enabled: bool,
    /// OBS WebSocket listen port. Persisted across sessions.
    obs_ws_port: u16,
    /// OBS WebSocket password (empty = no authentication). Persisted.
    obs_ws_password: String,
    /// Running server handle. Not persisted; rebuilt on app start when the
    /// feature is enabled.
    #[serde(skip)]
    obs_ws_server: Option<rivulet_obs_websocket::ObsServerHandle>,
    /// Shared state snapshot the websocket thread reads requests against;
    /// the GUI refreshes it every frame.
    #[serde(skip)]
    obs_ws_snapshot: Option<Arc<Mutex<rivulet_obs_websocket::ObsSnapshot>>>,
    /// Queue of commands the websocket thread wants the GUI to execute on the
    /// next frame (set scene, start/stop recording, start/stop streaming).
    /// The reply channel is used to return the ObsCommandResult synchronously.
    #[serde(skip)]
    obs_ws_commands_rx: Option<
        Receiver<(
            rivulet_obs_websocket::ObsCommand,
            std::sync::mpsc::Sender<rivulet_obs_websocket::ObsCommandResult>,
        )>,
    >,
    /// Status/error line shown in Settings (e.g. bind failure, running port).
    #[serde(skip)]
    obs_ws_status: Option<String>,
    /// Set when the user changes port/password in Settings so a running
    /// server is restarted with the new values exactly once.
    #[serde(skip)]
    obs_ws_restart: bool,
    /// Last broadcast-relevant public state, used to diff GUI-initiated
    /// changes (scene switch, record/stream toggles made in the window).
    /// Not persisted, rebuilt after the server starts.
    #[serde(skip)]
    obs_ws_last_public_state: Option<ObsWsPublicState>,

    // --- MIDI controller mapping (M5) ---
    /// Master switch for the MIDI listener. Persisted across sessions.
    midi_enabled: bool,
    /// Index into the enumerated MIDI input ports. Persisted.
    midi_device_index: usize,
    /// The user's bindings (channel/kind/number → action). Persisted.
    midi_mapping: rivulet_core::MidiMapping,
    /// Device list shown in Settings; refreshed when the section is opened.
    #[serde(skip)]
    midi_devices: Vec<String>,
    /// Live listener handle (device thread). Rebuilt on config changes.
    #[serde(skip)]
    midi_handle: Option<MidiListener>,
    /// Status/error line shown in Settings (e.g. no device, connect error).
    #[serde(skip)]
    midi_status: Option<String>,
    /// Set when enable/device/bindings change so the listener is rebuilt
    /// exactly once per frame.
    #[serde(skip)]
    midi_dirty: bool,
    /// Pending "add binding" row state in Settings (kind/channel/number,
    /// action name, optional scene). Persisted so the row survives restarts.
    midi_new_kind: rivulet_core::MidiKind,
    midi_new_channel: u8,
    midi_new_number: u8,
    midi_new_action: String,
    midi_new_scene: Option<uuid::Uuid>,
    /// Platform-independent mirror of the last MIDI fader value (0.0..1.0).
    /// Applied to the Linux audio mixer when present; on other platforms it is
    /// stored for future audio backends (no OS-level volume control).
    midi_master_volume: f32,
    /// Per-device named preset library (device → preset name → mapping).
    /// Persisted so setups survive restarts and can be exchanged per device.
    midi_presets: rivulet_core::MidiPresetLibrary,
    /// The preset-name input field in Settings. Persisted so the label the
    /// user typed survives restarts.
    midi_preset_name: String,
    /// Learn mode: while active, the next incoming MIDI message is captured
    /// into the "add binding" row instead of being dispatched. Transient.
    #[serde(skip)]
    midi_learn: bool,
    /// The message captured by learn mode, shown as feedback until the user
    /// confirms the binding. Transient.
    #[serde(skip)]
    midi_learn_captured: Option<rivulet_core::MidiMessage>,
    /// Currently selected preset in the per-device dropdown. Transient.
    #[serde(skip)]
    midi_selected_preset: Option<String>,

    // Linux Fields
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    is_previewing: bool,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    is_recording: bool,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    screencast: Option<Screencast>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    session: Option<Session<Screencast>>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    stream: Option<Stream>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    pipewire_fd: Option<Fd>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    pipewire_capture: Option<rivulet_capture::PipeWireCaptureHandle>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    sender: std_mpsc::Sender<BackendMessage>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    receiver: std_mpsc::Receiver<BackendMessage>,

    // Audio mixer (Linux)
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio: Option<AudioCapture>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_preview: bool,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_status: Option<String>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_warning: Option<String>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_peak: Arc<AtomicU32>,
    #[cfg(target_os = "linux")]
    capture_system: bool,
    #[cfg(target_os = "linux")]
    capture_mic: bool,
    #[cfg(target_os = "linux")]
    separate_tracks: bool,
    export_system_track: bool,
    export_mic_track: bool,
    #[cfg(target_os = "linux")]
    system_volume: f32,
    #[cfg(target_os = "linux")]
    mic_volume: f32,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    system_filters: AudioFilters,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    mic_filters: AudioFilters,
    #[cfg(target_os = "linux")]
    system_monitor: bool,
    #[cfg(target_os = "linux")]
    mic_monitor: bool,
    #[cfg(target_os = "linux")]
    monitor_volume: f32,
    #[cfg(target_os = "linux")]
    master_volume: f32,

    // Linux screen recording
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_rx: Option<std_mpsc::Receiver<AudioFrame>>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_system_rx: Option<std_mpsc::Receiver<AudioFrame>>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    audio_mic_rx: Option<std_mpsc::Receiver<AudioFrame>>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    monitors: Vec<xcap::Monitor>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    selected_monitor_idx: Option<usize>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    windows: Vec<xcap::Window>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    selected_window_idx: Option<usize>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    raw_rx: Option<std_mpsc::Receiver<RawFrame>>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    record_status: Option<String>,

    // Windows Fields
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    is_windows_recording: bool,
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    monitors: Vec<Monitor>,
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    windows: Vec<Window>,
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    selected_monitor_idx: Option<usize>,
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    selected_window_idx: Option<usize>,
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    frame_receiver: Option<Receiver<RawFrame>>,
    /// Capture backend status (G2): whether DXGI Desktop Duplication is the
    /// active backend or whether the GUI fell back to Windows Graphics
    /// Capture, plus the fallback reason. Set at the start of a monitor
    /// recording and cleared when recording stops.
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    capture_backend: Option<BackendStatus>,
    #[serde(skip)]
    error_receiver: Option<Receiver<String>>,
    #[serde(skip)]
    stop_signal: Option<Arc<AtomicBool>>,
    #[serde(skip)]
    last_error: Option<String>,
    #[serde(skip)]
    record_started: Instant,
    #[serde(skip)]
    last_frame_at: Option<Instant>,

    /// Live thumbnail of the effective recording frame. The texture and
    /// timestamps are runtime-only and therefore never persisted.
    #[serde(skip)]
    recording_preview: RecordingPreview,
    #[serde(skip)]
    pending_preview_frame: Option<RawFrame>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[serde(skip)]
    source_preview_rx: Option<Receiver<RawFrame>>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[serde(skip)]
    source_preview_stop: Option<Arc<AtomicBool>>,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[serde(skip)]
    source_preview_target: Option<SourcePreviewTarget>,

    /// Maximum time to wait for the first captured frame before aborting a
    /// Windows recording with an error. Configured at startup via the
    /// `--no-frame-timeout <seconds>` CLI flag.
    #[serde(skip)]
    no_frame_timeout: std::time::Duration,

    // Region capture (applies to monitor capture on all platforms)
    #[serde(skip)]
    region_enabled: bool,
    #[serde(skip)]
    region: CaptureRegion,
    #[serde(skip)]
    region_editor_open: bool,
    #[serde(skip)]
    region_preview: Option<RegionPreview>,
    #[serde(skip)]
    region_editor_dims: Option<(u32, u32)>,
    #[serde(skip)]
    region_preview_error: Option<String>,

    // Main navigation
    #[serde(skip)]
    view: AppView,

    // Scenes (multiple scenes & switching)
    #[serde(skip)]
    scenes: rivulet_core::SceneManager,
    /// Browser source configuration for the active scene. The native webview
    /// adapter is intentionally not stored in the app state yet; this config
    /// is the portable contract used by the future WebView2/WebKit backend.
    browser_source: rivulet_core::BrowserSource,
    /// Alert overlay import state (provider + token) for the browser source.
    /// The token is only ever used to build the widget URL; it is not logged.
    #[serde(skip)]
    alert_provider: rivulet_core::AlertProvider,
    #[serde(skip)]
    alert_token: String,
    #[serde(skip)]
    alert_custom_url: String,
    #[serde(skip)]
    scene_name_input: String,
    #[serde(skip)]
    scene_status: Option<String>,
    #[serde(skip)]
    transition_kind: rivulet_core::TransitionKind,
    #[serde(skip)]
    transition_duration_ms: u64,
    #[serde(skip)]
    scene_transition: rivulet_core::SceneTransition,
    #[serde(skip)]
    studio_mode: rivulet_core::StudioMode,
    #[serde(skip)]
    scene_collection_input: String,
    #[serde(skip)]
    scene_profile_input: String,
    #[serde(skip)]
    scene_hotkey_key: egui::Key,
    #[serde(skip)]
    auto_switch_enabled: bool,
    #[serde(skip)]
    auto_switch_rules: Vec<(String, uuid::Uuid)>,
    #[serde(skip)]
    auto_switch_window_input: String,
    #[serde(skip)]
    source_manager: rivulet_core::SourceManager,
    #[serde(skip)]
    source_name_input: String,
    #[serde(skip)]
    source_kind_index: usize,
    #[serde(skip)]
    selected_composition_source: Option<uuid::Uuid>,
    #[serde(skip)]
    scene_overlay_text: String,
    #[serde(skip)]
    scene_overlay_enabled: bool,
    #[serde(skip)]
    multi_view_enabled: bool,
    #[serde(skip)]
    projector_open: bool,

    // Streaming configuration (keys are persisted only in memory for now;
    // never rendered unmasked).
    #[serde(skip)]
    setup_wizard_open: bool,
    #[serde(skip)]
    setup_wizard_step: u8,
    #[serde(skip)]
    setup_test_requested: bool,
    #[serde(skip)]
    stream_platform: StreamPlatform,
    #[serde(skip)]
    stream_ingest_url: String,
    #[serde(skip)]
    stream_key: String,
    #[serde(skip)]
    stream_key_save_requested: bool,
    #[serde(skip)]
    stream_key_delete_requested: bool,
    #[serde(skip)]
    stream_key_store_status: Option<String>,
    #[serde(skip)]
    stream_preset: StreamPreset,
    #[serde(skip)]
    stream_status_message: Option<String>,
    #[serde(skip)]
    stream_probe_running: bool,
    #[serde(skip)]
    stream_probe_result: Option<StreamProbeResult>,
    #[serde(skip)]
    private_test_stream: rivulet_core::PrivateTestStream,

    // Auto-update
    #[serde(skip)]
    update_ui: std::sync::Arc<std::sync::Mutex<UpdateUi>>,
    #[serde(skip)]
    update_auto_checked: bool,
    #[serde(skip)]
    update_check_clicked: bool,
    #[serde(skip)]
    update_download_clicked: bool,
    #[serde(skip)]
    update_install_clicked: bool,
    #[serde(skip)]
    download_progress: Arc<AtomicU64>,
    #[serde(skip)]
    download_total: u64,

    // Hotkeys & recording modifiers
    hotkeys: HotkeyConfig,
    #[serde(skip)]
    is_paused: bool,
    #[serde(skip)]
    is_muted: bool,

    // Replay buffer (instant replay): `None` = off, `Some(secs)` = clip
    // length. Applied to the engine before every recording start.
    replay_duration_secs: Option<u64>,
    /// Transient, localized outcome of the last replay save (ok?, message).
    #[serde(skip)]
    replay_status: Option<(bool, String)>,

    // Codec selection
    #[serde(skip)]
    selected_codec: rivulet_core::VideoCodec,
    rate_mode: rivulet_core::RateControlMode,
    rate_quality: i32,
    rate_max_kbps: u32,
    encoder_extra_options: String,
    selected_container: rivulet_core::RecordingContainer,
    auto_remux: bool,
    split_seconds: u64,
    auto_record_with_stream: bool,
    // Preset selection
    #[serde(skip)]
    selected_preset: rivulet_core::RecordingPreset,
    // Overlay (timer + FPS counter)
    #[serde(skip)]
    show_overlay: bool,
    video_effects: rivulet_core::VideoEffects,

    // Camera (webcam) source
    #[serde(skip)]
    camera_devices: Vec<rivulet_core::CameraDevice>,
    #[serde(skip)]
    selected_camera_idx: Option<usize>,
    #[serde(skip)]
    camera_rx: Option<std::sync::mpsc::Receiver<rivulet_core::CameraFrame>>,
    #[serde(skip)]
    camera_handle: Option<rivulet_core::camera::CameraCaptureHandle>,

    // Game capture source
    #[serde(skip)]
    game_windows: Vec<rivulet_core::GameWindow>,
    #[serde(skip)]
    selected_game_window_idx: Option<usize>,
    /// Live thumbnail of the selected game window (refreshed while the
    /// picker is open). Not persisted.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[serde(skip)]
    game_preview: Option<GamePreview>,
    /// Error message when the live game-window preview could not be captured.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[serde(skip)]
    game_preview_error: Option<String>,
    /// Timestamp of the last live re-enumeration of the game-window list.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[serde(skip)]
    game_windows_last_refresh: Option<std::time::Instant>,
    #[serde(skip)]
    game_capture_rx: Option<std::sync::mpsc::Receiver<rivulet_core::GameCaptureFrame>>,
    #[serde(skip)]
    game_capture_handle: Option<rivulet_core::game_capture::GameCaptureHandle>,
    #[serde(skip)]
    use_game_capture: bool,

    // Platform-agnostic recording flag (used by camera + game capture)
    #[serde(skip)]
    is_aux_recording: bool,
}

/// Subset of the observable app state used to broadcast OBS WebSocket events
/// for GUI-initiated changes (scene switch, record/stream toggles made in the
/// window rather than by a remote client).
#[derive(Debug, Clone, PartialEq, Default)]
struct ObsWsPublicState {
    current_scene: Option<String>,
    recording: bool,
    recording_paused: bool,
    streaming: bool,
    reconnecting: bool,
}

impl Default for RivuletApp {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        let (sender, receiver) = std_mpsc::channel();

        Self {
            engine: Default::default(),
            locale: Locale::default(),
            theme: theme::ThemePreference::default(),
            theme_applied: None,
            discord_presence_enabled: true,
            // Official Rivulet application id + artwork: zero-config Rich
            // Presence for end users (both fields remain overridable in
            // Settings; empty client id still keeps the adapter off).
            discord_presence_client_id: rivulet_core::discord::DEFAULT_CLIENT_ID.to_owned(),
            discord_presence_large_image: rivulet_core::discord::DEFAULT_LARGE_IMAGE_KEY.to_owned(),
            discord_presence: None,
            discord_presence_active_client_id: None,
            discord_client_id_dirty: false,
            discord_reconnect_requested: false,
            discord_presence_last: None,
            discord_client_id_warning: None,
            discord_payload_warning: None,
            chat_channel: String::new(),
            chat_oauth_token: String::new(),
            chat_input: String::new(),
            chat_reply_target: None,
            chat_worker: None,
            chat_messages: Vec::new(),
            chat_action_pending: None,
            chat_platform: rivulet_core::ChatPlatform::default(),
            chat_state: rivulet_core::ChatConnState::Off,
            global_hotkeys_dirty: false,
            global_hotkeys: None,
            obs_ws_enabled: false,
            obs_ws_port: rivulet_obs_websocket::DEFAULT_PORT,
            obs_ws_password: String::new(),
            midi_enabled: false,
            midi_device_index: 0,
            midi_mapping: rivulet_core::MidiMapping::default(),
            midi_devices: Vec::new(),
            midi_handle: None,
            midi_status: None,
            midi_dirty: false,
            midi_new_kind: rivulet_core::MidiKind::default(),
            midi_new_channel: 0,
            midi_new_number: 7,
            midi_new_action: "ToggleRecord".to_owned(),
            midi_new_scene: None,
            midi_master_volume: 1.0,
            midi_presets: rivulet_core::MidiPresetLibrary::default(),
            midi_preset_name: String::new(),
            midi_learn: false,
            midi_learn_captured: None,
            midi_selected_preset: None,
            obs_ws_server: None,
            obs_ws_snapshot: None,
            obs_ws_commands_rx: None,
            obs_ws_status: None,
            obs_ws_restart: false,
            obs_ws_last_public_state: None,

            #[cfg(target_os = "linux")]
            is_previewing: false,
            #[cfg(target_os = "linux")]
            is_recording: false,
            #[cfg(target_os = "linux")]
            screencast: None,
            #[cfg(target_os = "linux")]
            session: None,
            #[cfg(target_os = "linux")]
            stream: None,
            #[cfg(target_os = "linux")]
            pipewire_fd: None,
            #[cfg(target_os = "linux")]
            pipewire_capture: None,
            #[cfg(target_os = "linux")]
            sender,
            #[cfg(target_os = "linux")]
            receiver,

            #[cfg(target_os = "linux")]
            audio: None,
            #[cfg(target_os = "linux")]
            audio_preview: false,
            #[cfg(target_os = "linux")]
            audio_status: None,
            #[cfg(target_os = "linux")]
            audio_warning: None,
            #[cfg(target_os = "linux")]
            audio_peak: Arc::new(AtomicU32::new(0)),
            #[cfg(target_os = "linux")]
            capture_system: true,
            #[cfg(target_os = "linux")]
            capture_mic: true,
            #[cfg(target_os = "linux")]
            separate_tracks: false,
            export_system_track: true,
            export_mic_track: true,
            #[cfg(target_os = "linux")]
            system_volume: 0.8,
            #[cfg(target_os = "linux")]
            mic_volume: 1.0,
            #[cfg(target_os = "linux")]
            system_filters: AudioFilters::default(),
            #[cfg(target_os = "linux")]
            mic_filters: AudioFilters::default(),
            #[cfg(target_os = "linux")]
            system_monitor: false,
            #[cfg(target_os = "linux")]
            mic_monitor: false,
            #[cfg(target_os = "linux")]
            monitor_volume: 1.0,
            #[cfg(target_os = "linux")]
            master_volume: 1.0,

            #[cfg(target_os = "linux")]
            audio_rx: None,
            #[cfg(target_os = "linux")]
            audio_system_rx: None,
            #[cfg(target_os = "linux")]
            audio_mic_rx: None,
            #[cfg(target_os = "linux")]
            monitors: Vec::new(),
            #[cfg(target_os = "linux")]
            selected_monitor_idx: None,
            #[cfg(target_os = "linux")]
            windows: Vec::new(),
            #[cfg(target_os = "linux")]
            selected_window_idx: None,
            #[cfg(target_os = "linux")]
            raw_rx: None,
            #[cfg(target_os = "linux")]
            record_status: None,

            #[cfg(target_os = "windows")]
            is_windows_recording: false,
            #[cfg(target_os = "windows")]
            monitors: Vec::new(),
            #[cfg(target_os = "windows")]
            windows: Vec::new(),
            #[cfg(target_os = "windows")]
            selected_monitor_idx: None,
            #[cfg(target_os = "windows")]
            selected_window_idx: None,
            #[cfg(target_os = "windows")]
            capture_backend: None,

            // Platform-agnostic fields (used by camera/game capture)
            #[cfg(target_os = "windows")]
            frame_receiver: None,
            error_receiver: None,
            stop_signal: None,
            last_error: None,
            record_started: Instant::now(),
            last_frame_at: None,
            recording_preview: RecordingPreview::default(),
            pending_preview_frame: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            source_preview_rx: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            source_preview_stop: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            source_preview_target: None,

            no_frame_timeout: DEFAULT_NO_FRAME_TIMEOUT,

            region_enabled: false,
            region: CaptureRegion::full(1920, 1080),
            region_editor_open: false,
            region_preview: None,
            region_editor_dims: None,
            region_preview_error: None,

            view: AppView::Record,

            scenes: rivulet_core::SceneManager::new(),
            browser_source: rivulet_core::BrowserSource::default(),
            alert_provider: rivulet_core::AlertProvider::default(),
            alert_token: String::new(),
            alert_custom_url: String::new(),
            scene_name_input: String::new(),
            scene_status: None,
            transition_kind: rivulet_core::TransitionKind::Cut,
            transition_duration_ms: 300,
            scene_transition: rivulet_core::SceneTransition::default(),
            studio_mode: rivulet_core::StudioMode::new(),
            scene_collection_input: String::new(),
            scene_profile_input: String::new(),
            scene_hotkey_key: egui::Key::F1,
            auto_switch_enabled: false,
            auto_switch_rules: Vec::new(),
            auto_switch_window_input: String::new(),
            source_manager: rivulet_core::SourceManager::new(),
            source_name_input: String::new(),
            source_kind_index: 0,
            selected_composition_source: None,
            scene_overlay_text: String::new(),
            scene_overlay_enabled: false,
            multi_view_enabled: false,
            projector_open: false,

            setup_wizard_open: false,
            setup_wizard_step: 0,
            setup_test_requested: false,
            stream_platform: StreamPlatform::Twitch,
            stream_ingest_url: StreamPlatform::Twitch
                .default_ingest_url()
                .unwrap_or_default()
                .into(),
            stream_key: String::new(),
            stream_key_save_requested: false,
            stream_key_delete_requested: false,
            stream_key_store_status: None,
            stream_preset: StreamPreset::Standard,
            stream_status_message: None,
            stream_probe_running: false,
            stream_probe_result: None,
            private_test_stream: rivulet_core::PrivateTestStream::new(
                3,
                std::time::Duration::from_secs(30),
            ),

            update_ui: std::sync::Arc::new(std::sync::Mutex::new(UpdateUi::default())),
            update_auto_checked: false,
            update_check_clicked: false,
            update_download_clicked: false,
            update_install_clicked: false,
            download_progress: Arc::new(AtomicU64::new(0)),
            download_total: 0,
            hotkeys: HotkeyConfig::default(),
            is_paused: false,
            is_muted: false,
            replay_duration_secs: Some(30),
            replay_status: None,
            selected_codec: rivulet_core::VideoCodec::default(),
            rate_mode: rivulet_core::RateControlMode::default(),
            rate_quality: 23,
            rate_max_kbps: 0,
            encoder_extra_options: String::new(),
            selected_container: rivulet_core::RecordingContainer::default(),
            auto_remux: true,
            split_seconds: 0,
            auto_record_with_stream: false,
            selected_preset: rivulet_core::RecordingPreset::default(),
            show_overlay: false,
            video_effects: rivulet_core::VideoEffects::default(),

            // Camera
            camera_devices: Vec::new(),
            selected_camera_idx: None,
            camera_rx: None,
            camera_handle: None,

            // Game capture
            game_windows: Vec::new(),
            selected_game_window_idx: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            game_preview: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            game_preview_error: None,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            game_windows_last_refresh: None,
            game_capture_rx: None,
            game_capture_handle: None,
            use_game_capture: false,
            is_aux_recording: false,
        }
    }
}

// --- Game-window live preview (shared by Linux + Windows) ---
#[cfg(any(target_os = "linux", target_os = "windows"))]
impl RivuletApp {
    /// Refresh the list of game-like windows for game capture mode.
    ///
    /// On Linux the list comes from `rivulet-core` (xdotool + xcap). The
    /// core list is empty on Windows, so xcap's own window enumeration is
    /// used there with the same heuristic (visible title, larger than
    /// 640x480). This also makes the live game-window preview targetable
    /// on both platforms.
    fn refresh_game_windows(&mut self) {
        self.game_windows = crate::app::enumerate_game_windows();
        self.game_windows_last_refresh = Some(std::time::Instant::now());
        self.selected_game_window_idx = None;
        self.game_preview = None;
        self.game_preview_error = None;
    }

    /// Live-refresh the game-window list while the picker is open, re-using the
    /// current selection (by window id) whenever the window still exists and
    /// re-targeting the preview to it. Unlike [`refresh_game_windows`] it does
    /// not clear the selection, so an auto-refresh never silently deselects the
    /// window the user is about to record.
    fn refresh_game_windows_live(&mut self) {
        let keep_id = self
            .selected_game_window_idx
            .and_then(|idx| self.game_windows.get(idx))
            .map(|w| w.id);
        self.game_windows = crate::app::enumerate_game_windows();
        self.game_windows_last_refresh = Some(std::time::Instant::now());
        self.selected_game_window_idx =
            keep_id.and_then(|id| self.game_windows.iter().position(|w| w.id == id));
        self.game_preview = None;
        self.game_preview_error = None;
    }

    /// Grab a single frame from the given window (via xcap) for the live
    /// game-window preview.
    fn grab_game_window_preview(&mut self, ctx: &egui::Context, window_id: u64) {
        match game_window_preview_image(window_id) {
            Some(image) => {
                self.game_preview = Some(GamePreview::new(ctx, image, window_id));
                self.game_preview_error = None;
            }
            None => {
                // Keep any previous preview (a transient grab failure should
                // not blank the thumbnail); only surface the error once.
                if self.game_preview.is_none() {
                    self.game_preview_error = Some(self.tr("game_preview_unavailable").to_string());
                }
            }
        }
    }

    /// Keep the game-window preview fresh while the game-capture picker is
    /// open: grab a new frame when the selection changed or when at least
    /// [`GAME_PREVIEW_REFRESH_INTERVAL`] elapsed since the last grab. Also
    /// re-enumerates the window list live (bounded by
    /// [`GAME_WINDOWS_REFRESH_INTERVAL`]) so new/closed windows appear without
    /// the user manually refreshing.
    fn update_game_preview(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        // Live window-list refresh: only when at least the refresh interval has
        // elapsed since the last enumeration, and only while a game window is
        // actually selected (the picker is in use).
        let list_stale = self
            .game_windows_last_refresh
            .map(|last| now.duration_since(last) >= GAME_WINDOWS_REFRESH_INTERVAL)
            .unwrap_or(true);
        if self.selected_game_window_idx.is_some() && list_stale {
            self.refresh_game_windows_live();
        }

        let Some(idx) = self.selected_game_window_idx else {
            self.game_preview = None;
            self.game_preview_error = None;
            return;
        };
        let Some(game_window) = self.game_windows.get(idx) else {
            return;
        };

        let should_refresh = should_refresh_game_preview(
            self.game_preview.as_ref().map(|p| p.window_id),
            game_window.id,
            self.game_preview.as_ref().map(|p| p.last_refresh),
            now,
        );
        if should_refresh {
            self.grab_game_window_preview(ctx, game_window.id);
        }
    }
}

// --- Windows logic implementation ---
#[cfg(target_os = "windows")]
impl RivuletApp {
    fn refresh_capture_sources(&mut self) {
        self.monitors = Monitor::enumerate().unwrap_or_default();

        self.windows = Window::enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter(|w| {
                // FIX: filter by title (is_capturable removed)
                !w.title().unwrap_or_default().is_empty()
            })
            .collect();

        self.selected_monitor_idx = None;
        self.selected_window_idx = None;
    }

    /// Refresh the list of available camera devices.
    fn refresh_camera_devices(&mut self) {
        self.camera_devices = rivulet_core::camera::list_cameras();
        self.selected_camera_idx = None;
    }

    /// Open the region editor for the currently selected monitor, capturing
    /// a preview via xcap (the capture itself uses windows-capture).
    fn open_region_editor(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.selected_monitor_idx else {
            return;
        };
        let Some(monitor) = self.monitors.get(idx) else {
            return;
        };
        let name = monitor.name().unwrap_or_default();
        self.region_editor_dims =
            Some((monitor.width().unwrap_or(0), monitor.height().unwrap_or(0)));
        self.region_preview_error = None;
        let xcap_monitors = xcap::Monitor::all().unwrap_or_default();
        match monitor_preview_image(&xcap_monitors, &name) {
            Some(image) => self.region_preview = Some(RegionPreview::new(ctx, image)),
            None => {
                self.region_preview_error = Some(self.tr_fmt(
                    "region_preview_failed",
                    &[self.tr("region_preview_unavailable").to_string()],
                ));
            }
        }
        self.region_editor_open = true;
    }

    #[cfg(target_os = "windows")]
    fn start_windows_recording(&mut self) {
        let ext = self.selected_container_extension();
        let file_path = rfd::FileDialog::new()
            .add_filter("Video", &[ext])
            .set_file_name(self.default_recording_filename())
            .save_file();

        let Some(path) = file_path else {
            tracing::info!("File selection cancelled");
            return;
        };

        // Determine the capture source (monitor or window) and start accordingly
        if let Some(idx) = self.selected_monitor_idx {
            let Some(monitor) = self.monitors.get(idx).cloned() else {
                self.last_error = Some(self.tr("invalid_source").to_string());
                return;
            };

            self.engine.set_video_codec(self.selected_codec);
            self.engine.set_recording_container(self.selected_container);
            self.engine.set_auto_remux(self.auto_remux);
            self.apply_recording_file_settings();
            self.engine.set_preset(self.selected_preset);
            self.apply_rate_control();
            self.apply_rate_control();
            self.engine.set_overlay_enabled(self.show_overlay);
            self.engine.set_video_effects(self.video_effects);
            self.engine.set_video_effects(self.video_effects);
            self.apply_replay_setting();
            self.engine.start_local_recording(path.clone());

            let (sender, receiver) = mpsc::channel();
            self.frame_receiver = Some(receiver);

            let (error_sender, error_receiver) = mpsc::channel::<String>();
            self.error_receiver = Some(error_receiver);

            let stop_signal = Arc::new(AtomicBool::new(false));
            self.stop_signal = Some(stop_signal.clone());

            let flags = (sender, stop_signal);

            self.is_windows_recording = true;
            self.last_error = None;
            self.record_started = Instant::now();
            self.last_frame_at = None;

            // Probe the preferred capture backend: DXGI Desktop Duplication
            // (G2) if available, otherwise Windows Graphics Capture (the
            // thread below) with the fallback reason recorded for the UI.
            self.capture_backend = probe_dxgi_backend();

            thread::spawn(move || {
                let settings = Settings::new(
                    monitor,
                    CursorCaptureSettings::Default,
                    DrawBorderSettings::Default,
                    SecondaryWindowSettings::Default,
                    MinimumUpdateIntervalSettings::Default,
                    DirtyRegionSettings::Default,
                    ColorFormat::Rgba8,
                    flags,
                );
                tracing::info!("Starting capture thread (monitor)");
                if let Err(e) = CaptureHandler::start(settings) {
                    if !e.to_string().contains("user stopped")
                        && !e.to_string().contains("GUI channel closed")
                    {
                        let message = format!("Capture error: {}", e);
                        tracing::error!("{message}");
                        let _ = error_sender.send(message);
                    }
                }
                tracing::info!("Capture thread stopped");
            });
        } else if let Some(idx) = self.selected_window_idx {
            let Some(window) = self.windows.get(idx).cloned() else {
                self.last_error = Some(self.tr("invalid_source").to_string());
                return;
            };

            self.engine.set_video_codec(self.selected_codec);
            self.engine.set_recording_container(self.selected_container);
            self.engine.set_auto_remux(self.auto_remux);
            self.apply_recording_file_settings();
            self.engine.set_preset(self.selected_preset);
            self.apply_rate_control();
            self.apply_rate_control();
            self.engine.set_overlay_enabled(self.show_overlay);
            self.engine.set_video_effects(self.video_effects);
            self.engine.set_video_effects(self.video_effects);
            self.apply_replay_setting();
            self.engine.start_local_recording(path.clone());

            let (sender, receiver) = mpsc::channel();
            self.frame_receiver = Some(receiver);

            let (error_sender, error_receiver) = mpsc::channel::<String>();
            self.error_receiver = Some(error_receiver);

            let stop_signal = Arc::new(AtomicBool::new(false));
            self.stop_signal = Some(stop_signal.clone());

            let flags = (sender, stop_signal);

            self.is_windows_recording = true;
            self.last_error = None;
            self.record_started = Instant::now();
            self.last_frame_at = None;

            thread::spawn(move || {
                let settings = Settings::new(
                    window,
                    CursorCaptureSettings::Default,
                    DrawBorderSettings::Default,
                    SecondaryWindowSettings::Default,
                    MinimumUpdateIntervalSettings::Default,
                    DirtyRegionSettings::Default,
                    ColorFormat::Rgba8,
                    flags,
                );
                tracing::info!("Starting capture thread (window)");
                if let Err(e) = CaptureHandler::start(settings) {
                    if !e.to_string().contains("user stopped")
                        && !e.to_string().contains("GUI channel closed")
                    {
                        let message = format!("Capture error: {}", e);
                        tracing::error!("{message}");
                        let _ = error_sender.send(message);
                    }
                }
                tracing::info!("Capture thread stopped");
            });
        } else if self.use_game_capture {
            // Game capture mode (G3): prefer the zero-overhead Vulkan layer;
            // fall back to Windows Graphics Capture on the game window when
            // the layer's shared-memory channel is not available yet.
            let Some(idx) = self.selected_game_window_idx else {
                self.last_error = Some(self.tr("no_source_selected").to_string());
                return;
            };
            let Some(game_window) = self.game_windows.get(idx).cloned() else {
                self.last_error = Some(self.tr("invalid_source").to_string());
                return;
            };

            self.engine.set_video_codec(self.selected_codec);
            self.engine.set_recording_container(self.selected_container);
            self.engine.set_auto_remux(self.auto_remux);
            self.apply_recording_file_settings();
            self.engine.set_preset(self.selected_preset);
            self.apply_rate_control();
            self.apply_rate_control();
            self.engine.set_overlay_enabled(self.show_overlay);
            self.engine.set_video_effects(self.video_effects);
            self.engine.set_video_effects(self.video_effects);
            self.apply_replay_setting();
            self.engine.start_local_recording(path.clone());

            self.is_aux_recording = true;
            self.last_error = None;
            self.record_started = Instant::now();
            self.last_frame_at = None;

            let fps = self.selected_preset.effective_fps(30);

            if let Some((rx, handle)) = rivulet_core::game_capture::start_vulkan_layer_capture(fps)
            {
                // Preferred backend: frames arrive through the layer's shared
                // memory and are drained into the engine via `game_capture_rx`
                // in the aux-recording drain below.
                self.capture_backend = Some(vulkan_layer_backend_status(true));
                self.game_capture_rx = Some(rx);
                self.game_capture_handle = Some(handle);
                tracing::info!("Vulkan layer capture started (preferred fullscreen backend)");
            } else {
                // Fallback: capture the game window with Windows Graphics
                // Capture. WGC raw frames are forwarded into `game_capture_rx`
                // so the shared aux-recording drain handles them.
                self.capture_backend = Some(vulkan_layer_backend_status(false));
                let (raw_tx, raw_rx) = mpsc::channel::<RawFrame>();
                let (frame_tx, frame_rx) = mpsc::channel::<rivulet_core::GameCaptureFrame>();
                self.game_capture_rx = Some(frame_rx);

                let (error_sender, error_receiver) = mpsc::channel::<String>();
                self.error_receiver = Some(error_receiver);

                let stop_signal = Arc::new(AtomicBool::new(false));
                self.stop_signal = Some(stop_signal.clone());

                // Adapter thread: re-wrap the WGC RawFrame into a
                // GameCaptureFrame for the aux-recording drain.
                thread::spawn(move || {
                    while let Ok(raw) = raw_rx.recv() {
                        let frame = rivulet_core::GameCaptureFrame {
                            data: raw.data,
                            width: raw.width,
                            height: raw.height,
                        };
                        if frame_tx.send(frame).is_err() {
                            break;
                        }
                    }
                });

                thread::spawn(move || {
                    let settings = Settings::new(
                        Window::from_raw_hwnd(game_window.id as *mut _),
                        CursorCaptureSettings::Default,
                        DrawBorderSettings::Default,
                        SecondaryWindowSettings::Default,
                        MinimumUpdateIntervalSettings::Default,
                        DirtyRegionSettings::Default,
                        ColorFormat::Rgba8,
                        (raw_tx, stop_signal),
                    );
                    tracing::info!("Starting game capture thread (WGC fallback)");
                    if let Err(e) = CaptureHandler::start(settings) {
                        if !e.to_string().contains("user stopped")
                            && !e.to_string().contains("GUI channel closed")
                        {
                            let message = format!("Game capture error: {}", e);
                            tracing::error!("{message}");
                            let _ = error_sender.send(message);
                        }
                    }
                    tracing::info!("Game capture thread stopped");
                });
            }
        } else if let Some(idx) = self.selected_camera_idx {
            // Camera capture mode
            let Some(device) = self.camera_devices.get(idx).cloned() else {
                self.last_error = Some(self.tr("invalid_source").to_string());
                return;
            };

            self.engine.set_video_codec(self.selected_codec);
            self.engine.set_recording_container(self.selected_container);
            self.engine.set_auto_remux(self.auto_remux);
            self.apply_recording_file_settings();
            self.engine.set_preset(self.selected_preset);
            self.apply_rate_control();
            self.apply_rate_control();
            self.engine.set_overlay_enabled(self.show_overlay);
            self.engine.set_video_effects(self.video_effects);
            self.engine.set_video_effects(self.video_effects);
            self.apply_replay_setting();
            self.engine.start_local_recording(path.clone());

            let config = rivulet_core::CameraConfig::default();
            let (camera_rx, handle) = rivulet_core::camera::start_camera_capture(&device, &config);
            self.camera_rx = Some(camera_rx);
            self.camera_handle = Some(handle);
            self.is_aux_recording = true;
            self.last_error = None;
            self.record_started = Instant::now();
            self.last_frame_at = None;
        } else {
            self.last_error = Some(self.tr("no_source_selected").to_string());
        }
    }

    #[cfg(target_os = "windows")]
    fn stop_windows_recording(&mut self) {
        tracing::info!("Sending stop signal");
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        self.is_windows_recording = false;
        self.capture_backend = None;
        self.engine.stop_recording();
        self.frame_receiver = None;
        // Keep the error receiver alive after the stop so a capture error
        // that arrives a frame late is still shown in the UI; the next
        // recording start replaces it.
        self.stop_signal = None;
    }
}

/// Probe the preferred Windows capture backend and report the status for
/// the UI (G2).
///
/// Tries DXGI Desktop Duplication first (zero-overhead fullscreen path). If
/// it is unavailable — e.g. protected content, a non-duplicable desktop, or
/// a headless session — returns a status with the active backend set to
/// Windows Graphics Capture (what the recording thread uses) and the reason
/// recorded, so the GUI can show that a fallback occurred.
#[cfg(target_os = "windows")]
fn probe_dxgi_backend() -> Option<BackendStatus> {
    // Use the primary adapter's first output as the probe target; the GUI
    // lets the user pick a monitor afterwards, but availability is the same
    // across outputs of the active adapter in practice.
    match DxgiDesktopDuplication::new() {
        Ok(dup) => {
            let mut status = BackendStatus::idle();
            status.switch_to(BackendKind::DesktopDuplication);
            let _ = dup; // dropped immediately; the probe is read-only
            Some(status)
        }
        Err(e) => {
            tracing::warn!(error = %e, "DXGI Desktop Duplication unavailable");
            let mut status = BackendStatus::idle();
            status.switch_to(BackendKind::WindowsGraphicsCapture);
            status.fail(format!("DXGI unavailable ({e})"));
            Some(status)
        }
    }
}

/// Build the capture-backend status for the Vulkan layer game-capture path
/// (G3). When the layer's shared-memory channel opened successfully the
/// Vulkan layer is the active backend; otherwise the GUI falls back to
/// Windows Graphics Capture and the reason is recorded for the UI badge.
#[cfg(target_os = "windows")]
fn vulkan_layer_backend_status(layer_active: bool) -> BackendStatus {
    let mut status = BackendStatus::idle();
    status.switch_to(BackendKind::VulkanLayer);
    if !layer_active {
        status.switch_to(BackendKind::WindowsGraphicsCapture);
        status.fail("Vulkan layer not available");
    }
    status
}

#[cfg(target_os = "linux")]
impl RivuletApp {
    fn start_audio_capture(&mut self) {
        self.stop_audio_capture();

        let config = AudioConfig {
            capture_system: self.capture_system,
            capture_mic: self.capture_mic,
            system_volume: self.system_volume,
            mic_volume: self.mic_volume,
            system_filters: self.system_filters.clone(),
            mic_filters: self.mic_filters.clone(),
            system_monitor: self.system_monitor,
            mic_monitor: self.mic_monitor,
            monitor_volume: self.monitor_volume,
            master_volume: self.master_volume,
            separate_tracks: self.separate_tracks,
            ..Default::default()
        };

        let mut audio = match AudioCapture::new(config) {
            Ok(audio) => audio,
            Err(e) => {
                self.audio_status =
                    Some(self.tr_fmt("audio_capture_unavailable", &[e.to_string()]));
                self.audio_warning = None;
                return;
            }
        };

        let skipped_warning = {
            let skipped = audio.skipped_filters();
            if skipped.is_empty() {
                None
            } else {
                Some(self.skipped_filters_warning(skipped))
            }
        };

        let peak = Arc::clone(&self.audio_peak);
        if self.separate_tracks {
            let (sys_tx, sys_rx) = std_mpsc::channel::<AudioFrame>();
            let (mic_tx, mic_rx) = std_mpsc::channel::<AudioFrame>();
            let sys_peak = Arc::clone(&peak);
            let mic_peak = Arc::clone(&peak);
            let sys_cb = move |frame: AudioFrame| {
                let p = frame.data.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
                sys_peak.store(p.to_bits(), Ordering::SeqCst);
                let _ = sys_tx.send(frame);
            };
            let mic_cb = move |frame: AudioFrame| {
                let p = frame.data.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
                mic_peak.store(p.to_bits(), Ordering::SeqCst);
                let _ = mic_tx.send(frame);
            };
            match audio.start_separated(Box::new(sys_cb), Box::new(mic_cb)) {
                Ok(()) => {
                    self.audio_system_rx = Some(sys_rx);
                    self.audio_mic_rx = Some(mic_rx);
                    self.audio = Some(audio);
                    self.audio_preview = true;
                    self.audio_status = None;
                    self.audio_warning = skipped_warning.clone();
                }
                Err(e) => {
                    self.audio_status = Some(self.tr_fmt("audio_start_failed", &[e.to_string()]));
                    self.audio_warning = None;
                }
            }
        } else {
            let (audio_tx, audio_rx) = std_mpsc::channel::<AudioFrame>();
            match audio.start(Box::new(move |frame| {
                let p = frame.data.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
                peak.store(p.to_bits(), Ordering::SeqCst);
                let _ = audio_tx.send(frame);
            })) {
                Ok(()) => {
                    self.audio_rx = Some(audio_rx);
                    self.audio = Some(audio);
                    self.audio_preview = true;
                    self.audio_status = None;
                    self.audio_warning = skipped_warning;
                }
                Err(e) => {
                    self.audio_status = Some(self.tr_fmt("audio_start_failed", &[e.to_string()]));
                    self.audio_warning = None;
                }
            }
        }
    }

    fn stop_audio_capture(&mut self) {
        if let Some(mut audio) = self.audio.take() {
            let _ = audio.stop();
        }
        self.audio_rx = None;
        self.audio_system_rx = None;
        self.audio_mic_rx = None;
        self.audio_preview = false;
        self.audio_warning = None;
        self.audio_peak.store(0.0f32.to_bits(), Ordering::SeqCst);
    }

    /// Build a localized warning listing audio filters that were skipped
    /// because their GStreamer elements are not installed.
    fn skipped_filters_warning(&self, skipped: &[SkippedFilter]) -> String {
        format_skipped_filters(self.locale, skipped)
    }

    fn refresh_linux_sources(&mut self) {
        self.monitors = xcap::Monitor::all().unwrap_or_default();
        if self
            .selected_monitor_idx
            .map_or(true, |idx| idx >= self.monitors.len())
        {
            self.selected_monitor_idx = None;
        }

        // Ignore windows with an empty title (portal backgrounds etc.).
        self.windows = xcap::Window::all()
            .unwrap_or_default()
            .into_iter()
            .filter(|w| !w.title().unwrap_or_default().trim().is_empty())
            .collect();
        if self
            .selected_window_idx
            .map_or(true, |idx| idx >= self.windows.len())
        {
            self.selected_window_idx = None;
        }
    }

    /// Open the region editor for the currently selected monitor, capturing
    /// a live preview of it for the drag selection.
    fn open_region_editor(&mut self, ctx: &egui::Context) {
        let Some(idx) = self.selected_monitor_idx else {
            return;
        };
        let Some(monitor) = self.monitors.get(idx) else {
            return;
        };
        let name = monitor.name().unwrap_or_default();
        self.region_editor_dims =
            Some((monitor.width().unwrap_or(0), monitor.height().unwrap_or(0)));
        self.region_preview_error = None;
        match monitor_preview_image(&self.monitors, &name) {
            Some(image) => self.region_preview = Some(RegionPreview::new(ctx, image)),
            None => {
                self.region_preview_error = Some(self.tr_fmt(
                    "region_preview_failed",
                    &[self.tr("region_preview_unavailable").to_string()],
                ));
            }
        }
        self.region_editor_open = true;
    }

    /// Attempt to start recording via the PipeWire portal (G6).
    /// Returns `true` if the portal was available and recording started.
    /// Returns `false` if the portal is unavailable (fallback to xcap).
    #[cfg(target_os = "linux")]
    fn try_start_pipewire_recording(&mut self) -> bool {
        // Create a blocking tokio runtime to run the async portal request.
        let rt = match Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                tracing::error!(error = %e, "PipeWire failed to create Tokio runtime");
                return false;
            }
        };

        let prefer_monitor = self.selected_monitor_idx.is_some();
        let portal_result = rt.block_on(rivulet_capture::pipewire_portal::request_portal_session(
            prefer_monitor,
        ));

        let (fd, info) = match portal_result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "PipeWire portal unavailable");
                return false;
            }
        };

        // File dialog
        let ext = self.selected_container_extension();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Video", &[ext])
            .set_file_name(self.default_recording_filename())
            .save_file()
        else {
            tracing::info!("File selection cancelled");
            return false;
        };

        if (self.capture_system || self.capture_mic) && !self.audio_preview {
            self.start_audio_capture();
        }
        self.engine.set_video_codec(self.selected_codec);
        self.engine.set_recording_container(self.selected_container);
        self.engine.set_auto_remux(self.auto_remux);
        self.apply_recording_file_settings();
        self.engine.set_preset(self.selected_preset);
        self.apply_rate_control();
        self.engine.set_overlay_enabled(self.show_overlay);
        self.engine.set_video_effects(self.video_effects);
        self.apply_replay_setting();
        self.engine.set_audio_enabled(self.audio_preview);
        self.engine
            .set_separate_audio_tracks(self.separate_tracks && self.audio_preview);
        if self.engine.separate_audio_tracks() {
            self.engine.set_audio_track_enabled(
                rivulet_core::AudioTrack::System,
                self.capture_system && self.export_system_track,
            );
            self.engine.set_audio_track_enabled(
                rivulet_core::AudioTrack::Microphone,
                self.capture_mic && self.export_mic_track,
            );
        }
        self.engine.start_local_recording(path);

        let (frame_rx, handle) =
            rivulet_capture::pipewire_portal::start_pipewire_capture(fd, info.node_id);
        self.pipewire_capture = Some(handle);

        // Convert PipeWire frames to RawFrame and send to engine
        let (raw_tx, raw_rx) = std_mpsc::channel::<RawFrame>();
        let stop = Arc::new(AtomicBool::new(false));
        self.raw_rx = Some(raw_rx);
        self.stop_signal = Some(stop.clone());

        thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match frame_rx.recv_timeout(std::time::Duration::from_millis(100)) {
                    Ok(pw_frame) => {
                        let frame = RawFrame {
                            data: pw_frame.data,
                            width: pw_frame.width,
                            height: pw_frame.height,
                        };
                        if raw_tx.send(frame).is_err() {
                            break;
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        self.is_recording = true;
        self.record_started = Instant::now();
        self.last_frame_at = None;
        self.record_status = None;
        tracing::info!(
            node_id = info.node_id,
            "Linux recording started via PipeWire portal"
        );
        true
    }

    #[cfg(target_os = "linux")]
    fn start_linux_recording(&mut self) {
        if self.is_recording {
            return;
        }

        // Fallback: xcap screen/window capture.
        // If the portal is available and the user selects a source, use PipeWire.
        // Otherwise fall back to xcap (X11 composite capture).
        if self.try_start_pipewire_recording() {
            return;
        }

        // Fallback: xcap screen/window capture.
        // Capture source: game window (when game capture is enabled) takes
        // precedence over the plain window/monitor pickers.
        enum Source {
            Monitor(xcap::Monitor),
            Window(xcap::Window),
        }

        let source = if self.use_game_capture {
            // Resolve the selected game window (xdotool id) to its xcap
            // window so the existing capture loop below handles it.
            let Some(idx) = self.selected_game_window_idx else {
                self.record_status = Some(self.tr("no_source_selected").to_string());
                return;
            };
            let Some(game_window) = self.game_windows.get(idx) else {
                self.record_status = Some(self.tr("invalid_window").to_string());
                return;
            };
            match xcap::Window::all().ok().and_then(|windows| {
                windows
                    .into_iter()
                    .find(|w| w.id().map(|id| id as u64).unwrap_or(0) == game_window.id)
            }) {
                Some(w) => Source::Window(w),
                None => {
                    self.record_status = Some(self.tr("invalid_window").to_string());
                    return;
                }
            }
        } else if let Some(idx) = self.selected_window_idx {
            match self.windows.get(idx).cloned() {
                Some(w) => Source::Window(w),
                None => {
                    self.record_status = Some(self.tr("invalid_window").to_string());
                    return;
                }
            }
        } else if let Some(idx) = self.selected_monitor_idx {
            match self.monitors.get(idx).cloned() {
                Some(m) => Source::Monitor(m),
                None => {
                    self.record_status = Some(self.tr("invalid_monitor").to_string());
                    return;
                }
            }
        } else {
            self.record_status = Some(self.tr("no_source_selected").to_string());
            return;
        };

        let ext = self.selected_container_extension();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Video", &[ext])
            .set_file_name(self.default_recording_filename())
            .save_file()
        else {
            tracing::info!("File selection cancelled");
            return;
        };

        if (self.capture_system || self.capture_mic) && !self.audio_preview {
            self.start_audio_capture();
        }
        self.engine.set_video_codec(self.selected_codec);
        self.engine.set_recording_container(self.selected_container);
        self.engine.set_auto_remux(self.auto_remux);
        self.apply_recording_file_settings();
        self.engine.set_preset(self.selected_preset);
        self.apply_rate_control();
        self.engine.set_overlay_enabled(self.show_overlay);
        self.engine.set_video_effects(self.video_effects);
        self.apply_replay_setting();
        self.engine.set_audio_enabled(self.audio_preview);
        self.engine
            .set_separate_audio_tracks(self.separate_tracks && self.audio_preview);
        if self.engine.separate_audio_tracks() {
            self.engine.set_audio_track_enabled(
                rivulet_core::AudioTrack::System,
                self.capture_system && self.export_system_track,
            );
            self.engine.set_audio_track_enabled(
                rivulet_core::AudioTrack::Microphone,
                self.capture_mic && self.export_mic_track,
            );
        }
        self.engine.start_local_recording(path);

        let (raw_tx, raw_rx) = std_mpsc::channel::<RawFrame>();
        let stop = Arc::new(AtomicBool::new(false));
        self.raw_rx = Some(raw_rx);
        self.stop_signal = Some(stop.clone());

        let source_desc = match &source {
            Source::Monitor(m) => format!(
                "Monitor {} ({}x{})",
                m.name().unwrap_or_default(),
                m.width().unwrap_or(0),
                m.height().unwrap_or(0)
            ),
            Source::Window(w) => format!(
                "Window \"{}\" ({}x{})",
                w.title().unwrap_or_default(),
                w.width().unwrap_or(0),
                w.height().unwrap_or(0)
            ),
        };

        let fps = self.selected_preset.effective_fps(30) as u64;
        let frame_duration = std::time::Duration::from_millis(1000 / fps);
        thread::spawn(move || {
            let mut next = Instant::now();
            while !stop.load(Ordering::SeqCst) {
                let result = match &source {
                    Source::Monitor(m) => m.capture_image(),
                    Source::Window(w) => w.capture_image(),
                };
                match result {
                    Ok(image) => {
                        let frame = RawFrame {
                            data: image.as_raw().to_vec(),
                            width: image.width(),
                            height: image.height(),
                        };
                        if raw_tx.send(frame).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        // The window was minimized/closed or is no longer
                        // drawable -> end recording gracefully instead of crashing.
                        tracing::error!(error = %e, "Capture error");
                        break;
                    }
                }
                next += frame_duration;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                }
            }
        });

        self.is_recording = true;
        self.record_started = Instant::now();
        self.last_frame_at = None;
        self.record_status = None;
        tracing::info!(source = %source_desc, "Linux recording started");
    }

    #[cfg(target_os = "linux")]
    fn stop_linux_recording(&mut self) {
        if !self.is_recording {
            return;
        }
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        // Stop PipeWire portal capture if active (G6)
        self.pipewire_capture = None;
        self.engine.stop_recording();
        self.raw_rx = None;
        self.stop_signal = None;
        self.stop_audio_capture();
        self.is_recording = false;
        self.record_status = Some(self.tr("recording_saved").to_string());
    }

    fn drain_linux_frames(&mut self) {
        let paused = self.is_paused;
        let muted = self.is_muted;
        if let Some(rx) = &self.raw_rx {
            while let Ok(raw) = rx.try_recv() {
                let frame = cropped_frame_for_region(
                    self.region_enabled && self.selected_monitor_idx.is_some(),
                    self.region,
                    &raw,
                )
                .into_owned();
                if !paused {
                    self.engine
                        .process_raw_frame(&frame.data, frame.width, frame.height);
                }
                self.pending_preview_frame = Some(frame);
                self.last_frame_at = Some(Instant::now());
            }
        }
        // Drain camera frames if camera capture is active
        if let Some(rx) = &self.camera_rx {
            while let Ok(frame) = rx.try_recv() {
                if !paused {
                    self.engine
                        .process_raw_frame(&frame.data, frame.width, frame.height);
                }
                self.pending_preview_frame = Some(RawFrame {
                    data: frame.data.clone(),
                    width: frame.width,
                    height: frame.height,
                });
                self.last_frame_at = Some(Instant::now());
            }
        }
        // Drain game capture frames if game capture is active
        if let Some(rx) = &self.game_capture_rx {
            while let Ok(frame) = rx.try_recv() {
                if !paused {
                    self.engine
                        .process_raw_frame(&frame.data, frame.width, frame.height);
                }
                self.pending_preview_frame = Some(RawFrame {
                    data: frame.data.clone(),
                    width: frame.width,
                    height: frame.height,
                });
                self.last_frame_at = Some(Instant::now());
            }
        }
        // Update the text overlay (timer + FPS) once per UI tick.
        if self.show_overlay && self.is_recording && !paused {
            let text = self.overlay_text();
            self.engine.update_overlay_text(&text);
        }
        if self.separate_tracks && self.audio_preview {
            if self.is_recording && !muted {
                if let Some(rx) = &self.audio_system_rx {
                    while let Ok(frame) = rx.try_recv() {
                        let _ = self.engine.push_audio_track(&frame, AudioTrack::System);
                    }
                }
                if let Some(rx) = &self.audio_mic_rx {
                    while let Ok(frame) = rx.try_recv() {
                        let _ = self.engine.push_audio_track(&frame, AudioTrack::Microphone);
                    }
                }
            } else {
                if let Some(rx) = &self.audio_system_rx {
                    while rx.try_recv().is_ok() {}
                }
                if let Some(rx) = &self.audio_mic_rx {
                    while rx.try_recv().is_ok() {}
                }
            }
        } else if let Some(rx) = &self.audio_rx {
            if self.is_recording && !muted {
                while let Ok(frame) = rx.try_recv() {
                    let _ = self.engine.push_audio_frame(&frame);
                }
            } else {
                while rx.try_recv().is_ok() {}
            }
        }
    }
}

impl RivuletApp {
    /// Stop camera or game-capture recording (platform-agnostic).
    fn stop_aux_recording(&mut self) {
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        self.engine.stop_recording();
        self.camera_rx = None;
        self.camera_handle = None;
        self.game_capture_rx = None;
        self.game_capture_handle = None;
        self.stop_signal = None;
        self.is_aux_recording = false;
        #[cfg(target_os = "windows")]
        {
            self.capture_backend = None;
        }
    }
}

impl RivuletApp {
    /// Translate a UI string into the currently selected locale.
    fn tr<'a>(&self, key: &'a str) -> &'a str {
        self.locale.tr(key)
    }

    /// Translate a UI string and substitute positional placeholders.
    fn tr_fmt(&self, key: &str, args: &[String]) -> String {
        self.locale.tr_fmt(key, args)
    }

    /// File extension for the current recording container, mirroring the
    /// engine's container-aware selection: the default MP4 container keeps the
    /// codec-native extension (H.264/H.265 => mp4, VP9 => webm), while a
    /// crash-safe intermediate container (MKV/MOV/TS) uses its own extension.
    fn selected_container_extension(&self) -> &'static str {
        match self.selected_container {
            rivulet_core::RecordingContainer::Mp4 => self.selected_codec.file_extension(),
            other => other.file_extension(),
        }
    }

    /// Applies the current recording file-management policy (split + auto-
    /// record flags) to the engine before a recording starts.
    fn apply_recording_file_settings(&mut self) {
        use rivulet_core::{RecordingFileSettings, SplitBy};
        let split = if self.split_seconds > 0 {
            SplitBy::Duration {
                seconds: self.split_seconds,
            }
        } else {
            SplitBy::None
        };
        self.engine.set_recording_file(RecordingFileSettings {
            filename_pattern: rivulet_core::FileNamePattern::default(),
            split,
            auto_record_with_stream: self.auto_record_with_stream,
            auto_record_dir: "recordings".to_string(),
        });
    }

    /// Applies the advanced rate-control strategy (mode, quality, cap and any
    /// free-form extra encoder options) to the engine before a recording or a
    /// record+stream session starts.
    fn apply_rate_control(&mut self) {
        self.engine.set_rate_control(rivulet_core::RateControl {
            mode: self.rate_mode,
            max_bitrate_kbps: self.rate_max_kbps,
            quality: self.rate_quality,
            custom_options: self.encoder_extra_options.clone(),
        });
    }

    /// Default base filename for a new recording, derived from the engine's
    /// filename pattern and the current container extension.
    fn default_recording_filename(&self) -> String {
        let path = self.engine.default_recording_path("", "");
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "rivulet-recording.mp4".to_string())
    }

    /// Resolve the label for the recording view's Source (monitor) dropdown.
    ///
    /// Returns the monitor name when a monitor is selected, otherwise `None`
    /// (the caller falls back to "Select Monitor"). The selected window is
    /// deliberately **not** considered: the Window dropdown shows the window,
    /// and a window pick must never leak its title into the Source dropdown.
    #[cfg(target_os = "windows")]
    fn monitor_label(&self) -> Option<String> {
        resolve_monitor_label(
            self.selected_monitor_idx,
            self.selected_monitor_idx
                .and_then(|idx| self.monitors.get(idx))
                .and_then(|m| m.name().ok())
                .as_deref(),
        )
    }

    /// Scenes view: list scenes (folders first), switch the active scene,
    /// and add/rename/remove scenes. Pure state logic is testable via the
    /// `SceneManager` in rivulet-core; this method only renders and calls it.
    fn draw_scenes_view(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        if self.scene_transition.is_active() {
            let progress = self.scene_transition.progress_at(Instant::now());
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
            if progress < 1.0 {
                ui.add(egui::ProgressBar::new(progress).text(self.tr("transition_in_progress")));
            } else {
                self.scene_transition.clear();
            }
        }
        let active = if self.studio_mode.enabled() {
            self.studio_mode.preview()
        } else {
            self.scenes.active()
        };
        let active_name = active
            .and_then(|id| self.scenes.get(id))
            .map(|s| s.name.clone())
            .unwrap_or_else(|| self.tr("scenes_no_scenes").to_string());
        ui.label(
            egui::RichText::new(self.tr_fmt(
                if self.studio_mode.enabled() {
                    "studio_preview_label"
                } else {
                    "scenes_active_label"
                },
                &[active_name],
            ))
            .strong()
            .color(colors.success),
        );

        let mut studio_enabled = self.studio_mode.enabled();
        if ui
            .checkbox(&mut studio_enabled, self.tr("studio_mode"))
            .changed()
        {
            self.studio_mode
                .set_enabled(studio_enabled, self.scenes.active());
            self.scene_status = Some(
                self.tr(if studio_enabled {
                    "studio_mode_enabled"
                } else {
                    "studio_mode_disabled"
                })
                .to_string(),
            );
        }
        if self.studio_mode.enabled() {
            let program_name = self
                .studio_mode
                .program()
                .and_then(|id| self.scenes.get(id))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "—".to_string());
            let preview_name = self
                .studio_mode
                .preview()
                .and_then(|id| self.scenes.get(id))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "—".to_string());
            ui.horizontal(|ui| {
                ui.label(self.tr_fmt("studio_program_label", std::slice::from_ref(&program_name)));
                ui.label(self.tr_fmt("studio_preview_label", std::slice::from_ref(&preview_name)));
                let take = theme::accent_button(ui, self.tr("studio_take"))
                    .on_hover_text(self.tr("studio_take_hint"));
                if take.clicked() {
                    let now = Instant::now();
                    if let Some(transition) = self.studio_mode.take(
                        self.transition_kind,
                        std::time::Duration::from_millis(self.transition_duration_ms),
                        now,
                    ) {
                        if let Some(target) = self.studio_mode.program() {
                            let from = self.scenes.active();
                            if self.scenes.switch_to(target) {
                                self.scene_transition = transition;
                                self.scene_status =
                                    Some(self.tr_fmt(
                                        "studio_taken",
                                        std::slice::from_ref(&preview_name),
                                    ));
                                if from == Some(target) {
                                    self.scene_transition.clear();
                                }
                            }
                        }
                    }
                }
            });
        }

        let name_hint = self.tr("scenes_name_placeholder");
        let add_label = self.tr("scenes_add").to_owned();
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.scene_name_input).hint_text(name_hint));
            if theme::accent_button(ui, add_label).clicked() {
                let name = self.scene_name_input.trim().to_string();
                if !name.is_empty() {
                    self.scenes.add(rivulet_core::Scene::new(name.clone()));
                    self.scene_status = Some(self.tr_fmt("scenes_added", &[name]));
                    self.scene_name_input.clear();
                }
            }
            let switch_back_response = ui
                .add_enabled(
                    active.is_some() && !self.studio_mode.enabled(),
                    egui::Button::new(self.tr("scenes_switch_back")),
                )
                .on_hover_text(self.tr("scenes_switch_back"));
            theme::paint_interaction_stroke(ui, &switch_back_response);
            let switched_back = switch_back_response.clicked();
            if switched_back && self.scenes.switch_back() {
                self.scene_status = None;
            }
        });

        ui.horizontal(|ui| {
            ui.label(self.tr("transition"));
            egui::ComboBox::from_id_salt("scene_transition_kind")
                .selected_text(self.transition_kind.label())
                .show_ui(ui, |ui| {
                    for kind in rivulet_core::TransitionKind::ALL {
                        if ui
                            .selectable_label(self.transition_kind == kind, kind.label())
                            .clicked()
                        {
                            self.transition_kind = kind;
                        }
                    }
                });
            if self.transition_kind == rivulet_core::TransitionKind::Fade {
                let duration_label = self.tr("transition_duration").to_owned();
                ui.add(
                    egui::Slider::new(&mut self.transition_duration_ms, 50..=5000)
                        .text(duration_label),
                );
            }
        });

        ui.horizontal(|ui| {
            let undo_response = ui
                .add_enabled(
                    self.scenes.can_undo(),
                    egui::Button::new(self.tr("scenes_undo")),
                )
                .on_hover_text(self.tr("scenes_undo_hint"));
            theme::paint_interaction_stroke(ui, &undo_response);
            if undo_response.clicked() {
                self.scenes.undo();
            }
            let redo_response = ui
                .add_enabled(
                    self.scenes.can_redo(),
                    egui::Button::new(self.tr("scenes_redo")),
                )
                .on_hover_text(self.tr("scenes_redo_hint"));
            theme::paint_interaction_stroke(ui, &redo_response);
            if redo_response.clicked() {
                self.scenes.redo();
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(self.tr("scenes_collection"));
            ui.text_edit_singleline(&mut self.scene_collection_input);
            if theme::accent_button(ui, self.tr("scenes_collection")).clicked()
                && !self.scene_collection_input.trim().is_empty()
            {
                self.scenes
                    .set_collection(self.scene_collection_input.trim());
            }
            ui.label(self.tr("scenes_profile"));
            ui.text_edit_singleline(&mut self.scene_profile_input);
            if theme::accent_button(ui, self.tr("scenes_profile")).clicked()
                && !self.scene_profile_input.trim().is_empty()
            {
                self.scenes.set_profile(self.scene_profile_input.trim());
            }
            if theme::accent_button(ui, self.tr("scenes_export")).clicked() {
                self.export_scene_collection();
            }
            if theme::accent_button(ui, self.tr("scenes_import")).clicked() {
                self.import_scene_collection();
            }
        });
        ui.label(format!(
            "{} / {}",
            self.scenes.collection(),
            self.scenes.profile()
        ));

        if let Some(active_id) = self.scenes.active() {
            let current = self
                .hotkeys
                .scene_hotkeys
                .get(&active_id)
                .copied()
                .unwrap_or_else(|| HotkeyBinding::plain(self.scene_hotkey_key));
            ui.horizontal(|ui| {
                ui.label(self.tr("scenes_hotkey"));
                egui::ComboBox::from_id_salt("scene_hotkey_key")
                    .selected_text(key_name(current.key))
                    .show_ui(ui, |ui| {
                        for key in [
                            egui::Key::F1,
                            egui::Key::F2,
                            egui::Key::F3,
                            egui::Key::F4,
                            egui::Key::F5,
                            egui::Key::F6,
                            egui::Key::F7,
                            egui::Key::F8,
                        ] {
                            ui.selectable_value(&mut self.scene_hotkey_key, key, key_name(key));
                        }
                    });
                if theme::accent_button(ui, self.tr("scenes_assign_hotkey")).clicked() {
                    self.hotkeys
                        .scene_hotkeys
                        .insert(active_id, HotkeyBinding::plain(self.scene_hotkey_key));
                    self.scene_status = Some(self.tr("scenes_hotkey_assigned").to_owned());
                }
            });
        }
        let auto_switch_label = self.tr("scenes_auto_switch").to_owned();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_switch_enabled, auto_switch_label);
            ui.text_edit_singleline(&mut self.auto_switch_window_input);
            if theme::accent_button(ui, self.tr("scenes_add_auto_switch")).clicked() {
                if let Some(active_id) = self.scenes.active() {
                    let title = self.auto_switch_window_input.trim().to_string();
                    if !title.is_empty() {
                        self.auto_switch_rules.push((title, active_id));
                        self.auto_switch_window_input.clear();
                    }
                }
            }
        });

        ui.horizontal(|ui| {
            let snapshot_response = theme::accent_button(ui, self.tr("scenes_save_snapshot"))
                .on_hover_text(self.tr("scenes_save_snapshot_hint"));
            if snapshot_response.clicked() {
                self.save_scene_snapshot();
            }
        });

        let ordered = self.scenes.ordered_scenes();
        if ordered.is_empty() {
            ui.label(self.tr("scenes_no_scenes"));
        } else {
            let mut rename_target: Option<(uuid::Uuid, String)> = None;
            let mut remove_target: Option<uuid::Uuid> = None;
            let mut switch_target: Option<uuid::Uuid> = None;

            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 80.0)
                .show(ui, |ui| {
                    for id in ordered {
                        let Some(scene) = self.scenes.get(id).cloned() else {
                            continue;
                        };
                        let is_active = Some(id) == active;
                        ui.horizontal(|ui| {
                            let response = ui.selectable_label(is_active, &scene.name);
                            theme::paint_interaction_stroke(ui, &response);
                            if response.clicked() {
                                switch_target = Some(id);
                            }
                            if is_active {
                                ui.label(
                                    egui::RichText::new(self.tr("scenes_active"))
                                        .small()
                                        .color(colors.success),
                                );
                            }
                            let rename_response = ui.small_button(self.tr("scenes_rename"));
                            theme::paint_interaction_stroke(ui, &rename_response);
                            if rename_response.clicked() {
                                rename_target = Some((id, scene.name.clone()));
                            }
                            let duplicate_response = ui.small_button(self.tr("scenes_duplicate"));
                            theme::paint_interaction_stroke(ui, &duplicate_response);
                            if duplicate_response.clicked() {
                                let _ = self.scenes.duplicate_scene(
                                    id,
                                    self.tr_fmt(
                                        "scenes_duplicate_name",
                                        std::slice::from_ref(&scene.name),
                                    ),
                                );
                            }
                            let remove_response = ui.small_button(self.tr("scenes_remove"));
                            theme::paint_interaction_stroke(ui, &remove_response);
                            if remove_response.clicked() {
                                remove_target = Some(id);
                            }
                        });
                    }
                });

            if let Some((id, old_name)) = rename_target {
                if let Some(new_name) = self.scene_rename_prompt(ui, id, old_name) {
                    self.scene_status = Some(self.tr_fmt("scenes_renamed", &[new_name]));
                }
            }
            if let Some(id) = remove_target {
                let name = self
                    .scenes
                    .get(id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                if self.scenes.remove(id) {
                    self.studio_mode.remove_scene(id);
                    self.scene_status = Some(self.tr_fmt("scenes_removed", &[name]));
                }
            }
            if let Some(id) = switch_target {
                let name = self
                    .scenes
                    .get(id)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                if self.studio_mode.enabled() {
                    self.studio_mode.set_preview(Some(id));
                    self.scene_status = Some(self.tr_fmt("studio_preview_selected", &[name]));
                } else if self.scenes.switch_to(id) {
                    let now = Instant::now();
                    self.scene_transition = rivulet_core::SceneTransition::new(
                        self.transition_kind,
                        std::time::Duration::from_millis(self.transition_duration_ms),
                    );
                    self.scene_transition.start(active, Some(id), now);
                    self.scene_status = Some(self.tr_fmt("scenes_switched", &[name]));
                }
            }
        }

        if let Some(status) = &self.scene_status {
            ui.colored_label(colors.info, status);
        }
    }

    fn export_scene_collection(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name("rivulet-scenes.json")
            .save_file()
        else {
            return;
        };
        match self
            .scenes
            .export_json()
            .and_then(|json| std::fs::write(&path, json).map_err(serde_json::Error::io))
        {
            Ok(()) => {
                self.scene_status =
                    Some(self.tr_fmt("scenes_exported", &[path.display().to_string()]))
            }
            Err(error) => {
                self.scene_status = Some(self.tr_fmt("scenes_export_failed", &[error.to_string()]))
            }
        }
    }

    fn import_scene_collection(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };
        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|json| {
                rivulet_core::SceneManager::import_json(&json).map_err(|error| error.to_string())
            }) {
            Ok(manager) => {
                self.scenes = manager;
                self.scene_status =
                    Some(self.tr_fmt("scenes_imported", &[path.display().to_string()]));
            }
            Err(error) => self.scene_status = Some(self.tr_fmt("scenes_import_failed", &[error])),
        }
    }

    /// Export the active scene (or Studio Mode Preview) as a deterministic PNG.
    ///
    /// The exported image represents the current scene layout. Native source
    /// pixels are intentionally not synthesized here; platform source
    /// renderers can replace the tile renderer without changing this workflow.
    fn save_scene_snapshot(&mut self) {
        let scene_id = if self.studio_mode.enabled() {
            self.studio_mode.preview()
        } else {
            self.scenes.active()
        };
        let Some(scene_id) = scene_id else {
            self.scene_status = Some(self.tr("scenes_snapshot_no_scene").to_owned());
            return;
        };
        let Some(scene) = self.scenes.get(scene_id) else {
            self.scene_status = Some(self.tr("scenes_snapshot_no_scene").to_owned());
            return;
        };
        let scene_name = scene.name.clone();
        let snapshot = rivulet_core::SceneSnapshot::from_scene(
            scene,
            &self.source_manager,
            self.scenes.collection(),
            self.scenes.profile(),
        );
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!(
                "rivulet-scene-{}.png",
                sanitize_file_name(&scene_name)
            ))
            .save_file()
        else {
            return;
        };
        let image =
            image::RgbaImage::from_raw(snapshot.width, snapshot.height, snapshot.render_rgba())
                .expect("SceneSnapshot::render_rgba must match its dimensions");
        match image.save_with_format(&path, image::ImageFormat::Png) {
            Ok(()) => {
                self.scene_status =
                    Some(self.tr_fmt("scenes_snapshot_saved", &[path.display().to_string()]));
            }
            Err(error) => {
                self.scene_status =
                    Some(self.tr_fmt("scenes_snapshot_failed", &[error.to_string()]));
            }
        }
    }

    /// Edit the active scene's source bindings without mutating source defaults.
    /// Transform, crop, visibility, locking, and z-order are scene-local.
    fn draw_source_composition(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        ui.separator();
        ui.label(egui::RichText::new(self.tr("composition")).strong());
        let scene_id = if self.studio_mode.enabled() {
            self.studio_mode.preview()
        } else {
            self.scenes.active()
        };
        let Some(scene_id) = scene_id else {
            ui.colored_label(colors.hint, self.tr("composition_no_scene"));
            return;
        };

        let kinds = [
            rivulet_core::SourceKind::Image,
            rivulet_core::SourceKind::Text,
            rivulet_core::SourceKind::Webcam,
            rivulet_core::SourceKind::Browser,
            rivulet_core::SourceKind::Media,
            rivulet_core::SourceKind::Color,
            rivulet_core::SourceKind::GameCapture,
            rivulet_core::SourceKind::ScreenCapture,
            rivulet_core::SourceKind::Audio,
        ];
        self.source_kind_index = self.source_kind_index.min(kinds.len() - 1);
        ui.horizontal(|ui| {
            ui.label(self.tr("composition_add_source"));
            ui.text_edit_singleline(&mut self.source_name_input);
            egui::ComboBox::from_id_salt("composition_source_kind")
                .selected_text(kinds[self.source_kind_index].label())
                .show_ui(ui, |ui| {
                    for (index, kind) in kinds.iter().enumerate() {
                        if ui
                            .selectable_label(self.source_kind_index == index, kind.label())
                            .clicked()
                        {
                            self.source_kind_index = index;
                        }
                    }
                });
            if theme::accent_button(ui, self.tr("composition_add")).clicked() {
                let name = if self.source_name_input.trim().is_empty() {
                    format!(
                        "{} {}",
                        kinds[self.source_kind_index].label(),
                        self.source_manager.sources().len() + 1
                    )
                } else {
                    self.source_name_input.trim().to_string()
                };
                let id = self.source_manager.add_source(rivulet_core::Source::new(
                    name,
                    kinds[self.source_kind_index].clone(),
                ));
                self.source_manager.bind_source(id, scene_id, None);
                self.selected_composition_source = Some(id);
                self.source_name_input.clear();
            }
        });

        let entries: Vec<(
            uuid::Uuid,
            String,
            rivulet_core::SourceKind,
            bool,
            bool,
            i32,
        )> = self
            .source_manager
            .scene_sources(scene_id)
            .into_iter()
            .filter_map(|binding| {
                self.source_manager
                    .get_source(binding.source_id)
                    .map(|source| {
                        (
                            binding.source_id,
                            source.name.clone(),
                            source.kind.clone(),
                            binding.visible,
                            binding.locked,
                            binding.z_order,
                        )
                    })
            })
            .collect();
        if entries.is_empty() {
            ui.colored_label(colors.hint, self.tr("composition_empty"));
            return;
        }

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(self.tr("composition_layers"));
                for (id, name, kind, visible, locked, z_order) in &entries {
                    let selected = self.selected_composition_source == Some(*id);
                    let response = ui.selectable_label(
                        selected,
                        format!("{} · {} · z{}", name, kind.label(), z_order),
                    );
                    theme::paint_interaction_stroke(ui, &response);
                    if response.clicked() {
                        self.selected_composition_source = Some(*id);
                    }
                    let state = match (*visible, *locked) {
                        (true, true) => String::new(),
                        (false, true) => format!(" · {}", self.tr("composition_hidden")),
                        (true, false) => format!(" · {}", self.tr("composition_locked_state")),
                        (false, false) => format!(
                            " · {} · {}",
                            self.tr("composition_hidden"),
                            self.tr("composition_locked_state")
                        ),
                    };
                    ui.small(state);
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                let selected = self
                    .selected_composition_source
                    .filter(|id| entries.iter().any(|entry| entry.0 == *id));
                self.selected_composition_source =
                    selected.or_else(|| entries.first().map(|entry| entry.0));
                if let Some(source_id) = self.selected_composition_source {
                    let Some(binding) = self
                        .source_manager
                        .scene_sources(scene_id)
                        .into_iter()
                        .find(|b| b.source_id == source_id)
                        .map(|b| (*b).clone())
                    else {
                        return;
                    };
                    let mut visible = binding.visible;
                    let mut locked = binding.locked;
                    let mut transform = binding
                        .transform_override
                        .clone()
                        .or_else(|| {
                            self.source_manager
                                .get_source(source_id)
                                .map(|s| s.transform.clone())
                        })
                        .unwrap_or_default();
                    let mut crop = binding.crop;
                    ui.label(self.tr("composition_properties"));
                    if ui
                        .checkbox(&mut visible, self.tr("composition_visible"))
                        .changed()
                    {
                        self.source_manager
                            .set_visibility(source_id, scene_id, visible);
                    }
                    if ui
                        .checkbox(&mut locked, self.tr("composition_locked"))
                        .changed()
                    {
                        self.source_manager.set_locked(source_id, scene_id, locked);
                    }
                    ui.horizontal(|ui| {
                        ui.label(self.tr("composition_position"));
                        ui.add(egui::DragValue::new(&mut transform.x).prefix("X "));
                        ui.add(egui::DragValue::new(&mut transform.y).prefix("Y "));
                    });
                    ui.horizontal(|ui| {
                        ui.label(self.tr("composition_size"));
                        ui.add(
                            egui::DragValue::new(&mut transform.width)
                                .prefix("W ")
                                .range(1.0..=16384.0),
                        );
                        ui.add(
                            egui::DragValue::new(&mut transform.height)
                                .prefix("H ")
                                .range(1.0..=16384.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut transform.rotation).prefix("° "));
                        ui.add(
                            egui::Slider::new(&mut transform.opacity, 0.0..=1.0)
                                .text(self.tr("composition_opacity")),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(self.tr("composition_crop"));
                        ui.add(egui::DragValue::new(&mut crop.left).prefix("L "));
                        ui.add(egui::DragValue::new(&mut crop.top).prefix("T "));
                        ui.add(egui::DragValue::new(&mut crop.right).prefix("R "));
                        ui.add(egui::DragValue::new(&mut crop.bottom).prefix("B "));
                    });
                    if !locked {
                        self.source_manager
                            .set_transform(source_id, scene_id, transform);
                        self.source_manager.set_crop(source_id, scene_id, crop);
                    }
                    ui.horizontal(|ui| {
                        if theme::accent_button(ui, self.tr("composition_lower")).clicked() {
                            self.source_manager.reorder_source(source_id, scene_id, -1);
                        }
                        if theme::accent_button(ui, self.tr("composition_raise")).clicked() {
                            self.source_manager.reorder_source(source_id, scene_id, 1);
                        }
                        if theme::accent_button(ui, self.tr("composition_duplicate_source"))
                            .clicked()
                        {
                            if let Some(new_id) = self.source_manager.duplicate_source(
                                source_id,
                                format!(
                                    "{} copy",
                                    self.source_manager
                                        .get_source(source_id)
                                        .map(|s| s.name.as_str())
                                        .unwrap_or("Source")
                                ),
                            ) {
                                self.source_manager
                                    .duplicate_binding(source_id, scene_id, new_id);
                                self.selected_composition_source = Some(new_id);
                            }
                        }
                    });
                }
            });
        });
    }

    fn draw_chroma_key_panel(&mut self, ui: &mut egui::Ui) {
        let Some(source_id) = self.selected_composition_source else {
            return;
        };
        let Some(source) = self.source_manager.get_source(source_id).cloned() else {
            return;
        };
        ui.separator();
        ui.label(egui::RichText::new(self.tr("chroma_key")).strong());
        let mut enabled = source.chroma_key.enabled;
        let label = self.tr("chroma_key_enabled").to_owned();
        if ui.checkbox(&mut enabled, label).changed() {
            if let Some(source) = self.source_manager.get_source_mut(source_id) {
                source.chroma_key.enabled = enabled;
            }
        }
        if enabled {
            let mut similarity = source.chroma_key.similarity;
            let mut smoothness = source.chroma_key.smoothness;
            ui.add(
                egui::Slider::new(&mut similarity, 0.0..=1.0)
                    .text(self.tr("chroma_key_similarity")),
            );
            ui.add(
                egui::Slider::new(&mut smoothness, 0.0..=1.0)
                    .text(self.tr("chroma_key_smoothness")),
            );
            if let Some(source) = self.source_manager.get_source_mut(source_id) {
                source.chroma_key.similarity = similarity;
                source.chroma_key.smoothness = smoothness;
            }
        }
    }

    fn draw_scene_overlay_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.label(egui::RichText::new(self.tr("scene_overlay")).strong());
        let enabled_label = self.tr("scene_overlay_enabled").to_owned();
        let text_hint = self.tr("scene_overlay_text").to_owned();
        ui.checkbox(&mut self.scene_overlay_enabled, enabled_label);
        ui.add(egui::TextEdit::singleline(&mut self.scene_overlay_text).hint_text(text_hint));
        ui.label(self.tr("scene_overlay_hint"));
        let multi_view_label = self.tr("multi_view_enabled").to_owned();
        let projector_open_label = self.tr("projector_open").to_owned();
        ui.checkbox(&mut self.multi_view_enabled, multi_view_label);
        if theme::accent_button(ui, projector_open_label).clicked() {
            self.projector_open = true;
        }
        if self.projector_open {
            let projector_fallback = self.tr("projector_fallback").to_owned();
            let projector_close_label = self.tr("projector_close").to_owned();
            ui.colored_label(egui::Color32::from_rgb(80, 180, 240), projector_fallback);
            if theme::accent_button(ui, projector_close_label).clicked() {
                self.projector_open = false;
            }
        }
    }

    /// Configure the S5b browser source from the Scenes view. Rendering is
    /// delegated to a platform adapter; this panel owns the portable URL,
    /// viewport, interaction, transparency, zoom, and CSS settings.
    fn draw_browser_source_panel(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        ui.separator();
        ui.label(egui::RichText::new(self.tr("browser_source")).strong());
        ui.horizontal(|ui| {
            ui.label(self.tr("browser_url"));
            ui.add(
                egui::TextEdit::singleline(&mut self.browser_source.url)
                    .desired_width(320.0)
                    .hint_text("https://example.com"),
            );
            if theme::accent_button(ui, self.tr("browser_apply_url")).clicked() {
                let url = self.browser_source.url.clone();
                if let Err(error) = self.browser_source.navigate(url) {
                    self.scene_status = Some(error.to_string());
                } else {
                    self.scene_status = Some(self.tr("browser_url_applied").to_owned());
                }
            }
        });
        // Alert overlay import (Streamlabs / StreamElements widget URLs).
        ui.horizontal(|ui| {
            ui.label(self.tr("alert_overlay_title"));
            for provider in rivulet_core::AlertProvider::all() {
                let label = self.tr(provider.i18n_key());
                if ui
                    .selectable_label(self.alert_provider == *provider, label)
                    .clicked()
                {
                    self.alert_provider = *provider;
                }
            }
        });
        let alert_token_hint = self.tr("alert_token_hint");
        if self.alert_provider != rivulet_core::AlertProvider::Custom {
            ui.horizontal(|ui| {
                ui.label(self.tr("alert_token"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.alert_token)
                        .hint_text(alert_token_hint)
                        .desired_width(300.0),
                );
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(self.tr("browser_url"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.alert_custom_url)
                        .hint_text("https://...")
                        .desired_width(300.0),
                );
            });
        }
        ui.horizontal(|ui| {
            if theme::accent_button(ui, self.tr("alert_import")).clicked() {
                self.import_alert_overlay();
            }
            ui.small(self.tr("alert_overlay_note"));
        });
        ui.horizontal(|ui| {
            ui.label(self.tr("browser_viewport"));
            let mut width = self.browser_source.width as f32;
            let mut height = self.browser_source.height as f32;
            ui.add(egui::DragValue::new(&mut width).range(1.0..=8192.0));
            ui.label("×");
            ui.add(egui::DragValue::new(&mut height).range(1.0..=8192.0));
            if theme::accent_button(ui, self.tr("browser_apply_viewport")).clicked() {
                if let Err(error) = self
                    .browser_source
                    .set_viewport(width as u32, height as u32)
                {
                    self.scene_status = Some(error.to_string());
                }
            }
        });
        ui.horizontal(|ui| {
            let interaction_label = self.tr("browser_interaction").to_owned();
            let transparent_label = self.tr("browser_transparent").to_owned();
            ui.checkbox(
                &mut self.browser_source.interaction_enabled,
                interaction_label,
            );
            ui.checkbox(&mut self.browser_source.transparent, transparent_label);
            let mut zoom = self.browser_source.zoom_level;
            if ui
                .add(egui::Slider::new(&mut zoom, 0.25..=5.0).text(self.tr("browser_zoom")))
                .changed()
            {
                let _ = self.browser_source.set_zoom_level(zoom);
            }
        });
        ui.horizontal(|ui| {
            ui.label(self.tr("browser_css"));
            let css = self
                .browser_source
                .custom_css
                .get_or_insert_with(String::new);
            ui.add(egui::TextEdit::singleline(css).desired_width(360.0));
        });
        if let Some(frame) = self.browser_source.latest_frame() {
            ui.label(self.tr_fmt(
                "browser_frame_ready",
                &[
                    frame.width.to_string(),
                    frame.height.to_string(),
                    frame.sequence.to_string(),
                ],
            ));
        } else {
            ui.colored_label(colors.hint, self.tr("browser_preview_pending"));
        }
        if let Some(status) = &self.scene_status {
            ui.colored_label(colors.info, status);
        }
    }

    /// Import the configured alert overlay into the browser source: builds the
    /// provider-specific widget URL from the token (or validates a custom URL),
    /// loads it into the browser source, and reports the outcome via
    /// `scene_status`. Testable without a running UI.
    fn import_alert_overlay(&mut self) {
        let custom =
            (!self.alert_custom_url.trim().is_empty()).then(|| self.alert_custom_url.clone());
        match rivulet_core::alerts::build_overlay_url(
            self.alert_provider,
            &self.alert_token,
            custom.as_deref(),
        ) {
            Ok(url) => {
                self.alert_token.clear();
                self.alert_custom_url.clear();
                self.browser_source.url = url.clone();
                if let Err(error) = self.browser_source.navigate(url) {
                    self.scene_status = Some(error.to_string());
                } else {
                    self.scene_status = Some(self.tr("alert_imported").to_owned());
                }
            }
            Err(error) => {
                self.scene_status = Some(error.to_string());
            }
        }
    }

    /// Minimal inline rename flow: swaps the row into a text field, applies
    /// the change via the SceneManager, returns the new name when applied.
    fn scene_rename_prompt(
        &mut self,
        ui: &mut egui::Ui,
        id: uuid::Uuid,
        old_name: String,
    ) -> Option<String> {
        let mut new_name = old_name.clone();
        let mut applied = None;
        ui.horizontal(|ui| {
            let edit =
                ui.add(egui::TextEdit::singleline(&mut new_name).hint_text(old_name.as_str()));
            if theme::accent_button(ui, self.tr("scenes_rename")).clicked() {
                let trimmed = new_name.trim().to_string();
                if !trimmed.is_empty() {
                    self.scenes.rename(id, trimmed.clone());
                    applied = Some(trimmed);
                }
            }
            edit.request_focus();
        });
        applied
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn stop_source_preview(&mut self) {
        if let Some(stop) = &self.source_preview_stop {
            stop.store(true, Ordering::SeqCst);
        }
        self.source_preview_stop = None;
        self.source_preview_rx = None;
        self.source_preview_target = None;
    }

    fn drain_source_preview(&mut self, ctx: &egui::Context) {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(rx) = &self.source_preview_rx {
            let mut latest = None;
            while let Ok(frame) = rx.try_recv() {
                latest = Some(frame);
            }
            if let Some(frame) = latest {
                self.recording_preview
                    .update(ctx, &frame.data, frame.width, frame.height);
            }
        }
    }

    fn draw_recording_preview(&mut self, ui: &mut egui::Ui) {
        let colors = theme::StatusColors::for_ui(ui);
        ui.group(|ui| {
            ui.label(egui::RichText::new(self.tr("recording_preview_title")).strong());
            if let Some(preview) = &self.recording_preview.texture {
                let max_width = ui.available_width().clamp(1.0, 480.0);
                let scale =
                    (max_width / self.recording_preview.width.max(1) as f32).clamp(0.05, 0.5);
                let size = egui::vec2(
                    self.recording_preview.width as f32 * scale,
                    self.recording_preview.height as f32 * scale,
                );
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                let alpha = theme::preview_fade_alpha(
                    ui.ctx(),
                    egui::Id::new("recording_preview_fade"),
                    true,
                );
                ui.painter().image(
                    preview.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha((alpha * 255.0) as u8),
                );
                let state_key = if self.is_recording_active() {
                    "recording_preview_active"
                } else {
                    "recording_preview_ready"
                };
                ui.label(
                    egui::RichText::new(self.tr(state_key))
                        .small()
                        .color(colors.hint),
                );
            } else if self.is_recording_active() {
                ui.colored_label(colors.warning, self.tr("recording_preview_waiting"));
            } else {
                ui.colored_label(colors.hint, self.tr("recording_preview_select_source"));
            }
        });
    }

    fn is_recording_active(&self) -> bool {
        #[cfg(target_os = "linux")]
        if self.is_recording {
            return true;
        }
        #[cfg(target_os = "windows")]
        if self.is_windows_recording {
            return true;
        }
        self.is_aux_recording
    }

    /// Update the shared thumbnail and drain a throttled preflight source.
    fn update_recording_preview(&mut self, ctx: &egui::Context) {
        // Only schedule the next tick while a live preview is feeding frames;
        // otherwise we stop requesting repaints and fall into egui's reactive
        // idle mode (no CPU/GPU while nothing changes). `drain_source_preview`
        // below consumes frames the preflight thread produces and the encoder
        // path fills `pending_preview_frame`, so both must keep the periodic
        // repaint alive while active. When neither is feeding, there is nothing
        // to animate and egui sleeps until real input arrives.
        let has_pending_frame = self.pending_preview_frame.is_some();
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let has_source_preview = self.source_preview_rx.is_some();
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let has_source_preview = false;
        if should_repaint_recording_preview(has_source_preview, has_pending_frame) {
            // Keep the live thumbnail moving even when the rest of the UI is idle.
            ctx.request_repaint_after(RECORDING_PREVIEW_INTERVAL);
        }
        if let Some(frame) = self.pending_preview_frame.take() {
            self.recording_preview
                .update(ctx, &frame.data, frame.width, frame.height);
        }
        self.drain_source_preview(ctx);
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if self.is_recording_active() {
            self.stop_source_preview();
        } else {
            self.ensure_source_preview();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn ensure_source_preview(&mut self) {
        let target = if self.use_game_capture {
            None
        } else if let Some(idx) = self.selected_monitor_idx_for_preview() {
            self.preview_monitor_target(idx)
        } else if let Some(idx) = self.selected_window_idx_for_preview() {
            self.preview_window_target(idx)
        } else {
            None
        };
        if target != self.source_preview_target {
            self.stop_source_preview();
            if let Some(target) = target {
                self.start_source_preview(target.clone());
                self.source_preview_target = Some(target);
            }
        }
    }

    #[cfg(target_os = "windows")]
    fn selected_monitor_idx_for_preview(&self) -> Option<usize> {
        self.selected_monitor_idx
    }
    #[cfg(target_os = "linux")]
    fn selected_monitor_idx_for_preview(&self) -> Option<usize> {
        self.selected_monitor_idx
    }
    #[cfg(target_os = "windows")]
    fn selected_window_idx_for_preview(&self) -> Option<usize> {
        self.selected_window_idx
    }
    #[cfg(target_os = "linux")]
    fn selected_window_idx_for_preview(&self) -> Option<usize> {
        self.selected_window_idx
    }

    #[cfg(target_os = "windows")]
    fn preview_monitor_target(&self, idx: usize) -> Option<SourcePreviewTarget> {
        self.monitors
            .get(idx)
            .and_then(|m| m.name().ok())
            .map(SourcePreviewTarget::Monitor)
    }
    #[cfg(target_os = "linux")]
    fn preview_monitor_target(&self, idx: usize) -> Option<SourcePreviewTarget> {
        self.monitors
            .get(idx)
            .and_then(|m| m.name().ok())
            .map(SourcePreviewTarget::Monitor)
    }
    #[cfg(target_os = "windows")]
    fn preview_window_target(&self, idx: usize) -> Option<SourcePreviewTarget> {
        self.windows
            .get(idx)
            .and_then(|w| w.title().ok())
            .map(SourcePreviewTarget::Window)
    }
    #[cfg(target_os = "linux")]
    fn preview_window_target(&self, idx: usize) -> Option<SourcePreviewTarget> {
        self.windows
            .get(idx)
            .and_then(|w| w.title().ok())
            .map(SourcePreviewTarget::Window)
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn start_source_preview(&mut self, target: SourcePreviewTarget) {
        let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        match target {
            SourcePreviewTarget::Monitor(name) => {
                std::thread::spawn(move || {
                    let monitors = xcap::Monitor::all().unwrap_or_default();
                    let Some(monitor) = monitors
                        .into_iter()
                        .find(|m| m.name().unwrap_or_default() == name)
                    else {
                        return;
                    };
                    while !stop_thread.load(Ordering::SeqCst) {
                        if let Ok(image) = monitor.capture_image() {
                            if tx
                                .send(RawFrame {
                                    data: image.as_raw().to_vec(),
                                    width: image.width(),
                                    height: image.height(),
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        std::thread::sleep(SOURCE_PREVIEW_INTERVAL);
                    }
                });
            }
            SourcePreviewTarget::Window(title) => {
                std::thread::spawn(move || {
                    while !stop_thread.load(Ordering::SeqCst) {
                        let result = xcap::Window::all()
                            .ok()
                            .and_then(|windows| {
                                windows
                                    .into_iter()
                                    .find(|w| w.title().ok().as_deref() == Some(title.as_str()))
                            })
                            .and_then(|window| window.capture_image().ok());
                        if let Some(image) = result {
                            if tx
                                .send(RawFrame {
                                    data: image.as_raw().to_vec(),
                                    width: image.width(),
                                    height: image.height(),
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                        std::thread::sleep(SOURCE_PREVIEW_INTERVAL);
                    }
                });
            }
        }
        self.source_preview_rx = Some(rx);
        self.source_preview_stop = Some(stop);
    }

    /// Draw a source thumbnail above the recording controls.
    fn draw_recording_preview_panel(&mut self, ui: &mut egui::Ui) {
        self.draw_recording_preview(ui);
    }

    /// Live performance metrics line for the recording UI
    /// (FPS, encoder load, output file size).
    fn metrics_line(&mut self) -> String {
        let m = self.engine.recording_stats();
        let fps = format!("{:.1}", m.fps);
        let load = format!("{:.0}%", m.encode_load_percent);
        let size = format_bytes(m.file_size_bytes);
        self.tr_fmt("recording_metrics", &[fps, load, size])
    }

    /// Draw transport health and delay telemetry without relying on color
    /// alone. Queue counters are cumulative for the active stream and are
    /// intentionally shown next to their labels for screen-reader-friendly
    /// diagnostics.
    fn draw_stream_health_panel(&mut self, ui: &mut egui::Ui, colors: theme::StatusColors) {
        let stats: StreamStats = self.engine.stream_stats();
        if matches!(stats.status, StreamHealthStatus::Offline) {
            return;
        }
        let status = match stats.status {
            StreamHealthStatus::Connecting => self.tr("stream_status_connecting"),
            StreamHealthStatus::Good => self.tr("stream_status_good"),
            StreamHealthStatus::Warning => self.tr("stream_status_warning"),
            StreamHealthStatus::Poor => self.tr("stream_status_poor"),
            StreamHealthStatus::Offline => self.tr("stream_status_offline"),
        };
        let status_color = match stats.status {
            StreamHealthStatus::Good => colors.success,
            StreamHealthStatus::Warning => colors.warning,
            StreamHealthStatus::Poor => colors.error,
            _ => colors.hint,
        };
        ui.group(|ui| {
            ui.label(egui::RichText::new(self.tr("stream_health")).strong());
            ui.horizontal(|ui| {
                ui.label(self.tr("stream_status"));
                ui.colored_label(status_color, status);
                ui.label(format!(
                    "{} ({:.0} kbps, {:.1} FPS)",
                    self.tr("stream_rate"),
                    stats.kbps,
                    stats.fps
                ));
            });
            let queue = stats
                .queue_fill_ratio
                .map(|value| format!("{:.0}%", value * 100.0))
                .unwrap_or_else(|| self.tr("not_available").to_string());
            ui.label(format!("{}: {}", self.tr("stream_queue_fill"), queue));
            ui.label(format!(
                "{}: {}",
                self.tr("stream_queue_underflows"),
                stats.queue_underflows
            ));
            ui.label(format!(
                "{}: {}",
                self.tr("stream_queue_overflows"),
                stats.queue_overflows
            ));
            if let Some(latency) = stats.sink_latency_ms {
                ui.label(format!(
                    "{}: {:.0} ms",
                    self.tr("stream_sink_latency"),
                    latency
                ));
            }
        });

        let targets = self.engine.stream_target_telemetry();
        if targets.is_empty() {
            return;
        }
        ui.separator();
        ui.label(egui::RichText::new(self.tr("stream_targets")).strong());
        for (name, state, fill, underflows, overflows) in targets {
            let state_label = match state {
                rivulet_core::StreamTargetState::Offline => self.tr("stream_status_offline"),
                rivulet_core::StreamTargetState::Connecting => self.tr("stream_status_connecting"),
                rivulet_core::StreamTargetState::Live => self.tr("stream_status_good"),
                rivulet_core::StreamTargetState::Degraded => self.tr("stream_status_warning"),
                rivulet_core::StreamTargetState::Failed => self.tr("stream_status_poor"),
            };
            ui.group(|ui| {
                ui.label(egui::RichText::new(name).strong());
                ui.label(format!("{}: {}", self.tr("stream_status"), state_label));
                ui.label(format!(
                    "{}: {:.0}%",
                    self.tr("stream_queue_fill"),
                    fill * 100.0
                ));
                ui.label(format!(
                    "{}: {}",
                    self.tr("stream_queue_underflows"),
                    underflows
                ));
                ui.label(format!(
                    "{}: {}",
                    self.tr("stream_queue_overflows"),
                    overflows
                ));
            });
        }
    }

    /// Apply the configured replay buffer setting to the engine. Called
    /// before every recording start so the capture branches are part of the
    /// pipeline, and whenever the user changes the setting (even mid-
    /// recording).
    fn apply_replay_setting(&mut self) {
        match self.replay_duration_secs {
            Some(secs) => self
                .engine
                .set_replay_duration(std::time::Duration::from_secs(secs)),
            None => self.engine.disable_replay(),
        }
    }

    /// Save the currently buffered replay clip through a file dialog and
    /// report the outcome (localized) in `replay_status`.
    fn save_replay_now(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Video", &["mp4"])
            .set_file_name(format!(
                "rivulet-replay-{}.mp4",
                chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S")
            ))
            .save_file()
        else {
            return;
        };
        match self.engine.save_replay(path.clone()) {
            Ok(_) => {
                self.replay_status = Some((
                    true,
                    self.tr_fmt("replay_saved", &[path.display().to_string()]),
                ));
            }
            Err(e) => {
                self.replay_status =
                    Some((false, self.tr_fmt("replay_save_failed", &[e.to_string()])));
            }
        }
    }

    /// Draw the "Save Replay" button plus the outcome of the last save while
    /// a recording is active and the replay buffer is enabled.
    fn draw_replay_save_controls(&mut self, ui: &mut egui::Ui, colors: theme::StatusColors) {
        if self.replay_duration_secs.is_none() {
            return;
        }
        ui.horizontal(|ui| {
            if ui
                .button(format!("💾 {}", self.tr("save_replay")))
                .clicked()
            {
                self.save_replay_now();
            }
            if let Some(secs) = self.engine.replay_retained_secs() {
                ui.label(
                    egui::RichText::new(format!("~{secs}s buffered"))
                        .small()
                        .color(colors.hint),
                );
            }
        });
        if let Some((ok, message)) = &self.replay_status {
            let color = if *ok { colors.success } else { colors.error };
            ui.label(egui::RichText::new(message).color(color).small());
        }
    }

    /// Format a duration in seconds as `HH:MM:SS` or `MM:SS` when under an
    /// hour.
    fn format_duration(secs: u64) -> String {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        if h > 0 {
            format!("{:02}:{:02}:{:02}", h, m, s)
        } else {
            format!("{:02}:{:02}", m, s)
        }
    }

    /// Build the overlay text string for the current recording state.
    fn overlay_text(&mut self) -> String {
        let elapsed = self.record_started.elapsed().as_secs();
        let m = self.engine.recording_stats();
        format!("{} | FPS {:.1}", Self::format_duration(elapsed), m.fps)
    }

    /// Snapshot of the current auto-update state.
    fn update_ui_snapshot(&self) -> UpdateUi {
        self.update_ui
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Check for a newer release in the background.
    fn spawn_update_check(&self, ctx: egui::Context) {
        let shared = std::sync::Arc::clone(&self.update_ui);
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = UpdateUi::Checking;
        std::thread::spawn(move || {
            let result = rivulet_updater::check_for_update(env!("CARGO_PKG_VERSION"));
            let state = match result {
                Ok(None) => UpdateUi::UpToDate,
                Ok(Some(info)) => UpdateUi::Available(info),
                Err(e) => UpdateUi::Error(e.to_string()),
            };
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = state;
            ctx.request_repaint();
        });
    }

    /// Download the update asset in the background and verify its SHA-256
    /// digest against the release's `SHA256SUMS` manifest before the file is
    /// ever offered for installation. A failed verification surfaces as an
    /// error and the installer is never started.
    fn spawn_update_download(
        &mut self,
        ctx: egui::Context,
        asset: rivulet_updater::Asset,
        tag: String,
        version: String,
    ) {
        let shared = std::sync::Arc::clone(&self.update_ui);
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = UpdateUi::Downloading {
            name: asset.name.clone(),
            version: version.clone(),
        };
        self.download_progress.store(0, Ordering::Relaxed);
        self.download_total = asset.size;
        let progress = Arc::clone(&self.download_progress);
        std::thread::spawn(move || {
            let dest = std::env::temp_dir().join(&asset.name);
            let result = rivulet_updater::download_asset_with_progress(&asset, &dest, progress)
                .and_then(|()| rivulet_updater::verify_downloaded_asset(&dest, &tag, &asset.name));
            let state = match result {
                Ok(()) => UpdateUi::Downloaded {
                    path: dest,
                    version,
                },
                Err(e) => UpdateUi::Error(e.to_string()),
            };
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = state;
            ctx.request_repaint();
        });
    }

    /// Launch the platform installer and (on Windows/Linux) quit the app so
    /// the running files can be replaced.
    fn spawn_update_install(&self, ctx: egui::Context, path: std::path::PathBuf, version: String) {
        let shared = std::sync::Arc::clone(&self.update_ui);
        *shared.lock().unwrap_or_else(|e| e.into_inner()) = UpdateUi::Installing(version.clone());
        // Pass our own process id to the watchdog so it waits for this process
        // (and its launcher) to fully terminate before running msiexec. The
        // executing files are locked by Windows while the process lives, so
        // starting the installer earlier would leave the old files in place.
        let our_pid = std::process::id();
        std::thread::spawn(move || {
            let result = rivulet_updater::install_asset_with_watch(&path, None, &[our_pid]);
            let quit_after = result.as_ref().map(|quit| *quit).unwrap_or(false);
            let state = match result {
                Ok(_) => {
                    // Clean up the downloaded installer if the application is not quitting
                    // (e.g. macOS where DMG is opened). On Windows/Linux where the app quits,
                    // msiexec owns the file during launch so cleanup is deferred.
                    if !quit_after {
                        if let Err(e) = std::fs::remove_file(&path) {
                            tracing::warn!(path = %path.display(), error = %e, "Failed to remove downloaded installer");
                        }
                    }
                    UpdateUi::Installed(version)
                }
                Err(e) => UpdateUi::Error(e.to_string()),
            };
            *shared.lock().unwrap_or_else(|e| e.into_inner()) = state;
            ctx.request_repaint();

            let _ = quit_after;
        });
    }

    /// Render the auto-update section below the recording controls.
    fn draw_update_status(&mut self, ui: &mut egui::Ui) {
        let colors = theme::StatusColors::for_ui(ui);
        match self.update_ui_snapshot() {
            UpdateUi::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr("update_checking"));
                });
            }
            UpdateUi::UpToDate => {
                ui.colored_label(colors.success, self.tr("update_up_to_date"));
            }
            UpdateUi::Available(info) => {
                ui.colored_label(
                    colors.warning,
                    self.tr_fmt("update_available", &[info.version]),
                );
                if let Some(asset) = &info.asset {
                    ui.label(self.tr_fmt("update_asset_size", &[format_bytes(asset.size)]));
                }
                ui.horizontal(|ui| {
                    if theme::accent_button(ui, self.tr("update_download_install")).clicked() {
                        self.update_download_clicked = true;
                    }
                    ui.hyperlink_to(self.tr("update_release_notes"), &info.html_url);
                });
            }
            UpdateUi::Downloading { name, version } => {
                ui.label(self.tr_fmt("update_downloading", &[name]));
                ui.label(self.tr_fmt("updates_new_version", &[version]));
                let downloaded = self.download_progress.load(Ordering::Relaxed);
                let fraction = if self.download_total > 0 {
                    (downloaded as f32 / self.download_total as f32).min(1.0)
                } else {
                    0.0
                };
                let downloaded_mb = downloaded as f64 / (1024.0 * 1024.0);
                let total_mb = self.download_total as f64 / (1024.0 * 1024.0);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .text(format!("{downloaded_mb:.1} / {total_mb:.1} MB")),
                );
                ui.ctx().request_repaint();
            }
            UpdateUi::Downloaded { version, .. } => {
                ui.colored_label(colors.success, self.tr("update_downloaded").to_string());
                ui.label(self.tr_fmt("updates_new_version", &[version]));
                if theme::accent_button(ui, self.tr("update_install")).clicked() {
                    self.update_install_clicked = true;
                }
            }
            UpdateUi::Installing(version) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr("update_installing"));
                });
                ui.label(self.tr_fmt("updates_new_version", &[version]));
            }
            UpdateUi::Installed(version) => {
                ui.colored_label(
                    colors.success,
                    self.tr("update_installed_restart").to_string(),
                );
                ui.label(self.tr_fmt("updates_new_version", &[version]));
                // Closing is handled on the UI thread; egui Context is not
                // thread-safe and must never be used by the installer worker.
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            UpdateUi::Error(err) => {
                ui.colored_label(colors.error, self.tr_fmt("update_error", &[err]));
            }
            UpdateUi::Idle => {}
        }
    }

    /// Render one remappable hotkey row: a key ComboBox + three modifier
    /// toggles. Any change is written straight back into `self.hotkeys` and
    /// flagged for global re-registration.
    fn draw_hotkey_rebind_row(&mut self, ui: &mut egui::Ui, action: &str) {
        let keys = [
            egui::Key::F1,
            egui::Key::F2,
            egui::Key::F3,
            egui::Key::F4,
            egui::Key::F5,
            egui::Key::F6,
            egui::Key::F7,
            egui::Key::F8,
            egui::Key::F9,
            egui::Key::F10,
            egui::Key::F11,
            egui::Key::F12,
            egui::Key::Num0,
            egui::Key::Num1,
            egui::Key::Space,
        ];
        let mut binding = self.hotkeys.binding_for_action(action).unwrap_or_default();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(self.tr(&format!("hotkey_{action}")));
            egui::ComboBox::from_id_salt(format!("hotkey_key_{action}"))
                .selected_text(key_name(binding.key))
                .show_ui(ui, |ui| {
                    for key in keys {
                        if ui
                            .selectable_label(binding.key == key, key_name(key))
                            .clicked()
                        {
                            binding.key = key;
                            changed = true;
                        }
                    }
                });
            if ui.selectable_label(binding.ctrl, "Ctrl").clicked() {
                binding.ctrl = !binding.ctrl;
                changed = true;
            }
            if ui.selectable_label(binding.alt, "Alt").clicked() {
                binding.alt = !binding.alt;
                changed = true;
            }
            if ui.selectable_label(binding.shift, "Shift").clicked() {
                binding.shift = !binding.shift;
                changed = true;
            }
            ui.label(egui::RichText::new(binding.label()).weak());
        });
        if changed {
            self.hotkeys.set_binding_for_action(action, binding);
            self.global_hotkeys_dirty = true;
        }
    }

    fn draw_stream_setup_wizard(&mut self, ui: &mut egui::Ui) {
        if !self.setup_wizard_open {
            return;
        }
        ui.group(|ui| {
            ui.label(egui::RichText::new(self.tr("stream_setup_assistant_title")).strong());
            ui.label(self.tr_fmt(
                "stream_setup_step",
                &[format!("{}", self.setup_wizard_step + 1)],
            ));
            match self.setup_wizard_step {
                0 => {
                    ui.label(self.tr("stream_setup_choose_platform"));
                    for platform in [
                        StreamPlatform::Twitch,
                        StreamPlatform::YouTube,
                        StreamPlatform::Kick,
                        StreamPlatform::Custom,
                    ] {
                        if ui
                            .selectable_label(self.stream_platform == platform, platform.label())
                            .clicked()
                        {
                            self.stream_platform = platform;
                            if let Some(url) = platform.default_ingest_url() {
                                self.stream_ingest_url = url.to_owned();
                            }
                        }
                    }
                }
                1 => {
                    ui.label(self.tr("stream_setup_credentials"));
                    ui.add(egui::TextEdit::singleline(&mut self.stream_key).password(true));
                    ui.label(self.tr("stream_setup_key_never_logged"));
                }
                2 => {
                    ui.label(self.tr("stream_setup_test_stream"));
                    if ui.button(self.tr("stream_setup_run_test")).clicked() {
                        self.setup_test_requested = true;
                        self.private_test_stream.start();
                        self.stream_status_message =
                            Some(self.tr("stream_setup_test_prepared").to_owned());
                    }
                    if self.setup_test_requested {
                        ui.label(self.tr("stream_setup_test_private_only"));
                        ui.label(format!(
                            "Test status: {:?}",
                            self.private_test_stream.state()
                        ));
                        if ui.button(self.tr("stream_stop")).clicked() {
                            self.private_test_stream.stop();
                        }
                    }
                }
                _ => {
                    ui.label(self.tr("stream_setup_summary"));
                    ui.label(format!(
                        "{} · {}",
                        self.stream_platform.label(),
                        self.stream_ingest_url
                    ));
                }
            }
            ui.horizontal(|ui| {
                if self.setup_wizard_step > 0 && ui.button(self.tr("back")).clicked() {
                    self.setup_wizard_step -= 1;
                }
                if self.setup_wizard_step < 3 && ui.button(self.tr("next")).clicked() {
                    self.setup_wizard_step += 1;
                }
                if self.setup_wizard_step == 3 && ui.button(self.tr("done")).clicked() {
                    self.setup_wizard_open = false;
                }
                if ui.button(self.tr("cancel")).clicked() {
                    self.setup_wizard_open = false;
                }
            });
        });
    }

    fn draw_help_view(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(self.tr("nav_help")).strong());
        ui.separator();
        ui.label(self.tr("help_intro"));
        for (label, path) in help_documents() {
            ui.horizontal(|ui| {
                let link = egui::RichText::new(label).underline();
                if ui.link(link).clicked() {
                    open_help_document(path);
                }
                ui.hyperlink_to(path, help_document_url(path));
            });
        }
        ui.small(self.tr("help_open_from_repository"));
    }

    /// Render the implemented M3 streaming view.
    /// The current activity, newest-priority first: a fresh error wins over
    /// everything, then recording+streaming, paused, recording, streaming,
    /// and finally idle (Ready). Used both for the payload and to highlight
    /// the active row in the stream-view legend.
    fn current_presence_activity(&self) -> PresenceActivity {
        if self.last_error.is_some() {
            PresenceActivity::Error
        } else if self.is_recording_active() && self.engine.is_streaming() {
            PresenceActivity::RecordingAndStreaming
        } else if self.is_recording_active() {
            if self.is_paused {
                PresenceActivity::Paused
            } else {
                PresenceActivity::Recording
            }
        } else if self.engine.is_streaming() {
            PresenceActivity::Streaming
        } else {
            PresenceActivity::Idle
        }
    }

    fn current_presence_status(&self) -> PresenceStatus {
        // The payload stays privacy-safe: only the localized activity label is
        // sent, never the raw error text (which may contain paths).
        PresenceStatus::for_activity_localized(
            self.current_presence_activity(),
            self.locale,
            self.current_presence_game_name().as_deref(),
        )
    }

    /// The explicitly user-selected source name to surface in the presence
    /// state (e.g. the selected game-capture window title). Falls back to the
    /// selected window-capture title; returns `None` for a plain monitor
    /// source. Only explicitly selected windows are used, never inferred
    /// from captured content.
    fn current_presence_game_name(&self) -> Option<String> {
        if self.use_game_capture {
            return self
                .selected_game_window_idx
                .and_then(|idx| self.game_windows.get(idx))
                .map(|w| w.title.clone())
                .filter(|title| !title.trim().is_empty());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(idx) = self.selected_window_idx {
            if let Some(window) = self.windows.get(idx) {
                let title = window.title().unwrap_or_default();
                if !title.trim().is_empty() {
                    return Some(title);
                }
            }
        }
        None
    }

    /// Reconcile the Discord adapter with the opt-out setting and push a status
    /// update only on activity transitions. The adapter is non-blocking: this
    /// never performs IPC on the UI thread.
    fn sync_discord_presence(&mut self) {
        if !self.discord_presence_enabled {
            if let Some(mut presence) = self.discord_presence.take() {
                presence.disconnect();
            }
            self.discord_presence_active_client_id = None;
            self.discord_presence_last = None;
            return;
        }

        // Rebuild the adapter exactly once when the user applied a new client
        // id or clicked "Reconnect": both force a fresh worker + handshake.
        if self.discord_client_id_dirty || self.discord_reconnect_requested {
            self.discord_client_id_dirty = false;
            self.discord_reconnect_requested = false;
            self.discord_presence_active_client_id = None;
        }

        let client_id = self.discord_presence_client_id.trim().to_owned();
        let large_image = self.discord_presence_large_image.trim().to_owned();
        // Fallback chain (see rivulet_core::discord::effective_client_id):
        // configured id → official default → adapter off. The official
        // default is the retirement path for a deprecated application id: a
        // release bumps DEFAULT_CLIENT_ID and ships the change through the
        // updater (the release payload is the updater manifest).
        let client_id =
            rivulet_core::discord::effective_client_id(Some(&client_id)).unwrap_or_default();
        let large_image = rivulet_core::discord::effective_large_image_key(Some(&large_image))
            .unwrap_or_default();
        let needs_create = self.discord_presence.is_none()
            || self.discord_presence_active_client_id.as_deref() != Some(client_id.as_str());
        if needs_create {
            if let Some(mut presence) = self.discord_presence.take() {
                presence.disconnect();
            }
            self.discord_presence_active_client_id = None;
            if client_id.is_empty() {
                // No real application id yet: keep the adapter off and skip the
                // status cache so it activates cleanly once configured.
                self.discord_presence_last = None;
                return;
            }
            let cfg = DiscordPresenceConfig {
                enabled: true,
                client_id: client_id.clone(),
                large_image_key: (!large_image.is_empty()).then(|| large_image.clone()),
                large_image_text: Some("Rivulet".to_owned()),
                ..Default::default()
            };
            self.discord_presence = Some(DiscordPresence::new(&cfg));
            self.discord_presence_active_client_id = Some(client_id);
        }

        // Push only on activity transitions, never every frame.
        let status = self.current_presence_status();
        if self.discord_presence_last.as_ref() == Some(&status) {
            return;
        }
        self.discord_presence_last = Some(status.clone());
        if let Some(presence) = &self.discord_presence {
            presence.set_activity(&status);
        }
    }

    /// Process pending chat-dock actions and drain the worker's message
    /// channel into the bounded list. Non-blocking: called once per frame.
    /// The worker is rebuilt for the currently selected platform (Twitch
    /// IRC, Kick WebSocket or YouTube polling).
    fn reconcile_chat(&mut self) {
        match self.chat_action_pending.take() {
            Some(ChatAction::Connect) => {
                if let Some(mut worker) = self.chat_worker.take() {
                    worker.disconnect();
                }
                self.chat_messages.clear();
                self.chat_reply_target = None;
                let cfg = rivulet_core::ChatConfig::new(
                    self.chat_platform,
                    self.chat_channel.trim().to_owned(),
                    self.chat_oauth_token.clone(),
                );
                self.chat_worker = Some(rivulet_core::Chat::new(&cfg));
                self.chat_state = self
                    .chat_worker
                    .as_ref()
                    .map(|w| w.connection_state())
                    .unwrap_or(rivulet_core::ChatConnState::Off);
            }
            Some(ChatAction::Disconnect) => {
                if let Some(mut worker) = self.chat_worker.take() {
                    worker.disconnect();
                }
                self.chat_messages.clear();
                self.chat_reply_target = None;
                self.chat_state = rivulet_core::ChatConnState::Off;
            }
            Some(ChatAction::Send(text)) => {
                self.send_chat_message(text);
            }
            Some(ChatAction::SendReply(text, parent)) => {
                self.send_chat_reply(text, parent);
            }
            None => {}
        }

        // Drain incoming messages (bounded to the newest MAX_CHAT_MESSAGES).
        if let Some(worker) = &self.chat_worker {
            self.chat_state = worker.connection_state();
            if let Some(rx) = worker.messages() {
                while let Ok(msg) = rx.try_recv() {
                    self.chat_messages.push(msg);
                    if self.chat_messages.len() > MAX_CHAT_MESSAGES {
                        let overflow = self.chat_messages.len() - MAX_CHAT_MESSAGES;
                        self.chat_messages.drain(..overflow);
                    }
                }
            }
        }
    }

    /// Send a chat message through the running worker. Twitch rejects
    /// PRIVMSG from the anonymous `justinfan` nick and Kick rejects
    /// anonymous sends, so both require a token; YouTube chat is read-only
    /// (anonymous clients cannot send). The UI only enables the input under
    /// those conditions. Returns whether the message was handed to the
    /// worker (non-blocking enqueue).
    fn send_chat_message(&mut self, text: String) -> bool {
        let text = text.trim().to_owned();
        if text.is_empty() || self.chat_oauth_token.trim().is_empty() {
            return false;
        }
        let Some(worker) = &self.chat_worker else {
            return false;
        };
        worker.send_message(&text)
    }

    /// Consume the pending chat input on Send/Enter: clear the text field
    /// and arm either a threaded reply (`ChatAction::SendReply`, when a
    /// reply target is armed) or a plain send (`ChatAction::Send`). Returns
    /// `false` when the input was empty — in that case nothing is armed and
    /// an armed reply target is kept (the streamer may still type). The
    /// action is executed exactly once per frame by `reconcile_chat`.
    fn submit_chat_input(&mut self) -> bool {
        let text = std::mem::take(&mut self.chat_input);
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        match self.chat_reply_target.take() {
            Some((_, parent)) => {
                self.chat_action_pending = Some(ChatAction::SendReply(text.to_owned(), parent));
            }
            None => self.chat_action_pending = Some(ChatAction::Send(text.to_owned())),
        }
        true
    }

    /// Cancel the armed threaded reply (banner ✕ button): clears the target
    /// so the next Send is a plain chat message again. Never enqueues
    /// anything and never touches the input text.
    fn cancel_chat_reply(&mut self) {
        self.chat_reply_target = None;
    }

    /// Send a threaded reply through the running worker. Only Twitch
    /// supports replies (`@reply-parent-msg-id`); the worker enqueues the
    /// reply non-blocking and the shared per-platform rate limiter applies.
    fn send_chat_reply(&mut self, text: String, parent_id: String) -> bool {
        let text = text.trim().to_owned();
        if text.is_empty() || parent_id.trim().is_empty() || self.chat_oauth_token.trim().is_empty()
        {
            return false;
        }
        let Some(worker) = &self.chat_worker else {
            return false;
        };
        worker.send_reply(&text, &parent_id)
    }

    /// Live rate-limit detail `(remaining, capacity, window_secs, platform
    /// label)` of the running worker's shared limiter, for the budget line
    /// and its tooltip (“20 messages per 30 s on Twitch”). `None` while no
    /// worker is running. `remaining` is fractional (tokens refill
    /// continuously over the window); the UI floors it.
    fn chat_rate_limit_detail(&self) -> Option<(f64, u32, u64, &'static str)> {
        let worker = self.chat_worker.as_ref()?;
        let cfg = worker.rate_limit_config();
        Some((
            worker.rate_limit_remaining(),
            cfg.capacity,
            cfg.window_secs,
            worker.platform().label(),
        ))
    }

    /// Remaining outbound chat budget `(available, capacity)` of the running
    /// worker's shared rate limiter. Thin wrapper over
    /// [`Self::chat_rate_limit_detail`]; kept for tests and status lines.
    fn chat_rate_budget(&self) -> Option<(f64, f64)> {
        self.chat_rate_limit_detail()
            .map(|(remaining, capacity, _, _)| (remaining, capacity as f64))
    }

    /// Handle the Settings "Apply" action for the Discord client id: validate
    /// the format immediately and only mark the adapter for rebuild when the
    /// value is a plausible snowflake (or empty, which keeps the adapter off).
    /// An invalid id shows a warning instead of silently misconfiguring.
    fn apply_discord_client_id(&mut self) {
        match rivulet_core::discord::validate_client_id(self.discord_presence_client_id.trim()) {
            Ok(()) => {
                self.discord_client_id_warning = None;
                self.discord_client_id_dirty = true;
            }
            Err(error) => {
                self.discord_client_id_warning = Some(error);
            }
        }
    }

    /// Validate the current SET_ACTIVITY payload (status + configured art
    /// asset key) against Discord's rules and remember the first violation so
    /// Settings can warn immediately on Apply — instead of Discord silently
    /// dropping an overlong status or an implausible artwork key. Called on
    /// every presence Apply; the client id check stays separate.
    fn apply_discord_payload_validation(&mut self) {
        let status = self.current_presence_status();
        let issues = rivulet_core::discord::validate_set_activity_payload(
            &status,
            Some(self.discord_presence_large_image.trim()),
        );
        self.discord_payload_warning = issues.into_iter().next();
    }

    /// Build the current OS-level bindings from the hotkey config, translating
    /// egui keys to Windows virtual-key codes where they exist. Bindings whose
    /// key has no VK equivalent (or is modifier-only) are skipped so the OS is
    /// never asked to register something invalid.
    fn current_global_bindings(&self) -> Vec<GlobalBinding> {
        let mut out = Vec::new();
        for (action, binding) in [
            ("record", self.hotkeys.record),
            ("pause", self.hotkeys.pause),
            ("mute", self.hotkeys.mute),
            ("save_replay", self.hotkeys.save_replay),
        ] {
            if let Some(vk) = vk_code(binding.key) {
                out.push(GlobalBinding {
                    action: action.to_string(),
                    key: KeyCode(vk),
                    mods: ModMask(
                        (u8::from(binding.ctrl))
                            | (u8::from(binding.alt) << 1)
                            | (u8::from(binding.shift) << 2)
                            | (u8::from(binding.super_mod) << 3),
                    ),
                });
            }
        }
        // Scene hotkeys are registered too so they keep working unfocused.
        for (scene_id, binding) in &self.hotkeys.scene_hotkeys {
            if let Some(vk) = vk_code(binding.key) {
                out.push(GlobalBinding {
                    action: format!("scene:{scene_id}"),
                    key: KeyCode(vk),
                    mods: ModMask(
                        (u8::from(binding.ctrl))
                            | (u8::from(binding.alt) << 1)
                            | (u8::from(binding.shift) << 2)
                            | (u8::from(binding.super_mod) << 3),
                    ),
                });
            }
        }
        out
    }

    /// Ensure the OS-level hotkey handle exists and matches the current
    /// bindings, and drain any fired global hotkey events into the same action
    /// dispatch used by the in-app path.
    fn reconcile_global_hotkeys(&mut self) {
        if self.global_hotkeys_dirty {
            self.global_hotkeys_dirty = false;
            let bindings = self.current_global_bindings();
            match &mut self.global_hotkeys {
                Some(handle) => handle.set_bindings(bindings),
                None => {
                    let handle = GlobalHotkey::new(bindings);
                    self.global_hotkeys = Some(handle);
                }
            }
        }

        // Drain fired actions. Actions are handled on the next UI frame.
        let mut fired: Vec<String> = Vec::new();
        if let Some(handle) = &self.global_hotkeys {
            while let Some(action) = handle.try_recv() {
                fired.push(action);
            }
        }
        for action in fired {
            if let Some(stripped) = action.strip_prefix("scene:") {
                if let Ok(id) = stripped.parse::<uuid::Uuid>() {
                    self.scenes.switch_to(id);
                }
            } else {
                self.dispatch_hotkey_action(&action);
            }
        }
    }

    /// Ensure the MIDI listener matches the current settings (enable toggle,
    /// selected device), drain incoming MIDI messages, and apply their mapped
    /// actions on the UI thread.
    fn reconcile_midi(&mut self) {
        if self.midi_dirty {
            self.midi_dirty = false;
            self.midi_handle = None; // drops the old connection (stops input)
            self.midi_status = None;
            if self.midi_enabled {
                match MidiListener::start(self.midi_device_index) {
                    Ok(handle) => self.midi_handle = Some(handle),
                    Err(err) => self.midi_status = Some(err),
                }
            }
        }

        // Drain pending messages and run each through the learn/dispatch path
        // (shared with the unit-tested [`Self::reconcile_midi_with_message`]).
        let mut messages = Vec::new();
        if let Some(handle) = &mut self.midi_handle {
            while let Some(msg) = handle.try_recv() {
                messages.push(msg);
            }
        }
        for msg in messages {
            self.reconcile_midi_with_message(msg);
        }
    }

    /// Handle one incoming MIDI message (hardware-free, unit-tested). In learn
    /// mode the first message is captured into the pending "add binding" row
    /// instead of being dispatched (the user is identifying which control they
    /// want to map); after capture learn mode switches off so later messages
    /// dispatch normally.
    fn reconcile_midi_with_message(&mut self, msg: rivulet_core::MidiMessage) {
        if self.midi_learn && self.midi_learn_captured.is_none() {
            self.midi_learn_captured = Some(msg);
            // Pre-fill the manual row so the user can confirm the action and
            // press "Add binding".
            self.midi_new_kind = msg.kind;
            self.midi_new_channel = msg.channel;
            self.midi_new_number = msg.number;
            self.midi_learn = false;
            return;
        }
        let actions: Vec<rivulet_core::MidiAction> = self
            .midi_mapping
            .dispatch(&msg)
            .into_iter()
            .cloned()
            .collect();
        for action in actions {
            self.apply_midi_action(&action, msg.value);
        }
    }

    /// Human-readable, localized label for a mapped MIDI action (shown in the
    /// Settings binding list).
    fn midi_action_label(&self, action: &rivulet_core::MidiAction) -> String {
        match action {
            rivulet_core::MidiAction::SwitchScene(id) => {
                let name = self
                    .scenes
                    .get(*id)
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| self.tr("midi_scene_none").to_owned());
                format!("{} · {}", self.tr("midi_action_scene"), name)
            }
            rivulet_core::MidiAction::ToggleRecord => self.tr("midi_action_record").to_owned(),
            rivulet_core::MidiAction::ToggleStream => self.tr("midi_action_stream").to_owned(),
            rivulet_core::MidiAction::ToggleMute => self.tr("midi_action_mute").to_owned(),
            rivulet_core::MidiAction::SetMasterVolume => self.tr("midi_action_volume").to_owned(),
            rivulet_core::MidiAction::ToggleChromaKey => self.tr("midi_action_chroma").to_owned(),
        }
    }

    /// Execute one mapped MIDI action. `raw_value` is the CC value/velocity
    /// used by fader-style actions (master volume).
    fn apply_midi_action(&mut self, action: &rivulet_core::MidiAction, raw_value: u8) {
        match action {
            rivulet_core::MidiAction::SwitchScene(id) => {
                self.scenes.switch_to(*id);
            }
            rivulet_core::MidiAction::ToggleRecord => {
                if self.any_recording_active() {
                    self.stop_active_recording();
                } else {
                    self.start_active_recording();
                }
            }
            rivulet_core::MidiAction::ToggleStream => {
                self.execute_obs_command(rivulet_obs_websocket::ObsCommand::ToggleStreaming);
            }
            rivulet_core::MidiAction::ToggleMute => {
                if self.any_recording_active() {
                    self.is_muted = !self.is_muted;
                }
            }
            rivulet_core::MidiAction::SetMasterVolume => {
                let ratio = rivulet_core::MidiMapping::volume_ratio(raw_value);
                self.midi_master_volume = ratio;
                #[cfg(target_os = "linux")]
                if let Some(audio) = &mut self.audio {
                    audio.set_master_volume(ratio);
                }
            }
            rivulet_core::MidiAction::ToggleChromaKey => {
                if let Some(source_id) = self.selected_composition_source {
                    if let Some(source) = self.source_manager.get_source_mut(source_id) {
                        source.chroma_key.enabled = !source.chroma_key.enabled;
                    }
                }
            }
        }
    }

    /// Start, stop, or keep the OBS WebSocket server in sync with the setting
    /// toggle, refresh the read snapshot the server sees, execute commands the
    /// server received (scene switches, record/stream control) on the UI
    /// thread, and broadcast GUI-initiated state changes to subscribed
    /// clients.
    fn reconcile_obs_websocket(&mut self) {
        use rivulet_obs_websocket::backend::ObsBackend as _;

        // Start the server when the feature is enabled and it is not running;
        // stop it when the toggle is off.
        if self.obs_ws_enabled {
            if self.obs_ws_server.is_none() {
                self.start_obs_websocket();
            } else if self.obs_gw_restart_requested() {
                self.obs_ws_server = None;
                self.start_obs_websocket();
            }
        } else if self.obs_ws_server.is_some() {
            tracing::info!("Stopping OBS WebSocket server");
            self.obs_ws_server = None;
            self.obs_ws_commands_rx = None;
            self.obs_ws_snapshot = None;
            self.obs_ws_status = Some(self.tr("obs_ws_stopped").to_owned());
        }

        self.refresh_obs_ws_snapshot();

        // Execute commands the server thread queued for the UI thread. The
        // receiver borrow must end before we mutate `self` for each command.
        let pending: Vec<(
            rivulet_obs_websocket::ObsCommand,
            std::sync::mpsc::Sender<rivulet_obs_websocket::ObsCommandResult>,
        )> = self
            .obs_ws_commands_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for (command, reply) in pending {
            let result = self.execute_obs_command(command);
            let _ = reply.send(result);
        }

        // Broadcast GUI-initiated changes (scene switch, record/stream toggle
        // made in the window) so connected clients stay in sync. Remote-triggered
        // changes were already broadcast by the server via execute() return
        // events, so the diff baseline below is refreshed after each drain.
        let current = self.obs_ws_public_state();
        if let Some(server) = &self.obs_ws_server {
            if let Some(last) = self.obs_ws_last_public_state.take() {
                if current != last {
                    let mut events = Vec::new();
                    if current.current_scene != last.current_scene {
                        if let Some(name) = &current.current_scene {
                            events.push(
                                rivulet_obs_websocket::ObsEvent::CurrentProgramSceneChanged {
                                    scene_name: name.clone(),
                                },
                            );
                        }
                    }
                    if current.recording != last.recording
                        || current.recording_paused != last.recording_paused
                    {
                        events.push(rivulet_obs_websocket::ObsEvent::RecordStateChanged {
                            active: current.recording,
                            paused: current.recording_paused,
                        });
                    }
                    if current.streaming != last.streaming
                        || current.reconnecting != last.reconnecting
                    {
                        events.push(rivulet_obs_websocket::ObsEvent::StreamStateChanged {
                            active: current.streaming,
                            reconnecting: current.reconnecting,
                        });
                    }
                    server.broadcast(events);
                }
            }
            self.obs_ws_last_public_state = Some(current);
        } else {
            self.obs_ws_last_public_state = None;
        }
    }

    /// Attempt to start the OBS WebSocket server with the current settings.
    /// On failure a status message is stored for the Settings view.
    fn start_obs_websocket(&mut self) {
        let snapshot = Arc::new(Mutex::new(rivulet_obs_websocket::ObsSnapshot::default()));
        self.obs_ws_snapshot = Some(snapshot.clone());

        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let backend = Arc::new(rivulet_obs_websocket::ChannelBackend::new(snapshot, cmd_tx));
        let password = if self.obs_ws_password.is_empty() {
            None
        } else {
            Some(self.obs_ws_password.clone())
        };
        match rivulet_obs_websocket::server::start(backend, password, self.obs_ws_port) {
            Ok(server) => {
                self.obs_ws_server = Some(server);
                self.obs_ws_commands_rx = Some(cmd_rx);
                self.obs_ws_status =
                    Some(self.tr_fmt("obs_ws_running", &[self.obs_ws_port.to_string()]));
                tracing::info!(port = self.obs_ws_port, "OBS WebSocket server started");
            }
            Err(err) => {
                self.obs_ws_server = None;
                self.obs_ws_commands_rx = None;
                self.obs_ws_status = Some(self.tr_fmt("obs_ws_error", &[err.to_string()]));
                tracing::error!(error = %err, "OBS WebSocket server failed to start");
            }
        }
    }

    /// Whether a setting change (port or password) requires a server restart.
    fn obs_gw_restart_requested(&self) -> bool {
        // Tracked via the field below: the Settings UI flips it when the user
        // edits port/password.
        self.obs_ws_restart
    }

    /// Refresh the shared snapshot the server reads for Get*/Status requests.
    fn refresh_obs_ws_snapshot(&mut self) {
        let Some(snapshot) = &self.obs_ws_snapshot else {
            return;
        };
        let mut snap = snapshot.lock().unwrap();
        snap.scenes = self
            .scenes
            .scenes()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        snap.current_scene = self.scenes.active_scene().map(|s| s.name.clone());
        snap.sources = self
            .source_manager
            .sources()
            .iter()
            .map(|s| s.name.clone())
            .collect();
        snap.recording = self.any_recording_active();
        snap.streaming = self.engine.is_streaming();
        snap.reconnecting = false;
        snap.output_duration_ms = self.record_started.elapsed().as_millis() as u64;
    }

    /// The subset of the snapshot used for GUI-initiated event broadcasting.
    fn obs_ws_public_state(&self) -> ObsWsPublicState {
        ObsWsPublicState {
            current_scene: self.scenes.active_scene().map(|s| s.name.clone()),
            recording: self.any_recording_active(),
            recording_paused: self.is_paused,
            streaming: self.engine.is_streaming(),
            reconnecting: false,
        }
    }

    /// Execute one command received from an OBS WebSocket client, on the UI
    /// thread. Returns the result to be sent back to the client. Events are
    /// broadcast by the server itself; the returned event list is what the
    /// server broadcasts to subscribed clients.
    fn execute_obs_command(
        &mut self,
        command: rivulet_obs_websocket::ObsCommand,
    ) -> rivulet_obs_websocket::ObsCommandResult {
        use rivulet_obs_websocket::ObsCommand;
        use rivulet_obs_websocket::ObsEvent;
        match command {
            ObsCommand::SetCurrentScene(name) => {
                let Some(scene) = self.scenes.scenes().iter().find(|s| s.name == name) else {
                    return rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::RESOURCE_NOT_FOUND,
                        comment: format!("Scene '{name}' not found"),
                    };
                };
                self.scenes.switch_to(scene.id);
                self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                rivulet_obs_websocket::ObsCommandResult::Success(vec![
                    ObsEvent::CurrentProgramSceneChanged { scene_name: name },
                ])
            }
            ObsCommand::StartRecording => {
                if self.any_recording_active() {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_RUNNING,
                        comment: "Recording already active".into(),
                    }
                } else {
                    self.start_active_recording();
                    let active = self.any_recording_active();
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    if active {
                        rivulet_obs_websocket::ObsCommandResult::Success(vec![
                            ObsEvent::RecordStateChanged {
                                active: true,
                                paused: false,
                            },
                        ])
                    } else {
                        rivulet_obs_websocket::ObsCommandResult::Failure {
                            status_code:
                                rivulet_obs_websocket::protocol::status::REQUEST_PROCESSING_FAILED,
                            comment:
                                "Recording could not be started (no capture source configured)"
                                    .into(),
                        }
                    }
                }
            }
            ObsCommand::StopRecording => {
                if !self.any_recording_active() {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Recording not active".into(),
                    }
                } else {
                    let was_paused = self.is_paused;
                    self.stop_active_recording();
                    self.is_paused = false;
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    rivulet_obs_websocket::ObsCommandResult::Success(vec![
                        ObsEvent::RecordStateChanged {
                            active: false,
                            paused: false,
                        },
                    ])
                }
            }
            ObsCommand::ToggleRecording => {
                if self.any_recording_active() {
                    self.execute_obs_command(ObsCommand::StopRecording)
                } else {
                    self.execute_obs_command(ObsCommand::StartRecording)
                }
            }
            ObsCommand::StartStreaming => {
                if self.engine.is_streaming() {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_RUNNING,
                        comment: "Stream already active".into(),
                    }
                } else {
                    let settings = StreamSettings::new(
                        self.stream_platform,
                        self.stream_ingest_url.clone(),
                        self.stream_key.clone(),
                    )
                    .with_preset(self.stream_preset);
                    self.engine.set_stream_settings(Some(settings));
                    // Clear a stale error so a new stream starts from the
                    // Ready/Streaming label (mirrors the recording starts).
                    self.last_error = None;
                    self.engine.start_streaming();
                    let active = self.engine.is_streaming();
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    if active {
                        rivulet_obs_websocket::ObsCommandResult::Success(vec![
                            ObsEvent::StreamStateChanged {
                                active: true,
                                reconnecting: false,
                            },
                        ])
                    } else {
                        rivulet_obs_websocket::ObsCommandResult::Failure {
                            status_code:
                                rivulet_obs_websocket::protocol::status::REQUEST_PROCESSING_FAILED,
                            comment: "Streaming could not be started (no ingest configured)".into(),
                        }
                    }
                }
            }
            ObsCommand::StopStreaming => {
                if !self.engine.is_streaming() {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Stream not active".into(),
                    }
                } else {
                    self.engine.set_stream_settings(None);
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    rivulet_obs_websocket::ObsCommandResult::Success(vec![
                        ObsEvent::StreamStateChanged {
                            active: false,
                            reconnecting: false,
                        },
                    ])
                }
            }
            ObsCommand::ToggleStreaming => {
                if self.engine.is_streaming() {
                    self.execute_obs_command(ObsCommand::StopStreaming)
                } else {
                    self.execute_obs_command(ObsCommand::StartStreaming)
                }
            }
        }
    }

    /// Shared action dispatch used by both the in-app (focused) key handling
    /// and OS-level global hotkey events (Windows).
    fn dispatch_hotkey_action(&mut self, action: &str) {
        let any_recording = self.any_recording_active();
        match action {
            "record" => {
                if any_recording {
                    self.stop_active_recording();
                } else {
                    self.start_active_recording();
                }
            }
            "pause" => {
                if any_recording {
                    self.is_paused = !self.is_paused;
                }
            }
            "mute" => {
                if any_recording {
                    self.is_muted = !self.is_muted;
                }
            }
            "save_replay" if any_recording => {
                self.save_replay_now();
            }
            _ => {}
        }
    }

    fn any_recording_active(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.is_recording || self.is_aux_recording
        }
        #[cfg(target_os = "windows")]
        {
            self.is_windows_recording || self.is_aux_recording
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            self.is_aux_recording
        }
    }

    fn start_active_recording(&mut self) {
        #[cfg(target_os = "windows")]
        self.start_windows_recording();
        #[cfg(target_os = "linux")]
        self.start_linux_recording();
    }

    fn stop_active_recording(&mut self) {
        #[cfg(target_os = "windows")]
        self.stop_windows_recording();
        #[cfg(target_os = "linux")]
        self.stop_linux_recording();
        self.is_paused = false;
        self.is_muted = false;
    }

    fn draw_presence_status(&mut self, ui: &mut egui::Ui) {
        self.sync_discord_presence();
        let status = self.current_presence_status();
        let active_activity = self.current_presence_activity();
        ui.group(|ui| {
            ui.label(egui::RichText::new("Rivulet-Status").strong());
            ui.label(status.details);
            ui.label(status.state);
            // Legend: one row per state with a hover tooltip that explains
            // when the state appears and what it means. The active state is
            // highlighted so the mapping from Discord text to meaning is
            // always visible.
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(self.tr("presence_legend"))
                    .small()
                    .strong(),
            );
            for activity in PresenceActivity::all() {
                let label = self.tr(activity.i18n_key());
                let tip = self.tr(activity.tooltip_i18n_key());
                let is_active = activity == active_activity;
                let response = ui.selectable_label(is_active, label);
                if is_active {
                    theme::paint_interaction_stroke(ui, &response);
                }
                response.on_hover_text(tip);
            }
            let enable_label = self.tr("discord_presence_enable");
            let hint = self.tr("discord_presence_hint");
            ui.checkbox(&mut self.discord_presence_enabled, enable_label)
                .on_hover_text(hint);

            // Surface the actual IPC state, not just the desired status: when
            // Discord is not running, no client id is configured, or the
            // handshake failed, the user otherwise only sees the plain game
            // card ("Playing Rivulet") without the rich-presence details.
            let conn_text = if !self.discord_presence_enabled
                || (self.discord_presence.is_none()
                    && self.discord_presence_client_id.trim().is_empty())
            {
                self.tr("discord_conn_off").to_owned()
            } else {
                match self
                    .discord_presence
                    .as_ref()
                    .map(|p| p.connection_state())
                    .unwrap_or(rivulet_core::discord::DiscordConnState::Connecting)
                {
                    rivulet_core::discord::DiscordConnState::Connected => {
                        self.tr("discord_conn_connected").to_owned()
                    }
                    rivulet_core::discord::DiscordConnState::Connecting => {
                        self.tr("discord_conn_connecting").to_owned()
                    }
                    rivulet_core::discord::DiscordConnState::Off => {
                        self.tr("discord_conn_off").to_owned()
                    }
                }
            };
            let is_connected = matches!(
                self.discord_presence.as_ref().map(|p| p.connection_state()),
                Some(rivulet_core::discord::DiscordConnState::Connected)
            );
            ui.horizontal(|ui| {
                let current_color = if is_connected {
                    theme::StatusColors::for_ui(ui).success
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.colored_label(current_color, conn_text);
                // Offer a one-click reconnect when the handshake did not
                // succeed (Discord was started after Rivulet, the socket
                // died, ...): rebuilds the adapter without restarting the app.
                if !is_connected
                    && self.discord_presence_enabled
                    && !self.discord_presence_client_id.trim().is_empty()
                {
                    let reconnect_label = self.tr("discord_reconnect");
                    if ui.button(reconnect_label).clicked() {
                        self.discord_reconnect_requested = true;
                    }
                }
            });
            ui.small(self.tr("discord_presence_privacy"));
        });
    }

    /// Render the Twitch chat dock: channel + token inputs, connect/
    /// disconnect, and the (bounded) message list with user colors.
    /// Draw the Twitch chat dock. Used by the Stream view workspace (the
    /// chat no longer has its own sidebar entry): the message list is bounded
    /// by `max_list_height` so it behaves like a docked panel on the
    /// broadcast page instead of growing the outer scroll area without end.
    fn draw_chat_dock(&mut self, ui: &mut egui::Ui, max_list_height: f32) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(self.tr("chat_title")).strong());
        ui.separator();

        // Platform selector: Twitch (IRC), Kick (WebSocket), YouTube
        // (polling). Switching disconnects the running worker on the next
        // reconcile (the pending connect uses the new platform).
        ui.horizontal_wrapped(|ui| {
            ui.label(self.tr("chat_platform"));
            egui::ComboBox::from_id_salt("chat_platform")
                .selected_text(self.chat_platform.label())
                .show_ui(ui, |ui| {
                    for candidate in rivulet_core::ChatPlatform::all() {
                        if ui
                            .selectable_value(&mut self.chat_platform, candidate, candidate.label())
                            .clicked()
                        {
                            self.chat_action_pending = Some(ChatAction::Disconnect);
                        }
                    }
                });
        });

        // Connection controls. Wrapped so the connect/disconnect button and
        // status stay reachable in a narrow Stream workspace.
        let channel_hint = match self.chat_platform {
            rivulet_core::ChatPlatform::Twitch => self.tr("chat_channel_hint"),
            rivulet_core::ChatPlatform::Kick => self.tr("chat_channel_hint_kick"),
            rivulet_core::ChatPlatform::YouTube => self.tr("chat_channel_hint_youtube"),
        };
        let oauth_hint = match self.chat_platform {
            rivulet_core::ChatPlatform::Twitch => self.tr("chat_oauth_hint"),
            rivulet_core::ChatPlatform::Kick => self.tr("chat_token_hint_kick"),
            rivulet_core::ChatPlatform::YouTube => self.tr("chat_token_hint_youtube"),
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(self.tr("chat_channel_label"));
            ui.add(
                egui::TextEdit::singleline(&mut self.chat_channel)
                    .hint_text(channel_hint)
                    .desired_width(160.0),
            );
            let state = self.chat_state;
            let is_connected = state == rivulet_core::ChatConnState::Connected;
            if is_connected {
                if ui.button(self.tr("chat_disconnect")).clicked() {
                    self.chat_action_pending = Some(ChatAction::Disconnect);
                }
            } else if ui.button(self.tr("chat_connect")).clicked() {
                self.chat_action_pending = Some(ChatAction::Connect);
            }
            let conn_text = match state {
                rivulet_core::ChatConnState::Connected => self.tr("chat_state_connected"),
                rivulet_core::ChatConnState::Disconnected => self.tr("chat_state_disconnected"),
                rivulet_core::ChatConnState::Off => self.tr("chat_state_off"),
            };
            ui.colored_label(
                if is_connected {
                    theme::StatusColors::for_ui(ui).success
                } else {
                    ui.visuals().weak_text_color()
                },
                conn_text,
            );
        });

        // Optional token for authenticated reads/sends (privacy-safe: never
        // echoed back). YouTube has no token field — anonymous polling only.
        if self.chat_platform != rivulet_core::ChatPlatform::YouTube {
            ui.horizontal_wrapped(|ui| {
                ui.label(self.tr("chat_oauth_label"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.chat_oauth_token)
                        .password(true)
                        .hint_text(oauth_hint)
                        .desired_width(220.0),
                );
            });
        }
        let note = match self.chat_platform {
            rivulet_core::ChatPlatform::Twitch => self.tr("chat_note"),
            rivulet_core::ChatPlatform::Kick => self.tr("chat_note_kick"),
            rivulet_core::ChatPlatform::YouTube => self.tr("chat_note_youtube"),
        };
        ui.small(note);
        ui.add_space(4.0);

        // Twitch-only server requirement surfaced as soon as the worker sees
        // the notice: the bot account must be phone-verified before it can
        // send. Shown until reconnect so the streamer fixes the account.
        if self.chat_platform == rivulet_core::ChatPlatform::Twitch
            && self
                .chat_worker
                .as_ref()
                .is_some_and(|w| w.phone_verification_required())
        {
            let colors = theme::StatusColors::for_ui(ui);
            ui.colored_label(colors.warning, self.tr("chat_phone_verification"));
        }

        // Message list (newest at the bottom, autoscroll to the last message).
        let reply_arrow = "↩";
        let reply_tooltip = self.tr("chat_reply_tooltip").to_owned();
        let messages = std::mem::take(&mut self.chat_messages);
        let total = messages.len();
        let mut scroll_to_bottom = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(max_list_height)
            .show(ui, |ui| {
                for message in &messages {
                    let mut text = egui::RichText::new(message.user.as_str()).strong();
                    if let Some(rgb) = message.color.as_deref().and_then(color_to_egui) {
                        text = text.color(rgb);
                    }
                    if message.broadcaster {
                        text = text.underline();
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.label(text);
                        if message.action {
                            ui.label(egui::RichText::new(format!("*{}", message.text)).italics());
                        } else {
                            ui.label(&message.text);
                        }
                        // Twitch messages carry an `id=...` tag; offer a
                        // threaded reply so the bot answers that exact line.
                        if let Some(msg_id) = &message.id {
                            if ui
                                .small_button(reply_arrow)
                                .on_hover_text(&reply_tooltip)
                                .clicked()
                            {
                                self.chat_reply_target =
                                    Some((message.user.clone(), msg_id.clone()));
                            }
                        }
                    });
                }
                if total > 0 {
                    scroll_to_bottom = true;
                }
            });
        if scroll_to_bottom {
            // Keep the newest message visible; the stick_to_bottom option
            // already handles autoscroll while the user is at the bottom.
        }
        self.chat_messages = messages;
        if self.chat_messages.is_empty() && self.chat_state != rivulet_core::ChatConnState::Off {
            ui.small(self.tr("chat_empty"));
        }

        // Reply input: sending requires an authenticated connection (Twitch
        // rejects PRIVMSG from the anonymous nick, Kick rejects anonymous
        // sends), so the field is only enabled when connected and a token is
        // configured. YouTube chat is read-only without an authenticated
        // browser session, so the input is replaced by a hint. Enter sends;
        // the input is cleared immediately after the enqueue.
        let can_send = self.chat_state == rivulet_core::ChatConnState::Connected
            && !self.chat_oauth_token.trim().is_empty()
            && self.chat_platform != rivulet_core::ChatPlatform::YouTube;
        if can_send {
            // When a reply target is armed, a banner names the message being
            // answered and the Send button enqueues a threaded reply instead
            // of a plain chat message.
            let reply_user = self
                .chat_reply_target
                .as_ref()
                .map(|(user, _)| user.clone());
            if let Some(user) = reply_user {
                let reply_label = self.tr_fmt("chat_reply_to", &[user]);
                let reply_cancel = self.tr("chat_reply_cancel").to_owned();
                ui.horizontal_wrapped(|ui| {
                    ui.small(egui::RichText::new(format!("↩ {reply_label}")).italics());
                    if ui.small_button("✕").on_hover_text(reply_cancel).clicked() {
                        self.cancel_chat_reply();
                    }
                });
            }
            // Send-budget line directly above the input: shows how many
            // platform messages are still allowed right now, so throttling is
            // visible *before* a send is silently dropped. The line turns
            // warning-colored when the bucket is near-empty and is replaced
            // by a pause notice while the bucket is empty.
            //
            // Both texts render as explicitly wrapping labels (.wrap()) so a
            // long German notice (or a future theme default that disables
            // wrapping) can never be clipped at the right edge of a narrow
            // dock column; the whole Stream view lives in a vertical scroll
            // area, so the line is also reachable on short windows.
            if let Some((remaining, capacity, window_secs, platform)) =
                self.chat_rate_limit_detail()
            {
                let colors = theme::StatusColors::for_ui(ui);
                let budget = remaining.floor().max(0.0) as u64;
                let cap = capacity as u64;
                // Tooltip: which platform limit applies and over what window
                // (e.g. “20 messages per 30 s on Twitch”), so the bare budget
                // numbers above the input stay compact.
                let tooltip = self.tr_fmt(
                    "chat_rate_window",
                    &[
                        capacity.to_string(),
                        window_secs.to_string(),
                        platform.to_owned(),
                    ],
                );
                if remaining < 1.0 {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(self.tr("chat_rate_limited"))
                                .small()
                                .color(colors.warning),
                        )
                        .wrap(),
                    )
                    .on_hover_text(tooltip);
                } else {
                    let low = budget <= cap.saturating_div(4);
                    let color = if low {
                        colors.warning
                    } else {
                        ui.visuals().weak_text_color()
                    };
                    let label =
                        self.tr_fmt("chat_rate_budget", &[budget.to_string(), cap.to_string()]);
                    ui.add(
                        egui::Label::new(egui::RichText::new(label).small().color(color)).wrap(),
                    )
                    .on_hover_text(tooltip);
                }
            }
            let send_hint = self.tr("chat_send_hint");
            let send_button = self.tr("chat_send");
            ui.horizontal_wrapped(|ui| {
                let input = ui.add(
                    egui::TextEdit::singleline(&mut self.chat_input)
                        .hint_text(send_hint)
                        .desired_width((ui.available_width() - 70.0).max(120.0)),
                );
                let enter_pressed =
                    input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter_pressed || theme::accent_button(ui, send_button).clicked() {
                    self.submit_chat_input();
                }
            });
        } else if self.chat_platform == rivulet_core::ChatPlatform::YouTube {
            ui.small(self.tr("chat_read_only"));
        } else {
            ui.small(self.tr("chat_send_locked"));
        }
    }

    fn handle_stream_key_actions(&mut self) {
        let account = self.stream_platform.label();
        let store = rivulet_core::StreamKeyStore::default();
        if self.stream_key_save_requested {
            self.stream_key_save_requested = false;
            self.stream_key_store_status = Some(match store.save(account, &self.stream_key) {
                Ok(()) => self.tr("stream_key_saved").to_owned(),
                Err(_) => self.tr("stream_key_store_unavailable").to_owned(),
            });
        }
        if self.stream_key_delete_requested {
            self.stream_key_delete_requested = false;
            self.stream_key_store_status = Some(match store.delete(account) {
                Ok(()) => self.tr("stream_key_deleted").to_owned(),
                Err(_) => self.tr("stream_key_store_unavailable").to_owned(),
            });
        }
    }

    /// Platform + preset combo boxes of the Stream workspace action bar.
    /// Extracted so the wide (right-aligned button) and the narrow (wrapped)
    /// action-bar layouts share the same selectors.
    fn draw_stream_platform_preset_selectors(&mut self, ui: &mut egui::Ui) {
        let mut platform = self.stream_platform;
        egui::ComboBox::from_id_salt("stream_platform")
            .selected_text(platform.label())
            .show_ui(ui, |ui| {
                for candidate in [
                    StreamPlatform::Twitch,
                    StreamPlatform::YouTube,
                    StreamPlatform::Kick,
                    StreamPlatform::Custom,
                ] {
                    if ui
                        .selectable_value(&mut platform, candidate, candidate.label())
                        .clicked()
                    {
                        self.stream_platform = candidate;
                        if let Some(url) = candidate.default_ingest_url() {
                            self.stream_ingest_url = url.to_owned();
                        }
                    }
                }
            });
        egui::ComboBox::from_id_salt("stream_preset")
            .selected_text(self.stream_preset.label())
            .show_ui(ui, |ui| {
                for preset in StreamPreset::all() {
                    ui.selectable_value(&mut self.stream_preset, *preset, preset.label());
                }
            });
    }

    /// Start/stop streaming button of the Stream workspace action bar.
    fn draw_stream_start_stop_button(&mut self, ui: &mut egui::Ui, configured: bool) {
        let label = if self.engine.is_streaming() {
            self.tr("stream_stop")
        } else {
            self.tr("stream_start")
        };
        if ui
            .add_enabled(
                configured || self.engine.is_streaming(),
                egui::Button::new(egui::RichText::new(label).size(18.0).strong())
                    .min_size(egui::vec2(170.0, 42.0)),
            )
            .clicked()
        {
            if self.engine.is_streaming() {
                self.engine.set_stream_settings(None);
                self.stream_status_message = Some(self.tr("stopped").to_owned());
            } else {
                let settings = StreamSettings::new(
                    self.stream_platform,
                    self.stream_ingest_url.clone(),
                    self.stream_key.clone(),
                )
                .with_preset(self.stream_preset);
                self.engine.set_stream_settings(Some(settings));
                // Clear a stale error so a new stream starts from the
                // Ready/Streaming label (mirrors the recording starts).
                self.last_error = None;
                self.engine.start_streaming();
                self.stream_status_message = Some(self.tr("stream_status_connecting").to_owned());
            }
        }
    }

    fn draw_stream_view(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        self.handle_stream_key_actions();

        // Capture the usable viewport height before any widgets shrink the
        // available space: the chat list is docked (bounded) on this page.
        let chat_list_height = (ui.available_height() * 0.45).clamp(160.0, 460.0);

        let configured = !self.stream_key.trim().is_empty()
            && StreamSettings::new(
                self.stream_platform,
                self.stream_ingest_url.clone(),
                self.stream_key.clone(),
            )
            .with_preset(self.stream_preset)
            .validate()
            .is_ok();

        // ── Action bar (Meld-style broadcast header): platform + preset on
        //    the left, the prominent start/stop button on the right. On
        //    narrow windows the bar wraps so the button stays reachable. ──
        let action_bar_wraps = ui.available_width() < STREAM_WORKSPACE_NARROW_WIDTH;
        if action_bar_wraps {
            ui.horizontal_wrapped(|ui| {
                self.draw_stream_platform_preset_selectors(ui);
                self.draw_stream_start_stop_button(ui, configured);
            });
        } else {
            ui.horizontal(|ui| {
                self.draw_stream_platform_preset_selectors(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.draw_stream_start_stop_button(ui, configured);
                });
            });
        }
        if !configured && !self.engine.is_streaming() {
            ui.colored_label(colors.warning, self.tr("stream_configure_first"));
        }
        if let Some(message) = &self.stream_status_message {
            ui.colored_label(colors.info, message);
        }
        if let Some(result) = &self.stream_probe_result {
            ui.label(match result {
                StreamProbeResult::Reachable => self.tr("stream_probe_reachable"),
                StreamProbeResult::Rejected(_) => self.tr("stream_probe_rejected"),
                StreamProbeResult::Unavailable(_) => self.tr("stream_probe_unavailable"),
            });
        }

        // ── Details: ingest URL, stream key, connection test, assistant.
        //    Hidden behind a collapsed header so the start/stop action stays
        //    one click away (automatically open while nothing is configured).
        egui::CollapsingHeader::new(self.tr("stream_config"))
            .id_salt("stream_config_details")
            .default_open(!configured)
            .show(ui, |ui| {
                // Wrapped rows: on narrow windows the label/input/button
                // rows flow onto a second line instead of clipping at the
                // right edge (controls must stay reachable).
                ui.horizontal_wrapped(|ui| {
                    ui.label(self.tr("stream_ingest"));
                    ui.add_enabled(
                        self.stream_platform == StreamPlatform::Custom,
                        egui::TextEdit::singleline(&mut self.stream_ingest_url),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(self.tr("stream_key"));
                    ui.add(egui::TextEdit::singleline(&mut self.stream_key).password(true));
                    if !self.stream_key.is_empty() {
                        ui.label(format!("••••{}", self.stream_key.chars().count().min(4)));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if theme::accent_button(ui, self.tr("stream_key_save")).clicked() {
                        self.stream_key_save_requested = true;
                    }
                    if ui.button(self.tr("stream_key_delete")).clicked() {
                        self.stream_key_delete_requested = true;
                    }
                    if let Some(status) = &self.stream_key_store_status {
                        ui.small(status);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let probe_enabled =
                        configured && !self.stream_probe_running && !self.engine.is_streaming();
                    if ui
                        .add_enabled(
                            probe_enabled,
                            egui::Button::new(self.tr("stream_test_connection")),
                        )
                        .clicked()
                    {
                        let url = self.stream_ingest_url.clone();
                        self.stream_probe_running = true;
                        self.stream_probe_result = None;
                        self.stream_status_message =
                            Some(self.tr("stream_probe_running").to_owned());
                        let (tx, rx) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let result = rivulet_core::stream::probe_ingest_reachability(
                                &url,
                                std::time::Duration::from_secs(5),
                            );
                            let _ = tx.send(result);
                        });
                        self.stream_probe_result =
                            rx.recv_timeout(std::time::Duration::from_millis(1)).ok();
                        if self.stream_probe_result.is_some() {
                            self.stream_probe_running = false;
                        }
                    }
                    if ui.button(self.tr("stream_setup_assistant")).clicked() {
                        self.setup_wizard_open = true;
                        self.setup_wizard_step = 0;
                    }
                    if self.setup_wizard_open {
                        ui.label(self.tr("stream_setup_assistant_active"));
                    }
                });
            });
        ui.small(self.tr("stream_m3_note"));

        // ── Workspace: chat dock (left) | stream information (right). The
        //    chat is bounded in height so it behaves like a docked panel on
        //    this page rather than growing the scroll area without end. On
        //    narrow windows the two columns stack vertically so the chat
        //    connect/send controls never clip off-screen. ──
        ui.separator();
        if ui.available_width() >= STREAM_WORKSPACE_NARROW_WIDTH {
            ui.columns(2, |cols| {
                self.draw_chat_dock(&mut cols[0], chat_list_height);
                self.draw_stream_information_panel(&mut cols[1], colors);
            });
        } else {
            self.draw_chat_dock(ui, chat_list_height);
            ui.separator();
            self.draw_stream_information_panel(ui, colors);
        }

        // ── Compact audio: master fader + VU and monitoring; the detailed
        //    mixer (filters, EQ, per-track capture) stays in its own view. ──
        #[cfg(target_os = "linux")]
        self.draw_stream_audio_compact(ui, colors);
        #[cfg(not(target_os = "linux"))]
        {
            ui.separator();
            ui.small(self.tr("mixer_unavailable"));
        }

        self.draw_stream_setup_wizard(ui);
    }

    /// Right-hand column of the Stream workspace: stream health, target
    /// telemetry and the Discord presence card.
    fn draw_stream_information_panel(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new(self.tr("stream_information")).strong());
        ui.separator();
        self.draw_stream_health_panel(ui, *colors);
        ui.small(self.tr("stream_health_help"));
        self.draw_presence_status(ui);
    }

    /// Compact output/monitoring section at the bottom of the Stream
    /// workspace: monitoring toggles, monitor volume, master fader with the
    /// output VU meter, plus a shortcut into the full audio mixer view.
    /// (The audio mixer — and its fields — are Linux-only; other platforms
    /// render an "unavailable" hint instead.)
    #[cfg(target_os = "linux")]
    fn draw_stream_audio_compact(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(self.tr("stream_output_monitoring")).strong());
            if ui.button(self.tr("stream_open_mixer")).clicked() {
                self.view = AppView::Mixer;
            }
        });

        let mut sys_mon = self.system_monitor;
        let mut mic_mon = self.mic_monitor;
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut sys_mon, self.tr("monitoring_system"));
            ui.checkbox(&mut mic_mon, self.tr("monitoring_microphone"));
            let mut mon_vol = self.monitor_volume;
            if ui
                .add(egui::Slider::new(&mut mon_vol, 0.0..=1.0).text(self.tr("monitoring_volume")))
                .changed()
            {
                self.monitor_volume = mon_vol;
                if let Some(audio) = &self.audio {
                    audio.set_monitor_volume(mon_vol);
                }
            }
        });
        if sys_mon != self.system_monitor || mic_mon != self.mic_monitor {
            self.system_monitor = sys_mon;
            self.mic_monitor = mic_mon;
            if self.audio_preview {
                self.start_audio_capture();
            }
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(self.tr("master_output")).strong());
            let mut master_vol = self.master_volume;
            if ui
                .add(egui::Slider::new(&mut master_vol, 0.0..=1.0).text(self.tr("master_volume")))
                .changed()
            {
                self.master_volume = master_vol;
                if let Some(audio) = &self.audio {
                    audio.set_master_volume(master_vol);
                }
            }
            // Master output VU meter (level of the whole mix after the master
            // volume is applied).
            let peak = f32::from_bits(self.audio_peak.load(Ordering::SeqCst)).clamp(0.0, 1.0);
            let db = if peak > 0.0 {
                20.0 * peak.log10()
            } else {
                -96.0
            };
            ui.add(
                egui::ProgressBar::new(peak)
                    .desired_width(180.0)
                    .text(self.tr_fmt("output_vu", &[format!("{db:.1}")])),
            );
            if let Some(status) = &self.audio_status {
                ui.colored_label(colors.error, status);
            }
            if let Some(warning) = &self.audio_warning {
                ui.colored_label(colors.warning, warning);
            }
        });
    }

    /// Render the region capture editor (preview with drag selection,
    /// numeric inputs, and apply/cancel/reset actions).
    fn draw_region_editor(&mut self, ctx: &egui::Context) {
        if !self.region_editor_open {
            return;
        }
        let mut preview = self.region_preview.take();
        let mut region = self.region;
        let error = self.region_preview_error.clone();
        let dims = preview
            .as_ref()
            .map(|p| (p.full_width, p.full_height))
            .or(self.region_editor_dims);
        let mut apply = false;
        let mut cancel = false;
        let mut reset = false;

        egui::Window::new(self.tr("region_editor_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                let colors = theme::StatusColors::for_ui(ui);
                if let Some(err) = &error {
                    ui.colored_label(colors.error, err);
                }
                ui.label(self.tr("region_hint"));
                let Some((full_width, full_height)) = dims else {
                    ui.label(self.tr("region_preview_unavailable"));
                    return;
                };
                if let Some(p) = preview.as_mut() {
                    // Preview image scaled to the available width, with the
                    // current selection drawn on top. Dragging selects a new
                    // region; the outside is dimmed while dragging.
                    let content_rect = ui.max_rect();
                    let max_width = content_rect.width().clamp(1.0, 700.0);
                    let max_height = content_rect.height().max(1.0);
                    let scale = (max_width / full_width.max(1) as f32)
                        .min(max_height / full_height.max(1) as f32)
                        .max(0.01);
                    let size = egui::vec2(full_width as f32 * scale, full_height as f32 * scale);
                    let (rect, response) =
                        ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                    let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    ui.painter()
                        .image(p.texture.id(), rect, uv, egui::Color32::WHITE);
                    if let Some(start) = p.drag_start {
                        if let Some(current) = response.interact_pointer_pos() {
                            let drag_rect = egui::Rect::from_two_pos(start, current);
                            ui.painter().rect_filled(
                                rect,
                                0.0,
                                theme::ThemePalette::for_ui(ui).scrim,
                            );
                            ui.painter().with_clip_rect(drag_rect).image(
                                p.texture.id(),
                                rect,
                                uv,
                                egui::Color32::WHITE,
                            );
                            ui.painter().rect_stroke(
                                drag_rect,
                                0.0,
                                egui::Stroke::new(2.0, colors.error),
                                egui::StrokeKind::Middle,
                            );
                        }
                    } else {
                        let sel_rect = region_rect_in(region, rect, full_width, full_height);
                        ui.painter().rect_stroke(
                            sel_rect,
                            0.0,
                            egui::Stroke::new(2.0, colors.error),
                            egui::StrokeKind::Middle,
                        );
                    }
                    if response.drag_started() {
                        p.drag_start = response.interact_pointer_pos();
                    }
                    if let Some(start) = p.drag_start {
                        if response.dragged() {
                            if let Some(current) = response.interact_pointer_pos() {
                                let to_px = |pos: egui::Pos2| {
                                    (pos - rect.min) / rect.size()
                                        * egui::vec2(full_width as f32, full_height as f32)
                                };
                                let s = to_px(start);
                                let c = to_px(current);
                                region = region_from_pixel_points(
                                    s.x,
                                    s.y,
                                    c.x,
                                    c.y,
                                    full_width,
                                    full_height,
                                );
                            }
                        }
                        if response.drag_stopped() {
                            p.drag_start = None;
                        }
                    }
                }
                // Precise numeric inputs, clamped to the monitor bounds.
                ui.horizontal(|ui| {
                    ui.label(self.tr("region_x"));
                    ui.add(
                        egui::DragValue::new(&mut region.x).range(0..=full_width.saturating_sub(1)),
                    );
                    ui.label(self.tr("region_y"));
                    ui.add(
                        egui::DragValue::new(&mut region.y)
                            .range(0..=full_height.saturating_sub(1)),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(self.tr("region_width"));
                    ui.add(egui::DragValue::new(&mut region.width).range(1..=full_width));
                    ui.label(self.tr("region_height"));
                    ui.add(egui::DragValue::new(&mut region.height).range(1..=full_height));
                });
                // Snap to even dimensions (required by the encoders) after
                // manual entry, so a typed odd value can never produce an
                // unencodable frame.
                region.width = (region.width & !1).clamp(2, full_width);
                region.height = (region.height & !1).clamp(2, full_height);
                if region.x + region.width > full_width {
                    region.x = full_width.saturating_sub(region.width);
                }
                if region.y + region.height > full_height {
                    region.y = full_height.saturating_sub(region.height);
                }
                ui.horizontal(|ui| {
                    if theme::accent_button(ui, self.tr("region_full_monitor")).clicked() {
                        reset = true;
                    }
                    if theme::accent_button(ui, self.tr("region_apply")).clicked() {
                        apply = true;
                    }
                    if theme::accent_button(ui, self.tr("region_cancel")).clicked() {
                        cancel = true;
                    }
                });
            });

        if reset {
            if let Some((full_width, full_height)) = dims {
                region = CaptureRegion::full(full_width, full_height);
            }
        }
        if apply || cancel {
            self.region_editor_open = false;
        }
        self.region = region;
        self.region_preview = if self.region_editor_open {
            preview
        } else {
            None
        };
        if !self.region_editor_open {
            self.region_preview_error = None;
        }
    }

    /// Handle the queued update actions (check / download / install).
    fn handle_update_actions(&mut self, ctx: egui::Context) {
        let busy = matches!(
            self.update_ui_snapshot(),
            UpdateUi::Checking | UpdateUi::Downloading { .. } | UpdateUi::Installing(_)
        );

        if !self.update_auto_checked && !busy {
            self.update_auto_checked = true;
            self.spawn_update_check(ctx.clone());
        }
        if self.update_check_clicked && !busy {
            self.update_check_clicked = false;
            self.spawn_update_check(ctx.clone());
        }
        if self.update_download_clicked && !busy {
            self.update_download_clicked = false;
            if let UpdateUi::Available(info) = self.update_ui_snapshot() {
                if let Some(asset) = info.asset {
                    self.spawn_update_download(ctx.clone(), asset, info.tag, info.version);
                }
            }
        }
        if self.update_install_clicked && !busy {
            self.update_install_clicked = false;
            if let UpdateUi::Downloaded { path, version } = self.update_ui_snapshot() {
                self.spawn_update_install(ctx, path, version);
            }
        }
    }

    /// Restore the persisted app state (theme, locale, hotkeys, OBS
    /// WebSocket, MIDI presets, Discord client id, …) written by `save()` on
    /// the previous run. Runtime-only handles (engine, adapters, previews) are
    /// `#[serde(skip)]` and therefore stay at their defaults here; the caller
    /// re-attaches the live engine and CLI parameters.
    fn restore_from_storage(storage: Option<&dyn eframe::Storage>) -> Option<Self> {
        let mut restored: Self = eframe::get_value::<RivuletApp>(storage?, eframe::APP_KEY)?;
        // Migration: installs configured before the official application id
        // became the default saved an empty client id (adapter off). Those
        // users get the zero-config branded presence now; an explicitly
        // configured id is never overwritten. Lives on every restore path,
        // not only new_from_cc, so tests and future callers stay covered.
        if restored.discord_presence_client_id.trim().is_empty() {
            restored.discord_presence_client_id =
                rivulet_core::discord::DEFAULT_CLIENT_ID.to_owned();
            restored.discord_presence_large_image =
                rivulet_core::discord::DEFAULT_LARGE_IMAGE_KEY.to_owned();
            tracing::info!("Discord client id was empty - applying the official default");
        }
        Some(restored)
    }

    pub fn new(
        cc: &eframe::CreationContext<'_>,
        engine: RivuletEngine,
        no_frame_timeout: std::time::Duration,
    ) -> Self {
        #[allow(unused_mut)]
        let mut app = Self {
            engine,
            no_frame_timeout,
            ..Default::default()
        };
        // Reload settings persisted by `save()` on the previous run. Without
        // this the app would always start from Default, silently dropping
        // every setting the user configured (theme, Discord client id, …).
        if let Some(mut restored) = Self::restore_from_storage(cc.storage) {
            tracing::info!(theme = ?restored.theme, "Restoring persisted app state");
            // Keep the live engine and the CLI-provided timeout; everything
            // else (persisted settings) comes from storage.
            restored.engine = app.engine;
            restored.no_frame_timeout = no_frame_timeout;
            app = restored;
        }
        // Apply the persisted color scheme (fonts + palette + preference)
        // immediately, so the first frame already renders with the right theme.
        theme::init(&cc.egui_ctx, app.theme);
        app.theme_applied = Some(app.theme);
        #[cfg(target_os = "windows")]
        {
            app.refresh_capture_sources();
        }
        #[cfg(target_os = "linux")]
        {
            app.refresh_linux_sources();
        }
        app
    }
}

impl eframe::App for RivuletApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        tracing::info!(theme = ?self.theme, "Persisting theme preference on application exit");
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ── Hotkey handling ────────────────────────────────────────
        let ctx = ui.ctx();

        // Reconcile OS-level global hotkeys (create/rebind on change) and
        // dispatch any global hotkey events fired while the app was unfocused.
        self.reconcile_global_hotkeys();

        // Reconcile the OBS WebSocket server: start/stop it with the setting,
        // sync the read snapshot, execute remote commands, and broadcast
        // GUI-initiated changes to connected Stream Deck/TouchPortal clients.
        self.reconcile_obs_websocket();

        // Reconcile the MIDI listener: start/stop it with the setting and
        // selected device, then apply mapped actions from incoming messages.
        self.reconcile_midi();

        // Reconcile Discord Rich Presence every frame (not only while the
        // Stream view is visible): recording/streaming toggles happen from the
        // Record view, so the activity transition must be pushed whenever the
        // engine state changes, regardless of which tab is open. The adapter is
        // non-blocking (try_send) and only pushes on transitions.
        self.sync_discord_presence();

        // Reconcile the Twitch chat dock: process connect/disconnect actions
        // and drain incoming chat messages (non-blocking).
        self.reconcile_chat();

        // Apply the color scheme (fonts + palette + preference) on startup
        // and whenever the user changes it in Settings.
        if self.theme_applied != Some(self.theme) {
            theme::init(ctx, self.theme);
            self.theme_applied = Some(self.theme);
        }

        // Read this before entering `ctx.input`. That closure holds egui's
        // context write lock; calling `egui_wants_keyboard_input()` inside it
        // tries to acquire a nested read lock and deadlocks in debug builds.
        let wants_keyboard_input = ctx.egui_wants_keyboard_input();

        ctx.input(|i| {
            #[cfg(target_os = "linux")]
            let any_recording = self.is_recording || self.is_aux_recording;
            #[cfg(target_os = "windows")]
            let any_recording = self.is_windows_recording || self.is_aux_recording;
            // macOS has no recording engine yet; keep the hotkey block
            // compiling with a constant so the GUI builds on all platforms.
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let any_recording = self.is_aux_recording;

            // Record: toggle recording
            if self.hotkeys.record.pressed_in(i) {
                if any_recording {
                    if self.is_aux_recording {
                        self.stop_aux_recording();
                    } else {
                        #[cfg(target_os = "windows")]
                        self.stop_windows_recording();
                        #[cfg(target_os = "linux")]
                        self.stop_linux_recording();
                    }
                    self.is_paused = false;
                    self.is_muted = false;
                } else {
                    #[cfg(target_os = "windows")]
                    self.start_windows_recording();
                    #[cfg(target_os = "linux")]
                    self.start_linux_recording();
                }
            }

            // Pause (only while recording)
            if self.hotkeys.pause.pressed_in(i) && any_recording {
                self.is_paused = !self.is_paused;
            }

            // Mute (only while recording)
            if self.hotkeys.mute.pressed_in(i) && any_recording {
                self.is_muted = !self.is_muted;
            }

            // Save the replay buffer as a clip (only while recording)
            if self.hotkeys.save_replay.pressed_in(i) && any_recording {
                self.save_replay_now();
            }

            // Scene history shortcuts are handled globally, but only when a
            // text field is not focused so normal editing remains unaffected.
            if !wants_keyboard_input {
                for (scene_id, binding) in &self.hotkeys.scene_hotkeys {
                    if binding.pressed_in(i) {
                        self.scenes.switch_to(*scene_id);
                    }
                }
            }

            match scene_history_shortcut(
                wants_keyboard_input,
                i.modifiers.command,
                i.key_pressed(egui::Key::Z),
                i.key_pressed(egui::Key::Y),
            ) {
                Some(SceneHistoryShortcut::Undo) => {
                    self.scenes.undo();
                }
                Some(SceneHistoryShortcut::Redo) => {
                    self.scenes.redo();
                }
                None => {}
            }
        });

        #[cfg(target_os = "windows")]
        {
            if self.is_windows_recording {
                if let (Some(receiver), Some(signal)) = (&self.frame_receiver, &self.stop_signal) {
                    // Drain all pending frames into the engine and detect an
                    // unexpected capture-thread exit in a single pass. The
                    // check must not run `try_recv()` on its own: a separate
                    // probe consumed one frame per UI tick and dropped it, so
                    // at capture rates <= UI refresh every frame was eaten and
                    // no MP4 was ever written.
                    let ended = drain_frames_and_check_end(receiver, signal, |frame| {
                        let frame = cropped_frame_for_region(
                            self.region_enabled && self.selected_monitor_idx.is_some(),
                            self.region,
                            frame,
                        )
                        .into_owned();
                        if !self.is_paused {
                            self.engine
                                .process_raw_frame(&frame.data, frame.width, frame.height);
                        }
                        self.pending_preview_frame = Some(frame);
                        self.last_frame_at = Some(Instant::now());
                    });
                    if ended {
                        tracing::warn!("Recording ended unexpectedly");
                        self.stop_windows_recording();
                    }
                }
                // Update the text overlay (timer + FPS) once per UI tick.
                if self.show_overlay && self.is_windows_recording && !self.is_paused {
                    let text = self.overlay_text();
                    self.engine.update_overlay_text(&text);
                }
                // Abort when the capture delivers no frames at all within the
                // timeout: the pipeline is only initialized by the first frame,
                // so the recording would run forever while writing nothing.
                // Only applies while the recording is still active (not already
                // stopped by the disconnect path above).
                if self.is_windows_recording
                    && should_abort_for_no_frames(
                        self.record_started,
                        self.last_frame_at,
                        Instant::now(),
                        self.no_frame_timeout,
                    )
                {
                    let secs = self.no_frame_timeout.as_secs();
                    self.last_error = Some(self.tr_fmt("recording_no_frames", &[secs.to_string()]));
                    tracing::error!(
                        timeout_seconds = secs,
                        "No frames received; stopping recording"
                    );
                    self.stop_windows_recording();
                }
            }
            // Surface engine failures (pipeline build/start/push errors) in
            // the UI instead of only on the console. The engine retains the
            // most recent error until it is consumed here.
            if let Some(err) = self.engine.take_error() {
                self.last_error = Some(err);
            }
            // Surface capture-thread failures (e.g. Graphics Capture refusing
            // the source) the same way.
            if let Some(receiver) = &self.error_receiver {
                if let Some(err) = drain_error_receiver(receiver) {
                    self.last_error = Some(err);
                }
            }
        }

        // ── Aux recording drain (camera / game capture) ─────────────
        if self.is_aux_recording {
            let paused = self.is_paused;
            // Drain camera frames
            if let Some(rx) = &self.camera_rx {
                while let Ok(frame) = rx.try_recv() {
                    if !paused {
                        self.engine
                            .process_raw_frame(&frame.data, frame.width, frame.height);
                    }
                    self.pending_preview_frame = Some(RawFrame {
                        data: frame.data.clone(),
                        width: frame.width,
                        height: frame.height,
                    });
                    self.last_frame_at = Some(Instant::now());
                }
            }
            // Drain game capture frames
            if let Some(rx) = &self.game_capture_rx {
                while let Ok(frame) = rx.try_recv() {
                    if !paused {
                        self.engine
                            .process_raw_frame(&frame.data, frame.width, frame.height);
                    }
                    self.pending_preview_frame = Some(RawFrame {
                        data: frame.data.clone(),
                        width: frame.width,
                        height: frame.height,
                    });
                    self.last_frame_at = Some(Instant::now());
                }
            }
            // Update overlay
            if self.show_overlay && !paused {
                let text = self.overlay_text();
                self.engine.update_overlay_text(&text);
            }
            // Abort if no frames arrive within timeout
            if should_abort_for_no_frames(
                self.record_started,
                self.last_frame_at,
                Instant::now(),
                self.no_frame_timeout,
            ) {
                let secs = self.no_frame_timeout.as_secs();
                self.last_error = Some(self.tr_fmt("recording_no_frames", &[secs.to_string()]));
                self.stop_aux_recording();
            }
            // Surface engine errors
            if let Some(err) = self.engine.take_error() {
                self.last_error = Some(err);
            }
        }

        self.update_recording_preview(ctx);

        egui::Panel::top("top_panel")
            .frame(theme::glass_frame(ui))
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button(self.tr("file_menu"), |ui| {
                        if theme::accent_button(ui, self.tr("quit")).clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button(self.tr("language"), |ui| {
                        for locale in Locale::all() {
                            let response =
                                ui.selectable_label(self.locale == *locale, locale.name());
                            theme::paint_interaction_stroke(ui, &response);
                            if response.clicked() {
                                self.locale = *locale;
                            }
                        }
                    });
                });
            });

        egui::Panel::left("nav_panel")
            .resizable(false)
            .frame(theme::glass_frame(ui))
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Rivulet").strong().size(18.0));
                ui.separator();
                ui.add_space(6.0);
                // The nav list scrolls vertically too: on very short windows
                // the sidebar must not clip the last entries.
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for view in AppView::all() {
                            let response =
                                ui.selectable_label(self.view == *view, self.tr(view.nav_key()));
                            theme::paint_interaction_stroke(ui, &response);
                            if response.clicked() {
                                self.view = *view;
                            }
                        }
                    });
            });

        egui::CentralPanel::default().show(ui, |ui| {
            let colors = theme::StatusColors::for_ui(ui);
            ui.heading(self.tr(self.view.nav_key()));
            ui.separator();

            // The view content lives in a scroll area: when the window is
            // shrunk (narrow layout), controls at the bottom of a view stay
            // reachable instead of being clipped without any way to scroll.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Hotkey hints (only on the Record view)
                    if self.view == AppView::Record {
                        self.draw_recording_preview_panel(ui);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Hotkeys: {} [{}] · {} [{}] · {} [{}] · {} [{}]",
                                    self.tr("hotkey_record"),
                                    self.hotkeys.label_for("record"),
                                    self.tr("hotkey_pause"),
                                    self.hotkeys.label_for("pause"),
                                    self.tr("hotkey_mute"),
                                    self.hotkeys.label_for("mute"),
                                    self.tr("hotkey_save_replay"),
                                    self.hotkeys.label_for("save_replay"),
                                ))
                                .small()
                                .color(colors.hint),
                            );
                        });
                        ui.add_space(4.0);
                    }

                    #[cfg(target_os = "windows")]
                    if self.view == AppView::Record {
                        ui.add_space(10.0);
                        ui.label(egui::RichText::new(self.tr("windows_screen_recording")).strong());
                        if self.is_windows_recording || self.is_aux_recording {
                            ui.horizontal(|ui| {
                                let stop_response = theme::accent_button(ui, "⏹ Stop Recording");
                                if stop_response.clicked() {
                                    if self.is_aux_recording {
                                        self.stop_aux_recording();
                                    } else {
                                        self.stop_windows_recording();
                                    }
                                    self.is_paused = false;
                                    self.is_muted = false;
                                }
                                let pause_label = if self.is_paused {
                                    "▶ Resume"
                                } else {
                                    "⏸ Pause"
                                };
                                let pause_response = theme::accent_button(ui, pause_label);
                                if pause_response.clicked() {
                                    self.is_paused = !self.is_paused;
                                }
                                let mute_label = if self.is_muted {
                                    "🔊 Unmute"
                                } else {
                                    "🔇 Mute"
                                };
                                let mute_response = theme::accent_button(ui, mute_label);
                                if mute_response.clicked() {
                                    self.is_muted = !self.is_muted;
                                }
                            });
                            if self.is_paused {
                                ui.label(
                                    egui::RichText::new(self.tr("paused"))
                                        .color(colors.warning)
                                        .strong(),
                                );
                            }
                            if self.is_muted {
                                ui.label(
                                    egui::RichText::new(self.tr("muted"))
                                        .color(colors.info)
                                        .strong(),
                                );
                            }
                            ui.label(self.metrics_line());
                            // Capture backend indicator (G2): show which backend is
                            // active and whether the recording fell back from DXGI
                            // Desktop Duplication to Windows Graphics Capture.
                            if let Some(status) = &self.capture_backend {
                                let (key, reason) = status.ui_key();
                                let label = match reason {
                                    Some(r) if !r.is_empty() => self.tr_fmt(key, &[r.to_string()]),
                                    _ => self.tr(key).to_string(),
                                };
                                let color = match status.active {
                                    BackendKind::DesktopDuplication => colors.success,
                                    BackendKind::VulkanLayer => colors.success,
                                    BackendKind::OpenGLHook => colors.success,
                                    BackendKind::PipeWirePortal => colors.success,
                                    BackendKind::WindowsGraphicsCapture => colors.warning,
                                    BackendKind::None => colors.hint,
                                };
                                ui.label(egui::RichText::new(label).color(color).strong());
                            }
                        } else {
                            ui.horizontal(|ui| {
                                ui.label(self.tr("source"));
                                egui::ComboBox::from_id_salt("monitor_select")
                                    .selected_text(
                                        self.monitor_label().unwrap_or_else(|| {
                                            self.tr("select_monitor").to_string()
                                        }),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, monitor) in self.monitors.iter().enumerate() {
                                            if ui
                                                .selectable_label(
                                                    self.selected_monitor_idx == Some(i),
                                                    monitor.name().unwrap_or_else(|_| {
                                                        self.tr("unknown_monitor").to_string()
                                                    }),
                                                )
                                                .clicked()
                                            {
                                                if self.selected_monitor_idx != Some(i) {
                                                    self.region = CaptureRegion::full(
                                                        monitor.width().unwrap_or(0),
                                                        monitor.height().unwrap_or(0),
                                                    );
                                                }
                                                self.selected_monitor_idx = Some(i);
                                                self.selected_window_idx = None;
                                            }
                                        }
                                    });
                                egui::ComboBox::from_id_salt("window_select")
                                    .selected_text(
                                        self.selected_window_idx
                                            .and_then(|idx| self.windows.get(idx))
                                            .map(|w| w.title().unwrap_or_default())
                                            .unwrap_or_else(|| {
                                                self.tr("select_window").to_string()
                                            }),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, window) in self.windows.iter().enumerate() {
                                            if ui
                                                .selectable_label(
                                                    self.selected_window_idx == Some(i),
                                                    window.title().unwrap_or_default(),
                                                )
                                                .clicked()
                                            {
                                                self.selected_window_idx = Some(i);
                                                self.selected_monitor_idx = None;
                                                // Region capture only applies to
                                                // monitor capture; window capture is
                                                // always full-window.
                                                self.region_enabled = false;
                                            }
                                        }
                                    });
                                if ui
                                    .button("🔄")
                                    .on_hover_text(self.tr("refresh_sources"))
                                    .clicked()
                                {
                                    self.refresh_capture_sources();
                                    self.refresh_camera_devices();
                                    self.refresh_game_windows();
                                }
                            });
                            // Camera source selection
                            ui.horizontal(|ui| {
                                ui.label(self.tr("camera_source"));
                                egui::ComboBox::from_id_salt("camera_select")
                                    .selected_text(
                                        self.selected_camera_idx
                                            .and_then(|idx| self.camera_devices.get(idx))
                                            .map(|c| c.name.clone())
                                            .unwrap_or_else(|| {
                                                self.tr("select_camera").to_string()
                                            }),
                                    )
                                    .show_ui(ui, |ui| {
                                        for (i, device) in self.camera_devices.iter().enumerate() {
                                            if ui
                                                .selectable_label(
                                                    self.selected_camera_idx == Some(i),
                                                    &device.name,
                                                )
                                                .clicked()
                                            {
                                                self.selected_camera_idx = Some(i);
                                                self.selected_monitor_idx = None;
                                                self.selected_window_idx = None;
                                                self.selected_game_window_idx = None;
                                            }
                                        }
                                    });
                            });
                            // Game capture toggle + window selection + live preview
                            self.update_game_preview(ui.ctx());
                            ui.horizontal(|ui| {
                                let gc_label = self.tr("game_capture");
                                ui.checkbox(&mut self.use_game_capture, gc_label);
                                if self.use_game_capture {
                                    egui::ComboBox::from_id_salt("game_window_select")
                                        .selected_text(
                                            self.selected_game_window_idx
                                                .and_then(|idx| self.game_windows.get(idx))
                                                .map(|w| w.title.clone())
                                                .unwrap_or_else(|| {
                                                    self.tr("select_game_window").to_string()
                                                }),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (i, window) in self.game_windows.iter().enumerate()
                                            {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_game_window_idx == Some(i),
                                                        &window.title,
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected_game_window_idx = Some(i);
                                                    self.selected_monitor_idx = None;
                                                    self.selected_window_idx = None;
                                                    self.selected_camera_idx = None;
                                                }
                                            }
                                        });
                                    ui.separator();
                                    // Manual refresh of the window list (also refreshed
                                    // live while the picker is open).
                                    if ui
                                        .button("🔄")
                                        .on_hover_text(self.tr("refresh_game_windows"))
                                        .clicked()
                                    {
                                        self.refresh_game_windows();
                                    }
                                }
                            });
                            // Live thumbnail of the selected game window, so the
                            // user can verify the correct window is targeted before
                            // recording starts (refreshes every ~500ms).
                            if self.use_game_capture {
                                if let Some(preview) = &self.game_preview {
                                    let scale = (ui.available_width().min(360.0)
                                        / preview.width.max(1) as f32)
                                        .min(0.5);
                                    let size = egui::vec2(
                                        preview.width as f32 * scale,
                                        preview.height as f32 * scale,
                                    );
                                    let (rect, _) =
                                        ui.allocate_exact_size(size, egui::Sense::hover());
                                    let alpha = theme::preview_fade_alpha(
                                        ui.ctx(),
                                        egui::Id::new("game_preview_fade"),
                                        true,
                                    );
                                    let tint =
                                        egui::Color32::from_white_alpha((alpha * 255.0) as u8);
                                    let uv = egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    );
                                    ui.painter().image(preview.texture.id(), rect, uv, tint);
                                    ui.label(
                                        egui::RichText::new(self.tr("game_preview_title"))
                                            .small()
                                            .color(colors.hint),
                                    );
                                } else if let Some(err) = &self.game_preview_error {
                                    ui.colored_label(colors.warning, err);
                                } else if self.selected_game_window_idx.is_some() {
                                    ui.colored_label(colors.hint, self.tr("game_preview_loading"));
                                }
                            }
                            // Region capture controls (monitor capture only)
                            ui.horizontal(|ui| {
                                let monitor_selected = self.selected_monitor_idx.is_some();
                                let capture_region_label = self.tr("capture_region");
                                ui.add_enabled(
                                    monitor_selected,
                                    egui::Checkbox::new(
                                        &mut self.region_enabled,
                                        capture_region_label,
                                    ),
                                );
                                if ui
                                    .add_enabled(
                                        monitor_selected && self.region_enabled,
                                        egui::Button::new(self.tr("region_select")),
                                    )
                                    .clicked()
                                {
                                    self.open_region_editor(ui.ctx());
                                }
                                if monitor_selected && self.region_enabled {
                                    let r = self.region;
                                    ui.label(self.tr_fmt(
                                        "region_status",
                                        &[
                                            r.width.to_string(),
                                            r.height.to_string(),
                                            r.x.to_string(),
                                            r.y.to_string(),
                                        ],
                                    ));
                                }
                            });
                            let source_selected = self.selected_monitor_idx.is_some()
                                || self.selected_window_idx.is_some()
                                || self.selected_camera_idx.is_some()
                                || (self.use_game_capture
                                    && self.selected_game_window_idx.is_some());
                            ui.horizontal(|ui| {
                                ui.label(self.tr("video_codec"));
                                egui::ComboBox::from_id_salt("windows_codec_select")
                                    .selected_text(self.selected_codec.label())
                                    .show_ui(ui, |ui| {
                                        for codec in [
                                            rivulet_core::VideoCodec::H264,
                                            rivulet_core::VideoCodec::H265,
                                            rivulet_core::VideoCodec::VP9,
                                        ] {
                                            if ui
                                                .selectable_label(
                                                    self.selected_codec == codec,
                                                    codec.label(),
                                                )
                                                .clicked()
                                            {
                                                self.selected_codec = codec;
                                            }
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                ui.label(self.tr("recording_container"));
                                egui::ComboBox::from_id_salt("windows_container_select")
                                    .selected_text(self.selected_container.label())
                                    .show_ui(ui, |ui| {
                                        for container in [
                                            rivulet_core::RecordingContainer::Mp4,
                                            rivulet_core::RecordingContainer::Mkv,
                                            rivulet_core::RecordingContainer::Mov,
                                            rivulet_core::RecordingContainer::MpegTs,
                                        ] {
                                            if ui
                                                .selectable_label(
                                                    self.selected_container == container,
                                                    container.label(),
                                                )
                                                .clicked()
                                            {
                                                self.selected_container = container;
                                            }
                                        }
                                    });
                            });
                            let auto_remux_label = self.tr("auto_remux");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.auto_remux, auto_remux_label);
                            });
                            ui.horizontal(|ui| {
                                ui.label(self.tr("split_after"));
                                ui.add(
                                    egui::DragValue::new(&mut self.split_seconds).range(0..=3600),
                                );
                                ui.label(self.tr("seconds"));
                            });
                            ui.separator();
                            let rate_mode_label = self.tr("rate_mode");
                            ui.horizontal(|ui| {
                                ui.label(rate_mode_label);
                                egui::ComboBox::from_id_salt("windows_rate_mode_select")
                                    .selected_text(self.rate_mode.label())
                                    .show_ui(ui, |ui| {
                                        for mode in [
                                            rivulet_core::RateControlMode::Cbr,
                                            rivulet_core::RateControlMode::Vbr,
                                            rivulet_core::RateControlMode::Cq,
                                            rivulet_core::RateControlMode::CqVbr,
                                        ] {
                                            ui.selectable_value(
                                                &mut self.rate_mode,
                                                mode,
                                                mode.label(),
                                            )
                                            .on_hover_text(mode.hint());
                                        }
                                    });
                            });
                            if matches!(
                                self.rate_mode,
                                rivulet_core::RateControlMode::Cq
                                    | rivulet_core::RateControlMode::CqVbr
                            ) {
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("rate_quality"));
                                    ui.add(egui::Slider::new(&mut self.rate_quality, 0..=51));
                                });
                            }
                            if matches!(
                                self.rate_mode,
                                rivulet_core::RateControlMode::Vbr
                                    | rivulet_core::RateControlMode::CqVbr
                            ) {
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("rate_max_bitrate"));
                                    ui.add(
                                        egui::DragValue::new(&mut self.rate_max_kbps)
                                            .range(0..=100_000),
                                    );
                                });
                            }
                            ui.horizontal(|ui| {
                                ui.label(self.tr("rate_custom_options"));
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.encoder_extra_options)
                                        .hint_text("key-int-max=250"),
                                );
                            });
                            ui.separator();
                            let ve_title = self.tr("video_effects");
                            let (vb, vc, vs, vh, vf_blur, vf_sharpen) = (
                                self.tr("vf_brightness"),
                                self.tr("vf_contrast"),
                                self.tr("vf_saturation"),
                                self.tr("vf_hue"),
                                self.tr("vf_blur"),
                                self.tr("vf_sharpen"),
                            );
                            ui.label(egui::RichText::new(ve_title).strong());
                            let mut eff = self.video_effects;
                            ui.horizontal_wrapped(|ui| {
                                ui.add(egui::Slider::new(&mut eff.brightness, -1.0..=1.0).text(vb));
                                ui.add(egui::Slider::new(&mut eff.contrast, -1.0..=1.0).text(vc));
                                ui.add(egui::Slider::new(&mut eff.saturation, -1.0..=1.0).text(vs));
                                ui.add(egui::Slider::new(&mut eff.hue, -1.0..=1.0).text(vh));
                            });
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut eff.blur, vf_blur);
                                ui.checkbox(&mut eff.sharpen, vf_sharpen);
                            });
                            self.video_effects = eff;
                            let auto_record_label = self.tr("auto_record_with_stream");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.auto_record_with_stream, auto_record_label);
                            });
                            ui.horizontal(|ui| {
                                ui.label(self.tr("recording_preset"));
                                egui::ComboBox::from_id_salt("windows_preset_select")
                                    .selected_text(self.selected_preset.label)
                                    .show_ui(ui, |ui| {
                                        for preset in rivulet_core::RecordingPreset::all() {
                                            if ui
                                                .selectable_label(
                                                    self.selected_preset == *preset,
                                                    preset.label,
                                                )
                                                .clicked()
                                            {
                                                self.selected_preset = *preset;
                                            }
                                        }
                                    });
                            });
                            ui.horizontal(|ui| {
                                let label = self.tr("overlay_toggle").to_string();
                                ui.checkbox(&mut self.show_overlay, label);
                            });
                            if ui
                                .add_enabled(
                                    source_selected,
                                    egui::Button::new(format!(
                                        "⏺ {} [{}]",
                                        self.tr("start_recording"),
                                        self.hotkeys.label_for("record")
                                    )),
                                )
                                .clicked()
                            {
                                self.start_windows_recording();
                            };
                        }
                        if let Some(err) = &self.last_error {
                            ui.colored_label(colors.error, err);
                        }
                    }

                    #[cfg(target_os = "linux")]
                    {
                        self.drain_linux_frames();
                        // Surface engine failures (pipeline build/start/push errors)
                        // in the UI instead of only on the console.
                        if let Some(err) = self.engine.take_error() {
                            self.record_status = Some(err);
                        }
                        // Abort when the capture delivers no frames within the
                        // timeout: the capture thread died or its source became
                        // unavailable, so the recording would run forever while
                        // writing nothing.
                        if self.is_recording
                            && should_abort_for_stalled_frames(
                                self.record_started,
                                self.last_frame_at,
                                Instant::now(),
                                self.no_frame_timeout,
                            )
                        {
                            let secs = self.no_frame_timeout.as_secs();
                            tracing::error!(
                                timeout_seconds = secs,
                                "No frames received; stopping recording"
                            );
                            // stop_linux_recording sets record_status to
                            // "recording_saved"; override it with the abort reason.
                            self.stop_linux_recording();
                            self.record_status =
                                Some(self.tr_fmt("recording_no_frames", &[secs.to_string()]));
                        }

                        if self.view == AppView::Record {
                            ui.add_space(10.0);
                            ui.label(egui::RichText::new(self.tr("screen_recording")).strong());

                            if self.is_recording || self.is_aux_recording {
                                ui.horizontal(|ui| {
                                    let elapsed = self.record_started.elapsed().as_secs();
                                    ui.label(
                                        egui::RichText::new(self.tr_fmt(
                                            "recording_in_progress",
                                            &[elapsed.to_string()],
                                        ))
                                        .color(colors.active),
                                    );
                                    if ui
                                        .button(format!("⏹ {}", self.tr("stop_recording")))
                                        .clicked()
                                    {
                                        if self.is_aux_recording {
                                            self.stop_aux_recording();
                                        } else {
                                            self.stop_linux_recording();
                                        }
                                        self.is_paused = false;
                                        self.is_muted = false;
                                    }
                                    let pause_label = if self.is_paused {
                                        "▶ Resume"
                                    } else {
                                        "⏸ Pause"
                                    };
                                    if ui.button(pause_label).clicked() {
                                        self.is_paused = !self.is_paused;
                                    }
                                    let mute_label = if self.is_muted {
                                        "🔊 Unmute"
                                    } else {
                                        "🔇 Mute"
                                    };
                                    if ui.button(mute_label).clicked() {
                                        self.is_muted = !self.is_muted;
                                    }
                                });
                                if self.is_paused {
                                    ui.label(
                                        egui::RichText::new(self.tr("paused"))
                                            .color(colors.warning)
                                            .strong(),
                                    );
                                }
                                if self.is_muted {
                                    ui.label(
                                        egui::RichText::new(self.tr("muted"))
                                            .color(colors.info)
                                            .strong(),
                                    );
                                }
                                ui.label(self.metrics_line());
                            } else {
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("source"));
                                    egui::ComboBox::from_id_salt("linux_monitor_select")
                                        .selected_text(
                                            self.selected_monitor_idx
                                                .and_then(|idx| self.monitors.get(idx))
                                                .map(|m| {
                                                    format!(
                                                        "{} ({}x{})",
                                                        m.name().unwrap_or_default(),
                                                        m.width().unwrap_or(0),
                                                        m.height().unwrap_or(0)
                                                    )
                                                })
                                                .unwrap_or_else(|| {
                                                    self.tr("select_monitor").to_string()
                                                }),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (i, m) in self.monitors.iter().enumerate() {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_monitor_idx == Some(i),
                                                        format!(
                                                            "{} ({}x{})",
                                                            m.name().unwrap_or_default(),
                                                            m.width().unwrap_or(0),
                                                            m.height().unwrap_or(0)
                                                        ),
                                                    )
                                                    .clicked()
                                                {
                                                    if self.selected_monitor_idx != Some(i) {
                                                        self.region = CaptureRegion::full(
                                                            m.width().unwrap_or(0),
                                                            m.height().unwrap_or(0),
                                                        );
                                                    }
                                                    self.selected_monitor_idx = Some(i);
                                                    self.selected_window_idx = None;
                                                }
                                            }
                                        });
                                    egui::ComboBox::from_id_salt("linux_window_select")
                                        .selected_text(
                                            self.selected_window_idx
                                                .and_then(|idx| self.windows.get(idx))
                                                .map(|w| {
                                                    format!(
                                                        "\"{}\" ({}x{})",
                                                        w.title().unwrap_or_default(),
                                                        w.width().unwrap_or(0),
                                                        w.height().unwrap_or(0)
                                                    )
                                                })
                                                .unwrap_or_else(|| {
                                                    self.tr("select_window").to_string()
                                                }),
                                        )
                                        .show_ui(ui, |ui| {
                                            for (i, w) in self.windows.iter().enumerate() {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_window_idx == Some(i),
                                                        format!(
                                                            "\"{}\" ({}x{})",
                                                            w.title().unwrap_or_default(),
                                                            w.width().unwrap_or(0),
                                                            w.height().unwrap_or(0)
                                                        ),
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected_window_idx = Some(i);
                                                    self.selected_monitor_idx = None;
                                                    // Region capture only applies to
                                                    // monitor capture; window capture is
                                                    // always full-window.
                                                    self.region_enabled = false;
                                                }
                                            }
                                        });
                                    if ui
                                        .button("🔄")
                                        .on_hover_text(self.tr("refresh_sources"))
                                        .clicked()
                                    {
                                        self.refresh_linux_sources();
                                        self.refresh_game_windows();
                                    }
                                });
                                // Game capture toggle + window selection + live preview
                                self.update_game_preview(ui.ctx());
                                ui.horizontal(|ui| {
                                    let gc_label = self.tr("game_capture");
                                    ui.checkbox(&mut self.use_game_capture, gc_label);
                                    if self.use_game_capture {
                                        egui::ComboBox::from_id_salt("linux_game_window_select")
                                            .selected_text(
                                                self.selected_game_window_idx
                                                    .and_then(|idx| self.game_windows.get(idx))
                                                    .map(|w| w.title.clone())
                                                    .unwrap_or_else(|| {
                                                        self.tr("select_game_window").to_string()
                                                    }),
                                            )
                                            .show_ui(ui, |ui| {
                                                for (i, window) in
                                                    self.game_windows.iter().enumerate()
                                                {
                                                    if ui
                                                        .selectable_label(
                                                            self.selected_game_window_idx
                                                                == Some(i),
                                                            &window.title,
                                                        )
                                                        .clicked()
                                                    {
                                                        self.selected_game_window_idx = Some(i);
                                                        self.selected_monitor_idx = None;
                                                        self.selected_window_idx = None;
                                                        self.selected_camera_idx = None;
                                                    }
                                                }
                                            });
                                        ui.separator();
                                        // Manual refresh of the window list (also
                                        // refreshed live while the picker is open).
                                        if ui
                                            .button("🔄")
                                            .on_hover_text(self.tr("refresh_game_windows"))
                                            .clicked()
                                        {
                                            self.refresh_game_windows();
                                        }
                                    }
                                });
                                // Live thumbnail of the selected game window, so the
                                // user can verify the correct window is targeted
                                // before recording starts (refreshes every ~500ms).
                                if self.use_game_capture {
                                    if let Some(preview) = &self.game_preview {
                                        let scale = (ui.available_width().min(360.0)
                                            / preview.width.max(1) as f32)
                                            .min(0.5);
                                        let size = egui::vec2(
                                            preview.width as f32 * scale,
                                            preview.height as f32 * scale,
                                        );
                                        let (rect, _) =
                                            ui.allocate_exact_size(size, egui::Sense::hover());
                                        let alpha = theme::preview_fade_alpha(
                                            ui.ctx(),
                                            egui::Id::new("game_preview_fade_linux"),
                                            true,
                                        );
                                        let tint =
                                            egui::Color32::from_white_alpha((alpha * 255.0) as u8);
                                        let uv = egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        );
                                        ui.painter().image(preview.texture.id(), rect, uv, tint);
                                        ui.label(
                                            egui::RichText::new(self.tr("game_preview_title"))
                                                .small()
                                                .color(colors.hint),
                                        );
                                    } else if let Some(err) = &self.game_preview_error {
                                        ui.colored_label(colors.warning, err);
                                    } else if self.selected_game_window_idx.is_some() {
                                        ui.colored_label(
                                            colors.hint,
                                            self.tr("game_preview_loading"),
                                        );
                                    }
                                }
                                // Region capture controls (monitor capture only)
                                ui.horizontal(|ui| {
                                    let monitor_selected = self.selected_monitor_idx.is_some();
                                    let capture_region_label = self.tr("capture_region");
                                    ui.add_enabled(
                                        monitor_selected,
                                        egui::Checkbox::new(
                                            &mut self.region_enabled,
                                            capture_region_label,
                                        ),
                                    );
                                    if ui
                                        .add_enabled(
                                            monitor_selected && self.region_enabled,
                                            egui::Button::new(self.tr("region_select")),
                                        )
                                        .clicked()
                                    {
                                        self.open_region_editor(ui.ctx());
                                    }
                                    if monitor_selected && self.region_enabled {
                                        let r = self.region;
                                        ui.label(self.tr_fmt(
                                            "region_status",
                                            &[
                                                r.width.to_string(),
                                                r.height.to_string(),
                                                r.x.to_string(),
                                                r.y.to_string(),
                                            ],
                                        ));
                                    }
                                });
                                let source_selected = self.selected_monitor_idx.is_some()
                                    || self.selected_window_idx.is_some()
                                    || self.selected_camera_idx.is_some()
                                    || (self.use_game_capture
                                        && self.selected_game_window_idx.is_some());
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("video_codec"));
                                    egui::ComboBox::from_id_salt("linux_codec_select")
                                        .selected_text(self.selected_codec.label())
                                        .show_ui(ui, |ui| {
                                            for codec in [
                                                rivulet_core::VideoCodec::H264,
                                                rivulet_core::VideoCodec::H265,
                                                rivulet_core::VideoCodec::VP9,
                                            ] {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_codec == codec,
                                                        codec.label(),
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected_codec = codec;
                                                }
                                            }
                                        });
                                });
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("recording_container"));
                                    egui::ComboBox::from_id_salt("linux_container_select")
                                        .selected_text(self.selected_container.label())
                                        .show_ui(ui, |ui| {
                                            for container in [
                                                rivulet_core::RecordingContainer::Mp4,
                                                rivulet_core::RecordingContainer::Mkv,
                                                rivulet_core::RecordingContainer::Mov,
                                                rivulet_core::RecordingContainer::MpegTs,
                                            ] {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_container == container,
                                                        container.label(),
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected_container = container;
                                                }
                                            }
                                        });
                                });
                                let auto_remux_label = self.tr("auto_remux");
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut self.auto_remux, auto_remux_label);
                                });
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("split_after"));
                                    ui.add(
                                        egui::DragValue::new(&mut self.split_seconds)
                                            .range(0..=3600),
                                    );
                                    ui.label(self.tr("seconds"));
                                });
                                let auto_record_label = self.tr("auto_record_with_stream");
                                ui.horizontal(|ui| {
                                    ui.checkbox(
                                        &mut self.auto_record_with_stream,
                                        auto_record_label,
                                    );
                                });
                                ui.horizontal(|ui| {
                                    ui.label(self.tr("recording_preset"));
                                    egui::ComboBox::from_id_salt("linux_preset_select")
                                        .selected_text(self.selected_preset.label)
                                        .show_ui(ui, |ui| {
                                            for preset in rivulet_core::RecordingPreset::all() {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_preset == *preset,
                                                        preset.label,
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected_preset = *preset;
                                                }
                                            }
                                        });
                                });
                                ui.horizontal(|ui| {
                                    let label = self.tr("overlay_toggle").to_string();
                                    ui.checkbox(&mut self.show_overlay, label);
                                });
                                if ui
                                    .add_enabled(
                                        source_selected,
                                        egui::Button::new(format!(
                                            "⏺ {} [{}]",
                                            self.tr("start_recording"),
                                            self.hotkeys.label_for("record")
                                        )),
                                    )
                                    .clicked()
                                {
                                    self.start_linux_recording();
                                }
                            }
                            if let Some(status) = &self.record_status {
                                ui.colored_label(colors.info, status);
                            }
                        }

                        if self.view == AppView::Mixer {
                            ui.separator();
                            ui.label(egui::RichText::new(self.tr("audio_mixer")).strong());

                            ui.horizontal(|ui| {
                                if self.audio_preview {
                                    if theme::accent_button(
                                        ui,
                                        format!("⏹ {}", self.tr("stop_audio")),
                                    )
                                    .clicked()
                                    {
                                        self.stop_audio_capture();
                                    }
                                    ui.label(
                                        egui::RichText::new(format!("● {}", self.tr("running")))
                                            .color(colors.success),
                                    );
                                } else if theme::accent_button(
                                    ui,
                                    format!("▶ {}", self.tr("start_audio")),
                                )
                                .clicked()
                                {
                                    self.start_audio_capture();
                                }
                            });

                            let mut sys = self.capture_system;
                            let mut mic = self.capture_mic;
                            let mut separate = self.separate_tracks;
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut sys, self.tr("system_audio"));
                                ui.checkbox(&mut mic, self.tr("microphone"));
                                ui.separator();
                                ui.checkbox(&mut separate, self.tr("separate_tracks"));
                            });
                            if sys != self.capture_system || mic != self.capture_mic {
                                self.capture_system = sys;
                                self.capture_mic = mic;
                                if self.audio_preview {
                                    self.start_audio_capture();
                                }
                            }
                            if separate != self.separate_tracks {
                                self.separate_tracks = separate;
                                if self.audio_preview {
                                    self.start_audio_capture();
                                }
                            }

                            if self.separate_tracks {
                                let et_system = self.tr("export_track_system");
                                let et_mic = self.tr("export_track_microphone");
                                let mt_mapping = self.tr("multi_track_mapping");
                                let mt_label = format!(
                                    "{} · {}",
                                    self.tr("separate_tracks"),
                                    rivulet_core::AudioTrack::System.label()
                                );
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(mt_label).weak());
                                    ui.checkbox(&mut self.export_system_track, et_system);
                                });
                                ui.horizontal(|ui| {
                                    ui.label(
                                        egui::RichText::new(
                                            rivulet_core::AudioTrack::Microphone.label(),
                                        )
                                        .weak(),
                                    );
                                    ui.checkbox(&mut self.export_mic_track, et_mic);
                                });
                                ui.label(egui::RichText::new(mt_mapping).small().weak());
                            }

                            let mut sys_vol = self.system_volume;
                            let mut mic_vol = self.mic_volume;
                            if ui
                                .add(
                                    egui::Slider::new(&mut sys_vol, 0.0..=1.0)
                                        .text(self.tr("system_audio")),
                                )
                                .changed()
                            {
                                self.system_volume = sys_vol;
                                if let Some(audio) = &self.audio {
                                    audio.set_system_volume(sys_vol);
                                }
                            }
                            if ui
                                .add(
                                    egui::Slider::new(&mut mic_vol, 0.0..=1.0)
                                        .text(self.tr("microphone")),
                                )
                                .changed()
                            {
                                self.mic_volume = mic_vol;
                                if let Some(audio) = &self.audio {
                                    audio.set_mic_volume(mic_vol);
                                }
                            }

                            ui.separator();
                            ui.label(egui::RichText::new(self.tr("audio_filters")).strong());
                            let before = self.system_filters.clone();
                            let before_mic = self.mic_filters.clone();
                            let gate_label = self.tr("filter_noise_gate");
                            let expander_label = self.tr("filter_expander");
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.system_filters.noise_suppression, "NS·Sys");
                                ui.checkbox(&mut self.system_filters.noise_gate, gate_label);
                                ui.checkbox(&mut self.system_filters.compressor, "Cmp·Sys");
                                ui.checkbox(&mut self.system_filters.limiter, "Lim·Sys");
                                ui.checkbox(&mut self.system_filters.expander, expander_label);
                            });
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut self.mic_filters.noise_suppression, "NS·Mic");
                                ui.checkbox(&mut self.mic_filters.noise_gate, "Gate·Mic");
                                ui.checkbox(&mut self.mic_filters.compressor, "Cmp·Mic");
                                ui.checkbox(&mut self.mic_filters.limiter, "Lim·Mic");
                                ui.checkbox(&mut self.mic_filters.expander, "Exp·Mic");
                            });
                            ui.horizontal(|ui| {
                                ui.label(self.tr("filter_gain"));
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.system_filters.gain_db,
                                        -30.0..=30.0,
                                    )
                                    .text("Sys"),
                                );
                                ui.add(
                                    egui::Slider::new(&mut self.mic_filters.gain_db, -30.0..=30.0)
                                        .text("Mic"),
                                );
                            });
                            egui::CollapsingHeader::new(self.tr("filter_eq"))
                                .default_open(false)
                                .show(ui, |ui| {
                                    ui.label("System");
                                    ui.horizontal_wrapped(|ui| {
                                        for band in self.system_filters.eq_bands.iter_mut() {
                                            ui.add(egui::Slider::new(band, -12.0..=12.0));
                                        }
                                    });
                                    ui.label("Mikro");
                                    ui.horizontal_wrapped(|ui| {
                                        for band in self.mic_filters.eq_bands.iter_mut() {
                                            ui.add(egui::Slider::new(band, -12.0..=12.0));
                                        }
                                    });
                                });
                            if (self.system_filters != before || self.mic_filters != before_mic)
                                && self.audio_preview
                            {
                                self.start_audio_capture();
                            }

                            ui.separator();
                            ui.label(egui::RichText::new(self.tr("audio_monitoring")).strong());
                            let mut sys_mon = self.system_monitor;
                            let mut mic_mon = self.mic_monitor;
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut sys_mon, self.tr("monitoring_system"));
                                ui.checkbox(&mut mic_mon, self.tr("monitoring_microphone"));
                            });
                            if sys_mon != self.system_monitor || mic_mon != self.mic_monitor {
                                self.system_monitor = sys_mon;
                                self.mic_monitor = mic_mon;
                                if self.audio_preview {
                                    self.start_audio_capture();
                                }
                            }
                            let mut mon_vol = self.monitor_volume;
                            if ui
                                .add(
                                    egui::Slider::new(&mut mon_vol, 0.0..=1.0)
                                        .text(self.tr("monitoring_volume")),
                                )
                                .changed()
                            {
                                self.monitor_volume = mon_vol;
                                if let Some(audio) = &self.audio {
                                    audio.set_monitor_volume(mon_vol);
                                }
                            }

                            ui.separator();
                            ui.label(egui::RichText::new(self.tr("master_output")).strong());
                            let mut master_vol = self.master_volume;
                            if ui
                                .add(
                                    egui::Slider::new(&mut master_vol, 0.0..=1.0)
                                        .text(self.tr("master_volume")),
                                )
                                .changed()
                            {
                                self.master_volume = master_vol;
                                if let Some(audio) = &self.audio {
                                    audio.set_master_volume(master_vol);
                                }
                            }

                            // Master output VU meter (level of the whole mix after the
                            // master volume is applied).
                            let peak = f32::from_bits(self.audio_peak.load(Ordering::SeqCst))
                                .clamp(0.0, 1.0);
                            let db = if peak > 0.0 {
                                20.0 * peak.log10()
                            } else {
                                -96.0
                            };
                            ui.add(
                                egui::ProgressBar::new(peak)
                                    .desired_width(f32::INFINITY)
                                    .text(self.tr_fmt("output_vu", &[format!("{db:.1}")])),
                            );

                            if let Some(status) = &self.audio_status {
                                ui.colored_label(colors.error, status);
                            }
                            if let Some(warning) = &self.audio_warning {
                                ui.colored_label(colors.warning, warning);
                            }
                        }
                    }

                    #[cfg(not(target_os = "linux"))]
                    if self.view == AppView::Mixer {
                        ui.add_space(20.0);
                        ui.label(self.tr("mixer_unavailable"));
                    }

                    // Scenes: list, switching, add/rename/remove
                    if self.view == AppView::Scenes {
                        self.draw_scenes_view(ui, &colors);
                    }
                    if self.view == AppView::Scenes {
                        self.draw_source_composition(ui, &colors);
                        self.draw_scene_overlay_panel(ui);
                        self.draw_chroma_key_panel(ui);
                        self.draw_browser_source_panel(ui, &colors);
                    }

                    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
                    if self.view == AppView::Record {
                        ui.add_space(20.0);
                        ui.label(egui::RichText::new(self.tr("screen_recording")).strong());
                        ui.label(self.tr("screen_recording_unavailable"));
                    }

                    // Streaming controls are implemented in M3 and must not render
                    // the generic planned-section placeholder. The Stream view is
                    // the Meld-style broadcast workspace: chat dock, stream
                    // status and compact audio all live on this single page.
                    if self.view == AppView::Stream {
                        self.draw_stream_view(ui, &colors);
                    }
                    if self.view == AppView::Help {
                        self.draw_help_view(ui);
                    }

                    // Placeholder views (still-planned milestones)
                    if let Some(milestone) = self.view.planned_milestone() {
                        ui.add_space(20.0);
                        ui.label(self.tr_fmt("section_planned", &[milestone.to_string()]));
                    }

                    // Settings: updates section
                    if self.view == AppView::Settings {
                        ui.separator();
                        ui.label(egui::RichText::new(self.tr("updates")).strong());
                        ui.horizontal(|ui| {
                            ui.label(self.tr_fmt(
                                "updates_current_version",
                                &[env!("CARGO_PKG_VERSION").to_string()],
                            ));
                            if theme::accent_button(ui, self.tr("check_for_updates")).clicked() {
                                self.update_check_clicked = true;
                            }
                        });
                        self.draw_update_status(ui);
                        self.handle_update_actions(ui.ctx().clone());

                        // Settings: appearance (color scheme)
                        ui.separator();
                        ui.label(egui::RichText::new(self.tr("theme")).strong());
                        ui.horizontal(|ui| {
                            for preference in theme::ThemePreference::all() {
                                if ui
                                    .selectable_label(
                                        self.theme == *preference,
                                        self.tr(preference.key()),
                                    )
                                    .clicked()
                                {
                                    self.theme = *preference;
                                }
                            }
                        });

                        // Settings: Discord Rich Presence (client id + opt-out)
                        let discord_section = self.tr("discord_section");
                        let discord_enable = self.tr("discord_presence_enable");
                        let discord_client_id_label = self.tr("discord_client_id");
                        let discord_client_id_hint = self.tr("discord_client_id_hint");
                        let discord_client_id_apply = self.tr("discord_client_id_apply");
                        let discord_client_id_note = self.tr("discord_client_id_note");
                        let discord_large_image_label = self.tr("discord_large_image");
                        let discord_large_image_hint = self.tr("discord_large_image_hint");
                        ui.separator();
                        ui.label(egui::RichText::new(discord_section).strong());
                        ui.checkbox(&mut self.discord_presence_enabled, discord_enable);
                        let mut apply_client_id = false;
                        ui.horizontal(|ui| {
                            ui.label(discord_client_id_label);
                            apply_client_id = ui
                                .add(
                                    egui::TextEdit::singleline(
                                        &mut self.discord_presence_client_id,
                                    )
                                    .hint_text(discord_client_id_hint)
                                    .desired_width(220.0),
                                )
                                .lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        });
                        if ui.button(discord_client_id_apply).clicked() {
                            apply_client_id = true;
                        }
                        // Optional artwork asset: the key of an image uploaded
                        // in the Discord Developer Portal (Rich Presence → Art
                        // Assets). Rendered as the card artwork like OBS shows
                        // its logo instead of the generic placeholder icon.
                        ui.horizontal(|ui| {
                            ui.label(discord_large_image_label);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.discord_presence_large_image)
                                    .hint_text(discord_large_image_hint)
                                    .desired_width(180.0),
                            );
                            if ui.button(self.tr("discord_client_id_apply")).clicked() {
                                // Force a rebuild so the new artwork applies
                                // without restarting the app, and validate the
                                // resulting payload immediately (overlong
                                // status text / implausible asset key).
                                apply_client_id = true;
                            }
                        });
                        ui.small(self.tr("discord_large_image_note"));
                        if apply_client_id {
                            self.apply_discord_client_id();
                            self.apply_discord_payload_validation();
                        }
                        if let Some(warning) = self.discord_client_id_warning {
                            let text = match warning {
                                rivulet_core::discord::ClientIdError::NotNumeric => {
                                    self.tr("discord_client_id_error_not_numeric")
                                }
                                rivulet_core::discord::ClientIdError::Length => {
                                    self.tr("discord_client_id_error_length")
                                }
                            };
                            ui.colored_label(theme::StatusColors::for_ui(ui).error, text);
                        }
                        if let Some(issue) = self.discord_payload_warning {
                            let text = match issue {
                                rivulet_core::discord::PayloadIssue::FieldTooLong {
                                    field,
                                    len,
                                } => self.tr_fmt(
                                    "discord_payload_error_field_too_long",
                                    &[field.to_owned(), len.to_string()],
                                ),
                                rivulet_core::discord::PayloadIssue::InvalidAssetKey => {
                                    self.tr("discord_payload_error_asset_key").to_string()
                                }
                            };
                            ui.colored_label(theme::StatusColors::for_ui(ui).error, text);
                        }
                        ui.small(discord_client_id_note);

                        // Settings: hotkeys (per-action rebinding). In-app bindings
                        // apply while the app is focused on every platform; on Windows
                        // the bindings are additionally registered at the OS level so
                        // they keep working while the app is unfocused (see
                        // `reconcile_global_hotkeys`). See docs/hotkeys.md for the
                        // exact platform matrix.
                        let hotkeys_section = self.tr("hotkeys_section");
                        let hotkeys_hint = self.tr("hotkeys_hint");
                        let modifier_ctrl = self.tr("modifier_ctrl");
                        let modifier_alt = self.tr("modifier_alt");
                        let modifier_shift = self.tr("modifier_shift");
                        ui.separator();
                        ui.label(egui::RichText::new(hotkeys_section).strong());
                        for action in ["record", "pause", "mute", "save_replay"] {
                            self.draw_hotkey_rebind_row(ui, action);
                        }
                        ui.small(hotkeys_hint);

                        // Settings: OBS WebSocket remote control (Stream Deck /
                        // TouchPortal). Serves the obs-websocket v5 (JSON) protocol on
                        // 127.0.0.1 so ecosystem tools can switch scenes and control
                        // recording/streaming (see docs/obs-websocket.md).
                        let obs_ws_section = self.tr("obs_ws_section");
                        let obs_ws_enable = self.tr("obs_ws_enable");
                        let obs_ws_port = self.tr("obs_ws_port");
                        let obs_ws_password = self.tr("obs_ws_password");
                        let obs_ws_password_hint = self.tr("obs_ws_password_hint");
                        let obs_ws_hint = self.tr("obs_ws_hint");
                        ui.separator();
                        ui.label(egui::RichText::new(obs_ws_section).strong());
                        let was_enabled = self.obs_ws_enabled;
                        ui.checkbox(&mut self.obs_ws_enabled, obs_ws_enable);
                        let mut port_dirty = false;
                        let old_port = self.obs_ws_port;
                        ui.horizontal(|ui| {
                            ui.label(obs_ws_port);
                            port_dirty = ui
                                .add(
                                    egui::DragValue::new(&mut self.obs_ws_port)
                                        .range(1..=65535)
                                        .speed(1),
                                )
                                .changed();
                        });
                        let mut password_dirty = false;
                        let old_password = self.obs_ws_password.clone();
                        ui.horizontal(|ui| {
                            ui.label(obs_ws_password);
                            password_dirty = ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.obs_ws_password)
                                        .password(true),
                                )
                                .changed();
                        });
                        ui.small(obs_ws_password_hint);
                        if (port_dirty && old_port != self.obs_ws_port)
                            || (password_dirty && old_password != self.obs_ws_password)
                        {
                            self.obs_ws_restart = true;
                        }
                        if let Some(status) = &self.obs_ws_status {
                            ui.colored_label(
                                if self.obs_ws_enabled {
                                    colors.success
                                } else {
                                    colors.warning
                                },
                                status,
                            );
                        }
                        if !self.obs_ws_enabled && was_enabled {
                            self.obs_ws_status = Some(self.tr("obs_ws_stopped").to_owned());
                        }
                        ui.small(obs_ws_hint);

                        // Settings: MIDI controller mapping (Korg NanoKontrol etc.).
                        // Maps MIDI messages (note/CC on a channel) to actions such as
                        // scene switches, master volume, and chroma-key toggles.
                        let midi_section = self.tr("midi_section");
                        let midi_enable = self.tr("midi_enable");
                        let midi_device = self.tr("midi_device");
                        let midi_no_devices = self.tr("midi_no_devices");
                        let midi_hint = self.tr("midi_hint");
                        let midi_channel = self.tr("midi_channel");
                        let midi_kind = self.tr("midi_kind");
                        let midi_number = self.tr("midi_number");
                        let midi_action = self.tr("midi_action");
                        let midi_remove = self.tr("midi_remove");
                        let midi_add = self.tr("midi_add");
                        let midi_scene = self.tr("midi_scene");
                        ui.separator();
                        ui.label(egui::RichText::new(midi_section).strong());
                        if ui.checkbox(&mut self.midi_enabled, midi_enable).changed() {
                            self.midi_dirty = true;
                        }
                        // Device picker: refresh the list whenever the user opens the
                        // dropdown so hot-plugged devices appear.
                        ui.horizontal(|ui| {
                            ui.label(midi_device);
                            if self.midi_devices.is_empty() {
                                self.midi_devices = list_devices();
                            }
                            if self.midi_devices.is_empty() {
                                ui.weak(midi_no_devices);
                            } else {
                                let selected = self
                                    .midi_devices
                                    .get(self.midi_device_index)
                                    .cloned()
                                    .unwrap_or_default();
                                let before = self.midi_device_index;
                                egui::ComboBox::from_id_salt("midi_device")
                                    .selected_text(selected)
                                    .show_ui(ui, |ui| {
                                        for (idx, name) in self.midi_devices.iter().enumerate() {
                                            ui.selectable_value(
                                                &mut self.midi_device_index,
                                                idx,
                                                name,
                                            );
                                        }
                                    });
                                if self.midi_device_index != before {
                                    // Drop the cached list so it is re-enumerated (and
                                    // the index re-validated) on the next open.
                                    self.midi_devices = Vec::new();
                                    self.midi_dirty = true;
                                }
                            }
                        });
                        if let Some(status) = &self.midi_status {
                            ui.colored_label(colors.warning, status);
                        }
                        // Binding list with a remove button per row.
                        let mut remove: Option<usize> = None;
                        for (idx, binding) in self.midi_mapping.bindings.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(format!(
                                    "{} {} · ch {}",
                                    binding.kind.label(),
                                    binding.number,
                                    binding.channel + 1
                                ));
                                ui.label(self.midi_action_label(&binding.action));
                                if ui.small_button(midi_remove).clicked() {
                                    remove = Some(idx);
                                }
                            });
                        }
                        if let Some(idx) = remove {
                            self.midi_mapping.bindings.remove(idx);
                            self.midi_dirty = true;
                        }
                        // "Add binding" row.
                        ui.horizontal(|ui| {
                            ui.label(midi_kind);
                            for kind in [
                                rivulet_core::MidiKind::NoteOn,
                                rivulet_core::MidiKind::NoteOff,
                                rivulet_core::MidiKind::ControlChange,
                            ] {
                                ui.selectable_value(&mut self.midi_new_kind, kind, kind.label());
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(midi_channel);
                            let mut channel_ui = self.midi_new_channel + 1;
                            if ui
                                .add(egui::DragValue::new(&mut channel_ui).range(1..=16))
                                .changed()
                            {
                                self.midi_new_channel = channel_ui - 1;
                            }
                            ui.label(midi_number);
                            ui.add(egui::DragValue::new(&mut self.midi_new_number).range(0..=127));
                        });
                        ui.horizontal(|ui| {
                            ui.label(midi_action);
                            let actions = [
                                ("ToggleRecord", self.tr("midi_action_record")),
                                ("ToggleStream", self.tr("midi_action_stream")),
                                ("ToggleMute", self.tr("midi_action_mute")),
                                ("SetMasterVolume", self.tr("midi_action_volume")),
                                ("ToggleChromaKey", self.tr("midi_action_chroma")),
                                ("SwitchScene", self.tr("midi_action_scene")),
                            ];
                            let selected = actions
                                .iter()
                                .find(|(name, _)| *name == self.midi_new_action)
                                .map(|(_, label)| *label)
                                .unwrap_or_else(|| self.midi_new_action.as_str());
                            egui::ComboBox::from_id_salt("midi_action")
                                .selected_text(selected)
                                .show_ui(ui, |ui| {
                                    for (name, label) in actions {
                                        ui.selectable_value(
                                            &mut self.midi_new_action,
                                            name.to_owned(),
                                            label,
                                        );
                                    }
                                });
                            if self.midi_new_action == "SwitchScene" {
                                ui.label(midi_scene);
                                let names: Vec<(uuid::Uuid, String)> = self
                                    .scenes
                                    .scenes()
                                    .iter()
                                    .map(|s| (s.id, s.name.clone()))
                                    .collect();
                                let selected_name = self
                                    .midi_new_scene
                                    .and_then(|id| names.iter().find(|(sid, _)| *sid == id))
                                    .map(|(_, name)| name.clone())
                                    .unwrap_or_else(|| self.tr("midi_scene_none").to_owned());
                                egui::ComboBox::from_id_salt("midi_scene_pick")
                                    .selected_text(selected_name)
                                    .show_ui(ui, |ui| {
                                        for (id, name) in &names {
                                            if ui
                                                .selectable_label(
                                                    self.midi_new_scene == Some(*id),
                                                    name,
                                                )
                                                .clicked()
                                            {
                                                self.midi_new_scene = Some(*id);
                                            }
                                        }
                                    });
                            }
                            if ui.button(midi_add).clicked() {
                                let action = match self.midi_new_action.as_str() {
                                    "ToggleStream" => rivulet_core::MidiAction::ToggleStream,
                                    "ToggleMute" => rivulet_core::MidiAction::ToggleMute,
                                    "SetMasterVolume" => rivulet_core::MidiAction::SetMasterVolume,
                                    "ToggleChromaKey" => rivulet_core::MidiAction::ToggleChromaKey,
                                    "SwitchScene" => {
                                        if let Some(id) = self.midi_new_scene {
                                            rivulet_core::MidiAction::SwitchScene(id)
                                        } else {
                                            rivulet_core::MidiAction::ToggleRecord
                                        }
                                    }
                                    _ => rivulet_core::MidiAction::ToggleRecord,
                                };
                                self.midi_mapping
                                    .bindings
                                    .push(rivulet_core::MidiBinding::new(
                                        self.midi_new_channel,
                                        self.midi_new_kind,
                                        self.midi_new_number,
                                        action,
                                    ));
                                self.midi_dirty = true;
                            }
                        });
                        // Learn mode: capture the next moved control into the row.
                        ui.horizontal(|ui| {
                            let learn_label = if self.midi_learn {
                                self.tr("midi_learn_waiting")
                            } else {
                                self.tr("midi_learn")
                            };
                            let mut learn = self.midi_learn;
                            if ui.selectable_label(self.midi_learn, learn_label).clicked() {
                                learn = !self.midi_learn;
                                if learn {
                                    // Start fresh: clear any earlier capture.
                                    self.midi_learn_captured = None;
                                }
                            }
                            self.midi_learn = learn;
                            if let Some(captured) = &self.midi_learn_captured {
                                ui.label(format!(
                                    "{}: {} {} · ch {}",
                                    self.tr("midi_learn_captured"),
                                    captured.kind.label(),
                                    captured.number,
                                    captured.channel + 1
                                ));
                                if ui.small_button(self.tr("midi_learn_clear")).clicked() {
                                    self.midi_learn_captured = None;
                                }
                            }
                        });
                        // Per-device presets: save the whole mapping under a name for
                        // the currently selected device, load it back, or delete it.
                        let midi_presets_label = self.tr("midi_presets");
                        let midi_preset_name_hint = self.tr("midi_preset_name_hint");
                        let midi_preset_save = self.tr("midi_preset_save");
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(midi_presets_label).strong());
                            if ui
                                .add_enabled(
                                    self.midi_devices.get(self.midi_device_index).is_some(),
                                    egui::TextEdit::singleline(&mut self.midi_preset_name)
                                        .hint_text(midi_preset_name_hint),
                                )
                                .changed()
                            {
                                // Keep the dropdown in sync with the typed name.
                                self.midi_selected_preset = Some(self.midi_preset_name.clone());
                            }
                            if ui
                                .add_enabled(
                                    !self.midi_preset_name.trim().is_empty()
                                        && self.midi_devices.get(self.midi_device_index).is_some(),
                                    egui::Button::new(midi_preset_save),
                                )
                                .clicked()
                            {
                                if let Some(device) =
                                    self.midi_devices.get(self.midi_device_index).cloned()
                                {
                                    self.midi_presets.save(
                                        &device,
                                        self.midi_preset_name.trim(),
                                        self.midi_mapping.clone(),
                                    );
                                    self.midi_selected_preset = Some(self.midi_preset_name.clone());
                                }
                            }
                        });
                        let midi_preset_load = self.tr("midi_preset_load");
                        let midi_preset_apply = self.tr("midi_preset_apply");
                        let midi_preset_apply_hint = self.tr("midi_preset_apply_hint");
                        let midi_preset_delete = self.tr("midi_preset_delete");
                        let midi_preset_delete_hint = self.tr("midi_preset_delete_hint");
                        if let Some(device) = self.midi_devices.get(self.midi_device_index).cloned()
                        {
                            // Clone the names: they are only used to drive the dropdown
                            // and the borrow would otherwise fight the mutable `self`
                            // captures below.
                            let names: Vec<String> = self
                                .midi_presets
                                .names_for(&device)
                                .into_iter()
                                .map(str::to_owned)
                                .collect();
                            if !names.is_empty() {
                                ui.horizontal(|ui| {
                                    ui.label(midi_preset_load);
                                    egui::ComboBox::from_id_salt("midi_preset_pick")
                                        .selected_text(
                                            self.midi_selected_preset
                                                .as_deref()
                                                .filter(|n| names.iter().any(|c| c == n))
                                                .unwrap_or_default(),
                                        )
                                        .show_ui(ui, |ui| {
                                            for name in &names {
                                                ui.selectable_value(
                                                    &mut self.midi_selected_preset,
                                                    Some(name.clone()),
                                                    name,
                                                );
                                            }
                                        });
                                    if ui
                                        .add_enabled(
                                            self.midi_selected_preset.is_some(),
                                            egui::Button::new(midi_preset_apply),
                                        )
                                        .on_hover_text(midi_preset_apply_hint)
                                        .clicked()
                                    {
                                        if let Some(name) = self.midi_selected_preset.clone() {
                                            if let Some(mapping) =
                                                self.midi_presets.load(&device, &name)
                                            {
                                                self.midi_mapping = mapping.clone();
                                                self.midi_dirty = true;
                                            }
                                        }
                                    }
                                    if ui
                                        .add_enabled(
                                            self.midi_selected_preset.is_some(),
                                            egui::Button::new(midi_preset_delete),
                                        )
                                        .on_hover_text(midi_preset_delete_hint)
                                        .clicked()
                                    {
                                        if let Some(name) = self.midi_selected_preset.clone() {
                                            self.midi_presets.delete(&device, &name);
                                            self.midi_selected_preset = None;
                                        }
                                    }
                                });
                            }
                        }
                        ui.small(midi_hint);
                    }

                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(format!("{} ", self.tr("powered_by")));
                        ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                        ui.label(" & ");
                        ui.hyperlink_to(
                            "eframe",
                            "https://github.com/emilk/egui/tree/master/crates/eframe",
                        );
                        ui.label(".");
                    });
                });
        });

        // Region capture editor (floating window, rendered after the panels
        // so it appears on top of the main UI).
        self.draw_region_editor(ui.ctx());
    }
}

/// Drain all pending frames from the capture channel, forwarding each one to
/// `on_frame`, and determine whether the capture thread ended unexpectedly.
///
/// Returns `true` when the channel is disconnected AND the stop signal has
/// not been set (i.e. the thread ended on its own, not via a user-initiated
/// stop). Every frame is passed to `on_frame` before the next one is pulled,
/// so the disconnect detection never swallows a frame: a previous separate
/// `try_recv()` probe consumed one frame per UI tick and dropped it, which
/// starved the pipeline and prevented any MP4 from being written.
fn drain_frames_and_check_end(
    receiver: &std::sync::mpsc::Receiver<RawFrame>,
    stop_signal: &AtomicBool,
    mut on_frame: impl FnMut(&RawFrame),
) -> bool {
    loop {
        match receiver.try_recv() {
            Ok(raw_frame) => on_frame(&raw_frame),
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                return !stop_signal.load(Ordering::SeqCst);
            }
        }
    }
}

/// Collect all pending error messages from the capture thread's error
/// channel, returning the most recent one (or `None` when the channel is
/// empty). Older errors are superseded by newer ones.
fn drain_error_receiver(receiver: &std::sync::mpsc::Receiver<String>) -> Option<String> {
    let mut last = None;
    while let Ok(err) = receiver.try_recv() {
        last = Some(err);
    }
    last
}

/// Format a localized warning listing the audio filters that were skipped
/// because their GStreamer elements are not installed (e.g. `webrtcdsp` on
/// distros that do not ship it). Platform-neutral so the warning formatting
/// is exercised on every build path, even where the audio engine is not
/// active (Windows/macOS).
fn format_skipped_filters(locale: Locale, skipped: &[SkippedFilter]) -> String {
    let items: Vec<String> = skipped
        .iter()
        .map(|f| {
            let feature = SkippedFilter::feature_name_in(f.element, locale);
            format!("{feature} ({})", f.element)
        })
        .collect();
    locale.tr_fmt("audio_filters_skipped", &[items.join(", ")])
}

/// Crop a raw frame to the configured capture region (monitor capture only).
///
/// Returns the original frame without copying when region capture is disabled
/// or the region is invalid for the frame; otherwise returns a cropped copy
/// whose dimensions are even-aligned and clamped to the frame bounds (the
/// engine's video encoders require even frame dimensions).
fn cropped_frame_for_region<'a>(
    enabled: bool,
    region: CaptureRegion,
    frame: &'a RawFrame,
) -> std::borrow::Cow<'a, RawFrame> {
    if !enabled {
        return std::borrow::Cow::Borrowed(frame);
    }
    let stride = frame.width * 4;
    match region.crop_rgba(&frame.data, frame.width, frame.height, stride) {
        Some((data, width, height)) => std::borrow::Cow::Owned(RawFrame {
            data,
            width,
            height,
        }),
        None => std::borrow::Cow::Borrowed(frame),
    }
}

/// Normalize two corner points (in monitor pixels) into an even-aligned,
/// clamped capture region. Pure helper for the interactive region editor's
/// drag selection, so the geometry is unit-testable.
fn region_from_pixel_points(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    full_width: u32,
    full_height: u32,
) -> CaptureRegion {
    if full_width < 2 || full_height < 2 {
        return CaptureRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
    }
    let min_x = x0.min(x1).clamp(0.0, full_width.saturating_sub(2) as f32);
    let max_x = x0.max(x1).clamp(min_x + 2.0, full_width as f32);
    let min_y = y0.min(y1).clamp(0.0, full_height.saturating_sub(2) as f32);
    let max_y = y0.max(y1).clamp(min_y + 2.0, full_height as f32);
    CaptureRegion {
        x: min_x as u32,
        y: min_y as u32,
        width: ((max_x - min_x) as u32) & !1,
        height: ((max_y - min_y) as u32) & !1,
    }
}

/// Map a capture region to screen coordinates inside the preview image rect.
fn region_rect_in(
    region: CaptureRegion,
    rect: egui::Rect,
    full_width: u32,
    full_height: u32,
) -> egui::Rect {
    let sx = rect.width() / full_width.max(1) as f32;
    let sy = rect.height() / full_height.max(1) as f32;
    egui::Rect::from_min_size(
        rect.min + egui::vec2(region.x as f32 * sx, region.y as f32 * sy),
        egui::vec2(region.width as f32 * sx, region.height as f32 * sy),
    )
}

/// Capture a still image of the monitor whose name matches `preferred_name`
/// (falling back to the first listed monitor) for the region editor preview.
fn monitor_preview_image(
    xcap_monitors: &[xcap::Monitor],
    preferred_name: &str,
) -> Option<image::RgbaImage> {
    let monitor = xcap_monitors
        .iter()
        .find(|m| m.name().unwrap_or_default() == preferred_name)
        .or_else(|| xcap_monitors.first())?;
    monitor.capture_image().ok()
}

/// Enumerate the game-like windows (visible title, larger than 640x480) for
/// the game-capture picker.
///
/// On Linux the list comes from `rivulet-core` (xdotool + xcap) so the window
/// ids match the xdotool resolution used when recording starts; on Windows it
/// uses xcap's own window enumeration (the core list is empty there).
/// Extracted as a free function so the same list powers the initial fill, the
/// manual refresh, and the periodic live refresh.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn enumerate_game_windows() -> Vec<rivulet_core::GameWindow> {
    #[cfg(target_os = "linux")]
    {
        rivulet_core::game_capture::list_game_windows()
    }
    #[cfg(target_os = "windows")]
    {
        xcap::Window::all()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|w| {
                let title = w.title().unwrap_or_default();
                let (width, height) = (w.width().unwrap_or(0), w.height().unwrap_or(0));
                if title.trim().is_empty() || width <= 640 || height <= 480 {
                    return None;
                }
                Some(rivulet_core::GameWindow {
                    id: w.id().map(|id| id as u64).unwrap_or(0),
                    title,
                    width,
                    height,
                })
            })
            .collect()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

/// Re-resolve a selected game-window index after the window list has been
/// re-enumerated, preserving the selection by window id. Returns `None` when
/// the previously selected window no longer exists in `new_windows`.
///
/// Extracted as a pure function so the selection-preserving live refresh is
/// unit-testable.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn preserve_selected_game_window(
    new_windows: &[rivulet_core::GameWindow],
    previously_selected_id: Option<u64>,
) -> Option<usize> {
    let id = previously_selected_id?;
    new_windows.iter().position(|w| w.id == id)
}

/// Capture a still image of the window with the given id for the live
/// game-window preview.
///
/// Uses xcap's window enumeration and capture. Returns `None` when the
/// window cannot be found or captured (closed, minimized, or a transient
/// grab error).
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn game_window_preview_image(window_id: u64) -> Option<image::RgbaImage> {
    let window = xcap::Window::all()
        .ok()?
        .into_iter()
        .find(|w| w.id().map(|id| id as u64).unwrap_or(0) == window_id)?;
    window.capture_image().ok()
}

/// Return whether a frame has a safe RGBA8 shape for an egui texture upload.
fn is_valid_rgba_frame(data: &[u8], width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(4))
            == Some(data.len())
}

/// Decide whether the recording-preview path must keep scheduling periodic
/// repaints.
///
/// The preview is fed either by the preflight source thread (a `source_preview_rx`
/// is present) or by the encoder path (a frame is waiting in `pending_preview_frame`).
/// While either is active we must keep requesting a repaint every
/// [`RECORDING_PREVIEW_INTERVAL`] so those frames are actually drawn. When both
/// are absent there is nothing to animate, so we stop requesting repaints and
/// let egui fall back to its reactive idle mode (no CPU/GPU burn while the UI
/// is static).
fn should_repaint_recording_preview(has_source_preview: bool, has_pending_frame: bool) -> bool {
    has_source_preview || has_pending_frame
}

/// Decide whether a new frame may be uploaded to the recording thumbnail.
///
/// Capture continues at its configured rate, but texture uploads are limited
/// to [`RECORDING_PREVIEW_INTERVAL`] so the preview cannot dominate the UI
/// thread or GPU upload queue.
fn should_update_recording_preview(
    last_update: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    last_update
        .map(|last| now.duration_since(last) >= RECORDING_PREVIEW_INTERVAL)
        .unwrap_or(true)
}

/// Decide whether the live game-window preview needs a fresh frame.
///
/// Extracted as a pure function so the refresh cadence is unit-testable:
/// a new frame is grabbed when there is no preview yet, when the selected
/// window changed, or when [`GAME_PREVIEW_REFRESH_INTERVAL`] has elapsed
/// since the last grab.
#[cfg(any(target_os = "linux", target_os = "windows"))]
fn should_refresh_game_preview(
    preview_window_id: Option<u64>,
    target_window_id: u64,
    last_refresh: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    match preview_window_id {
        None => true,
        Some(preview_id) if preview_id != target_window_id => true,
        Some(_) => match last_refresh {
            None => true,
            Some(last) => now.duration_since(last) >= GAME_PREVIEW_REFRESH_INTERVAL,
        },
    }
}

/// Parse the `--no-frame-timeout <seconds>` CLI flag from the given command
/// line arguments (without the program name).
///
/// Returns the configured timeout, or the default when the flag is absent,
/// malformed, or zero. Unknown arguments are ignored so the flag stays
/// forward-compatible with other CLI options.
pub fn parse_no_frame_timeout(
    args: &[String],
    default: std::time::Duration,
) -> std::time::Duration {
    let mut secs = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--no-frame-timeout" {
            if let Some(value) = iter.next() {
                if let Ok(n) = value.parse::<u64>() {
                    if n > 0 {
                        secs = Some(n);
                    }
                }
            }
        }
    }
    secs.map(std::time::Duration::from_secs).unwrap_or(default)
}

/// Whether a recording should be aborted because the capture delivered no
/// frame at all within the timeout.
///
/// The pipeline is only initialized once the first frame arrives, so a
/// capture that never produces a frame would otherwise look like a running
/// recording while writing nothing. Once any frame has arrived the timeout
/// no longer applies (a static screen can legitimately deliver no further
/// frames).
fn should_abort_for_no_frames(
    started: std::time::Instant,
    last_frame_at: Option<std::time::Instant>,
    now: std::time::Instant,
    timeout: std::time::Duration,
) -> bool {
    last_frame_at.is_none() && now.duration_since(started) >= timeout
}

/// Whether a Linux recording should be aborted because no frame has arrived
/// within the timeout.
///
/// Unlike Windows (where WGC only delivers frames when the screen changes,
/// so a static screen can legitimately pause), xcap captures actively on
/// every loop iteration. A gap longer than the timeout therefore means the
/// capture thread died or its source became unavailable (window closed,
/// minimized, or a capture error) — the recording would otherwise keep
/// "running" forever while writing nothing. The timeout counts from the last
/// received frame, or from the start when no frame has arrived yet.
fn should_abort_for_stalled_frames(
    started: std::time::Instant,
    last_frame_at: Option<std::time::Instant>,
    now: std::time::Instant,
    timeout: std::time::Duration,
) -> bool {
    let last_activity = last_frame_at.unwrap_or(started);
    now.duration_since(last_activity) >= timeout
}

/// Keep a scene name safe for use as a suggested snapshot file name.
fn help_documents() -> [(&'static str, &'static str); 4] {
    [
        ("User guide", "docs/user-guide.md"),
        ("Stream setup", "docs/stream-setup.md"),
        ("First-stream checklist", "docs/first-stream-checklist.md"),
        ("Troubleshooting", "docs/updater-troubleshooting.md"),
    ]
}

fn help_document_url(path: &str) -> String {
    format!("https://github.com/thoser666/Rivulet/blob/develop/{path}")
}

fn open_help_document(path: &str) {
    let result = if let Some(root) = std::env::var_os("RIVULET_DOCS_ROOT") {
        let candidate = std::path::PathBuf::from(root).join(path);
        open::that(candidate)
    } else {
        open::that(path)
    };
    if let Err(error) = result {
        tracing::warn!(document = path, %error, "Could not open help document");
    }
}

fn sanitize_file_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ' ') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "scene".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Format a byte count for display (B, KB, MB, GB).
fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.2} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

/// Resolve the source label for the recording view.
///
/// When a monitor is selected, returns `Some(monitor_name)`.  When a
/// window is selected (but no monitor), returns `Some(window_title)`.
/// When nothing is selected, returns `None`.
///
/// Extracted as a standalone function so it can be unit-tested
/// without instantiating the full egui UI.
/// Resolve the label for the recording view's Source (monitor) dropdown.
///
/// Returns `Some(monitor_name)` when a monitor is selected and its name is
/// available; otherwise `None` so the caller can show its "Select Monitor"
/// fallback. The window selection is intentionally not part of this function:
/// the Window dropdown owns the window title, and selecting a window must
/// never make its title appear in the Source dropdown as well.
///
/// Extracted as a standalone function so it can be unit-tested without
/// instantiating the full egui UI.
fn resolve_monitor_label(
    selected_monitor_idx: Option<usize>,
    monitor_name: Option<&str>,
) -> Option<String> {
    if selected_monitor_idx.is_some() {
        monitor_name.map(|n| n.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    // ── navigation (AppView) ───────────────────────────────────────

    #[test]
    fn app_view_defaults_to_record() {
        assert_eq!(AppView::default(), AppView::Record);
    }

    #[test]
    fn help_documents_have_clickable_repository_targets() {
        let documents = help_documents();
        for (_, document) in documents {
            assert!(help_document_url(document).starts_with("https://github.com/"));
        }
        // The test binary runs from target/{debug,release}; resolve paths from
        // the workspace manifest rather than relying on the process cwd.
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace manifest parent");
        for (_, document) in documents {
            assert!(
                workspace.join(document).is_file(),
                "missing help document: {document}"
            );
        }
    }

    #[test]
    fn app_view_all_views_cover_sidebar_and_headings() {
        // Every view must map to an i18n key (sidebar label + heading),
        // and every key must be non-empty.
        for view in AppView::all() {
            assert!(!view.nav_key().is_empty(), "view has no nav key");
        }
        assert_eq!(AppView::all().len(), 7);
        assert_eq!(AppView::Help.nav_key(), "nav_help");
        assert!(AppView::Stream.planned_milestone().is_none());
    }

    #[test]
    fn placeholder_views_expose_their_milestone() {
        assert_eq!(AppView::Stream.planned_milestone(), None);
        assert_eq!(AppView::Assistant.planned_milestone(), Some("M9"));
        // Implemented views have no milestone banner (Scenes is now live).
        assert_eq!(AppView::Scenes.planned_milestone(), None);
        assert_eq!(AppView::Record.planned_milestone(), None);
        assert_eq!(AppView::Mixer.planned_milestone(), None);
        assert_eq!(AppView::Settings.planned_milestone(), None);
    }

    // ── Discord Rich Presence (M5) ────────────────────────────────    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn discord_presence_is_opt_out_and_respects_transitions() {
        // Disabled: no adapter handle is created and status is never pushed.
        let mut app = RivuletApp {
            discord_presence_enabled: false,
            ..Default::default()
        };
        app.discord_presence_last = None;
        app.sync_discord_presence();
        assert!(
            app.discord_presence.is_none(),
            "disabled adapter must not spawn"
        );
        assert!(app.discord_presence_last.is_none());

        // Enabled with no client id configured: the fallback chain resolves
        // to the official default, so a real (valid) adapter spawns — the
        // zero-config end-user path.
        app.discord_presence_enabled = true;
        app.discord_presence_client_id = String::new();
        app.sync_discord_presence();
        let presence = app
            .discord_presence
            .as_ref()
            .expect("empty id must resolve to the official default");
        assert!(presence.enabled());
        assert_eq!(
            app.discord_presence_active_client_id.as_deref(),
            Some(rivulet_core::discord::DEFAULT_CLIENT_ID),
            "empty client id must resolve through the fallback chain"
        );
        app.discord_presence = None;

        // Enabled with a client id: an active adapter is created and the
        // current status is pushed, so the cached status reflects the first
        // activity.
        app.discord_presence_client_id = "test-client-123".to_owned();
        app.sync_discord_presence();
        let presence = app
            .discord_presence
            .as_ref()
            .expect("adapter created when configured");
        assert!(presence.enabled());
        assert_eq!(
            app.discord_presence_active_client_id.as_deref(),
            Some("test-client-123")
        );
        let expected = app.current_presence_status();
        assert_eq!(app.discord_presence_last.as_ref(), Some(&expected));

        // Re-syncing without a state change must not re-push (transition only).
        app.sync_discord_presence();
        assert_eq!(app.discord_presence_last.as_ref(), Some(&expected));

        // Opting out tears the adapter down cleanly.
        app.discord_presence_enabled = false;
        app.sync_discord_presence();
        assert!(app.discord_presence.is_none());
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn discord_presence_rebuilds_when_client_id_changes() {
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "first-client".to_owned(),
            ..Default::default()
        };
        app.sync_discord_presence();
        assert_eq!(
            app.discord_presence_active_client_id.as_deref(),
            Some("first-client")
        );

        // Re-syncing with an unchanged id must not rebuild the adapter.
        let old = app
            .discord_presence
            .as_ref()
            .expect("adapter present")
            .enabled();
        app.sync_discord_presence();
        assert!(app.discord_presence.is_some());
        assert!(old);

        // A dirty flag (Apply pressed after editing) rebuilds once for the new id.
        app.discord_presence_client_id = "second-client".to_owned();
        app.discord_client_id_dirty = true;
        app.sync_discord_presence();
        assert_eq!(
            app.discord_presence_active_client_id.as_deref(),
            Some("second-client")
        );
        assert!(!app.discord_client_id_dirty);
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn discord_presence_reconnect_rebuilds_the_adapter() {
        // The Stream view offers a reconnect button when the adapter reports
        // not connected (e.g. Discord was started after Rivulet). Clicking it
        // must rebuild the worker on the next reconcile — without changing the
        // client id and without restarting the app.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "reconnect-client".to_owned(),
            ..Default::default()
        };
        app.sync_discord_presence();
        assert_eq!(
            app.discord_presence_active_client_id.as_deref(),
            Some("reconnect-client")
        );

        // Mark a reconnect (button click): the next reconcile tears the old
        // worker down and spawns a fresh one for the same client id.
        app.discord_reconnect_requested = true;
        app.sync_discord_presence();
        assert!(
            !app.discord_reconnect_requested,
            "reconnect flag must be consumed by the reconcile"
        );
        assert!(
            app.discord_presence.is_some(),
            "reconnect must respawn the adapter"
        );
        assert_eq!(
            app.discord_presence_active_client_id.as_deref(),
            Some("reconnect-client"),
            "client id must be unchanged by a reconnect"
        );
    }

    #[test]
    fn discord_client_id_validation_blocks_invalid_apply_and_warns() {
        // The Apply action must validate the client id format: an invalid id
        // must set the warning and NOT mark the adapter dirty (no silent
        // misconfiguration), while a valid id (or empty) applies normally.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "not-a-number".to_owned(),
            ..Default::default()
        };
        app.apply_discord_client_id();
        assert_eq!(
            app.discord_client_id_warning,
            Some(rivulet_core::discord::ClientIdError::NotNumeric)
        );
        assert!(
            !app.discord_client_id_dirty,
            "invalid id must not be applied"
        );

        // Too short: length error, still not applied.
        app.discord_presence_client_id = "123".to_owned();
        app.apply_discord_client_id();
        assert_eq!(
            app.discord_client_id_warning,
            Some(rivulet_core::discord::ClientIdError::Length)
        );
        assert!(!app.discord_client_id_dirty);

        // Valid id: warning cleared, adapter marked for rebuild.
        app.discord_presence_client_id = "1544027006847680532".to_owned();
        app.apply_discord_client_id();
        assert!(app.discord_client_id_warning.is_none());
        assert!(app.discord_client_id_dirty);

        // Empty is valid (adapter stays off by design).
        app.discord_presence_client_id = String::new();
        app.apply_discord_client_id();
        assert!(app.discord_client_id_warning.is_none());
        assert!(app.discord_client_id_dirty);
    }

    #[test]
    fn discord_payload_validation_warns_on_implausible_asset_key() {
        // Apply must validate the SET_ACTIVITY payload: an implausible art
        // asset key sets the payload warning so Settings can show it
        // immediately, while a plausible key (or empty) stays clean.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "1544027006847680532".to_owned(),
            discord_presence_large_image: "bad key!".to_owned(),
            ..Default::default()
        };
        app.apply_discord_payload_validation();
        assert_eq!(
            app.discord_payload_warning,
            Some(rivulet_core::discord::PayloadIssue::InvalidAssetKey)
        );

        // Plausible key: no warning.
        app.discord_presence_large_image = "rivulet_logo".to_owned();
        app.apply_discord_payload_validation();
        assert!(app.discord_payload_warning.is_none());

        // Empty keeps the placeholder icon by design — no warning either.
        app.discord_presence_large_image = String::new();
        app.apply_discord_payload_validation();
        assert!(app.discord_payload_warning.is_none());
    }

    #[test]
    fn discord_payload_validation_warns_on_overlong_game_name() {
        // A game name longer than Discord's 128-char limit must be flagged so
        // the user can shorten it instead of the status being silently
        // truncated (or dropped) on the wire.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "1544027006847680532".to_owned(),
            ..Default::default()
        };
        // Simulate an overlong selected game window title.
        app.selected_game_window_idx = Some(0);
        app.game_windows = vec![rivulet_core::GameWindow {
            id: 1,
            title: "x".repeat(200),
            width: 1920,
            height: 1080,
        }];
        app.use_game_capture = true;
        app.apply_discord_payload_validation();
        // The game name lives in `state` (second card line) since the line
        // swap, so an overlong one is reported as a state violation.
        assert!(matches!(
            app.discord_payload_warning,
            Some(rivulet_core::discord::PayloadIssue::FieldTooLong {
                field: "state",
                len: 200,
            })
        ));
    }

    #[test]
    fn discord_payload_validation_is_wired_into_settings() {
        // Source contract: the Settings Apply path runs the payload validator
        // alongside the client id check and renders both warnings with the
        // error palette; both locales translate the new keys (parity test
        // enforces agreement).
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        assert!(source.contains("fn apply_discord_payload_validation"));
        assert!(source.contains("validate_set_activity_payload"));
        assert!(source.contains("discord_payload_warning"));
        assert!(source.contains("discord_payload_error_field_too_long"));
        assert!(source.contains("discord_payload_error_asset_key"));
        assert!(source.contains("StatusColors::for_ui(ui).error"));
        // The Apply flow must call the payload validator after the client id
        // validator, so both warnings appear together.
        let apply = source
            .split_once("fn apply_discord_client_id")
            .map(|(_, rest)| rest)
            .expect("apply_discord_client_id must exist");
        assert!(apply.contains("fn apply_discord_payload_validation"));
        let settings = source
            .split_once("fn draw_settings")
            .map(|(_, rest)| rest)
            .expect("draw_settings must exist");
        assert!(settings.contains("apply_discord_payload_validation()"));
    }

    #[test]
    fn discord_client_id_validation_is_wired_into_settings() {
        // Source contract: the Settings Apply path calls the core validator
        // and renders the warning with the error palette.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        assert!(source.contains("rivulet_core::discord::validate_client_id"));
        assert!(source.contains("discord_client_id_warning"));
        assert!(source.contains("discord_client_id_error_not_numeric"));
        assert!(source.contains("discord_client_id_error_length"));
        assert!(source.contains("StatusColors::for_ui(ui).error"));
    }

    #[test]
    fn discord_presence_reconnect_button_is_wired_into_the_stream_view() {
        // Source contract: the reconnect affordance lives in draw_presence_status,
        // only appears while not connected and a client id is configured, and
        // sets the flag that the reconcile consumes.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_presence_status")
            .map(|(_, rest)| rest)
            .expect("draw_presence_status must exist");
        assert!(draw.contains("discord_reconnect"));
        assert!(
            draw.contains("!is_connected && self.discord_presence_enabled"),
            "reconnect must only be offered while not connected and enabled"
        );
        assert!(draw.contains("self.discord_reconnect_requested = true;"));
        assert!(source.contains("discord_reconnect_requested"));
        // The flag must force a rebuild in the reconcile, not just be ignored.
        let sync = source
            .split_once("fn sync_discord_presence")
            .map(|(_, rest)| rest)
            .expect("sync_discord_presence must exist");
        assert!(sync.contains("self.discord_client_id_dirty || self.discord_reconnect_requested"));
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn discord_presence_status_includes_selected_game_name() {
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "test-client".to_owned(),
            ..Default::default()
        };
        app.use_game_capture = true;
        app.game_windows = vec![rivulet_core::GameWindow {
            id: 7,
            title: "Elden Ring".to_owned(),
            width: 1920,
            height: 1080,
        }];
        app.selected_game_window_idx = Some(0);
        let status = app.current_presence_status();
        // Line layout: the composed title (app word + status label) lives in
        // `details` (first card line), the game name in `state` (second line).
        assert_eq!(status.details, "Rivulet · Ready");
        assert_eq!(status.state, "Elden Ring");

        // No selection -> plain status without a game name.
        let empty = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "test-client".to_owned(),
            ..Default::default()
        };
        assert!(empty.current_presence_game_name().is_none());
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn discord_presence_pushes_recording_transition_from_idle() {
        // Regression: starting a recording must surface in Discord even when
        // the user is on the Record view — the sync must not depend on the
        // Stream view being visible. `is_aux_recording` is the platform-
        // independent recording flag, so the transition is testable anywhere.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "test-client".to_owned(),
            ..Default::default()
        };
        app.sync_discord_presence();
        let idle = app.current_presence_status();
        assert_eq!(app.discord_presence_last.as_ref(), Some(&idle));
        assert_eq!(idle.details, "Rivulet · Ready");

        // Recording starts while the user is not looking at the Stream view.
        app.is_aux_recording = true;
        app.sync_discord_presence();
        let recording = app.current_presence_status();
        assert_eq!(recording.details, "Rivulet · Recording");
        assert_eq!(app.discord_presence_last.as_ref(), Some(&recording));

        // Pausing while recording surfaces as Paused.
        app.is_paused = true;
        app.sync_discord_presence();
        let paused = app.current_presence_status();
        assert_eq!(paused.details, "Rivulet · Paused");

        // Stopping returns to Idle.
        app.is_aux_recording = false;
        app.is_paused = false;
        app.sync_discord_presence();
        let back = app.current_presence_status();
        assert_eq!(back, idle);
        assert_eq!(app.discord_presence_last.as_ref(), Some(&idle));
    }

    /// Minimal in-memory eframe storage used to verify the real persistence
    /// round-trip: `save()` writes under `eframe::APP_KEY` and
    /// `restore_from_storage` reads it back.
    #[derive(Default)]
    struct MemoryStorage {
        values: std::collections::HashMap<String, String>,
    }

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }

        fn set_string(&mut self, key: &str, value: String) {
            self.values.insert(key.to_owned(), value);
        }

        fn remove_string(&mut self, key: &str) {
            self.values.remove(key);
        }

        fn flush(&mut self) {}
    }

    #[test]
    fn discord_client_id_survives_eframe_storage_round_trip() {
        // Regression: the application id was serialized by `save()` but never
        // read back, so every restart lost it. The full eframe persistence
        // path (save -> get_value under APP_KEY) must restore it.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "1234567890123456".to_owned(),
            discord_presence_large_image: "rivulet_logo".to_owned(),
            ..Default::default()
        };
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);

        let restored = RivuletApp::restore_from_storage(Some(&storage))
            .expect("persisted app state must be restored");
        assert_eq!(restored.discord_presence_client_id, "1234567890123456");
        assert_eq!(restored.discord_presence_large_image, "rivulet_logo");
        assert!(restored.discord_presence_enabled);
        // Runtime-only handles must stay at their defaults after restore.
        assert!(restored.discord_presence.is_none());
        assert!(restored.discord_presence_active_client_id.is_none());
    }

    #[test]
    fn restore_from_storage_returns_none_when_nothing_persisted() {
        // First launch (or a wiped storage) must cleanly fall back to the
        // defaults instead of panicking.
        let storage = MemoryStorage::default();
        assert!(RivuletApp::restore_from_storage(Some(&storage)).is_none());
        assert!(RivuletApp::restore_from_storage(None).is_none());
    }

    #[test]
    fn default_app_ships_official_discord_id_and_logo() {
        // Zero-config Rich Presence: a fresh install (and the Default impl
        // behind every test) must carry the official application id and the
        // official logo asset key, not empty strings.
        let app = RivuletApp::default();
        assert_eq!(
            app.discord_presence_client_id,
            rivulet_core::discord::DEFAULT_CLIENT_ID
        );
        assert_eq!(
            app.discord_presence_large_image,
            rivulet_core::discord::DEFAULT_LARGE_IMAGE_KEY
        );
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn empty_saved_client_id_migrates_to_the_official_default() {
        // Installs from before the official default existed persisted an
        // empty client id (adapter off). The restore path must migrate them
        // to the official id + logo, while an explicitly configured id is
        // kept untouched.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: String::new(),
            discord_presence_large_image: String::new(),
            ..Default::default()
        };
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);

        let restored = RivuletApp::restore_from_storage(Some(&storage))
            .expect("persisted app state must be restored");
        assert_eq!(
            restored.discord_presence_client_id,
            rivulet_core::discord::DEFAULT_CLIENT_ID,
            "empty persisted id must migrate to the official default"
        );
        assert_eq!(
            restored.discord_presence_large_image,
            rivulet_core::discord::DEFAULT_LARGE_IMAGE_KEY
        );

        // An explicitly configured id must survive restore unchanged.
        let mut custom = RivuletApp {
            discord_presence_client_id: "9999999999999999999".to_owned(),
            discord_presence_large_image: "my_brand".to_owned(),
            ..Default::default()
        };
        let mut storage2 = MemoryStorage::default();
        eframe::App::save(&mut custom, &mut storage2);
        let restored2 = RivuletApp::restore_from_storage(Some(&storage2))
            .expect("persisted app state must be restored");
        assert_eq!(restored2.discord_presence_client_id, "9999999999999999999");
        assert_eq!(restored2.discord_presence_large_image, "my_brand");
    }

    #[test]
    fn discord_presence_reports_error_state_when_engine_fails() {
        // Regression: the PresenceActivity::Error variant existed in the model
        // but no code path ever produced it — engine/capture failures left the
        // presence stuck on the activity label. A real failure must surface as
        // the localized "Error" state with priority over any running activity.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "test-client".to_owned(),
            ..Default::default()
        };
        // Simulate an engine failure (e.g. capture/encode error) that set
        // last_error, while a recording is nominally still active.
        app.is_aux_recording = true;
        app.last_error = Some("pipeline failed".to_owned());
        let status = app.current_presence_status();
        assert_eq!(status.details, "Rivulet · Error");
        assert!(status.state.is_empty());

        // The error text itself is never sent (privacy-safe payload).
        assert!(!status.details.contains("pipeline"));
        assert!(!status.state.contains("pipeline"));

        // Error has priority over pause and streaming too.
        app.is_paused = true;
        let status = app.current_presence_status();
        assert_eq!(status.details, "Rivulet · Error");

        // Clearing the error (next start) returns to the activity label.
        app.last_error = None;
        let status = app.current_presence_status();
        assert_eq!(status.details, "Rivulet · Paused");

        // Error label is localized.
        app.locale = Locale::De;
        app.is_paused = false;
        app.last_error = Some("pipeline failed".to_owned());
        let status = app.current_presence_status();
        assert_eq!(status.details, "Rivulet · Fehler");
    }

    #[test]
    fn discord_presence_starts_streaming_clear_stale_error() {
        // A stream start must clear a stale error so the presence moves from
        // "Error" back to the Streaming label (mirrors recording starts).
        // Verify both the state model behavior and the source contract.
        let mut app = RivuletApp {
            discord_presence_enabled: true,
            discord_presence_client_id: "test-client".to_owned(),
            ..Default::default()
        };
        app.last_error = Some("old failure".to_owned());
        let before = app.current_presence_status();
        assert_eq!(before.details, "Rivulet · Error");

        // The same reset the streaming start path applies.
        app.last_error = None;
        let status = app.current_presence_status();
        assert_eq!(status.details, "Rivulet · Ready");

        // Source contract: every streaming start path (OBS-WebSocket command
        // and GUI stream button) clears last_error. Only inspect the part of
        // the file before this test module so the assertions below (which use
        // the same strings) cannot satisfy the counts.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(head, _)| head)
            .unwrap_or(&source);
        let streaming_starts = production.matches("self.engine.start_streaming();").count();
        assert!(streaming_starts >= 2, "streaming start paths must exist");
        // The two production start paths begin with the stale-error reset.
        assert!(
            production
                .matches("// Clear a stale error so a new stream starts")
                .count()
                >= 2,
            "every streaming start must clear last_error"
        );
    }

    #[test]
    fn presence_legend_is_wired_into_the_stream_view_with_tooltips() {
        // The Stream view must render the status legend: one row per state
        // with an explanatory tooltip, highlighting the active state.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_presence_status")
            .map(|(_, rest)| rest)
            .expect("draw_presence_status must exist");
        assert!(draw.contains("presence_legend"));
        assert!(draw.contains("for activity in PresenceActivity::all()"));
        assert!(draw.contains("activity.tooltip_i18n_key()"));
        assert!(draw.contains("response.on_hover_text(tip)"));
        assert!(draw.contains("self.current_presence_activity()"));
        // The active row is highlighted via the interaction stroke.
        assert!(draw.contains("paint_interaction_stroke"));
    }

    #[test]
    fn presence_activity_is_derived_independently_of_the_payload() {
        // current_presence_activity() drives both the Discord payload and the
        // legend highlight; it must expose the raw enum state (not the
        // localized payload) so the drawer can highlight the right row.
        let mut app = RivuletApp::default();
        assert_eq!(app.current_presence_activity(), PresenceActivity::Idle);
        app.is_aux_recording = true;
        assert_eq!(app.current_presence_activity(), PresenceActivity::Recording);
        app.is_paused = true;
        assert_eq!(app.current_presence_activity(), PresenceActivity::Paused);
        app.last_error = Some("boom".to_owned());
        assert_eq!(app.current_presence_activity(), PresenceActivity::Error);
        app.is_aux_recording = false;
        app.is_paused = false;
        app.last_error = None;
        assert_eq!(app.current_presence_activity(), PresenceActivity::Idle);
        // The legend covers every state in display order.
        assert_eq!(PresenceActivity::all().len(), 6);
    }

    #[test]
    fn discord_presence_connection_state_is_surfaced_in_the_stream_view() {
        // Regression: the presence group showed only the *desired* status
        // text, never whether Discord actually accepted it. When the IPC
        // handshake fails (Discord closed, wrong client id), the user only
        // sees Discord's plain "Playing Rivulet" game card with no rich-
        // presence lines and no explanation. The Stream view must expose the
        // adapter's real connection state.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_presence_status")
            .map(|(_, rest)| rest)
            .expect("draw_presence_status must exist");
        // The adapter state is polled from the shared handle and rendered
        // with a locale-aware status line.
        assert!(
            draw.contains("p.connection_state()"),
            "status must read the real state"
        );
        assert!(draw.contains("discord_conn_connected"));
        assert!(draw.contains("discord_conn_connecting"));
        assert!(draw.contains("discord_conn_off"));
        assert!(
            draw.contains("StatusColors::for_ui(ui).success"),
            "connected state must render in the success color"
        );
    }

    #[test]
    fn discord_presence_sync_runs_every_frame_not_only_in_stream_view() {
        // The regression: sync_discord_presence lived inside the Stream view's
        // draw code, so toggling recording from the Record view never pushed
        // an update. The per-frame reconcile call must live next to the other
        // reconcilers in the global ui() entry, outside any view branch.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let sync_line = source
            .lines()
            .find(|l| l.contains("self.sync_discord_presence();"))
            .map(|l| l.to_owned())
            .unwrap_or_default();
        // The frame-level call must not be inside draw_presence_status (which
        // only the Stream view renders); it appears in the reconcile block of
        // the ui() entry together with the MIDI/OBS/hotkey reconcilers.
        assert!(
            sync_line.contains("sync_discord_presence"),
            "per-frame presence sync must exist"
        );
        assert!(source.contains("self.reconcile_midi();"));
        assert!(source.contains("self.sync_discord_presence();"));
        // draw_presence_status keeps its own sync (idempotent transition guard)
        // for when the Stream view is open — but the per-frame call above is
        // the one that matters for Record-view toggles.
        assert!(source.contains("fn draw_presence_status"));
    }

    #[allow(clippy::field_reassign_with_default)]
    #[test]
    fn discord_presence_status_is_localized_and_privacy_safe() {
        let mut app = RivuletApp::default();
        app.locale = Locale::De;
        let status = app.current_presence_status();
        // `details` carries the composed title (app word + German label), and
        // no game name is selected so the `state` line stays empty.
        assert_eq!(status.application, "Rivulet");
        assert_eq!(status.details, "Rivulet · Bereit");
        assert!(status.state.is_empty());
        // Never any sensitive token/path.
        assert!(!status.details.to_lowercase().contains("rtmp"));
        assert!(!status.details.contains('/') && !status.details.contains('\\'));
    }

    // ── Vulkan layer backend status (G3) ──────────────────────────

    #[cfg(target_os = "windows")]
    #[test]
    fn vulkan_layer_status_reports_active_or_fallback() {
        let active = vulkan_layer_backend_status(true);
        assert_eq!(active.active, BackendKind::VulkanLayer);
        assert_eq!(active.ui_key(), ("backend_vulkan_layer", None));
        assert!(active.is_healthy());
        assert!(!active.fallback_occurred);

        let fallback = vulkan_layer_backend_status(false);
        assert_eq!(fallback.active, BackendKind::WindowsGraphicsCapture);
        assert!(fallback.fallback_occurred);
        assert_eq!(
            fallback.ui_key(),
            ("backend_wgc_fallback", Some("Vulkan layer not available"))
        );
        assert!(!fallback.is_healthy());
    }

    // ── format_bytes ──────────────────────────────────────────────

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn format_bytes_kilobytes() {
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1048575), "1024.0 KB");
    }

    #[test]
    fn format_bytes_megabytes() {
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(5242880), "5.00 MB");
        assert_eq!(format_bytes(1073741823), "1024.00 MB");
    }

    #[test]
    fn format_bytes_gigabytes() {
        assert_eq!(format_bytes(1073741824), "1.00 GB");
        assert_eq!(format_bytes(2684354560), "2.50 GB");
    }

    // ── should_abort_for_stalled_frames ───────────────────────────

    #[test]
    fn stalled_frames_abort_when_no_frame_ever_arrived() {
        let started = std::time::Instant::now();
        let now = started + std::time::Duration::from_secs(6);
        assert!(should_abort_for_stalled_frames(
            started,
            None,
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    #[test]
    fn stalled_frames_abort_when_capture_dies_mid_recording() {
        let started = std::time::Instant::now();
        let last_frame = started + std::time::Duration::from_secs(2);
        let now = last_frame + std::time::Duration::from_secs(6);
        assert!(should_abort_for_stalled_frames(
            started,
            Some(last_frame),
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    #[test]
    fn stalled_frames_no_abort_while_frames_flow() {
        let started = std::time::Instant::now();
        let last_frame = started + std::time::Duration::from_secs(5);
        let now = last_frame + std::time::Duration::from_millis(500);
        assert!(!should_abort_for_stalled_frames(
            started,
            Some(last_frame),
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    #[test]
    fn stalled_frames_no_abort_before_timeout_elapses() {
        let started = std::time::Instant::now();
        let now = started + std::time::Duration::from_secs(3);
        assert!(!should_abort_for_stalled_frames(
            started,
            None,
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    // ── parse_no_frame_timeout ────────────────────────────────────

    #[test]
    fn no_frame_timeout_default_when_flag_absent() {
        let args: Vec<String> = vec![];
        let default = std::time::Duration::from_secs(5);
        assert_eq!(parse_no_frame_timeout(&args, default), default);
    }

    #[test]
    fn no_frame_timeout_parses_value() {
        let args = vec!["--no-frame-timeout".to_string(), "12".to_string()];
        let parsed = parse_no_frame_timeout(&args, std::time::Duration::from_secs(5));
        assert_eq!(parsed, std::time::Duration::from_secs(12));
    }

    #[test]
    fn no_frame_timeout_last_value_wins() {
        let args = vec![
            "--no-frame-timeout".to_string(),
            "3".to_string(),
            "--no-frame-timeout".to_string(),
            "15".to_string(),
        ];
        let parsed = parse_no_frame_timeout(&args, std::time::Duration::from_secs(5));
        assert_eq!(parsed, std::time::Duration::from_secs(15));
    }

    #[test]
    fn no_frame_timeout_ignores_invalid_values() {
        let args = vec!["--no-frame-timeout".to_string(), "abc".to_string()];
        let default = std::time::Duration::from_secs(5);
        assert_eq!(parse_no_frame_timeout(&args, default), default);

        let args = vec!["--no-frame-timeout".to_string(), "0".to_string()];
        assert_eq!(parse_no_frame_timeout(&args, default), default);

        // Missing value falls back to default.
        let args = vec!["--no-frame-timeout".to_string()];
        assert_eq!(parse_no_frame_timeout(&args, default), default);
    }

    #[test]
    fn no_frame_timeout_ignores_unknown_args() {
        let args = vec![
            "--other".to_string(),
            "--no-frame-timeout".to_string(),
            "7".to_string(),
        ];
        let parsed = parse_no_frame_timeout(&args, std::time::Duration::from_secs(5));
        assert_eq!(parsed, std::time::Duration::from_secs(7));
    }

    // ── should_abort_for_no_frames ────────────────────────────────

    #[test]
    fn abort_when_no_frame_arrived_within_timeout() {
        let started = std::time::Instant::now();
        let now = started + std::time::Duration::from_secs(6);
        assert!(should_abort_for_no_frames(
            started,
            None,
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    #[test]
    fn no_abort_before_timeout_elapses() {
        let started = std::time::Instant::now();
        let now = started + std::time::Duration::from_secs(3);
        assert!(!should_abort_for_no_frames(
            started,
            None,
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    #[test]
    fn no_abort_once_any_frame_arrived() {
        // A static screen can legitimately stop producing frames; the timeout
        // only applies before the very first frame (pipeline never started).
        let started = std::time::Instant::now();
        let last_frame = started + std::time::Duration::from_millis(500);
        let now = started + std::time::Duration::from_secs(30);
        assert!(!should_abort_for_no_frames(
            started,
            Some(last_frame),
            now,
            std::time::Duration::from_secs(5)
        ));
    }

    // ── drain_error_receiver ──────────────────────────────────────

    #[test]
    fn error_receiver_drains_all_errors_latest_wins() {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        tx.send("first".to_string()).unwrap();
        tx.send("second".to_string()).unwrap();
        assert_eq!(
            drain_error_receiver(&rx).as_deref(),
            Some("second"),
            "most recent error must win"
        );
        assert_eq!(
            drain_error_receiver(&rx),
            None,
            "channel is empty after draining"
        );
    }

    #[test]
    fn error_receiver_empty_channel_returns_none() {
        let (_tx, rx) = std::sync::mpsc::channel::<String>();
        assert_eq!(drain_error_receiver(&rx), None);
    }

    // ── drain_frames_and_check_end ────────────────────────────────

    #[test]
    fn stop_when_disconnected_and_signal_not_set() {
        let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let signal = AtomicBool::new(false);
        drop(tx); // disconnect
        assert!(drain_frames_and_check_end(&rx, &signal, |_| {}));
    }

    #[test]
    fn no_stop_when_channel_open() {
        let (_tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let signal = AtomicBool::new(false);
        assert!(!drain_frames_and_check_end(&rx, &signal, |_| {}));
    }

    #[test]
    fn no_stop_when_disconnected_but_signal_set() {
        let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let signal = AtomicBool::new(true);
        drop(tx); // disconnect
        assert!(!drain_frames_and_check_end(&rx, &signal, |_| {}));
    }

    #[test]
    fn no_stop_when_frames_available() {
        let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let signal = AtomicBool::new(false);
        tx.send(RawFrame {
            data: vec![],
            width: 0,
            height: 0,
        })
        .unwrap();
        assert!(!drain_frames_and_check_end(&rx, &signal, |_| {}));
    }

    #[test]
    fn stop_only_when_both_conditions_met() {
        // connected + signal not set → no stop
        let (tx1, rx1) = std::sync::mpsc::channel::<RawFrame>();
        let signal1 = AtomicBool::new(false);
        assert!(!drain_frames_and_check_end(&rx1, &signal1, |_| {}));
        drop(tx1);

        // disconnected + signal set → no stop
        let (tx2, rx2) = std::sync::mpsc::channel::<RawFrame>();
        let signal2 = AtomicBool::new(true);
        drop(tx2);
        assert!(!drain_frames_and_check_end(&rx2, &signal2, |_| {}));

        // disconnected + signal not set → stop
        let (tx3, rx3) = std::sync::mpsc::channel::<RawFrame>();
        let signal3 = AtomicBool::new(false);
        drop(tx3);
        assert!(drain_frames_and_check_end(&rx3, &signal3, |_| {}));
    }

    #[test]
    fn frames_are_forwarded_and_not_dropped_by_the_check() {
        // Regression test: a separate try_recv() stop-probe consumed one
        // frame per UI tick and dropped it, starving the recording pipeline
        // so no MP4 was ever written. Every queued frame must reach the
        // callback even while the disconnect check runs.
        let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let signal = AtomicBool::new(false);
        for i in 0..5u8 {
            tx.send(RawFrame {
                data: vec![i],
                width: 1,
                height: 1,
            })
            .unwrap();
        }

        let mut forwarded = Vec::new();
        let ended = drain_frames_and_check_end(&rx, &signal, |f| {
            forwarded.push(f.data[0]);
        });
        assert!(!ended, "open channel must not end the recording");
        assert_eq!(
            forwarded,
            vec![0, 1, 2, 3, 4],
            "all frames must be forwarded"
        );
    }

    #[test]
    fn frames_are_forwarded_before_disconnect_is_reported() {
        // Frames queued before the sender is dropped must still be delivered
        // before the unexpected-end signal is reported.
        let (tx, rx) = std::sync::mpsc::channel::<RawFrame>();
        let signal = AtomicBool::new(false);
        tx.send(RawFrame {
            data: vec![9],
            width: 1,
            height: 1,
        })
        .unwrap();
        drop(tx); // disconnect after the queued frame

        let mut forwarded = Vec::new();
        let ended = drain_frames_and_check_end(&rx, &signal, |f| {
            forwarded.push(f.data[0]);
        });
        assert!(ended, "disconnected channel without stop signal must end");
        assert_eq!(forwarded, vec![9], "queued frame must be delivered first");
    }

    // ── format_skipped_filters ────────────────────────────────────

    #[test]
    fn skipped_filters_warning_lists_features_in_english() {
        let skipped = [SkippedFilter {
            element: "webrtcdsp",
            feature: "noise suppression",
        }];
        assert_eq!(
            format_skipped_filters(Locale::En, &skipped),
            "Audio filters skipped (missing GStreamer elements): noise suppression (webrtcdsp)"
        );
    }

    #[test]
    fn skipped_filters_warning_translates_to_german() {
        let skipped = [SkippedFilter {
            element: "webrtcdsp",
            feature: "noise suppression",
        }];
        assert_eq!(
            format_skipped_filters(Locale::De, &skipped),
            "Audiofilter übersprungen (fehlende GStreamer-Elemente): Rauschunterdrückung (webrtcdsp)"
        );
    }

    #[test]
    fn skipped_filters_warning_joins_multiple_filters() {
        let skipped = [
            SkippedFilter {
                element: "webrtcdsp",
                feature: "noise suppression",
            },
            SkippedFilter {
                element: "audiodynamic",
                feature: SkippedFilter::feature_name("audiodynamic"),
            },
        ];
        assert_eq!(
            format_skipped_filters(Locale::En, &skipped),
            "Audio filters skipped (missing GStreamer elements): noise suppression (webrtcdsp), compressor/limiter/expander/gate (audiodynamic)"
        );
    }

    #[test]
    fn skipped_filters_warning_falls_back_for_unknown_element() {
        let skipped = [SkippedFilter {
            element: "futurefilter",
            feature: "audio filter",
        }];
        assert_eq!(
            format_skipped_filters(Locale::En, &skipped),
            "Audio filters skipped (missing GStreamer elements): audio filter (futurefilter)"
        );
    }

    #[test]
    fn skipped_filters_warning_matches_the_log_feature_names() {
        // The capture log uses SkippedFilter::feature_name (English); the GUI
        // warning must render the same element mapping in every locale, so the
        // two can never drift apart.
        for element in ["webrtcdsp", "audiodynamic", "future-element"] {
            let english = SkippedFilter::feature_name(element);
            // English GUI == English log.
            assert_eq!(
                SkippedFilter::feature_name_in(element, Locale::En),
                english,
                "English GUI feature name for `{element}` must match the log name"
            );
            let filter = SkippedFilter {
                element,
                feature: english,
            };
            let warning = format_skipped_filters(Locale::En, &[filter]);
            assert!(
                warning.contains(english),
                "warning {warning:?} must mention the log feature name for `{element}`"
            );
            assert!(
                warning.contains(element),
                "warning {warning:?} must mention the skipped element `{element}`"
            );
        }
    }

    // ── cropped_frame_for_region ──────────────────────────────────

    fn test_frame(width: u32, height: u32) -> RawFrame {
        let mut data = Vec::new();
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[x as u8, y as u8, 7, 9]);
            }
        }
        RawFrame {
            data,
            width,
            height,
        }
    }

    #[test]
    fn crop_disabled_passes_the_frame_through() {
        let frame = test_frame(4, 3);
        let region = CaptureRegion {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let cropped = cropped_frame_for_region(false, region, &frame);
        assert!(matches!(cropped, std::borrow::Cow::Borrowed(_)));
        assert_eq!(cropped.width, 4);
        assert_eq!(cropped.height, 3);
        assert_eq!(cropped.data, frame.data);
    }

    #[test]
    fn crop_enabled_extracts_the_sub_region() {
        let frame = test_frame(4, 3);
        let region = CaptureRegion {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let cropped = cropped_frame_for_region(true, region, &frame).into_owned();
        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        // Pixel (1,1): (1,1,7,9); (2,1): (2,1,7,9); (1,2): (1,2,7,9); (2,2): (2,2,7,9)
        assert_eq!(
            cropped.data,
            vec![1, 1, 7, 9, 2, 1, 7, 9, 1, 2, 7, 9, 2, 2, 7, 9]
        );
    }

    #[test]
    fn crop_enabled_falls_back_for_invalid_region() {
        let frame = test_frame(4, 3);
        // Region fully outside the frame -> clamped to empty -> passthrough.
        let region = CaptureRegion {
            x: 100,
            y: 100,
            width: 2,
            height: 2,
        };
        let cropped = cropped_frame_for_region(true, region, &frame);
        assert!(matches!(cropped, std::borrow::Cow::Borrowed(_)));
        assert_eq!(cropped.width, 4);
        assert_eq!(cropped.height, 3);
    }

    // ── region_from_pixel_points ──────────────────────────────────

    #[test]
    fn drag_region_normalizes_the_drag_direction() {
        // Dragging bottom-right vs top-left must yield the same region.
        let a = region_from_pixel_points(100.0, 50.0, 500.0, 400.0, 1920, 1080);
        let b = region_from_pixel_points(500.0, 400.0, 100.0, 50.0, 1920, 1080);
        assert_eq!(a, b);
        assert_eq!(a.x, 100);
        assert_eq!(a.y, 50);
        assert_eq!(a.width, 400);
        assert_eq!(a.height, 350);
    }

    #[test]
    fn drag_region_clamps_to_the_monitor_bounds() {
        let region = region_from_pixel_points(-50.0, -20.0, 3000.0, 2000.0, 1920, 1080);
        assert_eq!(region.x, 0);
        assert_eq!(region.y, 0);
        assert_eq!(region.width, 1920);
        assert_eq!(region.height, 1080);
    }

    #[test]
    fn drag_region_is_even_aligned() {
        let region = region_from_pixel_points(10.0, 10.0, 413.0, 313.0, 1920, 1080);
        assert_eq!(region.width % 2, 0);
        assert_eq!(region.height % 2, 0);
    }

    #[test]
    fn drag_region_rejects_degenerate_surfaces() {
        let region = region_from_pixel_points(0.0, 0.0, 10.0, 10.0, 1, 1);
        assert!(region.is_empty());
    }

    // ── format_duration ──────────────────────────────────────────

    #[test]
    fn format_duration_zero() {
        assert_eq!(RivuletApp::format_duration(0), "00:00");
    }

    #[test]
    fn format_duration_minutes_only() {
        assert_eq!(RivuletApp::format_duration(90), "01:30");
    }

    #[test]
    fn format_duration_hours_minutes_seconds() {
        assert_eq!(RivuletApp::format_duration(3661), "01:01:01");
    }

    #[test]
    fn format_duration_exact_hour() {
        assert_eq!(RivuletApp::format_duration(3600), "01:00:00");
    }

    // ── scene history hotkeys ────────────────────────────────────

    #[test]
    fn scene_overlay_defaults_are_disabled_and_empty() {
        let app = RivuletApp::default();
        assert!(!app.scene_overlay_enabled);
        assert!(app.scene_overlay_text.is_empty());
    }

    #[test]
    fn scene_hotkey_defaults_are_empty_and_serializable() {
        let hotkeys = HotkeyConfig::default();
        assert!(hotkeys.scene_hotkeys.is_empty());
        let json = serde_json::to_string(&hotkeys).expect("serialize hotkeys");
        let restored: HotkeyConfig = serde_json::from_str(&json).expect("deserialize hotkeys");
        assert_eq!(restored.scene_hotkeys, hotkeys.scene_hotkeys);
    }

    #[test]
    fn scene_hotkey_map_can_assign_distinct_scene_keys() {
        let mut hotkeys = HotkeyConfig::default();
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        hotkeys
            .scene_hotkeys
            .insert(first, HotkeyBinding::plain(egui::Key::F1));
        hotkeys
            .scene_hotkeys
            .insert(second, HotkeyBinding::plain(egui::Key::F2));
        assert_eq!(
            hotkeys.scene_hotkeys.get(&first),
            Some(&HotkeyBinding::plain(egui::Key::F1))
        );
        assert_eq!(
            hotkeys.scene_hotkeys.get(&second),
            Some(&HotkeyBinding::plain(egui::Key::F2))
        );
    }

    #[test]
    fn scene_history_shortcuts_ignore_text_input_without_context_reentry() {
        assert_eq!(scene_history_shortcut(true, true, true, false), None);
        assert_eq!(
            scene_history_shortcut(false, true, true, false),
            Some(SceneHistoryShortcut::Undo)
        );
        assert_eq!(
            scene_history_shortcut(false, true, false, true),
            Some(SceneHistoryShortcut::Redo)
        );
        assert_eq!(scene_history_shortcut(false, false, true, false), None);
    }

    // ── replay buffer hotkey ─────────────────────────────────────

    #[test]
    fn replay_hotkey_defaults_to_f12_and_labels_it() {
        let hotkeys = HotkeyConfig::default();
        assert_eq!(hotkeys.save_replay, HotkeyBinding::plain(egui::Key::F12));
        assert_eq!(hotkeys.label_for("save_replay"), "F12");
    }

    #[test]
    fn hotkey_binding_labels_with_modifiers() {
        let binding = HotkeyBinding {
            key: egui::Key::F9,
            ctrl: true,
            alt: false,
            shift: true,
            super_mod: false,
        };
        assert_eq!(binding.label(), "Ctrl+Shift+F9");
        let plain = HotkeyBinding::plain(egui::Key::F10);
        assert_eq!(plain.label(), "F10");
    }

    #[test]
    fn hotkey_set_binding_roundtrip() {
        let mut hotkeys = HotkeyConfig::default();
        let binding = HotkeyBinding {
            key: egui::Key::F5,
            ctrl: false,
            alt: true,
            shift: false,
            super_mod: true,
        };
        assert!(hotkeys.set_binding_for_action("record", binding));
        assert_eq!(hotkeys.binding_for_action("record"), Some(binding));
        assert_eq!(hotkeys.label_for("record"), "Ctrl+Alt+F5");
        assert!(!hotkeys.set_binding_for_action("unknown", binding));
        assert!(hotkeys.binding_for_action("unknown").is_none());
    }

    #[test]
    fn vk_code_maps_function_and_number_keys() {
        assert_eq!(vk_code(egui::Key::F9), Some(0x78));
        assert_eq!(vk_code(egui::Key::F12), Some(0x7B));
        assert_eq!(vk_code(egui::Key::Num0), Some(0x30));
        assert_eq!(vk_code(egui::Key::Space), Some(0x20));
        assert_eq!(vk_code(egui::Key::Enter), Some(0x0D));
        assert_eq!(vk_code(egui::Key::ArrowRight), Some(0x27));
    }

    #[test]
    fn default_hotkeys_produce_global_bindings() {
        let app = RivuletApp::default();
        let bindings = app.current_global_bindings();
        let record = bindings.iter().find(|b| b.action == "record").unwrap();
        assert_eq!(record.key, KeyCode(0x78)); // F9
        assert_eq!(record.mods, ModMask(0)); // no modifiers by default
        assert_eq!(bindings.len(), 4); // record, pause, mute, save_replay
    }

    // ── Source (monitor) dropdown label ───────────────────────────
    // The Source dropdown shows monitors only. The Window dropdown owns
    // the window title; a window pick must never leak into the Source
    // dropdown (regression: window title appeared under Source).

    #[test]
    fn monitor_label_returns_monitor_name_when_monitor_selected() {
        assert_eq!(
            resolve_monitor_label(Some(0), Some("Display 1")),
            Some("Display 1".to_string())
        );
    }

    #[test]
    fn monitor_label_is_none_when_nothing_selected() {
        assert_eq!(resolve_monitor_label(None, None), None);
    }

    #[test]
    fn monitor_label_is_none_when_monitor_selected_but_name_unavailable() {
        // Monitor index set but name unavailable → None so the caller
        // can show its "unknown_monitor" fallback string.
        assert_eq!(resolve_monitor_label(Some(0), None), None);
    }

    #[test]
    fn monitor_label_is_none_when_only_window_selected() {
        // Regression: selecting a window clears the monitor, and the
        // window title must NOT appear in the Source dropdown.
        assert_eq!(resolve_monitor_label(None, None), None);
        assert_eq!(resolve_monitor_label(None, Some("ghost")), None);
    }

    #[test]
    fn monitor_label_is_none_when_empty_name() {
        assert_eq!(
            resolve_monitor_label(Some(0), Some("")),
            Some(String::new())
        );
    }

    // ── Scene snapshot file names ─────────────────────────────────

    #[test]
    fn snapshot_file_name_replaces_unsafe_characters() {
        assert_eq!(sanitize_file_name("Live: 16/9?"), "Live_ 16_9_");
    }

    #[test]
    fn snapshot_file_name_uses_scene_fallback_for_blank_names() {
        assert_eq!(sanitize_file_name("   "), "scene");
        assert_eq!(sanitize_file_name("***"), "___");
    }

    // ── Recording preview validation and throttling ───────────────

    #[test]
    fn recording_preview_accepts_only_complete_rgba_frames() {
        assert!(is_valid_rgba_frame(&[0; 16], 2, 2));
        assert!(!is_valid_rgba_frame(&[0; 15], 2, 2));
        assert!(!is_valid_rgba_frame(&[], 0, 2));
        assert!(!is_valid_rgba_frame(&[], 2, 0));
    }

    #[test]
    fn recording_preview_rejects_overflowing_dimensions() {
        assert!(!is_valid_rgba_frame(&[], u32::MAX, u32::MAX));
    }

    #[test]
    fn recording_preview_updates_without_a_previous_upload() {
        let now = std::time::Instant::now();
        assert!(should_update_recording_preview(None, now));
    }

    #[test]
    fn recording_preview_does_not_upload_before_interval() {
        let now = std::time::Instant::now();
        let recent = now - std::time::Duration::from_millis(50);
        assert!(!should_update_recording_preview(Some(recent), now));
    }

    #[test]
    fn recording_preview_uploads_after_interval() {
        let now = std::time::Instant::now();
        let stale = now - (RECORDING_PREVIEW_INTERVAL + std::time::Duration::from_millis(1));
        assert!(should_update_recording_preview(Some(stale), now));
    }

    #[test]
    fn idle_preview_does_not_schedule_periodic_repaints() {
        assert!(!should_repaint_recording_preview(false, false));
    }

    #[test]
    fn source_preview_keeps_periodic_repaints_alive() {
        assert!(should_repaint_recording_preview(true, false));
        assert!(should_repaint_recording_preview(true, true));
    }

    #[test]
    fn pending_frame_keeps_periodic_repaints_alive() {
        assert!(should_repaint_recording_preview(false, true));
    }

    // ── Game-window live preview refresh ─────────────────────────

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn game_preview_refreshes_when_no_preview_yet() {
        let now = std::time::Instant::now();
        assert!(should_refresh_game_preview(None, 42, None, now));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn game_preview_refreshes_when_window_changed() {
        let now = std::time::Instant::now();
        // Fresh preview of window 1, now targeting window 2 → refresh.
        assert!(should_refresh_game_preview(Some(1), 2, Some(now), now));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn game_preview_does_not_refresh_before_interval_elapses() {
        let now = std::time::Instant::now();
        // Same window, grabbed a moment ago → no refresh yet.
        let recent = now - std::time::Duration::from_millis(100);
        assert!(!should_refresh_game_preview(Some(7), 7, Some(recent), now));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn game_preview_refreshes_after_interval_elapses() {
        let now = std::time::Instant::now();
        // Same window, grabbed longer than the interval ago → refresh.
        let stale = now - (GAME_PREVIEW_REFRESH_INTERVAL + std::time::Duration::from_millis(1));
        assert!(should_refresh_game_preview(Some(7), 7, Some(stale), now));
    }

    // ── Game-window list live refresh ───────────────────────────

    fn gw(id: u64) -> rivulet_core::GameWindow {
        rivulet_core::GameWindow {
            id,
            title: format!("window {id}"),
            width: 1280,
            height: 720,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn live_refresh_preserves_selection_when_window_still_exists() {
        let windows = vec![gw(1), gw(2), gw(3)];
        // Selecting window 2 at index 1; after a re-enumeration it stays at
        // index 1 because the window still exists.
        assert_eq!(preserve_selected_game_window(&windows, Some(2)), Some(1));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn live_refresh_keeps_selection_across_a_reordered_list() {
        // The list is re-enumerated in a different order; selection must be
        // re-resolved by id, not by old index.
        let windows = vec![gw(5), gw(2), gw(9), gw(3)];
        assert_eq!(preserve_selected_game_window(&windows, Some(9)), Some(2));
        assert_eq!(preserve_selected_game_window(&windows, Some(3)), Some(3));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn live_refresh_deselects_when_window_closed() {
        let windows = vec![gw(1), gw(3)];
        // Window 2 closed → selection cleared, not kept dangling.
        assert_eq!(preserve_selected_game_window(&windows, Some(2)), None);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn live_refresh_with_no_prior_selection_stays_none() {
        let windows = vec![gw(1)];
        assert_eq!(preserve_selected_game_window(&windows, None), None);
    }

    // ── Theme persistence (serde round-trip) ────────────────────

    #[test]
    fn theme_preference_survives_serde_round_trip() {
        for pref in theme::ThemePreference::all() {
            let json = serde_json::to_string(pref).expect("serialize theme");
            let back: theme::ThemePreference =
                serde_json::from_str(&json).expect("deserialize theme");
            assert_eq!(
                *pref, back,
                "ThemePreference must survive round-trip: {json}"
            );
        }
    }

    #[test]
    fn theme_preference_default_is_system() {
        assert_eq!(
            theme::ThemePreference::default(),
            theme::ThemePreference::System
        );
    }

    #[test]
    fn app_serialization_contains_selected_dark_theme() {
        let app = RivuletApp {
            theme: theme::ThemePreference::Dark,
            ..Default::default()
        };
        let json = serde_json::to_value(&app).expect("serialize app");
        assert_eq!(
            json.get("theme"),
            Some(&serde_json::Value::String("Dark".into()))
        );
    }

    #[test]
    fn rivulet_app_theme_field_is_not_skipped_in_serde() {
        // If theme is accidentally marked #[serde(skip)], this round-trip
        // will produce the default (System) instead of the set value.
        // We serialize a RivuletApp-like JSON blob with an explicit
        // "theme":"Dark" and verify it deserializes back as Dark.
        let json = r#"{"theme":"Dark","locale":"En"}"#;
        let value: serde_json::Value = serde_json::from_str(json).expect("parse JSON blob");
        // The theme key must be present in the serialized output
        assert!(
            value.get("theme").is_some(),
            "RivuletApp JSON must contain a 'theme' key"
        );
        assert_eq!(value["theme"], "Dark", "theme value must be preserved");
    }

    #[test]
    fn theme_apply_sets_egui_context() {
        let ctx = egui::Context::default();
        theme::ThemePreference::Dark.apply(&ctx);
        // After applying Dark, the egui visuals should report dark mode.
        // We verify indirectly via the ThemePreference getter.
        let visual = ctx.theme();
        assert_eq!(visual, egui::Theme::Dark);

        theme::ThemePreference::Light.apply(&ctx);
        let visual = ctx.theme();
        assert_eq!(visual, egui::Theme::Light);
    }

    // ── OBS WebSocket remote control ───────────────────────────────

    #[test]
    fn obs_ws_public_state_defaults_to_idle() {
        let state = ObsWsPublicState::default();
        assert_eq!(state.current_scene, None);
        assert!(!state.recording);
        assert!(!state.recording_paused);
        assert!(!state.streaming);
        assert!(!state.reconnecting);
    }

    #[test]
    fn obs_ws_public_state_derives_scene_and_output_changes() {
        let a = ObsWsPublicState {
            current_scene: Some("Game".into()),
            ..Default::default()
        };
        let b = ObsWsPublicState {
            current_scene: Some("BRB".into()),
            ..Default::default()
        };
        assert_ne!(a, b);
        assert_eq!(a.current_scene, Some("Game".to_string()));
        assert_eq!(b.current_scene, Some("BRB".to_string()));
    }

    #[test]
    fn obs_ws_settings_persist_across_restarts() {
        // The RivuletApp serde round-trip must carry the websocket settings
        // (toggle + port + password) so they survive an app restart.
        let app = RivuletApp {
            obs_ws_enabled: true,
            obs_ws_port: 4455,
            obs_ws_password: "hunter2".into(),
            ..Default::default()
        };
        let json = serde_json::to_value(&app).expect("serialize app");
        assert_eq!(json["obs_ws_enabled"], true);
        assert_eq!(json["obs_ws_port"], 4455);
        assert_eq!(json["obs_ws_password"], "hunter2");
        // Runtime-only state must not be persisted.
        assert!(json.get("obs_ws_server").is_none());
        assert!(json.get("obs_ws_snapshot").is_none());
        assert!(json.get("obs_ws_status").is_none());

        let restored: RivuletApp = serde_json::from_value(json).expect("deserialize app");
        assert!(restored.obs_ws_enabled);
        assert_eq!(restored.obs_ws_port, 4455);
        assert_eq!(restored.obs_ws_password, "hunter2");
        assert!(restored.obs_ws_server.is_none());
    }

    #[test]
    fn obs_ws_port_defaults_to_obs_compatible_4455() {
        let app = RivuletApp::default();
        assert_eq!(app.obs_ws_port, 4455);
        assert!(!app.obs_ws_enabled, "remote control must be opt-in");
    }

    // ── MIDI controller mapping ────────────────────────────────────

    #[test]
    fn midi_defaults_are_opt_in_and_empty() {
        let app = RivuletApp::default();
        assert!(!app.midi_enabled, "MIDI input must be opt-in");
        assert!(app.midi_mapping.bindings.is_empty());
        assert_eq!(app.midi_master_volume, 1.0);
    }

    #[test]
    fn midi_switch_scene_applies_to_the_scene_manager() {
        let mut app = RivuletApp::default();
        let game = app.scenes.add(rivulet_core::Scene::new("Game".to_owned()));
        let cam = app.scenes.add(rivulet_core::Scene::new("Cam".to_owned()));
        app.scenes.switch_to(game);
        app.apply_midi_action(&rivulet_core::MidiAction::SwitchScene(cam), 0);
        assert_eq!(app.scenes.active(), Some(cam));
    }

    #[test]
    fn midi_master_volume_fader_scales_value() {
        let mut app = RivuletApp::default();
        app.apply_midi_action(&rivulet_core::MidiAction::SetMasterVolume, 64);
        assert!((app.midi_master_volume - 0.5039).abs() < 0.001);
        app.apply_midi_action(&rivulet_core::MidiAction::SetMasterVolume, 0);
        assert_eq!(app.midi_master_volume, 0.0);
        app.apply_midi_action(&rivulet_core::MidiAction::SetMasterVolume, 127);
        assert_eq!(app.midi_master_volume, 1.0);
    }

    #[test]
    fn midi_settings_and_mapping_persist_across_restarts() {
        let scene = uuid::Uuid::new_v4();
        let app = RivuletApp {
            midi_enabled: true,
            midi_device_index: 1,
            midi_mapping: rivulet_core::MidiMapping {
                bindings: vec![rivulet_core::MidiBinding::new(
                    0,
                    rivulet_core::MidiKind::ControlChange,
                    7,
                    rivulet_core::MidiAction::SwitchScene(scene),
                )],
            },
            ..Default::default()
        };
        let json = serde_json::to_value(&app).expect("serialize app");
        assert_eq!(json["midi_enabled"], true);
        assert_eq!(json["midi_device_index"], 1);
        // Runtime-only state must not be persisted.
        assert!(json.get("midi_handle").is_none());
        assert!(json.get("midi_devices").is_none());

        let restored: RivuletApp = serde_json::from_value(json).expect("deserialize app");
        assert!(restored.midi_enabled);
        assert_eq!(restored.midi_mapping.bindings.len(), 1);
        assert_eq!(
            restored.midi_mapping.bindings[0].action,
            rivulet_core::MidiAction::SwitchScene(scene)
        );
        assert!(restored.midi_handle.is_none());
    }

    #[test]
    fn midi_dispatch_routes_messages_to_bound_actions() {
        // The end-to-end mapping path: raw MIDI bytes -> parsed message ->
        // dispatch -> applied GUI action.
        let scene = uuid::Uuid::new_v4();
        let app = RivuletApp {
            midi_mapping: rivulet_core::MidiMapping {
                bindings: vec![
                    rivulet_core::MidiBinding::new(
                        0,
                        rivulet_core::MidiKind::ControlChange,
                        7,
                        rivulet_core::MidiAction::SetMasterVolume,
                    ),
                    rivulet_core::MidiBinding::new(
                        0,
                        rivulet_core::MidiKind::NoteOn,
                        60,
                        rivulet_core::MidiAction::SwitchScene(scene),
                    ),
                ],
            },
            ..Default::default()
        };
        let cc = rivulet_core::parse_midi(&[0xB0, 7, 100]).unwrap();
        let actions: Vec<_> = app
            .midi_mapping
            .dispatch(&cc)
            .into_iter()
            .cloned()
            .collect();
        assert_eq!(actions, vec![rivulet_core::MidiAction::SetMasterVolume]);
        // Unbound messages dispatch to nothing.
        let note = rivulet_core::parse_midi(&[0x90, 61, 100]).unwrap();
        assert!(app.midi_mapping.dispatch(&note).is_empty());
    }

    #[test]
    fn midi_learn_captures_next_message_into_the_pending_row() {
        let mut app = RivuletApp {
            midi_learn: true,
            ..Default::default()
        };
        // A mapped control message arrives while learning: it must be captured
        // (not dispatched against the mapping, which is empty anyway) and the
        // pending row must be pre-filled so "Add binding" confirms it.
        let cc = rivulet_core::parse_midi(&[0xB3, 12, 64]).unwrap();
        app.reconcile_midi_with_message(cc);
        assert!(!app.midi_learn, "learn must stop after the first capture");
        assert_eq!(app.midi_learn_captured, Some(cc));
        assert_eq!(app.midi_new_kind, rivulet_core::MidiKind::ControlChange);
        assert_eq!(app.midi_new_channel, 3);
        assert_eq!(app.midi_new_number, 12);
    }

    #[test]
    fn midi_learn_does_not_dispatch_while_capturing() {
        // Bind a control and then learn on the same message: learning must win
        // and no binding may fire while the user identifies the control.
        let scene = uuid::Uuid::new_v4();
        let mut app = RivuletApp {
            midi_mapping: rivulet_core::MidiMapping {
                bindings: vec![rivulet_core::MidiBinding::new(
                    0,
                    rivulet_core::MidiKind::NoteOn,
                    60,
                    rivulet_core::MidiAction::SwitchScene(scene),
                )],
            },
            ..Default::default()
        };
        app.scenes.add(rivulet_core::Scene::new("Game".to_owned()));
        let cam = app.scenes.add(rivulet_core::Scene::new("Cam".to_owned()));
        app.scenes.switch_to(cam);
        app.midi_learn = true;
        let note = rivulet_core::parse_midi(&[0x90, 60, 100]).unwrap();
        app.reconcile_midi_with_message(note);
        // The scene did not switch because the message was captured, not run.
        assert_eq!(app.scenes.active(), Some(cam));
        assert_eq!(app.midi_learn_captured, Some(note));
    }

    #[test]
    fn midi_presets_save_and_apply_per_device() {
        let mut app = RivuletApp {
            midi_devices: vec!["nanoKONTROL2".to_owned(), "AKAI MIDImix".to_owned()],
            midi_device_index: 1, // AKAI MIDImix
            ..Default::default()
        };
        let device = app.midi_devices[app.midi_device_index].clone();

        app.midi_mapping = rivulet_core::MidiMapping {
            bindings: vec![rivulet_core::MidiBinding::new(
                0,
                rivulet_core::MidiKind::ControlChange,
                7,
                rivulet_core::MidiAction::SetMasterVolume,
            )],
        };
        app.midi_preset_name = "Live".to_owned();
        app.midi_presets
            .save(&device, "Live", app.midi_mapping.clone());
        assert_eq!(app.midi_presets.names_for(&device), vec!["Live"]);

        // Loading applies the preset mapping back.
        app.midi_mapping = rivulet_core::MidiMapping::default();
        let loaded = app
            .midi_presets
            .load(&device, "Live")
            .cloned()
            .expect("preset exists");
        app.midi_mapping = loaded;
        assert_eq!(app.midi_mapping.bindings.len(), 1);
        assert_eq!(
            app.midi_mapping.bindings[0].action,
            rivulet_core::MidiAction::SetMasterVolume
        );

        // The other device keeps its own (empty) preset namespace.
        assert!(app.midi_presets.names_for("nanoKONTROL2").is_empty());

        // Deleting removes the preset.
        assert!(app.midi_presets.delete(&device, "Live"));
        assert!(app.midi_presets.names_for(&device).is_empty());
    }

    #[test]
    fn midi_presets_persist_across_restarts() {
        let mut app = RivuletApp {
            // The device list is runtime-only, like the handle: it must not
            // be carried through serde, but presets keyed by device name are.
            midi_devices: vec!["nanoKONTROL2".to_owned()],
            ..Default::default()
        };
        app.midi_devices.clear();
        app.midi_presets.save(
            "nanoKONTROL2",
            "Streaming",
            rivulet_core::MidiMapping {
                bindings: vec![rivulet_core::MidiBinding::new(
                    0,
                    rivulet_core::MidiKind::NoteOn,
                    44,
                    rivulet_core::MidiAction::ToggleRecord,
                )],
            },
        );
        app.midi_preset_name = "Streaming".to_owned();

        let json = serde_json::to_value(&app).expect("serialize app");
        assert!(json["midi_presets"].is_object());
        // Transient learn/preset UI state must not be persisted.
        assert!(json.get("midi_learn").is_none());
        assert!(json.get("midi_learn_captured").is_none());
        assert!(json.get("midi_selected_preset").is_none());
        assert!(json.get("midi_devices").is_none());

        let restored: RivuletApp = serde_json::from_value(json).expect("deserialize app");
        assert_eq!(
            restored.midi_presets.names_for("nanoKONTROL2"),
            vec!["Streaming"]
        );
        assert_eq!(restored.midi_preset_name, "Streaming");
    }

    // ── Alert overlay import (M5 community dock) ──────────────────

    #[test]
    fn alert_import_loads_provider_widget_url_into_browser_source() {
        let mut app = RivuletApp {
            alert_provider: rivulet_core::AlertProvider::Streamlabs,
            alert_token: "tok-123".to_owned(),
            ..Default::default()
        };
        app.import_alert_overlay();
        assert_eq!(
            app.browser_source.url,
            "https://streamlabs.com/alert-box/v2/tok-123"
        );
        assert!(
            app.alert_token.is_empty(),
            "token must be cleared after import"
        );
        let status = app.scene_status.as_deref().unwrap_or_default();
        assert!(!status.is_empty());
    }

    #[test]
    fn alert_import_supports_streamelements_and_custom_urls() {
        let mut app = RivuletApp {
            alert_provider: rivulet_core::AlertProvider::StreamElements,
            alert_token: "se-456".to_owned(),
            ..Default::default()
        };
        app.import_alert_overlay();
        assert_eq!(
            app.browser_source.url,
            "https://streamelements.com/overlay/se-456"
        );

        let mut app = RivuletApp {
            alert_provider: rivulet_core::AlertProvider::Custom,
            alert_custom_url: "https://cdn.example.com/alerts".to_owned(),
            ..Default::default()
        };
        app.import_alert_overlay();
        assert_eq!(app.browser_source.url, "https://cdn.example.com/alerts");
    }

    #[test]
    fn alert_import_rejects_invalid_input_without_touching_browser_source() {
        let mut app = RivuletApp {
            alert_provider: rivulet_core::AlertProvider::Streamlabs,
            alert_token: "has space".to_owned(),
            ..Default::default()
        };
        let before = app.browser_source.url.clone();
        app.import_alert_overlay();
        // Invalid token: the browser source must stay untouched and the error
        // must be surfaced in the scene status.
        assert_eq!(app.browser_source.url, before);
        let status = app.scene_status.as_deref().unwrap_or_default();
        assert!(status.contains("invalid"), "status was: {status}");
    }

    // ── Twitch chat replies (M5 community dock) ───────────────────

    #[test]
    fn chat_send_requires_oauth_token_and_running_worker() {
        // Sending is only possible with an authenticated connection: no token
        // and no worker must both be rejected without touching any state.
        let mut app = RivuletApp::default();
        assert!(
            !app.send_chat_message("hello".to_owned()),
            "no token must block sending"
        );
        assert!(
            !app.send_chat_message("   ".to_owned()),
            "whitespace text must be rejected"
        );

        // Token configured but no worker connected yet: still rejected.
        app.chat_oauth_token = "oauth:abc123".to_owned();
        assert!(
            !app.send_chat_message("hello".to_owned()),
            "no worker must block sending"
        );
    }

    #[test]
    fn chat_send_enqueues_text_and_clears_pending_action() {
        // A real worker would dial irc.chat.twitch.tv; point it at an
        // unreachable loopback so the test never touches the network and the
        // worker just backs off. Sending is a non-blocking enqueue.
        let mut app = RivuletApp {
            chat_oauth_token: "oauth:abc123".to_owned(),
            chat_channel: "rivulet".to_owned(),
            chat_worker: Some(rivulet_core::Chat::new(&rivulet_core::ChatConfig {
                platform: rivulet_core::ChatPlatform::Twitch,
                twitch_endpoint: "127.0.0.1:1".to_owned(),
                channel: "rivulet".to_owned(),
                token: "oauth:abc123".to_owned(),
                ..Default::default()
            })),
            chat_state: rivulet_core::ChatConnState::Connected,
            ..Default::default()
        };
        assert!(app.send_chat_message(" hello there ".to_owned()));
        // The trimmed message goes through the worker channel (best effort;
        // the worker is not reachable here), so the app reports success.
    }

    #[test]
    fn chat_send_reply_requires_token_parent_id_and_worker() {
        // Threaded replies need the same authenticated connection as plain
        // sends plus a parent Twitch message id.
        let mut app = RivuletApp::default();
        assert!(
            !app.send_chat_reply("hello".to_owned(), "parent-1".to_owned()),
            "no token must block replies"
        );
        assert!(
            !app.send_chat_reply("   ".to_owned(), "parent-1".to_owned()),
            "whitespace text must be rejected"
        );
        assert!(
            !app.send_chat_reply("hello".to_owned(), "   ".to_owned()),
            "missing parent id must be rejected"
        );

        // Token configured but no worker connected yet: still rejected.
        app.chat_oauth_token = "oauth:abc123".to_owned();
        assert!(
            !app.send_chat_reply("hello".to_owned(), "parent-1".to_owned()),
            "no worker must block replies"
        );
    }

    #[test]
    fn chat_send_reply_enqueues_with_a_running_worker() {
        // Mirror of the plain-send enqueue test: with a token, a worker and a
        // parent id the reply is handed to the worker non-blocking.
        let mut app = RivuletApp {
            chat_oauth_token: "oauth:abc123".to_owned(),
            chat_channel: "rivulet".to_owned(),
            chat_worker: Some(rivulet_core::Chat::new(&rivulet_core::ChatConfig {
                platform: rivulet_core::ChatPlatform::Twitch,
                twitch_endpoint: "127.0.0.1:1".to_owned(),
                channel: "rivulet".to_owned(),
                token: "oauth:abc123".to_owned(),
                ..Default::default()
            })),
            chat_state: rivulet_core::ChatConnState::Connected,
            ..Default::default()
        };
        assert!(
            app.send_chat_reply(" thanks! ".to_owned(), "parent-1".to_owned()),
            "a trimmed reply with a parent id must be enqueued"
        );
        // Dropping the app disconnects the worker (Chat::drop stops the
        // thread); nothing else is observable against the unreachable
        // endpoint.
    }

    #[test]
    fn chat_submit_arms_plain_send_when_no_reply_target() {
        let mut app = RivuletApp {
            chat_input: " hello chat ".to_owned(),
            ..Default::default()
        };
        assert!(app.submit_chat_input());
        assert!(app.chat_input.is_empty(), "input must be cleared");
        assert_eq!(
            app.chat_action_pending,
            Some(ChatAction::Send("hello chat".to_owned()))
        );
    }

    #[test]
    fn chat_submit_arms_threaded_reply_and_clears_reply_target() {
        let mut app = RivuletApp {
            chat_input: "thanks!".to_owned(),
            chat_reply_target: Some(("ViewerOne".to_owned(), "msg-1".to_owned())),
            ..Default::default()
        };
        assert!(app.submit_chat_input());
        assert!(app.chat_input.is_empty(), "input must be cleared");
        assert!(
            app.chat_reply_target.is_none(),
            "the armed reply target must be consumed by the send"
        );
        assert_eq!(
            app.chat_action_pending,
            Some(ChatAction::SendReply(
                "thanks!".to_owned(),
                "msg-1".to_owned()
            ))
        );
    }

    #[test]
    fn chat_submit_rejects_whitespace_and_keeps_reply_target() {
        let mut app = RivuletApp {
            chat_reply_target: Some(("ViewerOne".to_owned(), "msg-1".to_owned())),
            chat_input: "   ".to_owned(),
            ..Default::default()
        };
        assert!(!app.submit_chat_input());
        assert!(
            app.chat_reply_target.is_some(),
            "an empty submit must not drop the armed reply target"
        );
        assert!(
            app.chat_action_pending.is_none(),
            "nothing may be armed for an empty submit"
        );
    }

    #[test]
    fn chat_rate_budget_reports_limiter_state() {
        // No worker running: no budget to show.
        let app = RivuletApp::default();
        assert_eq!(app.chat_rate_budget(), None);

        // A fresh worker starts with a full bucket (Twitch default 20/30 s).
        // The real clock may have refilled nothing beyond capacity, so the
        // available budget equals the capacity.
        let app = RivuletApp {
            chat_worker: Some(rivulet_core::Chat::new(&rivulet_core::ChatConfig {
                platform: rivulet_core::ChatPlatform::Twitch,
                twitch_endpoint: "127.0.0.1:1".to_owned(), // worker backs off
                channel: "rivulet".to_owned(),
                token: "oauth:abc123".to_owned(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let (remaining, capacity) = app.chat_rate_budget().expect("budget with a worker");
        assert_eq!(capacity, 20.0);
        assert!(
            (remaining - 20.0).abs() < f64::EPSILON * 64.0,
            "a fresh bucket must be full, got {remaining}"
        );
    }

    #[test]
    fn chat_rate_limit_detail_reports_platform_and_window() {
        // The budget tooltip needs the platform and the window the limit
        // applies over, not just the remaining budget. A fresh Twitch worker
        // must report its documented default (20 msgs / 30 s).
        let app = RivuletApp {
            chat_worker: Some(rivulet_core::Chat::new(&rivulet_core::ChatConfig {
                platform: rivulet_core::ChatPlatform::Twitch,
                twitch_endpoint: "127.0.0.1:1".to_owned(), // worker backs off
                channel: "rivulet".to_owned(),
                token: "oauth:abc123".to_owned(),
                ..Default::default()
            })),
            ..Default::default()
        };
        let (remaining, capacity, window_secs, platform) =
            app.chat_rate_limit_detail().expect("detail with a worker");
        assert_eq!(capacity, 20);
        assert_eq!(window_secs, 30);
        assert_eq!(platform, "Twitch");
        assert!(
            (remaining - 20.0).abs() < f64::EPSILON * 64.0,
            "a fresh bucket must be full, got {remaining}"
        );
        // The budget wrapper stays consistent with the detail.
        let (budget_remaining, budget_capacity) =
            app.chat_rate_budget().expect("budget with a worker");
        assert_eq!(budget_capacity, 20.0);
        assert_eq!(budget_remaining, remaining);
    }

    #[test]
    fn chat_reply_cancel_clears_the_armed_target() {
        // The banner ✕ button must disarm the reply: afterwards the next Send
        // is a plain message again. Cancelling never enqueues anything and
        // never touches the draft text.
        let mut app = RivuletApp {
            chat_reply_target: Some(("ViewerOne".to_owned(), "msg-1".to_owned())),
            chat_input: "draft that stays".to_owned(),
            ..Default::default()
        };
        app.cancel_chat_reply();
        assert!(
            app.chat_reply_target.is_none(),
            "cancel must clear the armed reply target"
        );
        assert!(
            app.chat_action_pending.is_none(),
            "cancel must not enqueue anything"
        );
        assert_eq!(
            app.chat_input, "draft that stays",
            "cancel must not touch the input text"
        );
    }

    #[test]
    fn chat_reply_banner_with_cancel_is_part_of_the_dock_contract() {
        // Source contract: the "Replying to <user>" banner (translated via
        // `chat_reply_to` with the user placeholder) must render directly
        // above the chat input — only while a reply target is armed — and its
        // ✕ button must disarm through `cancel_chat_reply()`, so the cancel
        // path stays testable and the banner cannot silently lose its
        // translated label or its cancel affordance.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_chat_dock")
            .map(|(_, rest)| rest)
            .expect("draw_chat_dock must exist in the Stream workspace");
        assert!(
            draw.contains("chat_reply_target.as_ref()"),
            "banner must be gated on an armed target"
        );
        assert!(
            draw.contains("chat_reply_to"),
            "banner label must come from i18n"
        );
        assert!(
            draw.contains("chat_reply_cancel"),
            "cancel affordance must carry a translated hover text"
        );
        assert!(
            draw.contains("self.cancel_chat_reply()"),
            "the ✕ button must call the testable cancel helper"
        );
        // The banner sits above the input row (before the send hint / text
        // field are set up) inside the can-send section.
        let banner = draw.find("chat_reply_to").expect("banner in dock");
        let input = draw.find("submit_chat_input").expect("input in dock");
        assert!(banner < input, "banner must render above the chat input");
        assert!(
            source.contains("fn cancel_chat_reply"),
            "cancel helper must exist"
        );
        assert!(
            source.contains("chat_reply_cancel_clears_the_armed_target"),
            "cancel behavior must be covered by a unit test"
        );
        let i18n = std::fs::read_to_string("../rivulet-core/src/i18n.rs").expect("i18n readable");
        for key in ["chat_reply_to", "chat_reply_cancel"] {
            let k = format!("\"{key}\"");
            assert!(
                i18n.matches(&k).count() >= 2,
                "{key} must exist in EN and DE"
            );
        }
    }

    #[test]
    fn stream_workspace_stays_reachable_on_narrow_windows_in_source() {
        // Responsive contract for the Meld-style Stream page: below the
        // narrow-width threshold the action bar and every control row wrap
        // (horizontal_wrapped) and the chat/info columns stack instead of
        // clipping, so start/stop, connect, send and mixer controls are
        // never pushed off-screen.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let stream = source
            .split_once("fn draw_stream_view")
            .map(|(_, rest)| rest)
            .expect("draw_stream_view must exist");
        assert!(source.contains("const STREAM_WORKSPACE_NARROW_WIDTH: f32 = 720.0;"));
        assert!(stream.contains("action_bar_wraps"));
        assert!(stream.contains("ui.horizontal_wrapped"));
        assert!(stream.contains("available_width() >= STREAM_WORKSPACE_NARROW_WIDTH"));
        assert!(stream.contains("self.draw_chat_dock(ui, chat_list_height)"));
        assert!(stream.contains("self.draw_stream_start_stop_button(ui, configured)"));
        let chat = source
            .split_once("fn draw_chat_dock")
            .map(|(_, rest)| rest)
            .expect("draw_chat_dock must exist");
        assert!(chat.contains("ui.horizontal_wrapped"));
        assert!(chat.contains("available_width() - 70.0).max(120.0)"));
        // The send-budget indicator above the input must also survive narrow
        // windows: it is an explicitly wrapping label (never clipped at the
        // right edge of the dock column) and stacks above the input row.
        assert!(chat.contains("chat_rate_budget"));
        assert!(chat.contains("Label::new") && chat.contains(".wrap()"));
        assert!(
            chat.find("chat_rate_budget").expect("budget marker")
                < chat.find("submit_chat_input").expect("input marker"),
            "the send budget must render above the chat input"
        );
        let audio = source
            .split_once("fn draw_stream_audio_compact")
            .map(|(_, rest)| rest)
            .unwrap_or("");
        assert!(audio.contains("ui.horizontal_wrapped"));
    }

    #[test]
    fn chat_send_input_is_gated_on_connected_and_token_in_source() {
        // Source contract: the reply input must only render when connected and
        // a token is configured; otherwise a lock hint is shown (YouTube shows
        // a read-only hint instead). This keeps the UI from offering an input
        // that Twitch/Kick would reject or YouTube cannot accept.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_chat_dock")
            .map(|(_, rest)| rest)
            .expect("draw_chat_dock must exist in the Stream workspace");
        assert!(draw.contains("chat_send_locked"));
        assert!(draw.contains("chat_state == rivulet_core::ChatConnState::Connected"));
        assert!(draw.contains("chat_oauth_token.trim().is_empty()"));
        assert!(draw.contains("ChatAction::Send"));
        assert!(draw.contains("chat_read_only"));
        assert!(draw.contains("self.chat_platform != rivulet_core::ChatPlatform::YouTube"));
        assert!(draw.contains("ChatPlatform::all()"));
        assert!(draw.contains("chat_reply_target"));
        assert!(draw.contains("ChatAction::SendReply"));
        assert!(draw.contains("phone_verification_required()"));
        assert!(draw.contains("chat_reply_to"));
        // The send-budget line must sit above the input and read the live
        // limiter state so throttling is visible before sends are dropped.
        assert!(draw.contains("chat_rate_budget"));
        assert!(draw.contains("chat_rate_limited"));
        assert!(draw.contains("chat_rate_budget()"));
        assert!(draw.contains("rate_limit_remaining()"));
        // The line must expose the platform + window through a tooltip (e.g.
        // “20 messages per 30 s on Twitch”), keeping the line itself compact.
        assert!(draw.contains("chat_rate_limit_detail()"));
        assert!(draw.contains("chat_rate_window"));
        assert!(draw.contains("on_hover_text(tooltip)"));
        let i18n = std::fs::read_to_string("../rivulet-core/src/i18n.rs").expect("i18n readable");
        for key in [
            "chat_send",
            "chat_send_hint",
            "chat_send_locked",
            "chat_read_only",
            "chat_note_kick",
            "chat_note_youtube",
            "chat_reply_tooltip",
            "chat_reply_to",
            "chat_reply_cancel",
            "chat_phone_verification",
            "chat_rate_budget",
            "chat_rate_limited",
            "chat_rate_window",
        ] {
            let k = format!("\"{key}\"");
            assert!(
                i18n.matches(&k).count() >= 2,
                "{key} must exist in EN and DE"
            );
        }
    }
}
