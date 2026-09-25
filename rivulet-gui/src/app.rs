#![allow(unused_imports, dead_code, unused_variables)]

use crate::midi_io::{list_devices, MidiListener};
use crate::theme;
use eframe::egui;
use rivulet_core::{
    AudioFilterConfig, AudioRouting, AudioSource, CaptureRegion, DiscordPresence,
    DiscordPresenceConfig, GlobalBinding, GlobalHotkey, KeyCode, Locale, ModMask, PresenceActivity,
    PresenceStatus, RivuletEngine, SkippedFilter, StreamConnectionResult, StreamHealthStatus,
    StreamPlatform, StreamPreset, StreamProbeResult, StreamSettings, StreamStats, VodTrack,
};
use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::Instant;

// --- macOS imports (xcap screen/window capture + cpal audio for recording) ---
#[cfg(target_os = "macos")]
use {
    rivulet_audio::{AudioCapture, AudioConfig},
    rivulet_core::{AudioFrame, AudioTrack},
    std::sync::mpsc as std_mpsc,
    std::thread,
};

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

// --- Per-app audio capture (Windows WASAPI loopback / Linux PipeWire) ---
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
use rivulet_audio::{AppAudioCapture, AppAudioProcess};

// --- Device endpoints (issue #229 WASAPI / issue #231 PipeWire) ---
#[cfg(any(target_os = "windows", target_os = "linux"))]
use rivulet_audio::{list_audio_devices, AudioDeviceCapture};

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

/// How often the WASAPI device list behind the Mixer's Input/Output source
/// picker re-enumerates while the picker is in use (issue #229 follow-up):
/// plain polling, so a newly plugged endpoint appears without a restart.
const AUDIO_DEVICES_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How often the per-app process list behind the Mixer's Application source
/// picker re-enumerates while the picker is in use (#154 follow-up): plain
/// polling with the same cadence as the device picker, so a newly started
/// process appears without a manual refresh.
const APP_AUDIO_PROCESSES_REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Maximum UI update rate for the recording thumbnail. The capture pipeline
/// remains at its configured frame rate; only texture uploads are throttled.
const RECORDING_PREVIEW_INTERVAL: std::time::Duration = std::time::Duration::from_millis(200);

/// How long the aggregated global issue (sidebar footer, ui-007) stays
/// visible after it first appeared (renewed when a different issue takes
/// over). Long enough to be noticed, short enough to not become wallpaper.
const GLOBAL_ISSUE_TTL: std::time::Duration = std::time::Duration::from_secs(30);

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
/// Alerts dock: the live alert list is bounded separately from the chat list
/// so a raid/donation burst can never evict chat history (and vice versa).
const MAX_ALERT_EVENTS: usize = 200;

/// Minimum workspace width (px) at which the Stream page keeps its
/// side-by-side layout. Below this the action bar, the chat dock and the
/// audio section switch to wrapped/stacked layouts so no control (start/
/// stop, connect, send, mixer) is clipped off-screen.
const STREAM_WORKSPACE_NARROW_WIDTH: f32 = 720.0;
/// Below this width the Stream workspace stacks the chat / alerts / info
/// columns vertically instead of a 3-column row (same responsive contract
/// as the chat dock's narrow threshold).
const STREAM_WORKSPACE_ALERTS_WIDTH: f32 = 1080.0;

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

/// Severity of an aggregated view-local status shown in the sidebar footer
/// (audit finding ui-007). Ordering drives the pick: `Error` beats `Warning`
/// beats `Info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GlobalIssueKind {
    Info,
    Warning,
    Error,
}

impl GlobalIssueKind {
    /// i18n key of the severity prefix shown before the message.
    fn label_key(self) -> &'static str {
        match self {
            GlobalIssueKind::Info => "global_issue_info",
            GlobalIssueKind::Warning => "global_issue_warning",
            GlobalIssueKind::Error => "global_issue_error",
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
    /// Threaded reply `(text, parent Twitch message id, parent platform)`,
    /// sent via `@reply-parent-msg-id` on the parent's platform.
    SendReply(String, String, rivulet_core::ChatPlatform),
}

/// Review state of a discovered plugin, shown in the Plugins settings list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PluginReviewState {
    /// Every requested capability decided and the plugin enabled.
    Enabled,
    /// Every requested capability decided, but the plugin disabled.
    Disabled,
    /// At least one requested capability is unreviewed.
    Pending,
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

/// Serde fallback for hotkey configs saved before `delete_source` existed:
/// old files must migrate to the OBS-parity `Delete` default instead of the
/// generic placeholder binding.
fn default_delete_binding() -> HotkeyBinding {
    HotkeyBinding::plain(egui::Key::Delete)
}

/// Keyboard shortcut sequence definition (serde-friendly; each enum is matched
/// by name in tests).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct HotkeyConfig {
    record: HotkeyBinding,
    pause: HotkeyBinding,
    mute: HotkeyBinding,
    save_replay: HotkeyBinding,
    #[serde(default = "default_delete_binding")]
    delete_source: HotkeyBinding,
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
            delete_source: HotkeyBinding::plain(egui::Key::Delete),
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
        egui::Key::Delete => "Delete",
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
        Delete => 0x2E,
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
            "delete_source" => Some(self.delete_source),
            _ => None,
        }
    }

    fn set_binding_for_action(&mut self, action: &str, binding: HotkeyBinding) -> bool {
        match action {
            "record" => self.record = binding,
            "pause" => self.pause = binding,
            "mute" => self.mute = binding,
            "save_replay" => self.save_replay = binding,
            "delete_source" => self.delete_source = binding,
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

/// A destructive action that is staged until the user confirms it in the
/// modal lives in `draw_confirmation_modal`. The payload carries everything
/// needed to execute the action (and to render the confirmation message), so
/// the modal can be drawn without mutating app state, and tests drive the
/// same request → confirm → execute path used by the GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingConfirmation {
    DeleteCompositionSource {
        scene_id: uuid::Uuid,
        source_id: uuid::Uuid,
        name: String,
    },
    RemoveAudioSource {
        id: uuid::Uuid,
        name: String,
    },
    RemoveChatAccount {
        index: usize,
        platform: rivulet_core::ChatPlatform,
        channel: String,
    },
}

impl PendingConfirmation {
    fn title_key(&self) -> &'static str {
        match self {
            PendingConfirmation::DeleteCompositionSource { .. } => {
                "confirm_dialog_delete_source_title"
            }
            PendingConfirmation::RemoveAudioSource { .. } => "confirm_dialog_remove_audio_title",
            PendingConfirmation::RemoveChatAccount { .. } => "confirm_dialog_remove_chat_title",
        }
    }

    fn message_key(&self) -> &'static str {
        match self {
            PendingConfirmation::DeleteCompositionSource { .. } => {
                "confirm_dialog_delete_source_message"
            }
            PendingConfirmation::RemoveAudioSource { .. } => "confirm_dialog_remove_audio_message",
            PendingConfirmation::RemoveChatAccount { .. } => "confirm_dialog_remove_chat_message",
        }
    }

    fn confirm_key(&self) -> &'static str {
        match self {
            PendingConfirmation::DeleteCompositionSource { .. } => {
                "confirm_dialog_delete_source_confirm"
            }
            PendingConfirmation::RemoveAudioSource { .. } => "confirm_dialog_remove_audio_confirm",
            PendingConfirmation::RemoveChatAccount { .. } => "confirm_dialog_remove_chat_confirm",
        }
    }

    fn subject(&self) -> String {
        match self {
            PendingConfirmation::DeleteCompositionSource { name, .. }
            | PendingConfirmation::RemoveAudioSource { name, .. } => name.clone(),
            PendingConfirmation::RemoveChatAccount {
                platform, channel, ..
            } => format!("{} · {}", platform.label(), channel),
        }
    }
}

/// Result of the confirmation dialog, returned by its content closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfirmationChoice {
    Pending,
    Confirm,
    Cancel,
}

#[cfg(target_os = "linux")]
enum BackendMessage {
    Stream(Stream, Fd),
    Recording(Fd),
    Done,
    Error(anyhow::Error),
}

/// A restream target configuration persisted in the GUI state.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct RestreamTargetConfig {
    pub name: String,
    pub platform: StreamPlatform,
    pub ingest_url: String,
    pub stream_key: String,
    pub enabled: bool,
}

impl Default for RestreamTargetConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            platform: StreamPlatform::Twitch,
            ingest_url: String::new(),
            stream_key: String::new(),
            enabled: true,
        }
    }
}

impl RestreamTargetConfig {
    /// Convert to a [`rivulet_core::StreamTarget`] for engine consumption.
    pub fn to_stream_target(&self) -> Option<rivulet_core::StreamTarget> {
        if !self.enabled || self.stream_key.trim().is_empty() {
            return None;
        }
        let url = if self.ingest_url.trim().is_empty() {
            self.platform
                .default_ingest_url()
                .unwrap_or("rtmps://live.twitch.tv/app")
                .to_owned()
        } else {
            self.ingest_url.clone()
        };
        Some(rivulet_core::StreamTarget::new(
            &self.name,
            rivulet_core::StreamSettings::new(self.platform, url, &self.stream_key),
        ))
    }
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

    /// User motion preference (WCAG 2.3.3 Animation from Interactions,
    /// ui-005). Persisted across sessions; `System` follows the OS
    /// reduce-motion setting. Persisted across sessions.
    motion_preference: theme::MotionPreference,
    /// Reduce-motion state currently applied to the egui context (`None`
    /// until first applied). Not persisted.
    #[serde(skip)]
    motion_applied: Option<bool>,
    /// Cached result of the OS reduce-motion probe (refreshed at most every
    /// [`OS_MOTION_PROBE_INTERVAL`]; the platform queries spawn a subprocess
    /// on macOS/Linux and must never run per frame). Not persisted.
    #[serde(skip)]
    os_reduced_motion: Option<bool>,
    /// When the OS reduce-motion probe last ran. Not persisted.
    #[serde(skip)]
    os_reduced_motion_probed_at: Option<Instant>,

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

    /// Telemetry: opt-in toggle. Persisted. Off by default — usage events are
    /// never captured (and never sent by the shipped build, which wires no
    /// transport sink) unless the user opts in.
    telemetry_enabled: bool,
    /// Alerts: master switch for the local chat-dock ingestion queue. Persisted
    /// (on by default — the queue is purely local and never transmits).
    alert_ingest_enabled: bool,
    /// Telemetry: runtime collector (bounded queue + optional sink). Not
    /// persisted; recreated empty and disabled on restore.
    #[serde(skip)]
    telemetry: rivulet_core::TelemetryReporter,
    /// Whether the once-per-session Startup event was already reported. Not
    /// persisted; re-reported after every restart while opted in.
    #[serde(skip)]
    telemetry_startup_reported: bool,

    /// Chat dock: configured accounts (one per platform). Persisted; the
    /// combined dock connects one worker per configured account.
    #[serde(default)]
    chat_accounts: Vec<rivulet_core::ChatAccount>,
    /// Legacy single-platform fields (configs saved before the combined
    /// dock). Kept so old persisted state deserializes; `restore_from_storage`
    /// migrates them into `chat_accounts`, after which they stay empty.
    #[serde(default)]
    chat_platform: rivulet_core::ChatPlatform,
    #[serde(default)]
    chat_channel: String,
    #[serde(default)]
    chat_oauth_token: String,
    /// Chat dock: platform selected in the "add account" row. Persisted so
    /// the row reopens with the last used platform.
    #[serde(default)]
    chat_add_platform: rivulet_core::ChatPlatform,
    /// Chat dock: draft fields of the "add account" row. Not persisted.
    #[serde(skip)]
    chat_add_channel: String,
    #[serde(skip)]
    chat_add_token: String,
    /// Chat dock: validation error of the last "add account" attempt
    /// (empty channel or duplicate platform). Cleared on a successful add.
    /// Not persisted.
    #[serde(skip)]
    chat_add_error: Option<String>,
    /// Chat dock: per-platform outcome of the last broadcast send (false =
    /// rejected by that platform: read-only, no token or rate-limited).
    /// Shown until the next send. Not persisted.
    #[serde(skip)]
    chat_last_send_outcomes: Vec<(rivulet_core::ChatPlatform, bool)>,
    /// Chat dock: pending message text to send on Enter. Not persisted.
    #[serde(skip)]
    chat_input: String,
    /// Chat dock: threaded reply target `(display name, platform, message
    /// id)`. Armed by the reply affordance on a message; cleared on send,
    /// cancel, connect and disconnect. Not persisted. Only Twitch messages
    /// carry an IRC id, so the send path rejects the other platforms.
    #[serde(skip)]
    chat_reply_target: Option<(String, rivulet_core::ChatPlatform, String)>,
    /// Chat dock: running multi-platform worker handle. Not persisted.
    #[serde(skip)]
    chat_worker_multi: Option<rivulet_core::MultiChat>,
    /// Chat dock: messages collected from the worker since connect. Not
    /// persisted. Bounded to the newest entries (oldest dropped).
    #[serde(skip)]
    chat_messages: Vec<rivulet_core::ChatMessage>,
    /// Chat dock: pending connect/disconnect action (processed once per
    /// frame). Not persisted.
    #[serde(skip)]
    chat_action_pending: Option<ChatAction>,

    /// Chat dock: stream-info editor — per-platform title drafts, indexed
    /// by [`rivulet_core::InfoPlatform::index`]. Not persisted (pure input
    /// state, no reason to survive a restart).
    #[serde(skip)]
    chat_info_title: [String; 3],
    /// Chat dock: stream-info editor — per-platform game/category drafts,
    /// indexed the same way. Not persisted.
    #[serde(skip)]
    chat_info_game: [String; 3],
    /// Chat dock: stream-info editor — which platform's drafts are edited
    /// (`None` = the shared "all platforms" drafts, the pre-#214 behavior).
    /// Not persisted.
    #[serde(skip)]
    chat_info_scope: Option<rivulet_core::InfoPlatform>,
    /// Chat dock: stream-info editor — in-flight update marker. Blocks a
    /// second apply while the background thread is running. Not persisted.
    #[serde(skip)]
    chat_info_busy: bool,
    /// Chat dock: stream-info editor — receiver for the finished batch of
    /// per-platform outcomes. Drained in `reconcile_chat`. Not persisted.
    #[serde(skip)]
    chat_info_rx: Option<
        std::sync::mpsc::Receiver<
            Vec<(rivulet_core::InfoPlatform, rivulet_core::InfoUpdateOutcome)>,
        >,
    >,
    /// Shared Chat room-name cache: resolves the numeric `source-room-id` of
    /// duplicated Twitch messages to channel logins via a background Helix
    /// lookup (`via <login>` badge). Never persisted, never logs
    /// credentials; see `rivulet_core::SharedRoomNameService`. Not persisted.
    #[serde(skip)]
    chat_room_names: rivulet_core::SharedRoomNameService,
    /// In-flight room-name lookup: dispatches one batched Helix request on a
    /// background thread. Not persisted.
    #[serde(skip)]
    chat_room_name_rx: Option<std::sync::mpsc::Receiver<Vec<(String, String)>>>,
    /// Chat dock: stream-info editor — last per-platform outcomes (empty
    /// until the first apply). Shown until the next apply. Not persisted.
    #[serde(skip)]
    chat_info_outcomes: Vec<(rivulet_core::InfoPlatform, rivulet_core::InfoUpdateOutcome)>,
    /// Chat dock: stream-info editor — validation error (e.g. nothing to
    /// apply). Cleared on the next apply attempt. Not persisted.
    #[serde(skip)]
    chat_info_error: Option<String>,

    /// Plugins: user approvals / enable decisions per plugin id (persisted
    /// inside the eframe storage with the rest of the app state). See
    /// [`rivulet_core::plugin_registry`] for the store semantics and the
    /// plugin-system RFC for the approval flow.
    plugin_approvals: rivulet_core::PluginApprovals,
    /// Plugins: bundles discovered by the last scan of the install root.
    /// Runtime-only; refreshed on app start and on manual rescan.
    #[serde(skip)]
    plugins_discovered: Vec<rivulet_core::DiscoveredPlugin>,
    /// Plugins: bundle folders that failed to parse (id or manifest broken).
    /// Runtime-only.
    #[serde(skip)]
    plugins_broken: Vec<String>,
    /// Plugins: plugin id currently open in the permission-review dialog.
    #[serde(skip)]
    plugin_review_open: Option<String>,
    /// Plugins: working copy of per-capability review decisions in the open
    /// dialog (capability → approved). Applied to the store on "Done".
    #[serde(skip)]
    plugin_review_draft: BTreeMap<String, bool>,
    /// Plugins: last load/start error for display in the list.
    #[serde(skip)]
    plugin_load_error: Option<String>,
    /// Chat dock: last connection state shown to the user. Not persisted.
    #[serde(skip)]
    chat_state: rivulet_core::ChatConnState,

    /// Alerts: bounded local ingestion queue drained into the chat dock.
    /// Not persisted (ephemeral, mirrors the chat-message list). `#[serde(skip)]`
    /// so a session restore can never replay or log alert entries.
    #[serde(skip)]
    alert_ingest: rivulet_core::AlertIngest,
    /// Alerts: generated sample entries to demo the chat-dock surfacing.
    /// Not persisted.
    #[serde(skip)]
    alert_preview_dirty: bool,
    /// Alerts dock: bounded live list of drained alert events, newest last
    /// (rendered bottom-anchored like the chat dock). Every ingestion source
    /// (loopback webhook receiver, outbound EventSub, preview) lands here in
    /// the same localized rendering as the chat-dock action lines, so all
    /// platforms appear merged in one list. Not persisted — `#[serde(skip)]`
    /// so a session restore can never replay or log alert entries.
    #[serde(skip)]
    alert_events: Vec<rivulet_core::ChatMessage>,
    /// Alerts: local webhook receiver enabled (`127.0.0.1` only). Persisted,
    /// off by default — the loopback receiver is the optional delivery half of
    /// the honest ingestion story and stays off unless the streamer enables it.
    alerts_receiver_enabled: bool,
    /// Alerts: loopback port for the webhook receiver. Persisted; the Settings
    /// UI constrains it to 1..=65535, tests may use ephemeral port 0.
    alerts_receiver_port: u16,
    /// Alerts: Twitch EventSub secret used to verify webhook signatures.
    /// Persisted (masked in the UI like the chat token), never logged and
    /// never embedded in any event. Empty disables the EventSub route.
    alerts_twitch_secret: String,
    /// Alerts: running loopback receiver handle. Not persisted.
    #[serde(skip)]
    alerts_receiver: Option<rivulet_core::AlertsReceiver>,
    /// Alerts: settings the running receiver was started with (so a changed
    /// port or secret restarts it once per frame, no more). Not persisted.
    #[serde(skip)]
    alerts_receiver_applied: rivulet_core::AlertsReceiverConfig,
    /// Alerts: receiver bind/IO error shown in Settings. Not persisted.
    #[serde(skip)]
    alerts_receiver_error: Option<String>,
    /// Alerts: Twitch EventSub WebSocket transport enabled (outbound
    /// `wss://` — no forwarder or shared secret needed). Persisted, off by
    /// default: the streamer opts into contacting Twitch.
    alerts_eventsub_enabled: bool,
    /// Alerts: Twitch application client ID for EventSub subscription
    /// creation. Persisted, masked like the chat token, never logged.
    alerts_eventsub_client_id: String,
    /// Alerts: Twitch user access token (scopes `moderator:read:followers`,
    /// `channel:read:subscriptions`). Persisted, masked, never logged and
    /// never embedded in any event. Empty disables the transport.
    alerts_eventsub_token: String,
    /// Alerts: target broadcaster numeric user ID. Persisted.
    alerts_eventsub_broadcaster_id: String,
    /// Alerts: running EventSub worker handle. Not persisted.
    #[serde(skip)]
    alerts_eventsub: Option<rivulet_core::EventsubReceiver>,
    /// Alerts: settings the running EventSub worker was started with (so a
    /// changed client ID, token or broadcaster restarts it once per frame).
    /// Not persisted; the token stays inside and is never Debug-printed.
    #[serde(skip)]
    alerts_eventsub_applied: rivulet_core::EventsubWsConfig,
    /// Alerts: Twitch EventSub raid alert direction — raid out, raided, or
    /// both. Persisted; changing it restarts the worker (see
    /// `apply_alerts_eventsub`).
    alerts_raid_direction: rivulet_core::RaidAlertDirection,
    /// Alerts: EventSub worker/IO error shown in Settings. Not persisted.
    #[serde(skip)]
    alerts_eventsub_error: Option<String>,
    /// Alerts: EventSub WebSocket endpoint override (Twitch default when
    /// empty; only tests point it at a local puppet). Not persisted.
    #[serde(skip)]
    alerts_eventsub_ws_endpoint: String,
    /// Alerts: Twitch Helix API base override (default when empty; only tests
    /// point it at a local stub). Not persisted.
    #[serde(skip)]
    alerts_eventsub_api_base: String,

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

    // --- Mobile & HTTP remote companion (M6) ---
    /// Master switch for the companion HTTP page server (phone/browser
    /// remote control). Persisted across sessions.
    remote_companion_enabled: bool,
    /// HTTP port for the companion page. Persisted across sessions.
    remote_companion_port: u16,
    /// Bind the companion page AND the obs-websocket server to `0.0.0.0` so a
    /// phone on the same network can reach them. Requires an obs-websocket
    /// password. Persisted across sessions.
    remote_companion_bind_lan: bool,
    /// Explicit permission gate: stream start/stop/toggle from beyond
    /// loopback requires this to be enabled (enforced by the obs server).
    /// Persisted across sessions.
    remote_allow_stream_control: bool,
    /// Running companion server. Rebuilt whenever it is enabled. Not
    /// persisted.
    #[serde(skip)]
    remote_companion_server: Option<rivulet_obs_websocket::CompanionServerHandle>,
    /// Status/error line shown in Settings (e.g. bind failure, page URL).
    #[serde(skip)]
    remote_companion_status: Option<String>,
    /// URL used by the "Open page" button (kept so the handler and the status
    /// line agree on what is served). Not persisted, rebuilt on start.
    #[serde(skip)]
    remote_companion_url: Option<String>,

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
    /// Transient feedback under the record controls (recording platforms), e.g.
    /// "Finalizing recording…" while the background stop teardown runs and
    /// "Recording saved." once it finished. Runtime-only, never persisted.
    #[serde(skip)]
    record_status: Option<String>,
    /// Completion handle of an in-flight background stop (see
    /// `begin_background_stop`/`poll_stop_finalization`): while `Some`, the UI
    /// shows the "finalizing" status; the receiver fires once the background
    /// teardown (EOS finalization, auto-remux, cloud upload) has finished.
    #[serde(skip)]
    stop_finalizing: Option<Receiver<()>>,

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

    // macOS Fields (xcap screen/window capture; cpal audio capture, M5
    // Windows/macOS feature parity)
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    is_recording: bool,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    audio: Option<AudioCapture>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    audio_preview: bool,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    audio_warning: Option<String>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    audio_system_rx: Option<std_mpsc::Receiver<AudioFrame>>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    audio_mic_rx: Option<std_mpsc::Receiver<AudioFrame>>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    monitors: Vec<xcap::Monitor>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    selected_monitor_idx: Option<usize>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    windows: Vec<xcap::Window>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    selected_window_idx: Option<usize>,
    #[cfg(target_os = "macos")]
    #[serde(skip)]
    raw_rx: Option<std_mpsc::Receiver<RawFrame>>,

    /// Live WASAPI process-loopback captures for Application-kind sources
    /// (Phase 3, Windows). One capture per routed Application source with a
    /// resolved `pid:<n>` device id; frames are pushed into the engine's
    /// routed appsrcs every UI tick.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[serde(skip)]
    app_audio_captures: Vec<(uuid::Uuid, rivulet_audio::AppAudioCapture)>,
    /// Frame channels for the live per-app captures; drained on the UI tick.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[serde(skip)]
    app_audio_frame_receivers: Vec<(
        uuid::Uuid,
        std::sync::mpsc::Receiver<rivulet_core::AudioFrame>,
    )>,
    /// Cache of the process list for the Application-source picker, refreshed
    /// when the picker is opened (a ToolHelp snapshot on every frame would be
    /// wasteful).
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[serde(skip)]
    app_audio_processes: Option<Vec<rivulet_audio::AppAudioProcess>>,
    /// When the process list was last enumerated — drives the picker's
    /// bounded auto-refresh (same pattern as `audio_device_list_last_refresh`).
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[serde(skip)]
    app_audio_processes_last_refresh: Option<std::time::Instant>,
    /// pid selection for the source being added (Application kind).
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[serde(skip)]
    audio_mixer_new_source_pid: Option<u32>,
    /// Cache of the device list for the device picker (issue #229 WASAPI
    /// endpoints / issue #231 PipeWire nodes), refreshed when the picker is
    /// opened or refreshed (an enumeration on every frame would be wasteful).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[serde(skip)]
    audio_device_list: Option<Vec<rivulet_audio::AudioDeviceInfo>>,
    /// When the device list was last enumerated — drives the picker's
    /// bounded auto-refresh (same pattern as `game_windows_last_refresh`).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[serde(skip)]
    audio_device_list_last_refresh: Option<std::time::Instant>,
    /// Device-id selection for the source being added (Input/Output kind).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[serde(skip)]
    audio_mixer_new_source_device_id: Option<String>,
    /// Live device captures (issue #229 / #231): one thread per routed
    /// device source, keyed by source id and device id so a re-target
    /// restarts the capture; dropped when the source disappears or is
    /// unrouted.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[serde(skip)]
    device_audio_captures: Vec<(uuid::Uuid, String, AudioDeviceCapture)>,
    /// Frame channels for the live device captures; drained on the UI tick.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[serde(skip)]
    device_audio_frame_receivers: Vec<(
        uuid::Uuid,
        std::sync::mpsc::Receiver<rivulet_core::AudioFrame>,
    )>,

    #[serde(skip)]
    error_receiver: Option<Receiver<String>>,
    #[serde(skip)]
    stop_signal: Option<Arc<AtomicBool>>,
    #[serde(skip)]
    last_error: Option<String>,
    #[serde(skip)]
    /// Timestamp of the last non-global status change (drives the expiry of
    /// the aggregated sidebar issue — ui-007).
    global_issue_stamp: Option<Instant>,
    #[serde(skip)]
    /// View whose issue is currently surfaced in the sidebar footer (set
    /// alongside [`Self::global_issue_stamp`], used by the click-to-navigate
    /// handler).
    global_issue_view: Option<AppView>,
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
    /// A destructive action that awaits explicit user confirmation before it
    /// runs (see [`PendingConfirmation`] and `draw_confirmation_modal`).
    /// Runtime-only, never persisted.
    #[serde(skip)]
    pending_confirmation: Option<PendingConfirmation>,
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
    /// Live native webview backend driven by `browser_source` (issue #227).
    /// The wry adapter only exists on Windows; on other platforms this stays
    /// `None` and the panel shows the pending preview. Kept platform-neutral
    /// so GUI sync tests can inject the synthetic reference backend.
    #[serde(skip)]
    browser_backend: Option<
        Box<dyn rivulet_core::BrowserSourceBackend<Error = rivulet_core::BrowserSourceError>>,
    >,
    /// Set once spawning the native backend failed, so the tick does not
    /// retry (and spam status lines) on every frame.
    #[serde(skip)]
    browser_backend_failed: bool,
    /// Tracks which `browser_source` settings were already pushed to the
    /// backend, so the tick only forwards diffs.
    #[serde(skip)]
    browser_applied: Option<rivulet_core::BrowserAppliedState>,
    /// Uploaded preview of the latest [`rivulet_core::BrowserFrame`].
    #[serde(skip)]
    browser_preview_texture: Option<egui::TextureHandle>,
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
    /// Scene-item clipboard for copy/paste (issue #192). Holds the copied
    /// source+binding pair verbatim; paste is deterministic and
    /// undoable via the SourceManager paste stack.
    #[serde(skip)]
    scene_item_clipboard: Option<rivulet_core::SceneItemClipboard>,
    /// Index into the device list shown by the scene dialog's device picker
    /// for capture sources. `None` = use the renderer default.
    #[serde(skip)]
    selected_scene_device_idx: Option<usize>,
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
    /// VOD track selection (issue #78): `enabled` adds the copyright-safe
    /// third audio branch to the dual-output recording, `recorded` keeps the
    /// leakage-safety contract visible and editable.
    #[serde(skip)]
    stream_vod_enabled: bool,
    #[serde(skip)]
    stream_vod_recorded: bool,
    #[serde(skip)]
    stream_status_message: Option<String>,
    #[serde(skip)]
    stream_probe_running: bool,
    #[serde(skip)]
    stream_probe_result: Option<StreamProbeResult>,
    #[serde(skip)]
    private_test_stream: rivulet_core::PrivateTestStream,

    // Restream (M6) — additional multistream targets beyond the primary.
    #[serde(default)]
    restream_targets: Vec<RestreamTargetConfig>,
    #[serde(skip)]
    restream_add_requested: bool,
    #[serde(skip)]
    restream_status: Option<String>,

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

    // Auto-clip (M6): chat-driven replay saves on spike / !clip command.
    #[serde(default)]
    auto_clip_config: rivulet_core::AutoClipConfig,
    #[serde(skip)]
    auto_clip_detector: rivulet_core::SpikeDetector,
    #[serde(skip)]
    auto_clip_status: Option<String>,

    // Multi-track audio routing (issue #154 Phase 2). The sources are the
    // single source of truth: the engine is synced from them before every
    // session start and on every mixer edit. Empty = legacy System/Mic.
    audio_sources: Vec<AudioSource>,
    /// Filter panel: the id of the source whose filter chain is being edited.
    #[serde(skip)]
    audio_mixer_filter_source: Option<uuid::Uuid>,
    #[serde(skip)]
    audio_mixer_new_source_name: String,
    #[serde(skip)]
    audio_mixer_new_source_kind: usize,
    /// Set by the mixer edits; `sync_audio_routing` clears it after pushing
    /// the list into the engine.
    #[serde(skip)]
    audio_mixer_needs_sync: bool,

    // NDI (LAN) monitor feed: when enabled, every recording/streaming session
    // additionally publishes the encoded H.264 video as an NDI source (M5
    // #77). Applied to the engine before every session start.
    ndi_output_enabled: bool,
    /// NDI source name announced on the LAN.
    ndi_output_name: String,
    /// Optional NDI group filter (e.g. VLAN workflows). Empty = no group.
    ndi_output_group: String,
    /// Transient, localized configuration warning (e.g. empty source name).
    #[serde(skip)]
    ndi_warning: Option<String>,

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
    muted: bool,
    replay_buffer_active: bool,
    studio_mode: bool,
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
            motion_preference: theme::MotionPreference::default(),
            motion_applied: None,
            os_reduced_motion: None,
            os_reduced_motion_probed_at: None,
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
            telemetry_enabled: false,
            alert_ingest_enabled: true,
            telemetry: rivulet_core::TelemetryReporter::default(),
            telemetry_startup_reported: false,
            chat_accounts: Vec::new(),
            chat_platform: rivulet_core::ChatPlatform::default(),
            chat_channel: String::new(),
            chat_oauth_token: String::new(),
            chat_add_platform: rivulet_core::ChatPlatform::default(),
            chat_add_channel: String::new(),
            chat_add_token: String::new(),
            chat_add_error: None,
            chat_last_send_outcomes: Vec::new(),
            chat_input: String::new(),
            chat_reply_target: None,
            chat_worker_multi: None,
            chat_messages: Vec::new(),
            chat_action_pending: None,
            chat_info_title: Default::default(),
            chat_info_game: Default::default(),
            chat_info_scope: None,
            chat_info_busy: false,
            chat_info_rx: None,
            chat_room_names: rivulet_core::SharedRoomNameService::default(),
            chat_room_name_rx: None,
            chat_info_outcomes: Vec::new(),
            chat_info_error: None,
            plugin_approvals: rivulet_core::PluginApprovals::default(),
            plugins_discovered: Vec::new(),
            plugins_broken: Vec::new(),
            plugin_review_open: None,
            plugin_review_draft: BTreeMap::new(),
            plugin_load_error: None,
            chat_state: rivulet_core::ChatConnState::Off,
            alert_ingest: rivulet_core::AlertIngest::default(),
            alert_preview_dirty: false,
            alert_events: Vec::new(),
            alerts_receiver_enabled: false,
            alerts_receiver_port: rivulet_core::DEFAULT_ALERTS_RECEIVER_PORT,
            alerts_twitch_secret: String::new(),
            alerts_receiver: None,
            alerts_receiver_applied: rivulet_core::AlertsReceiverConfig::default(),
            alerts_receiver_error: None,
            alerts_eventsub_enabled: false,
            alerts_eventsub_client_id: String::new(),
            alerts_eventsub_token: String::new(),
            alerts_eventsub_broadcaster_id: String::new(),
            alerts_eventsub: None,
            alerts_eventsub_applied: rivulet_core::EventsubWsConfig::default(),
            alerts_raid_direction: rivulet_core::RaidAlertDirection::default(),
            alerts_eventsub_error: None,
            alerts_eventsub_ws_endpoint: String::new(),
            alerts_eventsub_api_base: String::new(),
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
            remote_companion_enabled: false,
            remote_companion_port: rivulet_obs_websocket::COMPANION_DEFAULT_PORT,
            remote_companion_bind_lan: false,
            remote_allow_stream_control: false,
            remote_companion_server: None,
            remote_companion_status: None,
            remote_companion_url: None,

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
            record_status: None,
            stop_finalizing: None,

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
            #[cfg(target_os = "macos")]
            is_recording: false,
            #[cfg(target_os = "macos")]
            audio: None,
            #[cfg(target_os = "macos")]
            audio_preview: false,
            #[cfg(target_os = "macos")]
            audio_warning: None,
            #[cfg(target_os = "macos")]
            audio_system_rx: None,
            #[cfg(target_os = "macos")]
            audio_mic_rx: None,
            #[cfg(target_os = "macos")]
            monitors: Vec::new(),
            #[cfg(target_os = "macos")]
            selected_monitor_idx: None,
            #[cfg(target_os = "macos")]
            windows: Vec::new(),
            #[cfg(target_os = "macos")]
            selected_window_idx: None,
            #[cfg(target_os = "macos")]
            raw_rx: None,

            // Platform-agnostic fields (used by camera/game capture)
            #[cfg(target_os = "windows")]
            frame_receiver: None,
            error_receiver: None,
            stop_signal: None,
            last_error: None,
            global_issue_stamp: None,
            global_issue_view: None,
            record_started: Instant::now(),
            last_frame_at: None,
            recording_preview: RecordingPreview::default(),
            pending_preview_frame: None,
            pending_confirmation: None,
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
            browser_backend: None,
            browser_backend_failed: false,
            browser_applied: None,
            browser_preview_texture: None,
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
            scene_item_clipboard: None,
            selected_scene_device_idx: None,
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
            stream_vod_enabled: false,
            stream_vod_recorded: false,
            stream_status_message: None,
            stream_probe_running: false,
            stream_probe_result: None,
            private_test_stream: rivulet_core::PrivateTestStream::new(
                3,
                std::time::Duration::from_secs(30),
            ),
            restream_targets: Vec::new(),
            restream_add_requested: false,
            restream_status: None,

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
            auto_clip_config: rivulet_core::AutoClipConfig::default(),
            auto_clip_detector: rivulet_core::SpikeDetector::new(
                rivulet_core::AutoClipConfig::default(),
            ),
            auto_clip_status: None,

            // Multi-track audio routing (issue #154 Phase 2). Empty by
            // default: the legacy System/Microphone capture keeps working
            // until the user adds routed sources in the Mixer view.
            audio_sources: Vec::new(),
            audio_mixer_filter_source: None,
            audio_mixer_new_source_name: String::new(),
            audio_mixer_new_source_kind: 0,
            audio_mixer_needs_sync: false,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            app_audio_captures: Vec::new(),
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            app_audio_frame_receivers: Vec::new(),
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            app_audio_processes: None,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            app_audio_processes_last_refresh: None,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            audio_mixer_new_source_pid: None,
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            audio_device_list: None,
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            audio_device_list_last_refresh: None,
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            audio_mixer_new_source_device_id: None,
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            device_audio_captures: Vec::new(),
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            device_audio_frame_receivers: Vec::new(),

            ndi_output_enabled: false,
            ndi_output_name: "Rivulet".into(),
            ndi_output_group: String::new(),
            ndi_warning: None,
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

impl RivuletApp {
    // == Multi-track audio routing (issue #154 Phase 2) ==================

    /// Push the current [`AudioSource`] list into the engine (idempotent --
    /// called on every mixer edit and before every session start).
    fn sync_audio_routing(&mut self) {
        if self.audio_mixer_needs_sync {
            self.engine.set_audio_sources(self.audio_sources.clone());
            self.audio_mixer_needs_sync = false;
        }
    }

    /// Start or stop the WASAPI per-application captures so they exactly
    /// mirror the Application-kind sources with a resolved `pid:<n>` target
    /// while a capture session is active (Phase 3 Windows / Phase 4 Linux,
    /// issue #154). Called every UI tick; starting is idempotent per source id
    /// and stopping joins the capture threads.
    ///
    /// Frames travel through an mpsc channel and are drained on the UI thread
    /// (the same architecture as the macOS audio capture), so the engine is
    /// only ever touched from the UI thread.
    /// Re-enumerate the per-app process list in place (#154 follow-up): a
    /// newly started process appears without a manual refresh; one that has
    /// exited drops out. The current pid selection survives when the process
    /// still exists; when it exited, the selection is cleared so the
    /// pending-process hint shows and the add button cannot silently target
    /// a dead pid. macOS lists devices under the shared fallback pid, so
    /// the existence check is the pid identity itself.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn refresh_app_audio_processes_live(&mut self) {
        self.app_audio_processes = Some(rivulet_audio::list_audio_processes());
        self.app_audio_processes_last_refresh = Some(std::time::Instant::now());
        if let Some(selected) = &self.audio_mixer_new_source_pid {
            let still_alive = self
                .app_audio_processes
                .as_ref()
                .is_some_and(|ps| ps.iter().any(|p| &p.pid == selected));
            if !still_alive {
                tracing::debug!(
                    pid = selected,
                    "selected process exited; clearing picker selection"
                );
                self.audio_mixer_new_source_pid = None;
            }
        }
    }

    /// Whether the process list is due for a live refresh: never enumerated,
    /// or the bounded interval has elapsed. Pure so the contract is testable
    /// on every platform.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn app_audio_processes_refresh_due(&self, now: std::time::Instant) -> bool {
        self.app_audio_processes_last_refresh
            .map(|last| now.duration_since(last) >= APP_AUDIO_PROCESSES_REFRESH_INTERVAL)
            .unwrap_or(true)
    }

    /// Start or stop the WASAPI per-application captures so they exactly
    /// mirror the Application-kind sources with a resolved `pid:<n>` target
    /// while a capture session is active (Phase 3 Windows / Phase 4 Linux,
    /// issue #154). Called every UI tick; starting is idempotent per source id
    /// and stopping joins the capture threads.
    ///
    /// Frames travel through an mpsc channel and are drained on the UI thread
    /// (the same architecture as the macOS audio capture), so the engine is
    /// only ever touched from the UI thread.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn sync_app_audio_captures(&mut self) {
        let session_active = self.is_recording_active() || self.engine.is_streaming();
        if !session_active {
            if !self.app_audio_captures.is_empty() {
                self.app_audio_captures.clear(); // Drop joins each capture thread.
            }
            return;
        }
        // Desired set: routed Application sources with a resolvable pid.
        let wanted = routed_application_targets(&self.audio_sources);
        // Stop captures whose source disappeared, was unrouted, or changed pid.
        self.app_audio_captures
            .retain(|(id, _)| wanted.iter().any(|(wid, _)| wid == id));
        // Start missing captures with an mpsc sender as the frame sink.
        for (id, pid) in wanted {
            if self.app_audio_captures.iter().any(|(wid, _)| *wid == id) {
                continue;
            }
            let (tx, rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
            match AppAudioCapture::start(
                pid,
                Box::new(move |frame| {
                    let _ = tx.send(frame); // Receiver gone (stopped) => drop frames.
                }),
            ) {
                Ok(capture) => {
                    self.app_audio_captures.push((id, capture));
                    self.app_audio_frame_receivers.push((id, rx));
                }
                Err(err) => {
                    tracing::warn!(pid, %err, "per-app audio capture failed to start");
                    self.last_error = Some(self.tr_fmt(
                        "audio_app_capture_failed",
                        &[pid.to_string(), err.to_string()],
                    ));
                }
            }
        }
    }

    /// Drain the per-app audio channels into the engine's routed appsrcs.
    /// Runs on the UI thread once per tick while a session is active.
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    fn drain_app_audio_frames(&mut self) {
        if self.app_audio_frame_receivers.is_empty() {
            return;
        }
        let paused = self.is_paused;
        for (id, rx) in &self.app_audio_frame_receivers {
            while let Ok(frame) = rx.try_recv() {
                if !paused {
                    let _ = self.engine.push_audio_source(*id, &frame);
                }
            }
        }
    }
    /// Cache accessor for the device list (issues #229/#231): lazily
    /// enumerated once, then reused so strips and pickers share one snapshot
    /// (WASAPI endpoints on Windows, PipeWire nodes on Linux).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn audio_device_list(&mut self) -> Vec<rivulet_audio::AudioDeviceInfo> {
        if self.audio_device_list.is_none() {
            self.audio_device_list = Some(list_audio_devices());
            self.audio_device_list_last_refresh = Some(std::time::Instant::now());
        }
        self.audio_device_list.clone().unwrap_or_default()
    }

    /// Re-enumerate the device list in place (issue #229 follow-up, issue
    /// #231 Linux): a newly plugged or unplugged device appears or
    /// disappears without a restart. The current device-id selection
    /// survives when the device still exists; when it vanished, the
    /// selection is cleared so the pending-device hint shows and the add
    /// button cannot silently target a dead device.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn refresh_audio_devices_live(&mut self) {
        self.audio_device_list = Some(list_audio_devices());
        self.audio_device_list_last_refresh = Some(std::time::Instant::now());
        if let Some(selected) = &self.audio_mixer_new_source_device_id {
            if !self
                .audio_device_list
                .as_ref()
                .is_some_and(|list| list.iter().any(|d| &d.device_id() == selected))
            {
                tracing::debug!(device_id = %selected, "selected device vanished; clearing picker selection");
                self.audio_mixer_new_source_device_id = None;
            }
        }
    }

    /// Whether the device list is due for a live refresh: never enumerated,
    /// or the bounded interval has elapsed. Pure so the contract is testable
    /// on every platform (the enumeration itself is platform-backed).
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn audio_devices_refresh_due(&self, now: std::time::Instant) -> bool {
        self.audio_device_list_last_refresh
            .map(|last| now.duration_since(last) >= AUDIO_DEVICES_REFRESH_INTERVAL)
            .unwrap_or(true)
    }

    /// Start/stop the device captures for the routed device sources
    /// (issue #229 WASAPI / issue #231 PipeWire): every Input/Output-kind
    /// source with a `wasapi-out:<id>` / `wasapi-in:<id>` / `pw-src:<node>` /
    /// `pw-mon:<node>` device id that is routed to at least one output gets
    /// a capture thread, mirroring the per-app capture lifecycle.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn sync_device_audio_captures(&mut self) {
        let session_active = self.is_recording_active() || self.engine.is_streaming();
        if !session_active {
            if !self.device_audio_captures.is_empty() {
                self.device_audio_captures.clear(); // Drop stops each capture thread.
            }
            return;
        }
        let wanted = routed_device_targets(&self.audio_sources);
        // Stop captures whose source disappeared, was unrouted, or re-targeted.
        self.device_audio_captures.retain(|(id, device_id, _)| {
            wanted
                .iter()
                .any(|(wid, wdevice)| wid == id && wdevice == device_id)
        });
        // Start missing captures with an mpsc sender as the frame sink.
        for (id, device_id) in wanted {
            if self
                .device_audio_captures
                .iter()
                .any(|(wid, wdevice, _)| *wid == id && *wdevice == device_id)
            {
                continue;
            }
            let target = match rivulet_core::device_target(&device_id) {
                Some(target) => target,
                None => continue,
            };
            let (tx, rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
            match AudioDeviceCapture::start_for_target(
                &target,
                Box::new(move |frame| {
                    let _ = tx.send(frame); // Receiver gone (stopped) => drop frames.
                }),
            ) {
                Ok(capture) => {
                    self.device_audio_captures
                        .push((id, device_id.clone(), capture));
                    self.device_audio_frame_receivers.push((id, rx));
                }
                Err(err) => {
                    tracing::warn!(%device_id, %err, "device audio capture failed to start");
                    self.last_error = Some(
                        self.tr_fmt("audio_device_capture_failed", &[device_id, err.to_string()]),
                    );
                }
            }
        }
    }

    /// Drain the device capture channels into the engine's routed appsrcs.
    /// Runs on the UI thread once per tick while a session is active.
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    fn drain_device_audio_frames(&mut self) {
        if self.device_audio_frame_receivers.is_empty() {
            return;
        }
        let paused = self.is_paused;
        for (id, rx) in &self.device_audio_frame_receivers {
            while let Ok(frame) = rx.try_recv() {
                if !paused {
                    let _ = self.engine.push_audio_source(*id, &frame);
                }
            }
        }
    }

    /// Record/Stream routing badges for the compact strips ("R", "S", both
    /// letters, or a dash when routed to neither output).
    fn audio_routing_badges(routing: AudioRouting) -> String {
        match (routing.record, routing.stream) {
            (true, true) => "R·S".to_owned(),
            (true, false) => "R".to_owned(),
            (false, true) => "S".to_owned(),
            (false, false) => "—".to_owned(),
        }
    }

    /// The shared per-source mixer strip: one row with icon, name, volume
    /// slider, mute toggle, routing checkboxes, and the filter button.
    ///
    /// Single implementation, three placements (Mixer view, Record strip,
    /// Stream strip) per the design doc. The full routing matrix lives only
    /// in the Mixer view (`show_routing`); the inline strips show routing
    /// badges and toggle volume/mute, which the engine applies live.
    fn draw_audio_source_strip(
        &mut self,
        ui: &mut egui::Ui,
        source: &AudioSource,
        show_routing: bool,
    ) {
        let id = source.id;
        let mut volume = source.volume;
        let mut muted = source.muted;
        let mut record = source.routing.record;
        let mut stream = source.routing.stream;
        let mut open_filters = false;

        ui.push_id(id, |ui| {
            ui.horizontal(|ui| {
                // Issue #229 / #231: device sources show the friendly
                // device name, not the raw `wasapi-out:`/`pw-mon:` id.
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                let display_name = audio_device_strip_label(source, &self.audio_device_list());
                #[cfg(not(any(target_os = "windows", target_os = "linux")))]
                let display_name = source.name.clone();
                ui.label(format!("{} {}", source.kind.icon(), display_name));
                ui.add_space(4.0);
                ui.add(egui::Slider::new(&mut volume, 0.0..=2.0).show_value(false));
                let mute_label = self.tr("audio_source_mute");
                if ui
                    .toggle_value(&mut muted, "🔇")
                    .on_hover_text(mute_label)
                    .changed()
                {
                    let _ = self.engine.set_audio_source_muted(id, muted);
                }
                if show_routing {
                    let record_label = self.tr("audio_routing_record");
                    let stream_label = self.tr("audio_routing_stream");
                    ui.checkbox(&mut record, record_label);
                    ui.checkbox(&mut stream, stream_label);
                } else {
                    ui.label(
                        egui::RichText::new(Self::audio_routing_badges(source.routing)).weak(),
                    );
                }
                let filter_label = self.tr("audio_filter_per_source");
                if theme::icon_button(ui, "⚙")
                    .on_hover_text(filter_label)
                    .clicked()
                {
                    open_filters = true;
                }
            });
        });

        let routing_changed = record != source.routing.record || stream != source.routing.stream;
        if routing_changed {
            let _ = self
                .engine
                .set_audio_source_routing(id, AudioRouting { record, stream });
        }
        if (volume - source.volume).abs() > f32::EPSILON {
            let _ = self.engine.set_audio_source_volume(id, volume);
        }
        // Mirror the engine call results back into the GUI list so the next
        // sync round-trips the same values.
        if let Some(s) = self.audio_sources.iter_mut().find(|s| s.id == id) {
            s.volume = volume.clamp(0.0, 2.0);
            s.muted = muted;
            if routing_changed {
                s.routing = AudioRouting { record, stream };
            }
        }
        self.audio_mixer_needs_sync = true;
        if open_filters {
            self.audio_mixer_filter_source = Some(id);
        }
    }

    /// The per-source filter panel (opened via the gear button in any mixer
    /// placement). Edits update the source config; the live pipeline chain is
    /// rebuilt on the next session start (the engine confirms the same
    /// chain deterministically).
    fn draw_audio_source_filter_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.audio_mixer_filter_source else {
            return;
        };
        let Some(source) = self.audio_sources.iter().find(|s| s.id == id) else {
            self.audio_mixer_filter_source = None;
            return;
        };
        let name = source.name.clone();
        let title = self.tr_fmt("audio_filter_per_source", &[name]);
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::new("audio_source_filter_panel"))
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                self.draw_audio_filter_fields(ui, id);
            });
        if !open {
            self.audio_mixer_filter_source = None;
        }
    }

    /// The filter fields for one source (gate, compressor, limiter, expander,
    /// gain, EQ). Mirrors the legacy per-track filter row so a source's chain
    /// sounds the same as the legacy System/Mic filters.
    fn draw_audio_filter_fields(&mut self, ui: &mut egui::Ui, id: uuid::Uuid) {
        let gate_label = self.tr("filter_noise_gate").to_owned();
        let compressor_label = self.tr("filter_compressor").to_owned();
        let limiter_label = self.tr("filter_limiter").to_owned();
        let expander_label = self.tr("filter_expander").to_owned();
        let gain_label = self.tr("filter_gain").to_owned();
        let eq_label = self.tr("filter_eq").to_owned();
        let Some(source) = self.audio_sources.iter_mut().find(|s| s.id == id) else {
            return;
        };
        let mut gate_on = source.filters.noise_gate.is_some();
        let mut compressor_on = source.filters.compressor.is_some();
        let mut limiter_on = source.filters.limiter.is_some();
        let mut expander_on = source.filters.expander.is_some();
        let mut gain_db = source.filters.gain_db;
        let mut eq_on = source.filters.eq.is_some();

        ui.checkbox(&mut gate_on, gate_label);
        ui.checkbox(&mut compressor_on, compressor_label);
        ui.checkbox(&mut limiter_on, limiter_label);
        ui.checkbox(&mut expander_on, expander_label);
        ui.add(egui::Slider::new(&mut gain_db, -30.0..=30.0).text(gain_label));
        ui.checkbox(&mut eq_on, eq_label);
        if eq_on {
            let mut bands = source.filters.eq.map(|eq| eq.bands).unwrap_or([0.0; 10]);
            ui.horizontal_wrapped(|ui| {
                for band in bands.iter_mut() {
                    ui.add(egui::Slider::new(band, -12.0..=12.0));
                }
            });
            source.filters.eq = Some(rivulet_core::audio_source::EqConfig { bands });
        } else {
            source.filters.eq = None;
        }

        let changed = gate_on != source.filters.noise_gate.is_some()
            || compressor_on != source.filters.compressor.is_some()
            || limiter_on != source.filters.limiter.is_some()
            || expander_on != source.filters.expander.is_some()
            || (gain_db - source.filters.gain_db).abs() > 1e-3;
        source.filters.noise_gate =
            gate_on.then(rivulet_core::audio_source::NoiseGateConfig::default);
        source.filters.compressor =
            compressor_on.then(rivulet_core::audio_source::CompressorConfig::default);
        source.filters.limiter =
            limiter_on.then(rivulet_core::audio_source::LimiterConfig::default);
        source.filters.expander =
            expander_on.then(rivulet_core::audio_source::ExpanderConfig::default);
        source.filters.gain_db = gain_db;
        if changed {
            let filters = source.filters;
            let _ = self.engine.set_audio_source_filters(id, filters);
            self.audio_mixer_needs_sync = true;
        }
    }

    /// The Mixer view: source list with the full routing matrix, add/remove,
    /// and the filter panel entry points.
    fn draw_mixer_sources(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(self.tr("audio_routing_sources")).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(self.tr("audio_source_add")).clicked() {
                    let kind = match self.audio_mixer_new_source_kind {
                        0 => rivulet_core::AudioSourceKind::Application,
                        1 => rivulet_core::AudioSourceKind::InputDevice,
                        2 => rivulet_core::AudioSourceKind::OutputDevice,
                        _ => rivulet_core::AudioSourceKind::Mixed,
                    };
                    let name = if self.audio_mixer_new_source_name.trim().is_empty() {
                        self.tr("audio_source_default_name").to_owned()
                    } else {
                        self.audio_mixer_new_source_name.trim().to_owned()
                    };
                    let device_id = match kind {
                        #[cfg(any(
                            target_os = "windows",
                            target_os = "linux",
                            target_os = "macos"
                        ))]
                        rivulet_core::AudioSourceKind::Application => self
                            .audio_mixer_new_source_pid
                            .map(|pid| format!("pid:{pid}"))
                            .unwrap_or_else(|| "pending_app".to_owned()),
                        #[cfg(not(any(
                            target_os = "windows",
                            target_os = "linux",
                            target_os = "macos"
                        )))]
                        #[allow(unused_variables)]
                        rivulet_core::AudioSourceKind::Application => "pending_app".to_owned(),
                        // Issue #229 / #231: on Windows and Linux the
                        // Input/Output pickers select a concrete device
                        // ("wasapi-out:<id>" / "wasapi-in:<id>" endpoints,
                        // "pw-src:<node>" / "pw-mon:<node>" nodes); without
                        // a selection the legacy placeholders keep the old
                        // behavior. Other platforms stay on the placeholders.
                        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
                        rivulet_core::AudioSourceKind::InputDevice => "default_input".to_owned(),
                        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
                        rivulet_core::AudioSourceKind::OutputDevice => "system_loopback".to_owned(),
                        #[cfg(any(target_os = "windows", target_os = "linux"))]
                        rivulet_core::AudioSourceKind::InputDevice => self
                            .audio_mixer_new_source_device_id
                            .clone()
                            .unwrap_or_else(|| "default_input".to_owned()),
                        #[cfg(any(target_os = "windows", target_os = "linux"))]
                        rivulet_core::AudioSourceKind::OutputDevice => self
                            .audio_mixer_new_source_device_id
                            .clone()
                            .unwrap_or_else(|| "system_loopback".to_owned()),
                        rivulet_core::AudioSourceKind::Mixed => "mixed".to_owned(),
                    };
                    #[cfg(any(target_os = "windows", target_os = "linux"))]
                    {
                        if self.audio_mixer_new_source_kind == 0 {
                            self.audio_mixer_new_source_pid = None;
                        }
                        if self.audio_mixer_new_source_kind == 1
                            || self.audio_mixer_new_source_kind == 2
                        {
                            self.audio_mixer_new_source_device_id = None;
                        }
                    }
                    let source = AudioSource::new(name, device_id, kind);
                    let _ = self.engine.add_audio_source(source.clone());
                    self.audio_sources.push(source);
                    self.audio_mixer_needs_sync = true;
                    self.audio_mixer_new_source_name.clear();
                }
                let hint = self.tr("audio_source_name");
                ui.add(
                    egui::TextEdit::singleline(&mut self.audio_mixer_new_source_name)
                        .hint_text(hint)
                        .desired_width(140.0),
                );
                egui::ComboBox::from_id_salt("audio_mixer_new_source_kind")
                    .selected_text(self.tr(self.audio_mixer_new_source_kind_key()).to_owned())
                    .width(110.0)
                    .show_ui(ui, |ui| {
                        for i in 0..4 {
                            let key = self.audio_mixer_new_source_kind_key_at(i);
                            let label = self.tr(key).to_owned();
                            ui.selectable_value(&mut self.audio_mixer_new_source_kind, i, label);
                        }
                    });
            });
        });
        ui.label(egui::RichText::new(self.tr("audio_routing_hint")).weak());
        // Phase 3 (Windows) / Phase 4 (Linux): process picker for
        // Application-kind sources.
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        if self.audio_mixer_new_source_kind == 0 {
            ui.horizontal(|ui| {
                ui.label(self.tr("audio_source_pick_process"));
                let refresh_clicked = ui
                    .button("⟳")
                    .on_hover_text(self.tr("audio_source_refresh_processes"))
                    .clicked();
                // Bounded live refresh: re-enumerate while the picker is
                // visible so a newly started process appears without a
                // manual refresh. A selection whose process exited is
                // cleared so the pending-process hint shows instead of
                // silently adding a dead pid (first open counts as due).
                if refresh_clicked
                    || self.app_audio_processes_refresh_due(std::time::Instant::now())
                {
                    self.refresh_app_audio_processes_live();
                }
                let picker_label = self
                    .audio_mixer_new_source_pid
                    .and_then(|pid| {
                        self.app_audio_processes
                            .as_ref()
                            .and_then(|ps| ps.iter().find(|p| p.pid == pid))
                            .map(|p| format!("{} ({pid})", p.name))
                    })
                    .unwrap_or_else(|| self.tr("audio_source_no_process_selected").to_owned());
                egui::ComboBox::from_id_salt("audio_mixer_new_source_pid")
                    .selected_text(picker_label)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if let Some(processes) = &self.app_audio_processes {
                            for proc in processes {
                                let label = format!("{} ({})", proc.name, proc.pid);
                                ui.selectable_value(
                                    &mut self.audio_mixer_new_source_pid,
                                    Some(proc.pid),
                                    label,
                                );
                            }
                        }
                    });
            });
            if self.audio_mixer_new_source_pid.is_none() {
                ui.label(egui::RichText::new(self.tr("audio_source_pid_required")).weak());
            }
            // macOS fallback semantics: Application sources all share the
            // system loopback mix (macOS has no per-app capture API).
            #[cfg(target_os = "macos")]
            ui.label(egui::RichText::new(self.tr("audio_app_fallback_hint")).weak());
        }
        // Issue #229 / #231: device picker for Input/Output-kind sources
        // (Windows WASAPI endpoints, Linux PipeWire nodes). Lists active
        // devices with friendly names; the defaults are marked. Mirrors the
        // process picker above: cached list, manual refresh,
        // pending-selection hint — plus a bounded live refresh so
        // plugged/unplugged devices appear or drop out without a restart
        // (same pattern as the game-window list).
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        if self.audio_mixer_new_source_kind == 1 || self.audio_mixer_new_source_kind == 2 {
            ui.horizontal(|ui| {
                ui.label(self.tr("audio_source_pick_device"));
                let refresh_clicked = ui
                    .button("⟳")
                    .on_hover_text(self.tr("audio_source_refresh_devices"))
                    .clicked();
                // Bounded live refresh: re-enumerate while the picker is
                // visible so a newly plugged endpoint appears without a
                // restart. A vanished endpoint clears the selection so the
                // pending-device hint shows instead of silently adding a
                // dead device id (first open counts as due).
                if refresh_clicked || self.audio_devices_refresh_due(std::time::Instant::now()) {
                    self.refresh_audio_devices_live();
                }
                let default_marker = self.tr("audio_source_default_device").to_owned();
                let picker_label = self
                    .audio_mixer_new_source_device_id
                    .as_deref()
                    .and_then(|id| {
                        self.audio_device_list
                            .as_ref()
                            .and_then(|ds| ds.iter().find(|d| d.device_id() == id))
                            .map(|d| audio_device_picker_label(d, &default_marker))
                    })
                    .unwrap_or_else(|| self.tr("audio_source_no_device_selected").to_owned());
                egui::ComboBox::from_id_salt("audio_mixer_new_source_device")
                    .selected_text(picker_label)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        if let Some(devices) = &self.audio_device_list {
                            for device in devices {
                                let label = audio_device_picker_label(device, &default_marker);
                                ui.selectable_value(
                                    &mut self.audio_mixer_new_source_device_id,
                                    Some(device.device_id()),
                                    label,
                                );
                            }
                        }
                    });
            });
            if self.audio_mixer_new_source_device_id.is_none() {
                ui.label(egui::RichText::new(self.tr("audio_source_device_required")).weak());
            }
        }
        if self.audio_sources.is_empty() {
            ui.label(egui::RichText::new(self.tr("audio_routing_legacy_active")).weak());
        }
        let mut remove_id: Option<uuid::Uuid> = None;
        egui::Grid::new("audio_routing_matrix")
            .num_columns(4)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.strong(self.tr("audio_source_name"));
                ui.strong(self.tr("audio_routing_record"));
                ui.strong(self.tr("audio_routing_stream"));
                ui.strong("");
                ui.end_row();
                for source in self.audio_sources.clone() {
                    ui.label(format!("{} {}", source.kind.icon(), source.name));
                    self.draw_audio_source_strip(ui, &source, true);
                    let remove_label = self.tr("audio_source_remove");
                    if theme::icon_button(ui, "🗑")
                        .on_hover_text(remove_label)
                        .clicked()
                    {
                        remove_id = Some(source.id);
                    }
                    ui.end_row();
                }
            });
        if let Some(id) = remove_id {
            self.remove_audio_source(id);
        }
        self.draw_audio_source_filter_panel(ui);
    }

    /// The compact inline mixer strip (Record/Stream views): shared per-source
    /// controls without the routing matrix (badges instead) so recording and
    /// streaming never require a view switch. The full matrix stays in the
    /// Mixer view.
    fn draw_inline_audio_mixer(&mut self, ui: &mut egui::Ui) {
        if self.audio_sources.is_empty() {
            return;
        }
        ui.separator();
        ui.label(egui::RichText::new(self.tr("audio_routing_inline")).strong());
        for source in self.audio_sources.clone() {
            self.draw_audio_source_strip(ui, &source, false);
        }
        self.draw_audio_source_filter_panel(ui);
    }

    /// The i18n key of the currently selected "new source" kind.
    fn audio_mixer_new_source_kind_key(&self) -> &'static str {
        self.audio_mixer_new_source_kind_key_at(self.audio_mixer_new_source_kind)
    }

    fn audio_mixer_new_source_kind_key_at(&self, index: usize) -> &'static str {
        match index {
            0 => "audio_source_kind_app",
            1 => "audio_source_kind_input",
            2 => "audio_source_kind_output",
            _ => "audio_source_kind_mixed",
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
        // A new session starts with clean feedback: drop any completion handle
        // still pending from a previous stop and clear its status text.
        self.stop_finalizing = None;
        self.record_status = None;
        let ext = self.selected_container_extension();
        let file_path = rfd::FileDialog::new()
            .add_filter("Video", &[ext])
            .set_file_name(self.default_recording_filename())
            .save_file();

        let Some(path) = file_path else {
            tracing::info!("File selection cancelled");
            return;
        };

        // Determine the capture source (window or monitor) and start accordingly
        if let Some(idx) = self.selected_window_idx {
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
            self.apply_ndi_output();
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
        } else if let Some(idx) = self.selected_monitor_idx {
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
            self.apply_ndi_output();
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
            self.apply_ndi_output();
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
            self.apply_ndi_output();
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
        // Teardown runs on a background thread (never freezes the UI); the
        // "finalizing…" → "Recording saved." status is driven by
        // begin_background_stop/poll_stop_finalization.
        self.begin_background_stop();
        self.frame_receiver = None;
        // Keep the error receiver alive after the stop so a capture error
        // that arrives a frame late is still shown in the UI; the next
        // recording start replaces it.
        self.stop_signal = None;
        self.complete_recording_session_telemetry();
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
        self.apply_ndi_output();
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
        self.apply_ndi_output();
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
        // A new session starts with clean feedback: drop any completion handle
        // still pending from a previous stop and clear its status text.
        self.stop_finalizing = None;
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
        // Teardown runs on a background thread (never freezes the UI); the
        // "finalizing…" → "Recording saved." status is driven by
        // begin_background_stop/poll_stop_finalization.
        self.begin_background_stop();
        self.raw_rx = None;
        self.stop_signal = None;
        self.stop_audio_capture();
        self.is_recording = false;
        self.complete_recording_session_telemetry();
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

// --- macOS logic implementation (M5 Windows/macOS feature parity) ---
#[cfg(target_os = "macos")]
impl RivuletApp {
    /// Refresh the xcap monitor/window lists used for macOS recording. Without
    /// the Screen Recording permission both lists stay empty and the Record
    /// view shows the permission hint instead.
    fn refresh_macos_sources(&mut self) {
        self.monitors = xcap::Monitor::all().unwrap_or_default();
        if self
            .selected_monitor_idx
            .map_or(true, |idx| idx >= self.monitors.len())
        {
            self.selected_monitor_idx = None;
        }
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

    /// Start the macOS audio capture (cpal): the microphone via the default
    /// input device and system audio via an installed loopback driver
    /// (BlackHole/Soundflower/VB-Cable). Frames are delivered on separate
    /// system/mic channels exactly like the Linux backend. Without a loopback
    /// driver system audio is skipped with a warning instead of failing the
    /// recording.
    fn start_macos_audio(&mut self) {
        self.stop_macos_audio();
        let config = AudioConfig {
            capture_system: true,
            capture_mic: true,
            system_volume: 0.8,
            mic_volume: 1.0,
            master_volume: 1.0,
            separate_tracks: true,
            ..Default::default()
        };
        let mut audio = match AudioCapture::new(config) {
            Ok(audio) => audio,
            Err(e) => {
                self.audio_warning = Some(format!("{}", e));
                return;
            }
        };
        let (sys_tx, sys_rx) = std_mpsc::channel::<AudioFrame>();
        let (mic_tx, mic_rx) = std_mpsc::channel::<AudioFrame>();
        match audio.start_separated(
            Box::new(move |frame: AudioFrame| {
                let _ = sys_tx.send(frame);
            }),
            Box::new(move |frame: AudioFrame| {
                let _ = mic_tx.send(frame);
            }),
        ) {
            Ok(()) => {
                // The backend note reports the resolved system source: either
                // the found loopback device or the "install BlackHole…" hint.
                // Empty when system capture is not requested.
                let note = audio.system_source_note().to_string();
                self.audio_warning = (!note.is_empty()).then_some(note);
                self.audio = Some(audio);
                self.audio_system_rx = Some(sys_rx);
                self.audio_mic_rx = Some(mic_rx);
                self.audio_preview = true;
            }
            Err(e) => {
                self.audio_warning = Some(format!("{}", e));
            }
        }
    }

    /// Stop the macOS audio capture and drop the frame channels.
    fn stop_macos_audio(&mut self) {
        if let Some(mut audio) = self.audio.take() {
            let _ = audio.stop();
        }
        self.audio_system_rx = None;
        self.audio_mic_rx = None;
        self.audio_preview = false;
        self.audio_warning = None;
    }

    /// Drain the macOS audio frame channels into the engine's separate audio
    /// tracks (system + mic). While paused or muted the frames are discarded,
    /// matching the Linux behaviour.
    fn drain_macos_audio(&mut self) {
        let paused = self.is_paused;
        let muted = self.is_muted;
        if !self.audio_preview {
            return;
        }
        if self.is_recording && !paused && !muted {
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
    }

    /// Start a macOS recording of the selected monitor or window. Video is
    /// captured via xcap; audio (system via loopback driver, microphone via
    /// cpal) is started alongside and pushed into the same engine recording
    /// pipeline as on Linux/Windows.
    fn start_macos_recording(&mut self) {
        if self.is_recording {
            return;
        }
        enum Source {
            Monitor(xcap::Monitor),
            Window(xcap::Window),
        }
        let source = if let Some(idx) = self.selected_window_idx {
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

        self.engine.set_video_codec(self.selected_codec);
        self.engine.set_recording_container(self.selected_container);
        self.engine.set_auto_remux(self.auto_remux);
        self.apply_recording_file_settings();
        self.engine.set_preset(self.selected_preset);
        self.apply_rate_control();
        self.engine.set_overlay_enabled(self.show_overlay);
        self.engine.set_video_effects(self.video_effects);
        self.apply_replay_setting();
        self.apply_ndi_output();
        self.engine.set_audio_enabled(true);
        self.engine.set_separate_audio_tracks(true);
        self.engine.start_local_recording(path);
        self.start_macos_audio();

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
        // A new session starts with clean feedback.
        self.stop_finalizing = None;
        self.record_status = None;
        tracing::info!(source = %source_desc, "macOS recording started");
    }

    /// Stop a macOS recording: signal the capture thread and tear the engine
    /// pipeline down on a background thread so the UI never freezes.
    fn stop_macos_recording(&mut self) {
        if !self.is_recording {
            return;
        }
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        self.begin_background_stop();
        self.stop_macos_audio();
        self.raw_rx = None;
        self.stop_signal = None;
        self.is_recording = false;
        self.complete_recording_session_telemetry();
    }

    /// Push captured macOS frames into the engine and refresh the live preview
    /// thumbnail; records the last-frame timestamp used by the no-frame abort.
    fn drain_macos_frames(&mut self) {
        let paused = self.is_paused;
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
        if self.show_overlay && self.is_recording && !paused {
            let text = self.overlay_text();
            self.engine.update_overlay_text(&text);
        }
        self.drain_macos_audio();
    }

    /// Draw the macOS Record view: source selection (monitor or window),
    /// video-only + Screen Recording permission hints, and start/stop with the
    /// finalizing status.
    fn draw_macos_record_view(&mut self, ui: &mut egui::Ui, colors: &theme::StatusColors) {
        ui.add_space(10.0);
        ui.label(egui::RichText::new(self.tr("screen_recording")).strong());
        if self.monitors.is_empty() && self.windows.is_empty() {
            ui.small(self.tr("mac_permission_hint"));
        }
        ui.small(self.tr("mac_audio_hint"));
        if let Some(warning) = &self.audio_warning {
            ui.colored_label(colors.warning, warning);
        }

        let monitors: Vec<(usize, String)> = self
            .monitors
            .iter()
            .enumerate()
            .map(|(idx, m)| {
                (
                    idx,
                    format!(
                        "{} ({}x{})",
                        m.name().unwrap_or_default(),
                        m.width().unwrap_or(0),
                        m.height().unwrap_or(0)
                    ),
                )
            })
            .collect();
        let monitor_selection = self.selected_monitor_idx;
        ui.horizontal(|ui| {
            ui.label(self.tr("mac_source_monitor"));
            egui::ComboBox::from_id_salt("macos_monitor_select")
                .selected_text(
                    monitor_selection
                        .and_then(|idx| monitors.iter().find(|(i, _)| *i == idx))
                        .map(|(_, label)| label.clone())
                        .unwrap_or_else(|| self.tr("mac_none").to_string()),
                )
                .show_ui(ui, |ui| {
                    for (idx, label) in &monitors {
                        if ui
                            .selectable_label(monitor_selection == Some(*idx), label.clone())
                            .clicked()
                        {
                            self.selected_monitor_idx = Some(*idx);
                            self.selected_window_idx = None;
                        }
                    }
                });
        });

        let windows: Vec<(usize, String)> =
            windows_on_selected_monitor(&self.windows, &self.monitors, self.selected_monitor_idx)
                .into_iter()
                .map(|(idx, w)| (idx, w.title().unwrap_or_default()))
                .collect();
        let window_selection = self.selected_window_idx;
        ui.horizontal(|ui| {
            ui.label(self.tr("mac_source_window"));
            egui::ComboBox::from_id_salt("macos_window_select")
                .selected_text(
                    window_selection
                        .and_then(|idx| windows.iter().find(|(i, _)| *i == idx))
                        .map(|(_, title)| title.clone())
                        .unwrap_or_else(|| self.tr("mac_none").to_string()),
                )
                .show_ui(ui, |ui| {
                    for (idx, title) in &windows {
                        if ui
                            .selectable_label(window_selection == Some(*idx), title.clone())
                            .clicked()
                        {
                            self.selected_window_idx = Some(*idx);
                        }
                    }
                });
        });

        let preset_selection = self.selected_preset;
        ui.horizontal(|ui| {
            ui.label(self.tr("recording_preset"));
            egui::ComboBox::from_id_salt("macos_preset_select")
                .selected_text(preset_selection.label)
                .show_ui(ui, |ui| {
                    for preset in rivulet_core::RecordingPreset::all() {
                        if ui
                            .selectable_label(preset_selection == *preset, preset.label)
                            .clicked()
                        {
                            self.selected_preset = *preset;
                        }
                    }
                });
        });
        let overlay_label = self.tr("overlay_toggle").to_string();
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_overlay, overlay_label);
        });

        let source_selected =
            self.selected_monitor_idx.is_some() || self.selected_window_idx.is_some();
        let stop_label = format!("⏹ {}", self.tr("stop_recording"));
        let start_label = format!("⏺ {}", self.tr("start_recording"));
        if self.is_recording {
            let elapsed = self.record_started.elapsed().as_secs();
            let in_progress = self.tr_fmt("recording_in_progress", &[elapsed.to_string()]);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(in_progress).color(colors.active));
                if ui.button(&stop_label).clicked() {
                    self.stop_macos_recording();
                    self.is_paused = false;
                }
            });
        } else if ui
            .add_enabled(source_selected, egui::Button::new(&start_label))
            .clicked()
        {
            self.start_macos_recording();
        }
        if let Some(err) = &self.last_error {
            ui.colored_label(colors.error, err);
        }
        if let Some(status) = &self.record_status {
            ui.colored_label(colors.info, status);
        }
    }
}

impl RivuletApp {
    /// Stop camera or game-capture recording (platform-agnostic).
    fn stop_aux_recording(&mut self) {
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        // Teardown runs on a background thread so the UI never freezes for the
        // EOS wait / auto-remux / cloud upload (see stop_recording_background).
        self.begin_background_stop();
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
        self.complete_recording_session_telemetry();
    }

    /// Arms the non-blocking recording stop: shows the translated
    /// "finalizing…" status and keeps the completion handle that fires once
    /// the background teardown (EOS finalization, auto-remux, cloud upload)
    /// has finished. [`Self::poll_stop_finalization`] flips the status to
    /// "Recording saved." when that happens.
    fn begin_background_stop(&mut self) {
        match self.engine.stop_recording_background() {
            Some(rx) => {
                self.record_status = Some(self.tr("recording_finalizing").to_string());
                self.stop_finalizing = Some(rx);
            }
            None => {
                // Nothing was active (idempotent stop): there is no background
                // work, so a "finalizing…" state would never clear itself.
                self.stop_finalizing = None;
            }
        }
    }

    /// Stop the streaming session (Stream-workspace button and OBS
    /// StopStreaming). The engine cannot reconfigure a live dual-output
    /// pipeline, so a dual session (recording still active) only drops the
    /// stream config — the running recording continues and the stream stops
    /// with the session. A pure-stream session has nothing to keep, so it ends
    /// the whole engine session via the non-blocking background stop (with the
    /// visible "Finalizing…" → "Recording saved." status) instead of leaking
    /// an active engine session.
    fn stop_streaming_session(&mut self) {
        let end_session = self.engine.is_recording() && !self.engine.is_dual_output();
        self.engine.set_stream_settings(None);
        if end_session {
            self.begin_background_stop();
        }
        self.stream_status_message = Some(self.tr("stopped").to_owned());
    }

    /// How often the OS reduce-motion setting is re-probed. The platform
    /// queries spawn a subprocess on macOS/Linux, so this must stay coarse;
    /// the setting changes at most when the user visits their OS settings.
    const OS_MOTION_PROBE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

    /// Refresh the cached OS reduce-motion hint at most every
    /// [`Self::OS_MOTION_PROBE_INTERVAL`] and apply the resolved motion
    /// state to the egui context (ui-005, WCAG 2.3.3): with reduced motion
    /// egui's global `animation_time` is 0, so all `animate_bool`-based
    /// fades (preview fades, egui-internal animations) snap to their target
    /// instead of interpolating. Reapplies whenever the preference or the
    /// cached OS hint changes.
    fn update_motion_preference(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let needs_probe = self.os_reduced_motion.is_none()
            || match self.os_reduced_motion_probed_at {
                Some(at) => now.saturating_duration_since(at) >= Self::OS_MOTION_PROBE_INTERVAL,
                None => true,
            };
        if needs_probe {
            self.os_reduced_motion = Some(theme::os_prefers_reduced_motion());
            self.os_reduced_motion_probed_at = Some(now);
        }
        let os_reduced = self.os_reduced_motion.unwrap_or(false);
        let reduced = self.motion_preference.resolves_to_reduced(os_reduced);
        if self.motion_applied != Some(reduced) {
            theme::apply_motion(ctx, reduced);
            self.motion_applied = Some(reduced);
        }
    }

    /// The scene transition to use for an upcoming switch: the user's
    /// configured transition, except that a Fade collapses to a Cut when
    /// reduced motion is active (WCAG 2.3.3; a fade is pure decoration and
    /// loses meaning when it cannot animate).
    fn scene_transition_for_switch(
        &self,
        from: Option<uuid::Uuid>,
        to: Option<uuid::Uuid>,
        now: Instant,
    ) -> rivulet_core::SceneTransition {
        let reduced = self.motion_applied.unwrap_or(false);
        let kind = if reduced {
            rivulet_core::TransitionKind::Cut
        } else {
            self.transition_kind
        };
        let mut transition = rivulet_core::SceneTransition::new(
            kind,
            std::time::Duration::from_millis(self.transition_duration_ms),
        );
        transition.start(from, to, now);
        transition
    }

    /// Called once per frame while a stop finalization may be in flight: when
    /// the background teardown completes, drop the handle and flip the status
    /// from "finalizing…" to "Recording saved.".
    fn poll_stop_finalization(&mut self) {
        let finished = match &self.stop_finalizing {
            None => false,
            Some(rx) => matches!(
                rx.try_recv(),
                Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected)
            ),
        };
        if finished {
            self.stop_finalizing = None;
            self.record_status = Some(self.tr("recording_saved").to_string());
        }
    }

    /// Draw the aggregated global issue at the sidebar footer (ui-007): the
    /// most severe current view-local status, with a severity prefix and the
    /// origin view. Clicking it navigates to that view. The entry expires
    /// [`GLOBAL_ISSUE_TTL`] after the issue first appeared (renewed on change)
    /// so stale one-shot confirmations do not linger forever.
    fn draw_global_issue_footer(&mut self, ui: &mut egui::Ui) {
        let issue = self.collect_global_issue();
        let now = Instant::now();
        match (&issue, &self.global_issue_view, self.global_issue_stamp) {
            (Some((_, view, _)), Some(active), _) if view == active => {}
            (Some((_, view, _)), _, _) => {
                self.global_issue_view = Some(*view);
                self.global_issue_stamp = Some(now);
            }
            (None, _, _) => {
                self.global_issue_view = None;
                self.global_issue_stamp = None;
            }
        }
        let Some((kind, view, text)) = issue else {
            return;
        };
        if let Some(stamp) = self.global_issue_stamp {
            if now.duration_since(stamp) > GLOBAL_ISSUE_TTL {
                return;
            }
        }

        let colors = theme::StatusColors::for_ui(ui);
        let color = match kind {
            GlobalIssueKind::Error => colors.error,
            GlobalIssueKind::Warning => colors.warning,
            GlobalIssueKind::Info => colors.info,
        };
        ui.separator();
        let label = format!(
            "{}: {} — {}",
            self.tr(kind.label_key()),
            text,
            self.tr(view.nav_key())
        );
        let response = ui
            .add(
                egui::Label::new(egui::RichText::new(label).color(color).small())
                    .wrap()
                    .sense(egui::Sense::click()),
            )
            .on_hover_text(self.tr("global_issue_jump_hint"));
        theme::paint_interaction_stroke(ui, &response);
        if response.clicked() {
            self.view = view;
        }
    }

    /// Aggregate the most relevant view-local status into one global issue
    /// for the sidebar footer (audit finding ui-007).
    ///
    /// A failure raised in a background tab used to stay invisible until the
    /// user navigated there. This picks the highest-severity issue across all
    /// views (`Error` beats `Warning` beats `Info`; ties resolve to the
    /// earliest view in sidebar order) so it can be surfaced globally. The
    /// inline view displays stay untouched — this is an additional mirror,
    /// not a replacement.
    fn collect_global_issue(&self) -> Option<(GlobalIssueKind, AppView, String)> {
        // (kind, view, text) candidates in sidebar order; the fold prefers
        // higher severity and, on ties, keeps the earlier view.
        let mut candidates: Vec<(GlobalIssueKind, AppView, String)> = Vec::new();
        // Record view — capture/preview failures block recording.
        if let Some(t) = self.last_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Record, t.clone()));
        }
        #[cfg(target_os = "linux")]
        if let Some(t) = self.audio_status.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Record, t.clone()));
        }
        if let Some(t) = self.record_status.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Record, t.clone()));
        }
        if let Some(t) = self.region_preview_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Record, t.clone()));
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(t) = self.game_preview_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Record, t.clone()));
        }
        // Scenes view — export/validation failures.
        if let Some(t) = self
            .scene_status
            .as_ref()
            .filter(|t| Self::scene_status_is_problem(t))
        {
            candidates.push((GlobalIssueKind::Error, AppView::Scenes, t.clone()));
        }
        // Stream view — keyring/config problems need attention.
        if let Some(t) = self
            .stream_key_store_status
            .as_ref()
            .filter(|t| *t == &Self::static_tr("stream_key_store_unavailable"))
        {
            candidates.push((GlobalIssueKind::Warning, AppView::Stream, t.clone()));
        }
        // Settings view — chat account and plugin errors.
        if let Some(t) = self.chat_add_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Settings, t.clone()));
        }
        if let Some(t) = self.alerts_receiver_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Settings, t.clone()));
        }
        if let Some(t) = self.alerts_eventsub_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Settings, t.clone()));
        }
        if let Some(t) = self.plugin_load_error.as_ref() {
            candidates.push((GlobalIssueKind::Error, AppView::Settings, t.clone()));
        }
        // Service statuses (obs websocket, remote companion, MIDI) live in
        // Settings too; only their failure texts are surfaced here.
        if let Some(t) = self
            .obs_ws_status
            .as_ref()
            .filter(|t| Self::service_status_is_problem(t))
        {
            candidates.push((GlobalIssueKind::Warning, AppView::Settings, t.clone()));
        }
        if let Some(t) = self
            .remote_companion_status
            .as_ref()
            .filter(|t| Self::service_status_is_problem(t))
        {
            candidates.push((GlobalIssueKind::Warning, AppView::Settings, t.clone()));
        }
        if let Some(t) = self.midi_status.as_ref() {
            // MIDI failures are raw error strings from the listener (no i18n
            // key), while the idle state is always `None` — so any text here
            // is a failure worth surfacing.
            candidates.push((GlobalIssueKind::Warning, AppView::Settings, t.clone()));
        }
        candidates
            .into_iter()
            .max_by_key(|(kind, view, _)| (*kind as u8, std::cmp::Reverse(*view as usize)))
    }

    /// `scene_status` carries both confirmations ("Scene added") and failures
    /// ("…failed"); only failures belong in the global surface.
    fn scene_status_is_problem(text: &str) -> bool {
        let mut problem = false;
        for key in ["scenes_export_failed", "scenes_import_failed"] {
            let localized = Self::static_tr(key);
            let prefix = localized
                .split_once("{0}")
                .map_or(localized.as_str(), |(head, _)| head);
            problem = problem || text.starts_with(prefix);
        }
        problem
            || text == Self::static_tr("invalid_source")
            || text == Self::static_tr("no_source_selected")
            || text == Self::static_tr("scenes_snapshot_no_scene")
    }

    /// Service status fields also carry idle states ("Server stopped"); only
    /// error-flavored texts are surfaced globally.
    fn service_status_is_problem(text: &str) -> bool {
        // "Could not start server: {0}" — match the fixed prefix.
        let obs_error = Self::static_tr("obs_ws_error");
        let obs_prefix = obs_error
            .split_once("{0}")
            .map_or(obs_error.as_str(), |(head, _)| head);
        text == Self::static_tr("remote_companion_lan_requires_password")
            || text.starts_with(obs_prefix)
    }

    /// Locale-independent translation for classification helpers (the
    /// aggregation must classify the same way regardless of the active UI
    /// language, because the stored texts are localized with the locale at
    /// set time).
    fn static_tr(key: &str) -> String {
        Locale::En.tr(key).to_owned()
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
                    // Reduced motion collapses a Fade take to a Cut (ui-005):
                    // reuse the same resolution as a direct scene switch.
                    let take_kind = if self.motion_applied.unwrap_or(false) {
                        rivulet_core::TransitionKind::Cut
                    } else {
                        self.transition_kind
                    };
                    if let Some(transition) = self.studio_mode.take(
                        take_kind,
                        std::time::Duration::from_millis(self.transition_duration_ms),
                        now,
                    ) {
                        if let Some(target) = self.studio_mode.program() {
                            let from = self.scenes.active();
                            if self.switch_active_scene(target) {
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
            if switched_back && self.switch_scene_back() {
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
                } else if self.switch_active_scene(id) {
                    let now = Instant::now();
                    self.scene_transition = self.scene_transition_for_switch(active, Some(id), now);
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

    /// Create a scene source from the add dialog, applying the picked device id
    /// when the kind supports a device picker.
    fn add_scene_source_from_dialog(
        &mut self,
        scene_id: uuid::Uuid,
        kind: &rivulet_core::SourceKind,
    ) -> uuid::Uuid {
        let name = if self.source_name_input.trim().is_empty() {
            format!(
                "{} {}",
                kind.label(),
                self.source_manager.sources().len() + 1
            )
        } else {
            self.source_name_input.trim().to_string()
        };
        let mut source = rivulet_core::Source::new(name, kind.clone());
        if let Some(device_id) = self.scene_device_id_for(kind) {
            self.scene_status = Some(self.tr_fmt(
                "composition_device_attached",
                std::slice::from_ref(&device_id),
            ));
            source = source.with_device_id(device_id);
        }
        let id = self.source_manager.add_source(source);
        self.source_manager.bind_source(id, scene_id, None);
        self.selected_composition_source = Some(id);
        self.source_name_input.clear();
        id
    }

    /// Draw the device picker for the currently selected scene source kind.
    ///
    /// Capture sources (webcam, game capture, screen capture) pick a concrete
    /// OS device from the same lists the record view uses; the choice is
    /// applied to the new source's `device_id` on add.
    fn draw_scene_device_picker(
        &mut self,
        ui: &mut egui::Ui,
        kind: &rivulet_core::SourceKind,
        colors: &theme::StatusColors,
    ) {
        let entries = self.scene_device_entries(kind);
        if entries.is_empty() {
            ui.colored_label(colors.hint, self.tr("composition_device_none"));
            return;
        }
        self.selected_scene_device_idx = self
            .selected_scene_device_idx
            .filter(|idx| *idx < entries.len());
        let selected_label = self
            .selected_scene_device_idx
            .and_then(|idx| entries.get(idx))
            .map(|(label, _)| label.as_str())
            .unwrap_or_else(|| self.tr("composition_device_default"));
        egui::ComboBox::from_id_salt("scene_device_select")
            .selected_text(selected_label)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        self.selected_scene_device_idx.is_none(),
                        self.tr("composition_device_default"),
                    )
                    .clicked()
                {
                    self.selected_scene_device_idx = None;
                }
                for (index, (label, _)) in entries.iter().enumerate() {
                    if ui
                        .selectable_label(
                            self.selected_scene_device_idx == Some(index),
                            label.as_str(),
                        )
                        .clicked()
                    {
                        self.selected_scene_device_idx = Some(index);
                    }
                }
            });
    }

    /// Resolve the device id selected in the scene dialog, if any.
    fn scene_device_id_for(&self, kind: &rivulet_core::SourceKind) -> Option<String> {
        let entries = self.scene_device_entries(kind);
        self.selected_scene_device_idx
            .and_then(|idx| entries.get(idx))
            .map(|(_, id)| id.clone())
    }

    /// List `(label, device_id)` for the current source kind using the same
    /// device tables as the record view.
    fn scene_device_entries(&self, kind: &rivulet_core::SourceKind) -> Vec<(String, String)> {
        match kind {
            rivulet_core::SourceKind::Webcam => self
                .camera_devices
                .iter()
                .map(|camera| {
                    let id = if camera.device_path.is_empty() {
                        camera.element_factory.clone()
                    } else {
                        camera.device_path.clone()
                    };
                    (camera.name.clone(), format!("camera:{id}"))
                })
                .collect(),
            rivulet_core::SourceKind::GameCapture => self
                .game_windows
                .iter()
                .map(|window| (window.title.clone(), format!("game:{}", window.id)))
                .collect(),
            rivulet_core::SourceKind::ScreenCapture => self.scene_monitor_entries(),
            _ => Vec::new(),
        }
    }

    #[cfg(target_os = "windows")]
    fn scene_monitor_entries(&self) -> Vec<(String, String)> {
        self.monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                let label = monitor
                    .name()
                    .ok()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("Monitor {}", index + 1));
                let id = format!(
                    "monitor:{}",
                    monitor
                        .index()
                        .map(|i| i.to_string())
                        .unwrap_or_else(|_| index.to_string())
                );
                (label, id)
            })
            .collect()
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn scene_monitor_entries(&self) -> Vec<(String, String)> {
        self.monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                let label = monitor
                    .name()
                    .ok()
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| format!("Monitor {}", index + 1));
                let id = format!(
                    "monitor:{}",
                    monitor
                        .id()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|_| index.to_string())
                );
                (label, id)
            })
            .collect()
    }

    /// Export the active scene (or Studio Mode Preview) as a deterministic PNG.
    ///
    /// The exported image represents the current scene layout. Native browser
    /// pixels (if a frame is available) are composited into Browser layers;
    /// the remaining source kinds fall back to their stable tile color until
    /// their platform renderers are wired into the snapshot pipeline.
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
        let mut snapshot = rivulet_core::SceneSnapshot::from_scene(
            scene,
            &self.source_manager,
            self.scenes.collection(),
            self.scenes.profile(),
        );
        // Attach native browser pixels (if the browser source has produced a
        // frame) so exported snapshots render real content for Browser layers.
        snapshot.attach_frames(&|_source_id| {
            self.browser_source.latest_frame().and_then(|frame| {
                rivulet_core::SnapshotFrame::new(frame.width, frame.height, frame.rgba.clone())
            })
        });
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
        let selected_kind = &kinds[self.source_kind_index];
        ui.horizontal(|ui| {
            ui.label(self.tr("composition_add_source"));
            ui.text_edit_singleline(&mut self.source_name_input);
            egui::ComboBox::from_id_salt("composition_source_kind")
                .selected_text(selected_kind.label())
                .show_ui(ui, |ui| {
                    for (index, kind) in kinds.iter().enumerate() {
                        if ui
                            .selectable_label(self.source_kind_index == index, kind.label())
                            .clicked()
                        {
                            self.source_kind_index = index;
                            // A different capture kind has its own device list.
                            if !rivulet_core::Source::supports_device_picker(kind) {
                                self.selected_scene_device_idx = None;
                            }
                        }
                    }
                });
            if rivulet_core::Source::supports_device_picker(selected_kind) {
                self.draw_scene_device_picker(ui, selected_kind, colors);
            }
            if theme::accent_button(ui, self.tr("composition_add")).clicked() {
                self.add_scene_source_from_dialog(scene_id, selected_kind);
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
                        if theme::accent_button(ui, self.tr("composition_copy")).clicked() {
                            self.copy_selected_composition_source();
                        }
                        if theme::accent_button(ui, self.tr("composition_paste")).clicked() {
                            self.paste_scene_item_clipboard();
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
                        if theme::accent_button(ui, self.tr("composition_delete_source"))
                            .on_hover_text(self.tr("composition_delete_hint"))
                            .clicked()
                        {
                            self.delete_selected_composition_source();
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
    /// Ensure the native webview backend exists (issue #227).
    ///
    /// Spawns the wry/WebView2 adapter on Windows exactly once; a failed
    /// spawn sets [`Self::browser_backend_failed`] so the tick never retries
    /// (and never spams the status line). On other platforms nothing happens
    /// and the panel keeps showing the pending preview.
    fn ensure_browser_backend(&mut self) {
        if self.browser_backend.is_some() || self.browser_backend_failed {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            match rivulet_browser::WryBrowserBackend::spawn() {
                Ok(backend) => self.browser_backend = Some(Box::new(backend)),
                Err(error) => {
                    tracing::warn!("native browser backend unavailable: {error}");
                    self.browser_backend_failed = true;
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            // Native adapter is Windows-only for now.
            self.browser_backend_failed = true;
        }
    }

    /// Run one backend tick: push changed settings, drain queued input, and
    /// poll the next RGBA frame into `browser_source`.
    ///
    /// Returns a status message when the backend reported an error; the
    /// caller (the Scenes view panel) surfaces it via `scene_status`. The
    /// poll is non-blocking — the wry backend only `try_recv`s a frame — so
    /// this is safe to call every frame.
    fn tick_browser_source(&mut self) -> Result<(), rivulet_core::BrowserSourceError> {
        self.ensure_browser_backend();
        let Some(backend) = self.browser_backend.as_deref_mut() else {
            return Ok(());
        };
        rivulet_core::sync_browser_backend(
            backend,
            &self.browser_source,
            &mut self.browser_applied,
        )?;
        for input in self.browser_source.take_input_events() {
            backend.send_input(input)?;
        }
        if let Some(frame) = backend.poll_frame()? {
            self.browser_source.submit_frame(frame)?;
        }
        Ok(())
    }

    /// Upload the latest browser frame to an egui texture (no-op when the
    /// backend has not produced one yet or the texture is already current).
    fn upload_browser_preview(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.browser_source.latest_frame() else {
            return;
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.rgba,
        );
        match &mut self.browser_preview_texture {
            Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.browser_preview_texture =
                    Some(ctx.load_texture("browser_preview", image, egui::TextureOptions::LINEAR));
            }
        }
    }

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
        // Run one backend tick (settings sync, input drain, frame poll) and
        // refresh the preview texture before drawing it (issue #227).
        if let Err(error) = self.tick_browser_source() {
            self.scene_status = Some(error.to_string());
        }
        self.upload_browser_preview(ui.ctx());
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
        // Live preview of the browser surface at the configured viewport.
        if let Some(texture) = &self.browser_preview_texture {
            let (width, height) = (
                self.browser_source.width as f32,
                self.browser_source.height as f32,
            );
            let available = ui.available_width();
            let scale = (available / width).min(1.0);
            let size = egui::vec2(width * scale, height * scale);
            ui.image((texture.id(), size));
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
        #[cfg(target_os = "macos")]
        if self.is_recording {
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
        } else if let Some(idx) = self.selected_window_idx_for_preview() {
            self.preview_window_target(idx)
        } else if let Some(idx) = self.selected_monitor_idx_for_preview() {
            self.preview_monitor_target(idx)
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
        // Inline per-source mixer strip (issue #154 Phase 2): recording
        // start/stop never requires a view switch. The routing matrix stays
        // in the Mixer view; here the sources show volume/mute + badges.
        self.draw_inline_audio_mixer(ui);
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
    /// Draw the restream targets section: a collapsible list of additional
    /// platforms to stream to simultaneously, with add/remove controls.
    fn draw_restream_section(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(self.tr("restream_section"))
            .id_salt("restream_targets")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(self.tr("restream_hint"));

                let target_count = self.restream_targets.len();
                // Show existing targets using index-based access to avoid
                // borrow conflicts between iter_mut() and self.tr().
                if target_count == 0 {
                    ui.label(self.tr("restream_no_targets"));
                } else {
                    let mut remove_idx: Option<usize> = None;
                    for idx in 0..target_count {
                        let platform = self.restream_targets[idx].platform;
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.checkbox(&mut self.restream_targets[idx].enabled, "");
                                ui.label(self.tr("restream_target_name"));
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.restream_targets[idx].name,
                                    )
                                    .desired_width(100.0),
                                );
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label(self.tr("restream_target_platform"));
                                egui::ComboBox::from_id_salt(format!("restream_platform_{idx}"))
                                    .selected_text(platform.label())
                                    .show_ui(ui, |ui| {
                                        for p in [
                                            StreamPlatform::Twitch,
                                            StreamPlatform::YouTube,
                                            StreamPlatform::Kick,
                                            StreamPlatform::Custom,
                                        ] {
                                            if ui
                                                .selectable_label(platform == p, p.label())
                                                .clicked()
                                            {
                                                self.restream_targets[idx].platform = p;
                                                if let Some(url) = p.default_ingest_url() {
                                                    self.restream_targets[idx].ingest_url =
                                                        url.to_owned();
                                                }
                                            }
                                        }
                                    });
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label(self.tr("restream_target_url"));
                                ui.add_enabled(
                                    platform.requires_manual_ingest_url(),
                                    egui::TextEdit::singleline(
                                        &mut self.restream_targets[idx].ingest_url,
                                    )
                                    .desired_width(200.0),
                                );
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label(self.tr("restream_target_key"));
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.restream_targets[idx].stream_key,
                                    )
                                    .password(true),
                                );
                                if ui.button(self.tr("restream_remove_target")).clicked() {
                                    remove_idx = Some(idx);
                                }
                            });
                        });
                    }
                    if let Some(idx) = remove_idx {
                        let removed = self.restream_targets.remove(idx);
                        self.restream_status =
                            Some(self.tr_fmt("restream_target_removed", &[removed.name]));
                    }
                }

                // Add target button
                if target_count < rivulet_core::MultistreamSettings::MAX_TARGETS {
                    if ui.button(self.tr("restream_add_target")).clicked() {
                        let n = target_count + 1;
                        self.restream_targets.push(RestreamTargetConfig {
                            name: format!("Target {n}"),
                            ..Default::default()
                        });
                        self.restream_status =
                            Some(self.tr_fmt("restream_target_added", &[format!("Target {n}")]));
                    }
                } else {
                    ui.label(self.tr_fmt(
                        "restream_max_targets",
                        &[rivulet_core::MultistreamSettings::MAX_TARGETS.to_string()],
                    ));
                }

                if let Some(status) = &self.restream_status {
                    ui.label(status.as_str());
                }
            });
    }

    /// Draw the auto-clip section: toggle, threshold, command name, and
    /// status display for chat-driven replay saves.
    fn draw_auto_clip_section(&mut self, ui: &mut egui::Ui) {
        // Pre-compute translated strings to avoid borrow conflicts with
        // mutable access to auto_clip_config.
        let section_title = self.tr("autoclip_section").to_owned();
        let hint = self.tr("autoclip_hint").to_owned();
        let label_enabled = self.tr("autoclip_enabled").to_owned();
        let label_threshold = self.tr("autoclip_spike_threshold").to_owned();
        let label_window = self.tr("autoclip_spike_window").to_owned();
        let label_cooldown = self.tr("autoclip_cooldown").to_owned();
        let label_command = self.tr("autoclip_command").to_owned();
        let status = self.auto_clip_status.clone();

        egui::CollapsingHeader::new(&section_title)
            .id_salt("auto_clip")
            .default_open(false)
            .show(ui, |ui| {
                ui.label(&hint);
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.auto_clip_config.enabled, &label_enabled);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(&label_threshold);
                    ui.add(
                        egui::DragValue::new(&mut self.auto_clip_config.spike_threshold)
                            .range(1..=100),
                    );
                    ui.label(&label_window);
                    let mut window_secs = self.auto_clip_config.window.as_secs();
                    ui.add(
                        egui::DragValue::new(&mut window_secs)
                            .range(1..=300)
                            .suffix("s"),
                    );
                    self.auto_clip_config.window = std::time::Duration::from_secs(window_secs);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(&label_cooldown);
                    let mut cooldown_secs = self.auto_clip_config.cooldown.as_secs();
                    ui.add(
                        egui::DragValue::new(&mut cooldown_secs)
                            .range(1..=300)
                            .suffix("s"),
                    );
                    self.auto_clip_config.cooldown = std::time::Duration::from_secs(cooldown_secs);
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(&label_command);
                    ui.add(
                        egui::TextEdit::singleline(&mut self.auto_clip_config.clip_command)
                            .desired_width(80.0),
                    );
                });
                // Sync detector config when user changes settings
                self.auto_clip_detector
                    .set_config(self.auto_clip_config.clone());
                if let Some(s) = &status {
                    ui.label(s.as_str());
                }
            });
    }

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

    /// Apply the configured NDI monitor feed to the engine. Called before
    /// every recording/streaming start so the feed branch is part of the
    /// pipeline, and whenever the user changes the setting. An empty source
    /// name cannot be enabled; the warning text is shown in Settings.
    fn apply_ndi_output(&mut self) {
        if self.ndi_output_enabled && self.ndi_output_name.trim().is_empty() {
            self.ndi_warning = Some(self.tr("ndi_name_required").to_string());
            let _ = self.engine.set_ndi_output(None);
            return;
        }
        let output = if self.ndi_output_enabled {
            let mut output = rivulet_core::NdiOutput::new(self.ndi_output_name.trim(), true);
            let group = self.ndi_output_group.trim();
            if !group.is_empty() {
                output = output.with_group(group);
            }
            Some(output)
        } else {
            None
        };
        match self.engine.set_ndi_output(output) {
            Ok(()) => self.ndi_warning = None,
            Err(e) => self.ndi_warning = Some(e.to_string()),
        }
    }

    /// Apply configured restream targets to the engine before streaming
    /// starts. Builds a [`rivulet_core::MultistreamSettings`] from the GUI
    /// target list and passes it to the engine, or clears it when no
    /// targets are enabled.
    fn apply_restream_targets(&mut self) {
        let targets: Vec<rivulet_core::StreamTarget> = self
            .restream_targets
            .iter()
            .filter_map(|cfg| cfg.to_stream_target())
            .collect();
        if targets.is_empty() {
            self.engine.set_multistream_settings(None);
        } else {
            let mut ms = rivulet_core::MultistreamSettings::default();
            for target in targets {
                let _ = ms.add_target(target);
            }
            self.engine.set_multistream_settings(Some(ms));
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
        self.save_replay_to(path);
    }

    /// Save the buffered replay clip to an explicit path and report the
    /// outcome (localized) in `replay_status`. Shared by the GUI button and
    /// the OBS WebSocket replay requests (`SaveReplayBuffer`).
    fn save_replay_to(
        &mut self,
        path: std::path::PathBuf,
    ) -> rivulet_obs_websocket::ObsCommandResult {
        match self.engine.save_replay(path.clone()) {
            Ok(_) => {
                self.replay_status = Some((
                    true,
                    self.tr_fmt("replay_saved", &[path.display().to_string()]),
                ));
                rivulet_obs_websocket::ObsCommandResult::Success(vec![
                    rivulet_obs_websocket::ObsEvent::ReplayBufferSaved {
                        saved_replay_path: Some(path.display().to_string()),
                    },
                ])
            }
            Err(e) => {
                self.replay_status =
                    Some((false, self.tr_fmt("replay_save_failed", &[e.to_string()])));
                rivulet_obs_websocket::ObsCommandResult::Failure {
                    status_code: rivulet_obs_websocket::protocol::status::REQUEST_PROCESSING_FAILED,
                    comment: e.to_string(),
                }
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
    /// Store the current chat connection state, reporting a `ChatConnect`
    /// telemetry event on every transition into a terminal state: reaching
    /// `Connected` records `ok: true`, losing the connection records
    /// `ok: false`. `Off` (worker not running) is not a connection attempt
    /// and is never reported. Only transitions emit, so a steady-state
    /// connection does not flood the batch.
    fn apply_chat_state(&mut self, state: rivulet_core::ChatConnState) {
        if state == self.chat_state {
            return;
        }
        match state {
            rivulet_core::ChatConnState::Connected => {
                self.telemetry
                    .record(rivulet_core::TelemetryEvent::ChatConnect { ok: true });
            }
            rivulet_core::ChatConnState::Disconnected => {
                self.telemetry
                    .record(rivulet_core::TelemetryEvent::ChatConnect { ok: false });
            }
            rivulet_core::ChatConnState::Off => {}
        }
        self.chat_state = state;
    }

    fn reconcile_chat(&mut self) {
        match self.chat_action_pending.take() {
            Some(ChatAction::Connect) => {
                if let Some(mut multi) = self.chat_worker_multi.take() {
                    multi.disconnect_all();
                }
                self.chat_messages.clear();
                self.alert_events.clear();
                self.chat_reply_target = None;
                self.chat_last_send_outcomes.clear();
                // Tokens live only in the OS credential vault — the roster
                // (and every struct derived from it) stays token-free, which
                // is the CodeQL rust/cleartext-logging root fix. The
                // resolver reads the vault at spawn time; a missing entry
                // yields "" and the worker connects read-only/anonymous.
                let store = rivulet_core::ChatTokenStore::default();
                let multi =
                    rivulet_core::MultiChat::new(&self.chat_accounts, |platform, channel| {
                        store
                            .load(platform, channel)
                            .unwrap_or(None)
                            .unwrap_or_default()
                    });
                self.apply_chat_state(multi.connection_state());
                self.chat_worker_multi = Some(multi);
            }
            Some(ChatAction::Disconnect) => {
                if let Some(mut worker) = self.chat_worker_multi.take() {
                    worker.disconnect_all();
                }
                self.chat_messages.clear();
                self.alert_events.clear();
                self.chat_reply_target = None;
                self.chat_last_send_outcomes.clear();
                self.apply_chat_state(rivulet_core::ChatConnState::Off);
            }
            Some(ChatAction::Send(text)) => {
                self.send_chat_message(text);
            }
            Some(ChatAction::SendReply(text, parent, platform)) => {
                self.send_chat_reply(text, parent, platform);
            }
            None => {}
        }

        // Drain every worker's message stream (bounded to the newest
        // MAX_CHAT_MESSAGES). Each message carries its platform tag, so the
        // combined dock needs no per-receiver bookkeeping.
        let worker_state = self
            .chat_worker_multi
            .as_ref()
            .map(|w| w.connection_state())
            .unwrap_or(rivulet_core::ChatConnState::Off);
        if let Some(multi) = &self.chat_worker_multi {
            for rx in multi.messages() {
                while let Ok(msg) = rx.try_recv() {
                    self.chat_messages.push(msg);
                    if self.chat_messages.len() > MAX_CHAT_MESSAGES {
                        let overflow = self.chat_messages.len() - MAX_CHAT_MESSAGES;
                        self.chat_messages.drain(..overflow);
                    }
                }
            }
        }
        self.apply_chat_state(worker_state);

        // Drain a finished shared-chat room-name lookup (non-blocking):
        // results land in the cache, the badge self-improves next frame.
        if let Some(rx) = &self.chat_room_name_rx {
            match rx.try_recv() {
                Ok(resolved) => {
                    self.chat_room_names.apply_results(resolved);
                    self.chat_room_name_rx = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The thread died without sending (panic) — unblock.
                    self.chat_room_names.release_in_flight();
                    self.chat_room_name_rx = None;
                }
            }
        }

        // Drain the stream-info editor: collect the per-platform outcomes
        // of a finished background apply (non-blocking poll).
        if let Some(rx) = &self.chat_info_rx {
            match rx.try_recv() {
                Ok(outcomes) => {
                    self.chat_info_outcomes = outcomes;
                    self.chat_info_rx = None;
                    self.chat_info_busy = false;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The thread died without sending (panic) — unblock.
                    self.chat_info_rx = None;
                    self.chat_info_busy = false;
                    self.chat_info_error = Some(self.tr("chat_info_thread_failed").to_owned());
                }
            }
        }

        // Keep the loopback webhook receiver in sync with the persisted
        // settings (start/restart/stop), then drain its parsed alert events
        // into the local queue (respecting the queue's enabled state).
        self.apply_alerts_receiver();
        if let Some(receiver) = &self.alerts_receiver {
            while let Ok(event) = receiver.events().try_recv() {
                self.alert_ingest
                    .push_from(rivulet_core::AlertSource::StreamlabsWebhook, event);
            }
        }

        // Keep the outbound EventSub worker in sync with the persisted
        // settings (start/restart/stop), then drain its pushed alert events
        // into the same local queue (respecting the queue's enabled state).
        self.apply_alerts_eventsub();
        if let Some(eventsub) = &self.alerts_eventsub {
            while let Ok(event) = eventsub.events().try_recv() {
                self.alert_ingest
                    .push_from(rivulet_core::AlertSource::TwitchEventSub, event);
            }
        }

        // Drain engagement events from the chat workers (Kick subs/gifts and
        // YouTube Super Chats parsed from their respective streams) into the
        // same local queue, so the alerts dock shows them next to the
        // EventSub/Streamlabs lines. Each platform is its own rate-limit
        // source, so one spamming platform cannot starve the others.
        if let Some(multi) = &self.chat_worker_multi {
            for (account, rx) in multi.alert_receivers_by_platform() {
                let source = match account.platform {
                    rivulet_core::ChatPlatform::Kick => rivulet_core::AlertSource::Kick,
                    rivulet_core::ChatPlatform::YouTube => rivulet_core::AlertSource::YouTube,
                    rivulet_core::ChatPlatform::Twitch => rivulet_core::AlertSource::TwitchEventSub,
                };
                while let Ok(event) = rx.try_recv() {
                    self.alert_ingest.push_from(source, event);
                }
            }
        }

        // Surface pending local alert events in the chat dock, newest first.
        // Ingestion is local by default; the preview button and the loopback
        // webhook receiver feed the same queue.
        if self.alert_preview_dirty {
            self.queue_alert_preview();
            self.alert_preview_dirty = false;
        }
        // Drain once, surface twice: the combined chat dock keeps showing
        // alerts as action lines, and the dedicated alerts dock accumulates
        // the same events in its own bounded live list (both lists newest
        // last, bottom-anchored). Suppression notices from the per-source
        // rate limiter render as their own line in both docks, so a spam
        // burst is visibly throttled instead of silently vanishing.
        for entry in self.alert_ingest.drain_with_sources() {
            let message = match entry {
                rivulet_core::AlertQueueEntry::Event(event) => {
                    self.alert_event_to_chat_message(&event)
                }
                rivulet_core::AlertQueueEntry::Suppressed(notice) => {
                    self.alert_suppression_to_chat_message(&notice)
                }
            };
            self.chat_messages.push(message.clone());
            if self.chat_messages.len() > MAX_CHAT_MESSAGES {
                let overflow = self.chat_messages.len() - MAX_CHAT_MESSAGES;
                self.chat_messages.drain(..overflow);
            }
            self.alert_events.push(message);
            if self.alert_events.len() > MAX_ALERT_EVENTS {
                let overflow = self.alert_events.len() - MAX_ALERT_EVENTS;
                self.alert_events.drain(..overflow);
            }
        }
    }

    /// Render a localized chat-dock line for an alert event. The line carries
    /// the acting viewer's name, a provider-neutral description and (for
    /// donations) the amount; everything stays in the local list and is never
    /// persisted or serialized (no data leaves the app).
    fn alert_event_to_chat_message(
        &self,
        event: &rivulet_core::AlertEvent,
    ) -> rivulet_core::ChatMessage {
        use rivulet_core::AlertKind;
        let kind = event.kind.i18n_key();
        let text = match event.kind {
            AlertKind::Follow => self.tr_fmt(kind, std::slice::from_ref(&event.user)),
            AlertKind::Subscribe => {
                let tier = event.tier.clone().unwrap_or_default();
                self.tr_fmt(kind, &[event.user.clone(), tier])
            }
            AlertKind::GiftSub => {
                let count = event.count.to_string();
                self.tr_fmt(kind, &[event.user.clone(), count])
            }
            AlertKind::Donation => {
                let amount = match (event.amount, event.currency.as_deref()) {
                    (Some(a), Some(c)) => format!("{a:.2} {c}"),
                    (Some(a), None) => format!("{a:.2}"),
                    (None, Some(c)) => c.to_owned(),
                    (None, None) => String::new(),
                };
                self.tr_fmt(kind, &[event.user.clone(), amount])
            }
            AlertKind::Raid => {
                let viewers = event.count.to_string();
                self.tr_fmt(kind, &[event.user.clone(), viewers])
            }
        };
        rivulet_core::ChatMessage {
            user: event.user.clone(),
            text,
            action: true,
            color: Some("#e0a458".to_owned()),
            badges: Vec::new(),
            broadcaster: false,
            id: None,
            timestamp: event.timestamp,
            platform: event.platform,
            source_room_id: None,
        }
    }

    /// Render the per-source rate-limit notice as a neutral dock line. The
    /// platform badge is deliberately `None`: the notice is about a delivery
    /// channel (which may aggregate providers), so the source name is spelled
    /// out in the text instead.
    fn alert_suppression_to_chat_message(
        &self,
        notice: &rivulet_core::AlertSuppressionNotice,
    ) -> rivulet_core::ChatMessage {
        let text = self.tr_fmt(
            "alerts_rate_limited",
            &[
                notice.source.display_name().to_owned(),
                notice.suppressed.to_string(),
            ],
        );
        rivulet_core::ChatMessage {
            user: "rivulet".to_owned(),
            text,
            action: true,
            color: Some("#8a8f98".to_owned()),
            badges: Vec::new(),
            broadcaster: false,
            id: None,
            timestamp: 0,
            platform: None,
            source_room_id: None,
        }
    }

    /// Push one deterministic sample of every alert kind into the local queue
    /// (chat-dock "Preview" button) so the surfacing can be verified without a
    /// live stream and the wiring is GUI-testable.
    fn queue_alert_preview(&mut self) {
        let events = vec![
            rivulet_core::AlertEvent::sample_follow(),
            rivulet_core::AlertEvent {
                kind: rivulet_core::AlertKind::Subscribe,
                user: "SubFan".to_owned(),
                tier: Some("Tier 3".to_owned()),
                count: 12,
                ..rivulet_core::AlertEvent::sample_follow()
            },
            rivulet_core::AlertEvent {
                kind: rivulet_core::AlertKind::GiftSub,
                user: "Gifter".to_owned(),
                count: 5,
                ..rivulet_core::AlertEvent::sample_follow()
            },
            rivulet_core::AlertEvent {
                kind: rivulet_core::AlertKind::Donation,
                user: "Donor".to_owned(),
                amount: Some(20.0),
                currency: Some("EUR".to_owned()),
                message: Some("for the next stream".to_owned()),
                ..rivulet_core::AlertEvent::sample_follow()
            },
            rivulet_core::AlertEvent {
                kind: rivulet_core::AlertKind::Raid,
                user: "RaidLeader".to_owned(),
                count: 42,
                ..rivulet_core::AlertEvent::sample_follow()
            },
        ];
        self.alert_ingest.set_enabled(true);
        for event in events {
            self.alert_ingest
                .push_from(rivulet_core::AlertSource::LocalPreview, event);
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
        if text.is_empty() {
            return false;
        }
        let Some(multi) = &self.chat_worker_multi else {
            return false;
        };
        let outcomes = multi.send_message(&text);
        let any_ok = outcomes.iter().any(|(_, ok)| *ok);
        self.chat_last_send_outcomes = outcomes;
        any_ok
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
            Some((_, platform, parent)) => {
                self.chat_action_pending =
                    Some(ChatAction::SendReply(text.to_owned(), parent, platform));
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
    /// reply non-blocking and that platform's rate limiter applies. The
    /// platform comes with the armed reply target, so the reply always
    /// lands where the parent message was written.
    fn send_chat_reply(
        &mut self,
        text: String,
        parent_id: String,
        platform: rivulet_core::ChatPlatform,
    ) -> bool {
        let text = text.trim().to_owned();
        let parent_id = parent_id.trim().to_owned();
        if text.is_empty() || parent_id.is_empty() {
            return false;
        }
        let Some(multi) = &self.chat_worker_multi else {
            return false;
        };
        multi.send_reply(&text, &parent_id, platform)
    }

    /// Live rate-limit detail `(remaining, capacity, window_secs, platform
    /// label)` of the running worker's shared limiter, for the budget line
    /// and its tooltip (“20 messages per 30 s on Twitch”). `None` while no
    /// worker is running. `remaining` is fractional (tokens refill
    /// continuously over the window); the UI floors it.
    fn chat_rate_limit_detail(&self) -> Option<(f64, u32, u64, &'static str)> {
        let multi = self.chat_worker_multi.as_ref()?;
        // The combined dock shows the tightest budget across accounts so a
        // near-empty bucket on one platform still warns before sends drop.
        let mut best: Option<(f64, u32, u64, &'static str)> = None;
        let mut seen: Vec<rivulet_core::ChatPlatform> = Vec::new();
        for (platform, _state) in multi.connection_states() {
            if seen.contains(&platform) {
                continue;
            }
            seen.push(platform);
            let Some((remaining, capacity, window)) = multi.rate_limit_detail(platform) else {
                continue;
            };
            let better = match best {
                None => true,
                Some((r, _, _, _)) => remaining < r,
            };
            if better {
                best = Some((remaining, capacity, window, platform.label()));
            }
        }
        best
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
    ///
    /// `delete_source` is deliberately NOT registered here: it is a destructive,
    /// repeat-sensitive action (OBS keeps it app-local too), and a global Delete
    /// would fire while the user types Delete in any other application.
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
                    self.switch_active_scene(id);
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
                self.switch_active_scene(*id);
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
        // stop it when the toggle is off. Port/password/bind edits are
        // coalesced into a single restart via `obs_ws_restart`.
        if self.obs_ws_enabled {
            if self.obs_ws_server.is_none() {
                self.start_obs_websocket();
            } else if self.obs_gw_restart_requested() {
                self.obs_ws_server = None;
                self.obs_ws_commands_rx = None;
                self.start_obs_websocket();
            }
        } else if self.obs_ws_server.is_some() {
            tracing::info!("Stopping OBS WebSocket server");
            self.obs_ws_server = None;
            self.obs_ws_commands_rx = None;
            self.obs_ws_snapshot = None;
            self.obs_ws_status = Some(self.tr("obs_ws_stopped").to_owned());
        }
        // The restart request is consumed exactly once a frame; a fresh edit
        // flips it again.
        self.obs_ws_restart = false;

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
                    if current.muted != last.muted {
                        events.push(rivulet_obs_websocket::ObsEvent::InputMuteStateChanged {
                            input_name: self.obs_ws_primary_input_name(),
                            muted: current.muted,
                        });
                    }
                    if current.replay_buffer_active != last.replay_buffer_active {
                        events.push(rivulet_obs_websocket::ObsEvent::ReplayBufferStateChanged {
                            active: current.replay_buffer_active,
                        });
                    }
                    if current.studio_mode != last.studio_mode {
                        events.push(rivulet_obs_websocket::ObsEvent::StudioModeStateChanged {
                            enabled: current.studio_mode,
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
        // The companion's "reachable from the LAN" setting widens the obs
        // server bind too: a phone drives scenes/record/stream through this
        // same v5 surface, so both listeners must be reachable together.
        let bind = if self.remote_companion_enabled && self.remote_companion_bind_lan {
            rivulet_obs_websocket::BindAddress::All
        } else {
            rivulet_obs_websocket::BindAddress::Loopback
        };
        let options = rivulet_obs_websocket::ServerOptions {
            port: self.obs_ws_port,
            bind,
            password,
            allow_remote_stream_control: self.remote_allow_stream_control,
        };
        // Friendly pre-check for the LAN-needs-password rule before the crate
        // refuses the bind (the crate enforces it too; this message is more
        // actionable in the Settings view).
        if bind == rivulet_obs_websocket::BindAddress::All && self.obs_ws_password.is_empty() {
            self.obs_ws_server = None;
            self.obs_ws_commands_rx = None;
            self.obs_ws_status = Some(self.tr("remote_companion_lan_requires_password").to_owned());
            self.remote_companion_status =
                Some(self.tr("remote_companion_lan_requires_password").to_owned());
            return;
        }
        match rivulet_obs_websocket::server::start_with_options(backend, options) {
            Ok(server) => {
                self.obs_ws_server = Some(server);
                self.obs_ws_commands_rx = Some(cmd_rx);
                if bind == rivulet_obs_websocket::BindAddress::All {
                    self.obs_ws_status = Some(self.tr("obs_ws_running_lan").to_owned());
                } else {
                    self.obs_ws_status =
                        Some(self.tr_fmt("obs_ws_running", &[self.obs_ws_port.to_string()]));
                }
                tracing::info!(
                    port = self.obs_ws_port,
                    lan = bind.is_lan(),
                    "OBS WebSocket server started"
                );
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

    /// Whether the companion page is configured to be reachable from the LAN.
    /// The obs-websocket server shares this decision (the page drives scenes
    /// through it), so the bind must widen for both together.
    fn companion_reaches_lan(&self) -> bool {
        self.remote_companion_enabled && self.remote_companion_bind_lan
    }

    /// Start, keep, or stop the companion HTTP page server in sync with the
    /// settings. The page is useless without the OBS WebSocket server (it
    /// connects there over WebSocket), so it only runs while that server is
    /// enabled — the status line says so instead of silently dying.
    fn reconcile_remote_companion(&mut self) {
        if self.remote_companion_enabled && self.obs_ws_enabled {
            if self.remote_companion_server.is_none() {
                self.start_remote_companion();
            }
        } else if self.remote_companion_server.is_some() {
            tracing::info!("Stopping remote companion page server");
            self.remote_companion_server = None;
            self.remote_companion_url = None;
            self.remote_companion_status = Some(self.tr("remote_companion_stopped").to_owned());
        } else if self.remote_companion_enabled && !self.obs_ws_enabled {
            // Honest dependency hint: the page requires the obs server above.
            self.remote_companion_status =
                Some(self.tr("remote_companion_requires_obs_ws").to_owned());
        }
    }

    /// Attempt to start the companion HTTP page server with the current
    /// settings. On failure a status message is stored for the Settings view.
    /// Secrets are never logged: no bodies, no passwords, no identifiers.
    fn start_remote_companion(&mut self) {
        // LAN binding requires a password on the obs-websocket server; both
        // the obs server (crate) and the page (config endpoint) are gated by
        // it. Refuse with an actionable message instead of exposing an
        // unauthenticated network surface.
        if self.companion_reaches_lan() && self.obs_ws_password.is_empty() {
            self.remote_companion_status =
                Some(self.tr("remote_companion_lan_requires_password").to_owned());
            return;
        }
        let bind_address = if self.companion_reaches_lan() {
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        } else {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        };
        let config = rivulet_obs_websocket::CompanionConfig {
            bind_address,
            port: self.remote_companion_port,
            ws_port: self.obs_ws_port,
            ws_auth_required: !self.obs_ws_password.is_empty(),
        };
        match rivulet_obs_websocket::companion::start(config) {
            Ok(server) => {
                // "Open page" always targets loopback (works on this machine);
                // the displayed status mentions the LAN URL when reachable.
                let url = self.remote_companion_page_url();
                self.remote_companion_server = Some(server);
                self.remote_companion_url = Some(url.clone());
                self.remote_companion_status = if self.companion_reaches_lan() {
                    Some(self.tr_fmt(
                        "remote_companion_running_lan",
                        &[self.remote_companion_port.to_string()],
                    ))
                } else {
                    Some(self.tr_fmt("remote_companion_running", &[url]))
                };
                tracing::info!(
                    port = self.remote_companion_port,
                    lan = self.companion_reaches_lan(),
                    "remote companion page server started"
                );
            }
            Err(err) => {
                self.remote_companion_server = None;
                self.remote_companion_url = None;
                self.remote_companion_status =
                    Some(self.tr_fmt("remote_companion_error", &[err.to_string()]));
                tracing::error!(error = %err, "remote companion page failed to start");
            }
        }
    }

    /// The URL handed to "Open page" — always loopback so it opens on this
    /// machine regardless of the bind.
    fn remote_companion_page_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.remote_companion_port)
    }

    /// Open the companion page in the default browser (loopback always works
    /// on the machine itself).
    fn open_remote_companion_page(&self) {
        if let Some(url) = &self.remote_companion_url {
            let _ = open::that(url);
        }
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
        snap.replay_buffer_active = self.replay_duration_secs.is_some();
        snap.studio_mode = self.studio_mode.enabled();
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
            muted: self.is_muted,
            replay_buffer_active: self.replay_duration_secs.is_some(),
            studio_mode: self.studio_mode.enabled(),
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
                self.switch_active_scene(scene.id);
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
            ObsCommand::PauseRecording => {
                if self.any_recording_active() && !self.is_paused {
                    self.is_paused = true;
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    rivulet_obs_websocket::ObsCommandResult::Success(vec![
                        ObsEvent::RecordStateChanged {
                            active: true,
                            paused: true,
                        },
                    ])
                } else {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_PAUSED,
                        comment: if self.any_recording_active() {
                            "Recording is already paused".into()
                        } else {
                            "Recording not active".into()
                        },
                    }
                }
            }
            ObsCommand::UnpauseRecording => {
                if self.any_recording_active() && self.is_paused {
                    self.is_paused = false;
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    rivulet_obs_websocket::ObsCommandResult::Success(vec![
                        ObsEvent::RecordStateChanged {
                            active: true,
                            paused: false,
                        },
                    ])
                } else {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_PAUSED,
                        comment: if self.any_recording_active() {
                            "Recording is not paused".into()
                        } else {
                            "Recording not active".into()
                        },
                    }
                }
            }
            ObsCommand::ToggleMute => {
                self.is_muted = !self.is_muted;
                self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                rivulet_obs_websocket::ObsCommandResult::Success(vec![
                    ObsEvent::InputMuteStateChanged {
                        input_name: self.obs_ws_primary_input_name(),
                        muted: self.is_muted,
                    },
                ])
            }
            ObsCommand::StartReplayBuffer => {
                if self.replay_duration_secs.is_some() {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_RUNNING,
                        comment: "Replay buffer already active".into(),
                    }
                } else {
                    self.replay_duration_secs = Some(30);
                    self.apply_replay_setting();
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    rivulet_obs_websocket::ObsCommandResult::Success(vec![
                        ObsEvent::ReplayBufferStateChanged { active: true },
                    ])
                }
            }
            ObsCommand::StopReplayBuffer => {
                if self.replay_duration_secs.is_none() {
                    rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Replay buffer not active".into(),
                    }
                } else {
                    self.replay_duration_secs = None;
                    self.apply_replay_setting();
                    self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                    rivulet_obs_websocket::ObsCommandResult::Success(vec![
                        ObsEvent::ReplayBufferStateChanged { active: false },
                    ])
                }
            }
            ObsCommand::ToggleReplayBuffer => {
                if self.replay_duration_secs.is_some() {
                    self.execute_obs_command(ObsCommand::StopReplayBuffer)
                } else {
                    self.execute_obs_command(ObsCommand::StartReplayBuffer)
                }
            }
            ObsCommand::SaveReplayBuffer => {
                if self.replay_duration_secs.is_none() {
                    return rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Replay buffer not active".into(),
                    };
                }
                // Derive a non-interactive path (the deck button has no file
                // dialog): the engine's filename pattern in the Videos dir.
                let dir = dirs::video_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("Rivulet");
                let _ = std::fs::create_dir_all(&dir);
                let path = self
                    .engine
                    .default_recording_path(&dir.display().to_string(), "replay");
                let path = path.with_extension("mp4");
                self.save_replay_to(path)
            }
            ObsCommand::SaveReplayBufferTo(path) => {
                if self.replay_duration_secs.is_none() {
                    return rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code: rivulet_obs_websocket::protocol::status::OUTPUT_NOT_RUNNING,
                        comment: "Replay buffer not active".into(),
                    };
                }
                let path = std::path::PathBuf::from(&path);
                self.save_replay_to(path)
            }
            ObsCommand::SetStudioMode(enabled) => {
                if self.studio_mode.enabled() == enabled {
                    return rivulet_obs_websocket::ObsCommandResult::Failure {
                        status_code:
                            rivulet_obs_websocket::protocol::status::RESOURCE_ALREADY_EXISTS,
                        comment: "Studio mode already in that state".into(),
                    };
                }
                self.studio_mode.set_enabled(enabled, self.scenes.active());
                self.obs_ws_last_public_state = Some(self.obs_ws_public_state());
                rivulet_obs_websocket::ObsCommandResult::Success(vec![
                    ObsEvent::StudioModeStateChanged { enabled },
                ])
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
                    .with_preset(self.stream_preset)
                    .with_vod_track(self.stream_vod_track());
                    self.engine.set_stream_settings(Some(settings));
                    // Clear a stale error so a new stream starts from the
                    // Ready/Streaming label (mirrors the recording starts).
                    self.last_error = None;
                    self.apply_ndi_output();
                    self.apply_restream_targets();
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
                    self.stop_streaming_session();
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

    /// The display name the websocket server publishes for the app's main
    /// (implicit) audio input. `ToggleInputMute` validates against this name,
    /// so it must stay in sync with the sources list written into the shared
    /// snapshot.
    fn obs_ws_primary_input_name(&self) -> String {
        self.obs_ws_snapshot
            .as_ref()
            .map(|snap| snap.lock().unwrap().clone())
            .and_then(|snap| snap.sources.first().cloned())
            .unwrap_or_else(|| "Microphone".to_string())
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
            "delete_source" => {
                self.delete_selected_composition_source();
            }
            _ => {}
        }
    }

    /// Copy the selected composition scene item to the in-app clipboard
    /// (issue #192). The clipboard carries the source and its scene binding
    /// verbatim so paste reproduces every scene-item property.
    fn copy_selected_composition_source(&mut self) {
        let Some(scene_id) = self.active_composition_scene() else {
            return;
        };
        let Some(source_id) = self.selected_composition_source else {
            return;
        };
        if let Some(clipboard) = self.source_manager.copy_scene_item(source_id, scene_id) {
            self.scene_item_clipboard = Some(clipboard);
            self.scene_status = Some(self.tr("composition_copy_ok").to_owned());
        }
    }

    /// Undo for the Scenes view: source pastes first (issue #192 — the most
    /// recent mutation wins and they have their own stack), then the M2
    /// scene-history stack.
    fn dispatch_scene_undo(&mut self) {
        if !self.source_manager.undo_paste() {
            self.scenes.undo();
        }
    }

    /// Redo counterpart of [`Self::dispatch_scene_undo`] with the same
    /// paste-first priority.
    fn dispatch_scene_redo(&mut self) {
        if !self.source_manager.redo_paste() {
            self.scenes.redo();
        }
    }

    /// Paste the scene-item clipboard into the composition scene. Deterministic
    /// ids are GUI-off (two pastes = two real duplicates); each paste lands on
    /// the SourceManager paste-undo stack so Ctrl+Z removes it again.
    fn paste_scene_item_clipboard(&mut self) {
        let Some(scene_id) = self.active_composition_scene() else {
            return;
        };
        let Some(clipboard) = self.scene_item_clipboard.clone() else {
            self.scene_status = Some(self.tr("composition_paste_empty").to_owned());
            return;
        };
        if let Some(new_id) = self.source_manager.paste_scene_item(&clipboard, scene_id) {
            self.selected_composition_source = Some(new_id);
            self.scene_status = Some(self.tr("composition_paste_ok").to_owned());
        }
    }

    /// The scene whose composition the Scenes view edits: the Studio Mode
    /// Preview when active, otherwise the active scene.
    fn active_composition_scene(&self) -> Option<uuid::Uuid> {
        if self.studio_mode.enabled() {
            self.studio_mode.preview()
        } else {
            self.scenes.active()
        }
    }

    /// Stage the deletion of the selected composition source behind the
    /// confirmation dialog instead of removing it immediately.
    ///
    /// Respects the scene-local lock (locked sources are never staged) and
    /// leaves the selection untouched until the user confirms.
    fn delete_selected_composition_source(&mut self) {
        let Some(scene_id) = self.active_composition_scene() else {
            return;
        };
        let Some(source_id) = self.selected_composition_source else {
            return;
        };
        let locked = self
            .source_manager
            .scene_sources(scene_id)
            .into_iter()
            .any(|binding| binding.source_id == source_id && binding.locked);
        if locked {
            self.scene_status = Some(self.tr("composition_source_locked").to_owned());
            return;
        }
        let name = self
            .source_manager
            .get_source(source_id)
            .map(|source| source.name.clone())
            .unwrap_or_default();
        self.pending_confirmation = Some(PendingConfirmation::DeleteCompositionSource {
            scene_id,
            source_id,
            name,
        });
    }

    /// Execute the staged destructive action after the user confirmed it in
    /// the dialog rendered by [`Self::draw_confirmation_modal`]. Resolves the
    /// lock/missing-source guards again so the confirmation path never bypasses
    /// them (the selection may have changed while the dialog was open).
    fn confirm_pending_confirmation(&mut self) {
        let Some(pending) = self.pending_confirmation.take() else {
            return;
        };
        match pending {
            PendingConfirmation::DeleteCompositionSource {
                scene_id,
                source_id,
                name,
            } => {
                let locked = self
                    .source_manager
                    .scene_sources(scene_id)
                    .into_iter()
                    .any(|binding| binding.source_id == source_id && binding.locked);
                if locked {
                    self.scene_status = Some(self.tr("composition_source_locked").to_owned());
                    return;
                }
                if self.source_manager.remove_source(source_id).is_some() {
                    self.selected_composition_source = None;
                    self.scene_status = Some(self.tr_fmt("composition_source_deleted", &[name]));
                }
            }
            PendingConfirmation::RemoveAudioSource { id, name } => {
                if self.audio_sources.iter().any(|source| source.id == id) {
                    let _ = self.engine.remove_audio_source(id);
                    self.audio_sources.retain(|source| source.id != id);
                    self.audio_mixer_needs_sync = true;
                    if self.audio_mixer_filter_source == Some(id) {
                        self.audio_mixer_filter_source = None;
                    }
                    self.scene_status = Some(self.tr_fmt("audio_source_removed", &[name]));
                }
            }
            PendingConfirmation::RemoveChatAccount { index, .. } => {
                if self.chat_accounts.get(index).is_some() {
                    let account = self.chat_accounts[index].clone();
                    // Best-effort vault cleanup; a stale entry for a removed
                    // account is harmless but we try not to leave one.
                    let _ = rivulet_core::ChatTokenStore::default()
                        .delete(account.platform, &account.channel);
                    self.chat_accounts.remove(index);
                    self.chat_action_pending = Some(ChatAction::Disconnect);
                }
            }
        }
    }

    /// Discard a staged destructive action without running it.
    fn cancel_pending_confirmation(&mut self) {
        self.pending_confirmation = None;
    }

    /// Render the confirmation dialog for a staged destructive action. Drawn
    /// as a modal on top of everything else; backdrop click, `Esc`, or the
    /// Cancel button discards the action, while the destructive Confirm button
    /// runs it.
    fn draw_confirmation_modal(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_confirmation.clone() else {
            return;
        };
        let title = self.tr(pending.title_key()).to_owned();
        let message = self.tr_fmt(pending.message_key(), &[pending.subject()]);
        let confirm_label = self.tr(pending.confirm_key()).to_owned();
        let cancel_label = self.tr("confirm_dialog_cancel").to_owned();

        let mut choice = ConfirmationChoice::Pending;
        let response =
            egui::Modal::new(egui::Id::new("rivulet_confirm_destructive")).show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.set_max_width(420.0);
                ui.heading(title);
                ui.add_space(6.0);
                ui.label(message);
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.button(cancel_label).clicked() {
                        choice = ConfirmationChoice::Cancel;
                    }
                    let confirm = theme::accent_button(
                        ui,
                        egui::RichText::new(confirm_label)
                            .color(theme::StatusColors::for_ui(ui).error),
                    );
                    if confirm.clicked() {
                        choice = ConfirmationChoice::Confirm;
                    }
                });
            });
        match choice {
            ConfirmationChoice::Confirm => self.confirm_pending_confirmation(),
            ConfirmationChoice::Cancel => self.cancel_pending_confirmation(),
            ConfirmationChoice::Pending => {
                if response.should_close() || response.backdrop_response.clicked() {
                    self.cancel_pending_confirmation();
                }
            }
        }
    }

    /// Stage an audio source removal behind the confirmation dialog.
    fn remove_audio_source(&mut self, id: uuid::Uuid) {
        let name = self
            .audio_sources
            .iter()
            .find(|source| source.id == id)
            .map(|source| source.name.clone())
            .unwrap_or_default();
        self.pending_confirmation = Some(PendingConfirmation::RemoveAudioSource { id, name });
    }

    /// Stage a chat account removal behind the confirmation dialog.
    fn remove_chat_account(&mut self, index: usize) {
        let Some(account) = self.chat_accounts.get(index) else {
            return;
        };
        self.pending_confirmation = Some(PendingConfirmation::RemoveChatAccount {
            index,
            platform: account.platform,
            channel: account.channel.clone(),
        });
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

    /// Mirror the persisted telemetry opt-in into the runtime collector and
    /// report the once-per-session Startup event when opted in. Called after
    /// restore in `new()` and whenever the Settings toggle changes.
    fn apply_telemetry_policy(&mut self) {
        self.telemetry.set_enabled(self.telemetry_enabled);
        if self.telemetry_enabled && !self.telemetry_startup_reported {
            self.telemetry_startup_reported = true;
            self.telemetry.record(rivulet_core::TelemetryEvent::Startup);
        }
    }

    // ── Plugins: Phase 3 install/permission flow (plugin-system RFC) ────────

    /// The plugin install root: `<local data dir>/Rivulet/plugins`, mirroring
    /// the log-directory convention. Falls back to the temp dir when the OS
    /// data dir is unavailable (same behavior as logging).
    fn plugin_install_root() -> std::path::PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("Rivulet")
            .join(rivulet_core::default_install_root(std::path::Path::new("")))
            .components()
            .collect::<std::path::PathBuf>()
    }

    /// Rescan the install root and update the discovered/broken lists.
    /// Missing root is not an error: it simply yields no plugins.
    fn rescan_plugins(&mut self) {
        let root = Self::plugin_install_root();
        let (found, broken) = rivulet_core::scan_install_root(&root);
        self.plugins_discovered = found;
        self.plugins_broken = broken;
    }

    /// The review state of one discovered plugin for the UI list.
    fn plugin_review_state(&self, plugin: &rivulet_core::DiscoveredPlugin) -> PluginReviewState {
        let record = self.plugin_approvals.record(plugin.id());
        if record.is_none() || record.is_some_and(|r| !r.enabled) {
            if self
                .plugin_approvals
                .fully_decided(plugin.id(), plugin.capabilities())
            {
                PluginReviewState::Disabled
            } else {
                PluginReviewState::Pending
            }
        } else {
            PluginReviewState::Enabled
        }
    }

    /// Open the permission-review dialog for a plugin (seeding the working
    /// copy from the persisted decisions).
    fn open_plugin_review(&mut self, plugin_id: &str) {
        self.plugin_load_error = None;
        self.plugin_review_draft.clear();
        if let Some(plugin) = self.plugins_discovered.iter().find(|p| p.id() == plugin_id) {
            for cap in plugin.capabilities().requested() {
                let approved = self
                    .plugin_approvals
                    .record(plugin_id)
                    .and_then(|r| r.capabilities.get(cap))
                    .is_some_and(|d| *d == rivulet_core::CapabilityDecision::Approved);
                self.plugin_review_draft.insert((*cap).to_owned(), approved);
            }
        }
        self.plugin_review_open = Some(plugin_id.to_owned());
    }

    /// Apply the dialog's working copy to the persisted approval store.
    fn apply_plugin_review(&mut self) {
        let Some(plugin_id) = self.plugin_review_open.clone() else {
            return;
        };
        // Deny everything not explicitly approved (default denial, RFC §6.1).
        let approved: Vec<String> = self
            .plugin_review_draft
            .iter()
            .filter(|(_, &ok)| ok)
            .map(|(cap, _)| cap.clone())
            .collect();
        for cap in self.plugin_review_draft.keys() {
            self.plugin_approvals.deny(&plugin_id, cap);
        }
        for cap in &approved {
            self.plugin_approvals.approve(&plugin_id, cap);
        }
        // Enabling a plugin is only valid when every requested capability
        // has a decision; otherwise the enable flag is reset conservatively.
        if let Some(plugin) = self.plugins_discovered.iter().find(|p| p.id() == plugin_id) {
            if self
                .plugin_approvals
                .fully_decided(plugin.id(), plugin.capabilities())
            {
                // Keep the previous enabled choice if it was already valid.
            } else {
                self.plugin_approvals.set_enabled(&plugin_id, false);
            }
        }
        self.plugin_review_open = None;
        self.plugin_review_draft.clear();
    }

    /// Toggle the enabled flag of a fully-decided plugin.
    fn set_plugin_enabled(&mut self, plugin_id: &str, enabled: bool) {
        if let Some(plugin) = self.plugins_discovered.iter().find(|p| p.id() == plugin_id) {
            if self
                .plugin_approvals
                .fully_decided(plugin.id(), plugin.capabilities())
            {
                self.plugin_approvals.set_enabled(plugin_id, enabled);
            }
        }
    }

    /// Draw the Settings → Plugins section (list + review dialog). Returns
    /// after drawing; the dialog is rendered on top of the settings view.
    fn draw_plugins_section(&mut self, ui: &mut egui::Ui) {
        let colors = theme::StatusColors::for_ui(ui);
        ui.separator();
        ui.label(egui::RichText::new(self.tr("plugins_section")).strong());
        if ui.button(self.tr("plugins_scan")).clicked() {
            self.rescan_plugins();
        }
        ui.small(self.tr("plugins_install_root_hint"));

        if self.plugins_discovered.is_empty() {
            ui.label(self.tr("plugins_none"));
        }
        for plugin in self.plugins_discovered.clone() {
            let state = self.plugin_review_state(&plugin);
            let caps = plugin.capabilities();
            ui.horizontal(|ui| {
                let state_text = match state {
                    PluginReviewState::Enabled => self.tr("plugins_status_enabled"),
                    PluginReviewState::Disabled => self.tr("plugins_status_disabled"),
                    PluginReviewState::Pending => self.tr("plugins_status_pending"),
                };
                let state_color = match state {
                    PluginReviewState::Enabled => colors.success,
                    PluginReviewState::Disabled => colors.warning,
                    PluginReviewState::Pending => colors.info,
                };
                ui.label(
                    egui::RichText::new(format!("{} v{}", plugin.name(), plugin.version()))
                        .strong(),
                );
                ui.label(format!("— {}", state_text));
                ui.colored_label(state_color, "●");
            });
            ui.indent(plugin.id(), |ui| {
                egui::Grid::new(format!("plugin_details_{}", plugin.id()))
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(self.tr("plugins_type"));
                        ui.label(plugin.manifest.plugin.type_info.kind.as_str());
                        ui.end_row();
                        ui.label(self.tr("plugins_publisher"));
                        if plugin.manifest.plugin.author.is_empty() {
                            ui.label("—");
                        } else {
                            ui.label(&plugin.manifest.plugin.author);
                        }
                        ui.end_row();
                        ui.label(self.tr("plugins_api"));
                        ui.label(format!(">= {}", plugin.manifest.plugin.api_version.min));
                        ui.end_row();
                        ui.label(self.tr("plugins_sandbox"));
                        ui.label(self.tr("plugins_sandbox_wasm"));
                        ui.end_row();
                    });
                let caps_label = if caps.requested().is_empty() {
                    self.tr("plugins_dialog_denied_note").to_owned()
                } else {
                    caps.requested().join(", ")
                };
                ui.label(caps_label);
                ui.horizontal(|ui| {
                    if ui.button(self.tr("plugins_review")).clicked() {
                        self.open_plugin_review(plugin.id());
                    }
                    let review_done = self
                        .plugin_approvals
                        .fully_decided(plugin.id(), plugin.capabilities());
                    let enabled_now = self
                        .plugin_approvals
                        .record(plugin.id())
                        .is_some_and(|r| r.enabled);
                    if !review_done {
                        ui.small(self.tr("plugins_enable_blocked_hint"));
                    } else if enabled_now && ui.button(self.tr("plugins_disable")).clicked() {
                        self.set_plugin_enabled(plugin.id(), false);
                    } else if !enabled_now && ui.button(self.tr("plugins_enable")).clicked() {
                        self.set_plugin_enabled(plugin.id(), true);
                    }
                    if ui.button(self.tr("plugins_forget")).clicked() {
                        self.plugin_approvals.forget(plugin.id());
                    }
                });
                if let Some(err) = &self.plugin_load_error {
                    ui.colored_label(colors.error, format!("{}: {err}", plugin.id()));
                }
            });
        }
        if !self.plugins_broken.is_empty() {
            ui.label(egui::RichText::new(self.tr("plugins_broken")).weak());
            for entry in &self.plugins_broken {
                ui.small(format!("⚠ {entry}"));
            }
        }
    }

    /// Draw the permission-review dialog (RFC §6.2) for the plugin currently
    /// open. Every requested capability shows its sensitive note (RFC §6.3)
    /// and an Approve/Deny choice; the working copy lands in the store on
    /// Done. While the dialog is open the store is untouched.
    fn draw_plugin_review_dialog(&mut self, ctx: &egui::Context) {
        let Some(plugin_id) = self.plugin_review_open.clone() else {
            return;
        };
        let Some(plugin) = self
            .plugins_discovered
            .iter()
            .find(|p| p.id() == plugin_id)
            .cloned()
        else {
            self.plugin_review_open = None;
            return;
        };
        let title = format!("{} — {}", self.tr("plugins_dialog_title"), plugin.name());
        let mut done = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} v{}",
                    plugin.name(),
                    plugin.manifest.plugin.version
                ));
                ui.label(self.tr("plugins_dialog_requests"));
                let caps = plugin.capabilities();
                if caps.requested().is_empty() {
                    ui.label(self.tr("plugins_none"));
                }
                for cap in caps.requested() {
                    let sensitive = matches!(cap, "secrets" | "capture" | "chat");
                    ui.horizontal(|ui| {
                        let approved = self.plugin_review_draft.get(cap).copied().unwrap_or(false);
                        ui.label(cap);
                        if ui
                            .selectable_label(approved, self.tr("plugins_dialog_approve"))
                            .clicked()
                        {
                            self.plugin_review_draft.insert((*cap).to_owned(), true);
                        }
                        if ui
                            .selectable_label(!approved, self.tr("plugins_dialog_deny"))
                            .clicked()
                        {
                            self.plugin_review_draft.insert((*cap).to_owned(), false);
                        }
                        if sensitive {
                            ui.small(format!("({})", self.tr("plugins_dialog_sensitive")));
                        }
                    });
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(self.tr("plugins_dialog_approve_all")).clicked() {
                        for cap in caps.requested() {
                            let sensitive = matches!(cap, "secrets" | "capture" | "chat");
                            self.plugin_review_draft
                                .insert((*cap).to_owned(), !sensitive);
                        }
                    }
                    if ui.button(self.tr("plugins_dialog_deny_all")).clicked() {
                        for cap in caps.requested() {
                            self.plugin_review_draft.insert((*cap).to_owned(), false);
                        }
                    }
                    if ui.button(self.tr("plugins_dialog_done")).clicked() {
                        done = true;
                    }
                });
                ui.small(self.tr("plugins_dialog_denied_note"));
            });
        if done {
            self.apply_plugin_review();
        }
    }

    /// Mirror the persisted alert-ingestion toggle into the runtime queue.
    /// Turning it off discards any pending entries (like telemetry: disabling
    /// clears what was queued). The queue is purely local — nothing is ever
    /// transmitted, so the default is on.
    fn apply_alerts_policy(&mut self) {
        if !self.alert_ingest_enabled {
            self.alert_ingest.drain();
        }
        self.alert_ingest.set_enabled(self.alert_ingest_enabled);
    }

    /// Mirror the persisted webhook-receiver settings into the running
    /// loopback listener: start when enabled and not running, restart when the
    /// port or the Twitch secret changed, stop when disabled. Loopback-only by
    /// design (providers need an HTTPS terminator or tunnel in front, see
    /// docs/alerts-ingest.md); a bind failure is surfaced as a settings warning,
    /// never a crash. Called once per frame, cheap when nothing changed.
    fn apply_alerts_receiver(&mut self) {
        if self.alerts_receiver_enabled {
            let desired = rivulet_core::AlertsReceiverConfig {
                port: self.alerts_receiver_port,
                twitch_secret: self.alerts_twitch_secret.trim().to_owned(),
            };
            let restart = self.alerts_receiver.is_none()
                || self.alerts_receiver_applied.port != desired.port
                || self.alerts_receiver_applied.twitch_secret != desired.twitch_secret;
            if restart {
                if let Some(mut old) = self.alerts_receiver.take() {
                    old.shutdown();
                }
                self.alerts_receiver_error = None;
                match rivulet_core::AlertsReceiver::start(desired.clone()) {
                    Ok(receiver) => {
                        self.alerts_receiver_applied = desired;
                        self.alerts_receiver = Some(receiver);
                    }
                    Err(err) => {
                        self.alerts_receiver_error = Some(err.to_string());
                        self.alerts_receiver_applied =
                            rivulet_core::AlertsReceiverConfig::default();
                    }
                }
            }
        } else {
            if let Some(mut old) = self.alerts_receiver.take() {
                old.shutdown();
            }
            self.alerts_receiver_applied = rivulet_core::AlertsReceiverConfig::default();
            self.alerts_receiver_error = None;
        }
    }

    /// Mirror the persisted EventSub settings into the running worker: start
    /// when enabled and all credentials are filled, restart when the client
    /// ID, token or broadcaster changed, stop otherwise. Missing credentials
    /// never dial out (the worker stays off), matching the webhook receiver's
    /// "empty disables" semantics. Called once per frame, cheap when nothing
    /// changed. Connection failures are asynchronous (counters + `connected()`),
    /// so no error is raised here.
    /// Localized label for one raid alert direction (settings ComboBox and
    /// selected text).
    fn raid_direction_label(&self, direction: rivulet_core::RaidAlertDirection) -> String {
        match direction {
            rivulet_core::RaidAlertDirection::Out => self.tr("alert_raid_direction_out").to_owned(),
            rivulet_core::RaidAlertDirection::In => self.tr("alert_raid_direction_in").to_owned(),
            rivulet_core::RaidAlertDirection::Both => {
                self.tr("alert_raid_direction_both").to_owned()
            }
        }
    }

    fn apply_alerts_eventsub(&mut self) {
        let ws_endpoint = if self.alerts_eventsub_ws_endpoint.trim().is_empty() {
            rivulet_core::DEFAULT_EVENTSUB_WS_ENDPOINT.to_owned()
        } else {
            self.alerts_eventsub_ws_endpoint.trim().to_owned()
        };
        let api_base = if self.alerts_eventsub_api_base.trim().is_empty() {
            rivulet_core::DEFAULT_TWITCH_API_BASE.to_owned()
        } else {
            self.alerts_eventsub_api_base.trim().to_owned()
        };
        let desired = rivulet_core::EventsubWsConfig {
            client_id: self.alerts_eventsub_client_id.trim().to_owned(),
            token: self.alerts_eventsub_token.trim().to_owned(),
            broadcaster_id: self.alerts_eventsub_broadcaster_id.trim().to_owned(),
            ws_endpoint,
            api_base,
            raid_direction: self.alerts_raid_direction,
        };
        let complete = !desired.client_id.is_empty()
            && !desired.token.is_empty()
            && !desired.broadcaster_id.is_empty();
        if self.alerts_eventsub_enabled && complete {
            let restart = self.alerts_eventsub.is_none() || self.alerts_eventsub_applied != desired;
            if restart {
                if let Some(mut old) = self.alerts_eventsub.take() {
                    old.shutdown();
                }
                self.alerts_eventsub_error = None;
                self.alerts_eventsub_applied = desired.clone();
                self.alerts_eventsub = Some(rivulet_core::EventsubReceiver::start(desired));
            }
        } else {
            if let Some(mut old) = self.alerts_eventsub.take() {
                old.shutdown();
            }
            self.alerts_eventsub_applied = rivulet_core::EventsubWsConfig::default();
            self.alerts_eventsub_error = None;
        }
    }

    /// Classify one finished recording session for telemetry. `healthy` is
    /// best-effort at the moment the session ends (`last_error` empty); events
    /// are only collected while the user opted in and never leave the device
    /// in the shipped build (no transport sink is wired). An unhealthy
    /// session also records the classified error category (`RecordingError`),
    /// never the raw error text.
    fn complete_recording_session_telemetry(&mut self) {
        let duration_secs = self.record_started.elapsed().as_secs().min(u32::MAX as u64) as u32;
        let healthy = self.last_error.is_none();
        if let Some(error) = &self.last_error {
            self.telemetry
                .record(rivulet_core::TelemetryEvent::RecordingError {
                    error: classify_record_error(error),
                });
        }
        self.telemetry
            .record(rivulet_core::TelemetryEvent::RecordingStop {
                duration_secs,
                healthy,
            });
    }

    /// Switch the active scene and report the `SceneSwitch` telemetry event
    /// only when the switch actually landed (mirrors `SceneManager::switch_to`
    /// success semantics, e.g. unknown ids do not count as a switch).
    fn switch_active_scene(&mut self, id: uuid::Uuid) -> bool {
        if self.scenes.switch_to(id) {
            self.telemetry
                .record(rivulet_core::TelemetryEvent::SceneSwitch);
            true
        } else {
            false
        }
    }

    /// Move back to the previous scene, reporting `SceneSwitch` like every
    /// other scene change.
    fn switch_scene_back(&mut self) -> bool {
        if self.scenes.switch_back() {
            self.telemetry
                .record(rivulet_core::TelemetryEvent::SceneSwitch);
            true
        } else {
            false
        }
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
    /// Apply the "add account" draft row: validate (non-empty channel, no
    /// duplicate platform), append to the account list and reset the draft.
    /// Extracted from the dock so the validation rules stay unit-testable.
    fn apply_chat_add_account(&mut self) {
        let channel = self.chat_add_channel.trim().to_owned();
        let duplicate = self
            .chat_accounts
            .iter()
            .any(|a| a.platform == self.chat_add_platform);
        if channel.is_empty() || duplicate {
            self.chat_add_error = Some(if duplicate {
                self.tr("chat_account_duplicate").to_owned()
            } else {
                self.tr("chat_account_channel_required").to_owned()
            });
            return;
        }
        // The token stays in the add-row draft; `store_chat_account_token`
        // moves it into the OS credential vault right after this call. The
        // roster itself never holds a secret (CodeQL
        // rust/cleartext-logging root fix).
        self.chat_accounts.push(rivulet_core::ChatAccount::new(
            self.chat_add_platform,
            channel,
        ));
        self.chat_add_channel.clear();
        self.chat_add_error = None;
        // Changing the roster stops the running workers; the next Connect
        // spawns one worker per account again.
        self.chat_action_pending = Some(ChatAction::Disconnect);
    }

    /// Persist the token entered in the add-account row into the OS
    /// credential vault and clear the draft. Empty tokens (YouTube, or a
    /// Kick read-only setup) skip the vault. Failure keeps a status hint but
    /// the account stays configured — the token can be re-entered later.
    fn store_chat_account_token(&mut self) {
        let token = std::mem::take(&mut self.chat_add_token);
        if token.trim().is_empty() {
            return;
        }
        let Some(account) = self.chat_accounts.last() else {
            return;
        };
        let platform = account.platform;
        let channel = account.channel.clone();
        if let Err(error) = rivulet_core::ChatTokenStore::default().save(platform, &channel, &token)
        {
            self.chat_add_error = Some(self.tr_fmt("chat_token_store_failed", &[error]).to_owned());
        }
    }

    /// Per-platform credentials for the stream-info editor. Tokens are
    /// read from the OS credential vault only at apply time (the roster
    /// stays token-free); failures surface as honest per-platform
    /// outcomes instead of leaking secrets into state or logs.
    fn chat_info_credentials_for(
        &self,
        platform: rivulet_core::InfoPlatform,
    ) -> rivulet_core::InfoCredentials {
        let mut credentials = rivulet_core::InfoCredentials::default();
        match platform {
            rivulet_core::InfoPlatform::Twitch => {
                credentials.client_id = self.alerts_eventsub_client_id.trim().to_owned();
                credentials.broadcaster_id = self.alerts_eventsub_broadcaster_id.trim().to_owned();
            }
            rivulet_core::InfoPlatform::Kick => {}
            rivulet_core::InfoPlatform::YouTube => {
                // The YouTube chat account's channel field carries the live
                // video id (documented in the chat setup notes).
                if let Some(account) = self
                    .chat_accounts
                    .iter()
                    .find(|a| a.platform == rivulet_core::ChatPlatform::YouTube)
                {
                    credentials.video_id = account.channel.trim().to_owned();
                }
            }
        }
        if let Some(account) = self
            .chat_accounts
            .iter()
            .find(|a| a.platform == rivulet_core::chat_info_platform(platform))
        {
            if let Ok(Some(token)) =
                rivulet_core::ChatTokenStore::default().load(account.platform, &account.channel)
            {
                credentials.token = token;
            }
        }
        credentials
    }

    /// Apply the current title/game drafts. With `only` set, one platform
    /// is updated; otherwise every configured platform gets the same
    /// update (per-platform outcomes, one failure never aborts the rest).
    /// The HTTP work runs on a background thread; the UI polls the result.
    fn apply_chat_stream_info(&mut self, only: Option<rivulet_core::InfoPlatform>) {
        if self.chat_info_busy {
            return;
        }
        // Per-platform updates (#214): every platform carries its own
        // title/game drafts; "apply to all" pushes each platform's own
        // values at once. Platforms with empty drafts have nothing to
        // apply and are skipped instead of failing with an empty update.
        let updates: Vec<(rivulet_core::InfoPlatform, rivulet_core::StreamInfoUpdate)> =
            rivulet_core::InfoPlatform::ALL
                .into_iter()
                .filter(|p| match only {
                    Some(only) => only == *p,
                    None => true,
                })
                .map(|p| {
                    let idx = p.index();
                    (
                        p,
                        rivulet_core::StreamInfoUpdate {
                            title: Some(self.chat_info_title[idx].clone()),
                            game: Some(self.chat_info_game[idx].clone()),
                        },
                    )
                })
                .filter(|(_, update)| update.has_changes())
                .collect();
        if updates.is_empty() {
            self.chat_info_error = Some(self.tr("chat_info_nothing").to_owned());
            return;
        }
        self.chat_info_error = None;

        let accounts = self.chat_accounts.clone();
        let mut platforms: Vec<rivulet_core::InfoPlatform> = accounts
            .iter()
            .filter_map(|a| rivulet_core::chat_info_platform_of_chat(a.platform))
            .collect();
        if let Some(only) = only {
            platforms.retain(|p| *p == only);
        }
        // A configured platform without any draft content is not part of
        // this batch (nothing would change on it).
        platforms.retain(|p| updates.iter().any(|(target, _)| target == p));
        if platforms.is_empty() {
            self.chat_info_error = Some(self.tr("chat_info_nothing").to_owned());
            return;
        }
        // Credentials are resolved *here*, on the UI thread, before any
        // background thread exists — the vault is only touched from the
        // main thread.
        let targets: Vec<(
            rivulet_core::InfoPlatform,
            rivulet_core::InfoCredentials,
            rivulet_core::StreamInfoUpdate,
        )> = platforms
            .iter()
            .map(|p| {
                let update = updates
                    .iter()
                    .find(|(target, _)| target == p)
                    .map(|(_, update)| update.clone())
                    .unwrap_or_default();
                (*p, self.chat_info_credentials_for(*p), update)
            })
            .collect();

        let (tx, rx) = std::sync::mpsc::channel();
        self.chat_info_rx = Some(rx);
        self.chat_info_busy = true;
        std::thread::spawn(move || {
            // One call per platform with its own update (the shared
            // `update_all_stream_info` helper applies one update to many
            // platforms; #214 needs per-platform values).
            let outcomes = targets
                .iter()
                .map(|(platform, credentials, update)| {
                    (
                        *platform,
                        rivulet_core::update_platform_stream_info(
                            *platform,
                            update,
                            credentials,
                            &rivulet_core::InfoEndpoints::default(),
                            &rivulet_core::UreqHttp,
                        ),
                    )
                })
                .collect::<Vec<_>>();
            let _ = tx.send(outcomes);
        });
    }

    /// Shared Chat badge label for `raw_id`: the cached channel login when
    /// known, otherwise the raw id. Unknown ids start one batched background
    /// Helix lookup (collapsed while in flight, released when dispatch is
    /// impossible) — resolution never blocks rendering and runs only while a
    /// shared-chat message is on screen.
    fn shared_room_badge(&mut self, raw_id: &str) -> Option<String> {
        match self.chat_room_names.step(raw_id) {
            rivulet_core::RoomNameStep::Known(login) => Some(login),
            // Until the lookup lands the raw id renders — same badge shape,
            // honest fallback.
            rivulet_core::RoomNameStep::Pending => Some(raw_id.to_owned()),
            rivulet_core::RoomNameStep::Started(batch) => {
                // Dispatch the lookup on a background thread only when a
                // Twitch credential pair is actually configured; otherwise
                // release the markers so a later frame can retry.
                let client_id = self.alerts_eventsub_client_id.trim().to_owned();
                let token = self
                    .chat_accounts
                    .iter()
                    .find(|a| a.platform == rivulet_core::ChatPlatform::Twitch)
                    .and_then(|account| {
                        rivulet_core::ChatTokenStore::default()
                            .load(rivulet_core::ChatPlatform::Twitch, &account.channel)
                            .unwrap_or(None)
                    });
                match (client_id.is_empty(), token.as_deref()) {
                    (false, Some(token)) if !token.trim().is_empty() => {
                        use rivulet_core::RoomNameResolver as _;
                        let (tx, rx) = std::sync::mpsc::channel();
                        let resolver = rivulet_core::HelixRoomResolver::default();
                        let token = token.to_owned();
                        let batch = batch.clone();
                        std::thread::spawn(move || {
                            let resolved = match resolver.resolve(&batch, &client_id, &token) {
                                Ok(body) => rivulet_core::helix_users_by_id(&body),
                                Err(_) => Vec::new(),
                            };
                            let _ = tx.send(resolved);
                        });
                        self.chat_room_name_rx = Some(rx);
                    }
                    _ => {
                        self.chat_room_names.release_in_flight();
                    }
                }
                Some(raw_id.to_owned())
            }
        }
    }

    fn draw_chat_dock(&mut self, ui: &mut egui::Ui, max_list_height: f32) {
        ui.add_space(8.0);
        ui.label(egui::RichText::new(self.tr("chat_title")).strong());
        ui.separator();

        // Accounts: one row per configured platform. The combined dock
        // connects one worker per account (see `rivulet_core::MultiChat`),
        // so every platform appears in the same message list with its own
        // connection state instead of a single selectable platform.
        for (index, account) in self.chat_accounts.clone().iter().enumerate() {
            let state = self
                .chat_worker_multi
                .as_ref()
                .and_then(|multi| {
                    multi
                        .connection_states()
                        .into_iter()
                        .find(|(p, _)| *p == account.platform)
                        .map(|(_, s)| s)
                })
                .unwrap_or(rivulet_core::ChatConnState::Off);
            let colors = theme::StatusColors::for_ui(ui);
            let (state_text, state_color) = match state {
                rivulet_core::ChatConnState::Connected => {
                    (self.tr("chat_state_connected"), colors.success)
                }
                rivulet_core::ChatConnState::Disconnected => {
                    (self.tr("chat_state_disconnected"), colors.warning)
                }
                rivulet_core::ChatConnState::Off => {
                    (self.tr("chat_state_off"), ui.visuals().weak_text_color())
                }
            };
            ui.horizontal_wrapped(|ui| {
                let platform_text = egui::RichText::new(account.platform.label()).strong();
                let channel_text = if account.channel.trim().is_empty() {
                    self.tr("chat_channel_empty").to_owned()
                } else {
                    account.channel.clone()
                };
                ui.label(platform_text);
                ui.label(&channel_text);
                ui.colored_label(state_color, state_text);
                // Removal is staged behind the confirmation dialog (a
                // reconnect re-reads the list); a disconnected worker means
                // the change takes effect on the next Connect.
                if ui.small_button(self.tr("chat_account_remove")).clicked() {
                    self.remove_chat_account(index);
                }
            });
        }

        // Add-account row: platform + channel + optional token, applied on
        // "Add". Duplicates per platform are refused (MultiChat would ignore
        // them anyway, but an explicit message beats a silent drop).
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("chat_add_platform")
                .selected_text(self.chat_add_platform.label())
                .show_ui(ui, |ui| {
                    for candidate in rivulet_core::ChatPlatform::all() {
                        ui.selectable_value(
                            &mut self.chat_add_platform,
                            candidate,
                            candidate.label(),
                        );
                    }
                });
            let channel_hint = match self.chat_add_platform {
                rivulet_core::ChatPlatform::Twitch => self.tr("chat_channel_hint"),
                rivulet_core::ChatPlatform::Kick => self.tr("chat_channel_hint_kick"),
                rivulet_core::ChatPlatform::YouTube => self.tr("chat_channel_hint_youtube"),
            };
            ui.add(
                egui::TextEdit::singleline(&mut self.chat_add_channel)
                    .hint_text(channel_hint)
                    .desired_width(140.0),
            );
            if self.chat_add_platform != rivulet_core::ChatPlatform::YouTube {
                let oauth_hint = match self.chat_add_platform {
                    rivulet_core::ChatPlatform::Twitch => self.tr("chat_oauth_hint"),
                    _ => self.tr("chat_token_hint_kick"),
                };
                ui.add(
                    egui::TextEdit::singleline(&mut self.chat_add_token)
                        .password(true)
                        .hint_text(oauth_hint)
                        .desired_width(150.0),
                );
            }
            if ui.button(self.tr("chat_account_add")).clicked() {
                self.apply_chat_add_account();
                if self.chat_add_error.is_none() {
                    self.store_chat_account_token();
                }
            }
        });
        if let Some(error) = &self.chat_add_error {
            let colors = theme::StatusColors::for_ui(ui);
            ui.colored_label(colors.warning, error);
        }

        // Stream-info editor: every platform keeps its own title/game draft
        // (#214), edited through the scope tabs; "apply to all" pushes each
        // platform's own values at once. Outcomes are per-platform (one
        // failure never hides the others).
        ui.add_space(4.0);
        ui.separator();
        ui.label(egui::RichText::new(self.tr("chat_info_title")).strong());
        let scope = self.chat_info_scope;
        let field_width = if scope.is_none() { 150.0 } else { 220.0 };
        let scopes: Vec<(Option<rivulet_core::InfoPlatform>, String)> =
            std::iter::once((None, self.tr("chat_info_scope_all").to_owned()))
                .chain(
                    rivulet_core::InfoPlatform::ALL
                        .into_iter()
                        .map(|p| (Some(p), p.label().to_owned())),
                )
                .collect();
        ui.horizontal_wrapped(|ui| {
            for (tab_scope, label) in &scopes {
                let configured = match tab_scope {
                    None => !self.chat_accounts.is_empty(),
                    Some(platform) => self.chat_accounts.iter().any(|a| {
                        rivulet_core::chat_info_platform_of_chat(a.platform) == Some(*platform)
                    }),
                };
                let selected = self.chat_info_scope == *tab_scope;
                let button = if selected {
                    egui::Button::new(egui::RichText::new(label.clone()).strong())
                } else {
                    egui::Button::new(label.clone())
                };
                if ui.add_enabled(configured, button).clicked() {
                    self.chat_info_scope = *tab_scope;
                }
            }
        });
        match scope {
            None => {
                // All-platforms view: one row per configured platform, each
                // editing that platform's own drafts.
                for platform in rivulet_core::InfoPlatform::ALL {
                    let configured = self.chat_accounts.iter().any(|a| {
                        rivulet_core::chat_info_platform_of_chat(a.platform) == Some(platform)
                    });
                    if !configured {
                        continue;
                    }
                    let idx = platform.index();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(egui::RichText::new(format!("{}:", platform.label())).strong());
                        ui.label(self.tr("chat_info_field_title"));
                        let title_hint = self.tr("chat_info_hint_title").to_owned();
                        ui.add(
                            egui::TextEdit::singleline(&mut self.chat_info_title[idx])
                                .hint_text(title_hint)
                                .desired_width(field_width),
                        );
                        ui.label(self.tr("chat_info_field_game"));
                        let game_hint = self.tr("chat_info_hint_game").to_owned();
                        ui.add(
                            egui::TextEdit::singleline(&mut self.chat_info_game[idx])
                                .hint_text(game_hint)
                                .desired_width(field_width),
                        );
                    });
                }
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.chat_info_busy && !self.chat_accounts.is_empty(),
                            egui::Button::new(self.tr("chat_info_apply_all")),
                        )
                        .clicked()
                    {
                        self.apply_chat_stream_info(None);
                    }
                    if self.chat_info_busy {
                        ui.spinner();
                        ui.label(self.tr("chat_info_busy"));
                    }
                });
            }
            Some(platform) => {
                let idx = platform.index();
                ui.horizontal_wrapped(|ui| {
                    ui.label(self.tr("chat_info_field_title"));
                    let title_hint = self.tr("chat_info_hint_title").to_owned();
                    ui.add(
                        egui::TextEdit::singleline(&mut self.chat_info_title[idx])
                            .hint_text(title_hint)
                            .desired_width(field_width),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(self.tr("chat_info_field_game"));
                    let game_hint = self.tr("chat_info_hint_game").to_owned();
                    ui.add(
                        egui::TextEdit::singleline(&mut self.chat_info_game[idx])
                            .hint_text(game_hint)
                            .desired_width(field_width),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    let label =
                        self.tr_fmt("chat_info_apply_platform", &[platform.label().to_owned()]);
                    if ui
                        .add_enabled(!self.chat_info_busy, egui::Button::new(label))
                        .clicked()
                    {
                        self.apply_chat_stream_info(Some(platform));
                    }
                    if self.chat_info_busy {
                        ui.spinner();
                        ui.label(self.tr("chat_info_busy"));
                    }
                });
            }
        }
        if let Some(error) = &self.chat_info_error {
            let colors = theme::StatusColors::for_ui(ui);
            ui.colored_label(colors.warning, error.clone());
        }
        if !self.chat_info_outcomes.is_empty() {
            let colors = theme::StatusColors::for_ui(ui);
            ui.horizontal_wrapped(|ui| {
                for (platform, outcome) in &self.chat_info_outcomes {
                    let (text, color) = if outcome.is_ok() {
                        (self.tr("chat_info_updated").to_owned(), colors.success)
                    } else {
                        (
                            match outcome {
                                rivulet_core::InfoUpdateOutcome::Failed(message) => {
                                    format!("{}: {}", platform.label(), message)
                                }
                                _ => platform.label().to_owned(),
                            },
                            colors.warning,
                        )
                    };
                    ui.colored_label(color, text);
                }
            });
        }

        // Connect/disconnect applies to every configured account at once;
        // the aggregate status line summarizes the multi-worker state.
        ui.horizontal_wrapped(|ui| {
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
            ui.small(self.tr("chat_multi_hint"));
        });
        ui.add_space(4.0);

        // Alerts: the local ingestion queue is surfaced as chat entries. The
        // optional loopback webhook receiver (Settings) feeds the same queue;
        // the preview button adds one deterministic sample per alert kind so
        // the streamer can check the dock layout offline and GUI tests verify
        // the wiring.
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.tr("alert_preview_button")).clicked() {
                self.alert_preview_dirty = true;
            }
            ui.small(self.tr("alert_dock_hint"));
        });

        // Twitch-only server requirement surfaced as soon as a worker sees
        // the notice: the bot account must be phone-verified before it can
        // send. Shown until reconnect so the streamer fixes the account.
        if self
            .chat_worker_multi
            .as_ref()
            .is_some_and(|w| w.phone_verification_required())
        {
            let colors = theme::StatusColors::for_ui(ui);
            ui.colored_label(colors.warning, self.tr("chat_phone_verification"));
        }

        // Message list (newest at the bottom, autoscroll to the last
        // message). Each line carries a platform badge from the tag the
        // parsers set, so combined traffic stays attributable; alerts keep
        // their platform when the ingestion source provides one.
        let reply_arrow = "\u{21a9}";
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
                        if let Some(platform) = message.platform {
                            ui.small(egui::RichText::new(format!("[{}]", platform.label())));
                        }
                        // Twitch Shared Chat: messages duplicated from another
                        // participating channel carry `source-room-id`; badge
                        // them so viewers can be attributed to their origin.
                        if let Some(source_room) = message.source_room_id.as_deref() {
                            // Resolve the numeric room id to a channel login
                            // in the background (cached); until the result
                            // lands the raw id renders — same shape, honest
                            // fallback.
                            if let Some(label) = self.shared_room_badge(source_room) {
                                ui.small(egui::RichText::new(format!("[\u{21aa} {label}]")))
                                    .on_hover_text(
                                        self.tr_fmt("chat_shared_chat_source_tooltip", &[label]),
                                    );
                            }
                        }
                        ui.label(text);
                        if message.action {
                            ui.label(egui::RichText::new(format!("*{}", message.text)).italics());
                        } else {
                            ui.label(&message.text);
                        }
                        // Twitch messages carry an `id=...` tag; offer a
                        // threaded reply so the bot answers that exact line.
                        if let Some(msg_id) = &message.id {
                            if theme::icon_button(ui, reply_arrow)
                                .on_hover_text(&reply_tooltip)
                                .clicked()
                            {
                                self.chat_reply_target = Some((
                                    message.user.clone(),
                                    message
                                        .platform
                                        .unwrap_or(rivulet_core::ChatPlatform::Twitch),
                                    msg_id.clone(),
                                ));
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

        // Send input: a broadcast goes to every capable account at once.
        // Enabled while any worker is connected (per-account readiness and
        // rate limits are enforced per platform; partial failures are
        // reported below). A YouTube-only roster stays read-only.
        let any_capable = self
            .chat_accounts
            .iter()
            .any(|a| a.platform != rivulet_core::ChatPlatform::YouTube);
        let can_send = self.chat_state == rivulet_core::ChatConnState::Connected && any_capable;
        if can_send {
            // When a reply target is armed, a banner names the message being
            // answered and the Send button enqueues a threaded reply instead
            // of a plain chat message.
            let reply_user = self
                .chat_reply_target
                .as_ref()
                .map(|(user, _, _)| user.clone());
            if let Some(user) = reply_user {
                let reply_label = self.tr_fmt("chat_reply_to", &[user]);
                let reply_cancel = self.tr("chat_reply_cancel").to_owned();
                ui.horizontal_wrapped(|ui| {
                    ui.small(egui::RichText::new(format!("\u{21a9} {reply_label}")).italics());
                    if theme::icon_button(ui, "\u{2715}")
                        .on_hover_text(reply_cancel)
                        .clicked()
                    {
                        self.cancel_chat_reply();
                    }
                });
            }
            // Send-budget line directly above the input: shows the tightest
            // budget across accounts, so throttling is visible *before* a
            // send is silently dropped on any platform. Rendered as an
            // explicitly wrapping label so a long German notice can never be
            // clipped at the right edge of a narrow dock column.
            if let Some((remaining, capacity, window_secs, platform)) =
                self.chat_rate_limit_detail()
            {
                let colors = theme::StatusColors::for_ui(ui);
                let budget = remaining.floor().max(0.0) as u64;
                let cap = capacity as u64;
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
            // Per-platform outcome of the last broadcast (visible until the
            // next send): rejected legs are named so a read-only or
            // rate-limited account never fails silently.
            if !self.chat_last_send_outcomes.is_empty() {
                let failed: Vec<&str> = self
                    .chat_last_send_outcomes
                    .iter()
                    .filter(|(_, ok)| !ok)
                    .map(|(p, _)| p.label())
                    .collect();
                if !failed.is_empty() {
                    let colors = theme::StatusColors::for_ui(ui);
                    let detail = self.tr_fmt("chat_send_partial", &[failed.join(", ")]);
                    ui.add(
                        egui::Label::new(egui::RichText::new(detail).small().color(colors.warning))
                            .wrap(),
                    );
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
        } else if !any_capable && !self.chat_accounts.is_empty() {
            ui.small(self.tr("chat_read_only"));
        } else {
            ui.small(self.tr("chat_send_locked"));
        }
    }

    /// Empty the alerts dock list and the pending ingestion queue (the
    /// dock's Clear button; a standalone helper so tests can exercise it).
    fn clear_alert_events(&mut self) {
        self.alert_events.clear();
        self.alert_ingest.drain();
    }

    /// Render the dedicated alerts dock: every drained alert event from all
    /// ingestion sources (loopback webhook receiver, outbound EventSub,
    /// preview) in one live, bottom-anchored list. Each line carries the
    /// localized, provider-neutral alert text and the platform badge from
    /// the event's origin tag, so multi-platform traffic stays attributable.
    fn draw_alerts_dock(&mut self, ui: &mut egui::Ui, max_list_height: f32) {
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(self.tr("alerts_dock_title")).strong());
            if ui.small_button(self.tr("alerts_dock_clear")).clicked() {
                self.clear_alert_events();
            }
        });
        ui.small(self.tr("alerts_dock_hint"));
        ui.separator();

        let events = std::mem::take(&mut self.alert_events);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .max_height(max_list_height)
            .show(ui, |ui| {
                for message in &events {
                    let mut text = egui::RichText::new(message.user.as_str()).strong();
                    if let Some(rgb) = message.color.as_deref().and_then(color_to_egui) {
                        text = text.color(rgb);
                    }
                    ui.horizontal_wrapped(|ui| {
                        if let Some(platform) = message.platform {
                            ui.small(egui::RichText::new(format!("[{}]", platform.label())));
                        }
                        ui.label(text);
                        ui.label(egui::RichText::new(&message.text).italics());
                    });
                }
            });
        self.alert_events = events;
        if self.alert_events.is_empty() {
            ui.small(self.tr("alerts_dock_empty"));
        }
    }

    /// The VodTrack derived from the Stream view's selectors (issue #78).
    /// `enabled` without `recorded` is normalized to inactive by
    /// `VodTrack::active()`, but the UI keeps both flags explicit so the
    /// leakage-safety rule stays visible instead of silently flipping bits.
    fn stream_vod_track(&self) -> VodTrack {
        VodTrack::new(self.stream_vod_enabled).with_recorded(self.stream_vod_recorded)
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
                self.stop_streaming_session();
            } else {
                let settings = StreamSettings::new(
                    self.stream_platform,
                    self.stream_ingest_url.clone(),
                    self.stream_key.clone(),
                )
                .with_preset(self.stream_preset)
                .with_vod_track(self.stream_vod_track());
                self.engine.set_stream_settings(Some(settings));
                // Clear a stale error so a new stream starts from the
                // Ready/Streaming label (mirrors the recording starts).
                self.last_error = None;
                self.apply_ndi_output();
                self.apply_restream_targets();
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

        // ── VOD track (issue #78): the copyright-safe third audio branch of
        //    the dual-output recording. Two explicit checkboxes keep the
        //    leakage-safety contract visible: `enabled` without `recorded`
        //    never reaches the live ingest, and the active() gate means the
        //    recording branch only appears when both are set. ──
        egui::CollapsingHeader::new(self.tr("stream_vod_section"))
            .id_salt("stream_vod_track")
            .default_open(false)
            .show(ui, |ui| {
                let mut vod_enabled = self.stream_vod_enabled;
                let mut vod_recorded = self.stream_vod_recorded;
                ui.checkbox(&mut vod_enabled, self.tr("stream_vod_enabled"));
                ui.add_enabled_ui(vod_enabled, |ui| {
                    ui.checkbox(&mut vod_recorded, self.tr("stream_vod_recorded"));
                });
                self.stream_vod_enabled = vod_enabled;
                self.stream_vod_recorded = vod_recorded;
                ui.small(self.tr("stream_vod_hint"));
                if !self.stream_vod_track().active() {
                    ui.small(self.tr("stream_vod_inactive"));
                }
            });

        // ── Restream targets (M6): additional platforms streamed
        //    simultaneously via the multi-target fan-out. ──
        self.draw_restream_section(ui);

        // ── Auto-clip (M6): chat-driven replay saves on spike / !clip. ──
        self.draw_auto_clip_section(ui);

        // ── Workspace: chat dock (left) | alerts dock (middle) | stream
        //    information (right). Both lists are bounded in height so they
        //    behave like docked panels on this page rather than growing the
        //    scroll area without end. On narrow windows the columns stack
        //    vertically so the chat connect/send controls never clip
        //    off-screen. ──
        ui.separator();
        if ui.available_width() >= STREAM_WORKSPACE_ALERTS_WIDTH {
            ui.columns(3, |cols| {
                self.draw_chat_dock(&mut cols[0], chat_list_height);
                self.draw_alerts_dock(&mut cols[1], chat_list_height);
                self.draw_stream_information_panel(&mut cols[2], colors);
            });
        } else {
            self.draw_chat_dock(ui, chat_list_height);
            ui.separator();
            self.draw_alerts_dock(ui, chat_list_height);
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
        // Inline per-source mixer strip (issue #154 Phase 2): the same shared
        // strip as the Record view, consistent per the design doc.
        self.draw_inline_audio_mixer(ui);

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
        // Migration: configs saved before the combined chat dock carried a
        // single platform/channel/token triple. Those become the first entry
        // of the account list; fresh configs (all empty) are left alone.
        if restored.chat_accounts.is_empty()
            && (!restored.chat_channel.trim().is_empty()
                || !restored.chat_oauth_token.trim().is_empty())
        {
            tracing::info!(
                platform = ?restored.chat_platform,
                "Migrating legacy single-platform chat config into the account list"
            );
            // The legacy token was stored in the config file; move it to the
            // OS credential vault and wipe the in-memory copy so the next
            // save writes a token-free config.
            let platform = restored.chat_platform;
            let channel = std::mem::take(&mut restored.chat_channel);
            let token = std::mem::take(&mut restored.chat_oauth_token);
            if !token.trim().is_empty() {
                let _ = rivulet_core::ChatTokenStore::default().save(platform, &channel, &token);
            }
            restored
                .chat_accounts
                .push(rivulet_core::ChatAccount::new(platform, channel));
        }
        // Push the restored audio sources into the engine so the first
        // session starts with the persisted routing (issue #154 Phase 2).
        if !restored.audio_sources.is_empty() {
            tracing::info!(
                count = restored.audio_sources.len(),
                "Restoring audio routing"
            );
            restored
                .engine
                .set_audio_sources(restored.audio_sources.clone());
            restored.audio_mixer_needs_sync = false;
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
        } else {
            // First launch: detect the OS locale so the UI starts in the
            // user's preferred language without requiring a manual switch.
            app.locale = detect_os_locale();
        }
        // Apply the persisted color scheme (fonts + palette + preference)
        // immediately, so the first frame already renders with the right theme.
        theme::init(&cc.egui_ctx, app.theme);
        app.theme_applied = Some(app.theme);
        // Apply the motion preference before the first frame (ui-005): the
        // cached OS probe starts empty, so the first ui() call would otherwise
        // render one animated frame before reducing.
        app.os_reduced_motion = Some(theme::os_prefers_reduced_motion());
        app.os_reduced_motion_probed_at = Some(Instant::now());
        let reduced = app
            .motion_preference
            .resolves_to_reduced(app.os_reduced_motion.unwrap_or(false));
        theme::apply_motion(&cc.egui_ctx, reduced);
        app.motion_applied = Some(reduced);
        #[cfg(target_os = "windows")]
        {
            app.refresh_capture_sources();
        }
        #[cfg(target_os = "linux")]
        {
            app.refresh_linux_sources();
        }
        // Mirror the persisted telemetry opt-in into the runtime collector
        // (disabled by default) and report the once-per-session Startup event
        // when the user opted in.
        app.apply_telemetry_policy();
        // Mirror the persisted alert-ingestion toggle into the local queue.
        app.apply_alerts_policy();
        // Mirror the persisted webhook-receiver settings into the loopback
        // listener (disabled by default).
        app.apply_alerts_receiver();
        // Discover plugin bundles so the Settings → Plugins list is populated
        // on first paint (re-scanned on demand via the Rescan button).
        app.rescan_plugins();
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

        // Flip "Finalizing recording…" → "Recording saved." as soon as the
        // background stop teardown (EOS, auto-remux, cloud upload) finished.
        self.poll_stop_finalization();
        // Push mixer edits into the engine before any session can start
        // (issue #154 Phase 2). No-op unless the mixer flagged a change.
        self.sync_audio_routing();
        // Keep repainting while a stop finalization is in flight, so the
        // status flips without requiring user input to trigger a frame.
        if self.stop_finalizing.is_some() {
            ctx.request_repaint();
        }

        // Reconcile OS-level global hotkeys (create/rebind on change) and
        // dispatch any global hotkey events fired while the app was unfocused.
        self.reconcile_global_hotkeys();

        // Reconcile the OBS WebSocket server: start/stop it with the setting,
        // sync the read snapshot, execute remote commands, and broadcast
        // GUI-initiated changes to connected Stream Deck/TouchPortal clients.
        self.reconcile_obs_websocket();

        // Reconcile the mobile & HTTP remote companion: keep the embedded
        // page server (M6) running while the obs-websocket surface is up.
        self.reconcile_remote_companion();

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

        // Apply the motion preference (ui-005, WCAG 2.3.3): egui animation
        // time snaps to zero under reduced motion, so every animate_bool
        // fade honors the OS/preference setting. Cheap after the first probe
        // (the OS query itself runs at most every 10 s).
        self.update_motion_preference(ctx);

        // Read this before entering `ctx.input`. That closure holds egui's
        // context write lock; calling `egui_wants_keyboard_input()` inside it
        // tries to acquire a nested read lock and deadlocks in debug builds.
        let wants_keyboard_input = ctx.egui_wants_keyboard_input();

        ctx.input(|i| {
            #[cfg(target_os = "linux")]
            let any_recording = self.is_recording || self.is_aux_recording;
            #[cfg(target_os = "windows")]
            let any_recording = self.is_windows_recording || self.is_aux_recording;
            #[cfg(target_os = "macos")]
            let any_recording = self.is_recording || self.is_aux_recording;
            #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
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
                        #[cfg(target_os = "macos")]
                        self.stop_macos_recording();
                    }
                    self.is_paused = false;
                    self.is_muted = false;
                } else {
                    #[cfg(target_os = "windows")]
                    self.start_windows_recording();
                    #[cfg(target_os = "linux")]
                    self.start_linux_recording();
                    #[cfg(target_os = "macos")]
                    self.start_macos_recording();
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
                let mut switch_targets: Vec<uuid::Uuid> = Vec::new();
                for (scene_id, binding) in &self.hotkeys.scene_hotkeys {
                    if binding.pressed_in(i) {
                        switch_targets.push(*scene_id);
                    }
                }
                for scene_id in switch_targets {
                    self.switch_active_scene(scene_id);
                }
                // Delete the selected composition source. Destructive and
                // repeat-sensitive: it is in-app only (never OS-global) and
                // shares the text-input guard so Delete keeps working for
                // ordinary text editing (rename fields, chat input).
                if self.hotkeys.delete_source.pressed_in(i) {
                    self.delete_selected_composition_source();
                }
                // Scene-item copy/paste (issue #192): in-app only and gated on
                // the Scenes view so Ctrl+C/V keep their text-edit meaning in
                // every other view (chat, rename fields, stream info editor).
                if self.view == AppView::Scenes {
                    if i.key_pressed(egui::Key::C) && i.modifiers.command {
                        self.copy_selected_composition_source();
                    }
                    if i.key_pressed(egui::Key::V) && i.modifiers.command {
                        self.paste_scene_item_clipboard();
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
                    // Issue #192: paste/duplicate undo takes priority — the
                    // most recent mutation wins, and source pastes have their
                    // own stack (the M2 scene stack only covers scenes).
                    self.dispatch_scene_undo();
                }
                Some(SceneHistoryShortcut::Redo) => {
                    // Same priority as undo: a fresh paste-redo beats scene redo.
                    self.dispatch_scene_redo();
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

            // ── Phase 3/4: per-application audio capture (issue #154) ──
            self.sync_app_audio_captures();
            self.drain_app_audio_frames();
            // ── Issue #229: WASAPI device endpoint capture ──
            self.sync_device_audio_captures();
            self.drain_device_audio_frames();
        }

        #[cfg(target_os = "linux")]
        {
            // ── Phase 4: PipeWire per-application audio capture (issue #154) ──
            self.sync_app_audio_captures();
            self.drain_app_audio_frames();
            // ── Issue #231: PipeWire device node capture ──
            self.sync_device_audio_captures();
            self.drain_device_audio_frames();
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

        // ── macOS recording drain (xcap screen/window capture) ────────
        #[cfg(target_os = "macos")]
        {
            if self.is_recording {
                self.drain_macos_frames();
                // Surface capture/engine failures in the UI and abort when no
                // frames arrive within the timeout (capture thread died).
                if let Some(err) = self.engine.take_error() {
                    self.last_error = Some(err);
                }
                if should_abort_for_no_frames(
                    self.record_started,
                    self.last_frame_at,
                    Instant::now(),
                    self.no_frame_timeout,
                ) {
                    let secs = self.no_frame_timeout.as_secs();
                    self.last_error = Some(self.tr_fmt("recording_no_frames", &[secs.to_string()]));
                    self.stop_macos_recording();
                }
            }

            // ── Phase 5: per-application audio fallback capture (issue #154) ──
            self.sync_app_audio_captures();
            self.drain_app_audio_frames();
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
                // Global status/error surface (audit finding ui-007): a
                // failure raised in a background tab must be visible without
                // navigating there. Clicking it jumps to the owning view.
                self.draw_global_issue_footer(ui);
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
                                let window_entries = windows_on_selected_monitor(
                                    &self.windows,
                                    &self.monitors,
                                    self.selected_monitor_idx,
                                );
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
                                        for (i, window) in &window_entries {
                                            if ui
                                                .selectable_label(
                                                    self.selected_window_idx == Some(*i),
                                                    window.title().unwrap_or_default(),
                                                )
                                                .clicked()
                                            {
                                                self.selected_window_idx = Some(*i);
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
                        // Stop feedback ("Finalizing recording…" → "Recording
                        // saved.") below the controls, visible in both the
                        // recording and the idle state.
                        if let Some(status) = &self.record_status {
                            ui.colored_label(colors.info, status);
                        }
                    }

                    #[cfg(target_os = "linux")]
                    {
                        self.drain_linux_frames();
                        // Surface engine failures (pipeline build/start/push errors)
                        // in the UI instead of only on the console.
                        if let Some(err) = self.engine.take_error() {
                            // An error beats the "Recording saved." flip of a
                            // still-pending background stop.
                            self.stop_finalizing = None;
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
                            // stop_linux_recording arms the background
                            // finalization; override its status with the abort
                            // reason and cancel the later "Recording saved." flip.
                            self.stop_linux_recording();
                            self.stop_finalizing = None;
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
                                    let window_entries = windows_on_selected_monitor(
                                        &self.windows,
                                        &self.monitors,
                                        self.selected_monitor_idx,
                                    );
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
                                            for (i, w) in &window_entries {
                                                if ui
                                                    .selectable_label(
                                                        self.selected_window_idx == Some(*i),
                                                        format!(
                                                            "\"{}\" ({}x{})",
                                                            w.title().unwrap_or_default(),
                                                            w.width().unwrap_or(0),
                                                            w.height().unwrap_or(0)
                                                        ),
                                                    )
                                                    .clicked()
                                                {
                                                    self.selected_window_idx = Some(*i);
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
                                    ui.label(self.tr("eq_system"));
                                    ui.horizontal_wrapped(|ui| {
                                        for band in self.system_filters.eq_bands.iter_mut() {
                                            ui.add(egui::Slider::new(band, -12.0..=12.0));
                                        }
                                    });
                                    ui.label(self.tr("eq_microphone"));
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

                    // Multi-track audio routing matrix (issue #154 Phase 2):
                    // the Mixer view is the one place with the full matrix.
                    if self.view == AppView::Mixer {
                        self.sync_audio_routing();
                        self.draw_mixer_sources(ui);
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

                    #[cfg(target_os = "macos")]
                    if self.view == AppView::Record {
                        self.refresh_macos_sources();
                        self.draw_macos_record_view(ui, &colors);
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

                        // Plugin permission-review dialog (drawn on top of
                        // the settings view while a review is open).
                        self.draw_plugin_review_dialog(ui.ctx());

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

                        // Settings: motion (ui-005, WCAG 2.3.3)
                        ui.label(egui::RichText::new(self.tr("motion")).strong());
                        ui.horizontal(|ui| {
                            for preference in theme::MotionPreference::all() {
                                if ui
                                    .selectable_label(
                                        self.motion_preference == *preference,
                                        self.tr(preference.key()),
                                    )
                                    .clicked()
                                {
                                    self.motion_preference = *preference;
                                }
                            }
                        });
                        ui.label(egui::RichText::new(self.tr("motion_hint")).small().weak());

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

                        // Settings: opt-in usage telemetry (off by default,
                        // privacy-first; see docs/telemetry.md).
                        let telemetry_section = self.tr("telemetry_section");
                        let telemetry_enable = self.tr("telemetry_enable");
                        let telemetry_note = self.tr("telemetry_note");
                        ui.separator();
                        ui.label(egui::RichText::new(telemetry_section).strong());
                        if ui
                            .checkbox(&mut self.telemetry_enabled, telemetry_enable)
                            .changed()
                        {
                            self.apply_telemetry_policy();
                        }
                        ui.small(telemetry_note);

                        // Settings: native alert ingestion (follows/subs/
                        // donations/raids) surfaced in the chat dock. Purely
                        // local, never transmitted; the optional loopback
                        // webhook receiver feeds the same queue (see
                        // docs/alerts-ingest.md for the honest scope).
                        let alert_section = self.tr("alert_section");
                        let alert_enable = self.tr("alert_enable");
                        let alert_note = self.tr("alert_note");
                        ui.separator();
                        ui.label(egui::RichText::new(alert_section).strong());
                        if ui
                            .checkbox(&mut self.alert_ingest_enabled, alert_enable)
                            .changed()
                        {
                            self.apply_alerts_policy();
                        }
                        ui.small(alert_note);

                        // Settings: loopback webhook receiver feeding the
                        // ingestion queue (Streamlabs donation + Twitch
                        // EventSub with HMAC verification; 127.0.0.1 only).
                        let receiver_section = self.tr("alert_receiver_section");
                        let receiver_enable = self.tr("alert_receiver_enable");
                        let receiver_port = self.tr("alert_receiver_port");
                        let receiver_secret = self.tr("alert_receiver_secret");
                        let receiver_secret_note = self.tr("alert_receiver_secret_note");
                        let receiver_note = self.tr("alert_receiver_note");
                        ui.separator();
                        ui.label(egui::RichText::new(receiver_section).strong());
                        if ui
                            .checkbox(&mut self.alerts_receiver_enabled, receiver_enable)
                            .changed()
                        {
                            self.alerts_receiver_error = None;
                        }
                        ui.horizontal(|ui| {
                            ui.label(receiver_port);
                            ui.add(
                                egui::DragValue::new(&mut self.alerts_receiver_port)
                                    .range(1..=65535),
                            );
                        });
                        egui::Grid::new("alerts_receiver_secret_grid")
                            .num_columns(2)
                            .show(ui, |ui| {
                                ui.label(receiver_secret);
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.alerts_twitch_secret)
                                        .password(true)
                                        .desired_width(220.0),
                                );
                                ui.end_row();
                            });
                        ui.small(receiver_secret_note);
                        ui.small(receiver_note);
                        if let Some(err) = &self.alerts_receiver_error {
                            let colors = theme::StatusColors::for_ui(ui);
                            let msg = err.clone();
                            ui.colored_label(
                                colors.warning,
                                self.tr_fmt("alert_receiver_error", std::slice::from_ref(&msg)),
                            );
                        } else if let Some(rx) = &self.alerts_receiver {
                            let addr = rx.addr().to_string();
                            ui.small(
                                self.tr_fmt("alert_receiver_running", std::slice::from_ref(&addr)),
                            );
                        }

                        // Settings: outbound Twitch EventSub WebSocket — a
                        // native, forwarder-free delivery path that pushes
                        // follows/subs/gifts/raids straight into the same
                        // ingestion queue (see docs/alerts-ingest.md).
                        let eventsub_section = self.tr("alert_eventsub_section");
                        let eventsub_enable = self.tr("alert_eventsub_enable");
                        let eventsub_client_id = self.tr("alert_eventsub_client_id");
                        let eventsub_token = self.tr("alert_eventsub_token");
                        let eventsub_broadcaster = self.tr("alert_eventsub_broadcaster");
                        let eventsub_raid_direction = self.tr("alert_eventsub_raid_direction");
                        let eventsub_note = self.tr("alert_eventsub_note");
                        ui.separator();
                        ui.label(egui::RichText::new(eventsub_section).strong());
                        if ui
                            .checkbox(&mut self.alerts_eventsub_enabled, eventsub_enable)
                            .changed()
                        {
                            self.alerts_eventsub_error = None;
                        }
                        ui.horizontal(|ui| {
                            let colors = theme::StatusColors::for_ui(ui);
                            if let Some(eventsub) = &self.alerts_eventsub {
                                if eventsub.connected() {
                                    let subs = eventsub.stats().2.to_string();
                                    ui.colored_label(
                                        colors.success,
                                        self.tr_fmt(
                                            "alert_eventsub_connected",
                                            std::slice::from_ref(&subs),
                                        ),
                                    );
                                }
                            }
                        });
                        egui::Grid::new("alerts_eventsub_credentials_grid")
                            .num_columns(2)
                            .show(ui, |ui| {
                                ui.label(eventsub_client_id);
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.alerts_eventsub_client_id)
                                        .password(true)
                                        .desired_width(220.0),
                                );
                                ui.end_row();
                                ui.label(eventsub_token);
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.alerts_eventsub_token)
                                        .password(true)
                                        .desired_width(220.0),
                                );
                                ui.end_row();
                                ui.label(eventsub_broadcaster);
                                ui.add(
                                    egui::TextEdit::singleline(
                                        &mut self.alerts_eventsub_broadcaster_id,
                                    )
                                    .desired_width(220.0),
                                );
                                ui.end_row();
                                // Raid direction: selects which channel.raid
                                // condition the worker subscribes with
                                // (from_/to_, or two subscriptions for both).
                                // Changing it feeds the config comparison in
                                // `apply_alerts_eventsub`, which restarts the
                                // worker so the new subscriptions are created.
                                ui.label(eventsub_raid_direction);
                                // Labels are pre-resolved outside the closure
                                // so the mutable field borrow and the `self.tr`
                                // immutable borrow never overlap.
                                let raid_labels: Vec<(rivulet_core::RaidAlertDirection, String)> =
                                    rivulet_core::RaidAlertDirection::all()
                                        .iter()
                                        .map(|d| (*d, self.raid_direction_label(*d)))
                                        .collect();
                                let selected_raid =
                                    self.raid_direction_label(self.alerts_raid_direction);
                                egui::ComboBox::from_id_salt("alerts_raid_direction")
                                    .selected_text(selected_raid)
                                    .show_ui(ui, |ui| {
                                        for (direction, label) in &raid_labels {
                                            ui.selectable_value(
                                                &mut self.alerts_raid_direction,
                                                *direction,
                                                label.clone(),
                                            );
                                        }
                                    });
                                ui.end_row();
                            });
                        if let Some(err) = &self.alerts_eventsub_error {
                            let colors = theme::StatusColors::for_ui(ui);
                            let msg = err.clone();
                            ui.colored_label(
                                colors.warning,
                                self.tr_fmt("alert_eventsub_error", std::slice::from_ref(&msg)),
                            );
                        }
                        ui.small(eventsub_note);

                        // Settings: NDI output — LAN monitor feed published
                        // next to a recording/streaming session (M5 #77).
                        let ndi_section = self.tr("ndi_section");
                        let ndi_enable = self.tr("ndi_enable");
                        let ndi_name = self.tr("ndi_name");
                        let ndi_group = self.tr("ndi_group");
                        let ndi_hint = self.tr("ndi_hint");
                        let ndi_plugin_missing = self.tr("ndi_plugin_missing");
                        ui.separator();
                        ui.label(egui::RichText::new(ndi_section).strong());
                        let ndi_colors = theme::StatusColors::for_ui(ui);
                        if ui
                            .checkbox(&mut self.ndi_output_enabled, ndi_enable)
                            .changed()
                        {
                            self.apply_ndi_output();
                        }
                        ui.horizontal(|ui| {
                            ui.label(ndi_name);
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.ndi_output_name)
                                        .hint_text("Rivulet")
                                        .desired_width(180.0),
                                )
                                .changed()
                            {
                                self.apply_ndi_output();
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(ndi_group);
                            if ui
                                .add(
                                    egui::TextEdit::singleline(&mut self.ndi_output_group)
                                        .desired_width(180.0),
                                )
                                .changed()
                            {
                                self.apply_ndi_output();
                            }
                        });
                        ui.small(ndi_hint);
                        if let Some(warning) = &self.ndi_warning {
                            ui.colored_label(ndi_colors.error, warning);
                        }
                        if self.ndi_output_enabled
                            && !rivulet_core::NdiOutput::new(self.ndi_output_name.trim(), true)
                                .plugin_available()
                        {
                            ui.colored_label(ndi_colors.warning, ndi_plugin_missing);
                        }

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
                        for action in ["record", "pause", "mute", "save_replay", "delete_source"] {
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

                        // Settings: Mobile & HTTP remote companion (M6). Serves a
                        // phone/browser page on the LAN that switches scenes and
                        // controls recording/streaming through the authenticated
                        // obs-websocket server above (see docs/remote-companion.md).
                        let companion_section = self.tr("remote_companion_section");
                        let companion_enable = self.tr("remote_companion_enable");
                        let companion_port = self.tr("remote_companion_port");
                        let companion_bind_lan = self.tr("remote_companion_bind_lan");
                        let companion_bind_lan_hint = self.tr("remote_companion_bind_lan_hint");
                        let companion_allow_stream_control =
                            self.tr("remote_companion_allow_stream_control");
                        let companion_allow_stream_control_hint =
                            self.tr("remote_companion_allow_stream_control_hint");
                        let companion_open_page = self.tr("remote_companion_open_page");
                        ui.separator();
                        ui.label(egui::RichText::new(companion_section).strong());
                        let was_companion_enabled = self.remote_companion_enabled;
                        let was_bind_lan = self.remote_companion_bind_lan;
                        ui.checkbox(&mut self.remote_companion_enabled, companion_enable);
                        let mut companion_port_dirty = false;
                        let old_companion_port = self.remote_companion_port;
                        ui.horizontal(|ui| {
                            ui.label(companion_port);
                            companion_port_dirty = ui
                                .add(
                                    egui::DragValue::new(&mut self.remote_companion_port)
                                        .range(1..=65535)
                                        .speed(1),
                                )
                                .changed();
                        });
                        ui.checkbox(&mut self.remote_companion_bind_lan, companion_bind_lan);
                        ui.small(companion_bind_lan_hint);
                        // Widening the obs bind or serving the page from a new
                        // port requires the obs server (and page) to restart.
                        if companion_port_dirty && old_companion_port != self.remote_companion_port
                        {
                            self.obs_ws_restart = true;
                        }
                        if was_companion_enabled != self.remote_companion_enabled
                            || was_bind_lan != self.remote_companion_bind_lan
                        {
                            self.obs_ws_restart = true;
                        }
                        if self.remote_companion_bind_lan {
                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    &mut self.remote_allow_stream_control,
                                    companion_allow_stream_control,
                                );
                                ui.small(companion_allow_stream_control_hint);
                            });
                        }
                        if let Some(status) = &self.remote_companion_status {
                            ui.colored_label(
                                if self.remote_companion_enabled && self.obs_ws_enabled {
                                    colors.success
                                } else {
                                    colors.warning
                                },
                                status,
                            );
                        }
                        if (self.remote_companion_enabled && !was_companion_enabled)
                            || (!self.remote_companion_enabled && was_companion_enabled)
                        {
                            // Flip the status line immediately on the toggle so
                            // the section does not show a stale message.
                            self.remote_companion_status = if self.remote_companion_enabled {
                                Some(self.tr("remote_companion_starting").to_owned())
                            } else {
                                Some(self.tr("remote_companion_stopped").to_owned())
                            };
                        }
                        if self.remote_companion_url.is_some()
                            && ui.button(companion_open_page).clicked()
                        {
                            self.open_remote_companion_page();
                        }
                        ui.small(self.tr("remote_companion_hint"));

                        // Settings: Plugins — discovered bundles, permission
                        // review and enable/disable (plugin-system RFC Phase 3).
                        self.draw_plugins_section(ui);

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

        // Destructive confirmation modal: rendered last so it always sits on
        // top of every dock and drawer.
        self.draw_confirmation_modal(ui.ctx());
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

/// The capture targets for the WASAPI per-application backend (Phase 3,
/// issue #154): every Application-kind source that is routed to at least one
/// output and carries a resolved `pid:<n>` device id. Pure so the mirror
/// contract is testable on every platform.
fn routed_application_targets(sources: &[AudioSource]) -> Vec<(uuid::Uuid, u32)> {
    sources
        .iter()
        .filter(|s| {
            s.kind == rivulet_core::AudioSourceKind::Application
                && s.device_pid().is_some()
                && (s.routing.record || s.routing.stream)
        })
        .map(|s| (s.id, s.device_pid().expect("filtered above")))
        .collect()
}

/// Label for one device in the device picker (issues #229/#231): the
/// friendly name plus the localized default marker for the flow's default
/// device. Pure so the picker contract is testable.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn audio_device_picker_label(
    device: &rivulet_audio::AudioDeviceInfo,
    default_marker: &str,
) -> String {
    if device.is_default {
        format!("{} {default_marker}", device.name)
    } else {
        device.name.clone()
    }
}

/// Strip label for one audio source (issues #229/#231): a device source
/// shows its friendly device name (resolved from the current device list,
/// falling back to the raw device id) instead of the cryptic device id;
/// every other source keeps its stored name.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn audio_device_strip_label(
    source: &AudioSource,
    devices: &[rivulet_audio::AudioDeviceInfo],
) -> String {
    match source.wasapi_device() {
        Some(target) => devices
            .iter()
            .find(|d| d.device_id() == target.device_id())
            .map(|d| d.name.clone())
            .unwrap_or_else(|| target.device_id()),
        None => source.name.clone(),
    }
}

/// The capture targets for the device backend (issues #229/#231): every
/// Input/Output-kind source carrying a parseable `wasapi-out:`/`wasapi-in:`
/// (Windows) or `pw-src:`/`pw-mon:` (Linux) device id that is routed to at
/// least one output. Pure so the mirror contract is testable; the capture
/// lifecycle itself is platform-backed.
#[cfg(any(target_os = "windows", target_os = "linux"))]
fn routed_device_targets(sources: &[AudioSource]) -> Vec<(uuid::Uuid, String)> {
    sources
        .iter()
        .filter(|s| {
            (s.kind == rivulet_core::AudioSourceKind::InputDevice
                || s.kind == rivulet_core::AudioSourceKind::OutputDevice)
                && s.wasapi_device().is_some()
                && (s.routing.record || s.routing.stream)
        })
        .map(|s| {
            let device_id = s.wasapi_device().expect("filtered above").device_id();
            (s.id, device_id)
        })
        .collect()
}

/// Best-effort, deterministic classification of a recording error into a
/// stable telemetry category. The raw error text is localized and free-form,
/// so it can never be serialized; only the category code is reported. The
/// mapping is intentionally conservative — unrecognized text falls back to
/// `Unknown` instead of being guessed.
fn classify_record_error(error: &str) -> rivulet_core::TelemetryErrorKind {
    let hay = error.to_lowercase();
    for (needles, kind) in [
        (
            &["permission", "denied", "access is denied", "zugriff"][..],
            rivulet_core::TelemetryErrorKind::Permission,
        ),
        (
            &[
                "source",
                "quelle",
                "capture",
                "frame",
                "graphics capture",
                "desktop duplication",
            ][..],
            rivulet_core::TelemetryErrorKind::Capture,
        ),
        (
            &["encode", "encoder", "output", "mp4", "mux", "container"][..],
            rivulet_core::TelemetryErrorKind::Output,
        ),
        (
            &["io", "disk", "write", "file", "eingabe/ausgabe"][..],
            rivulet_core::TelemetryErrorKind::Io,
        ),
        (
            &["engine", "pipeline", "gstreamer", "element", "session"][..],
            rivulet_core::TelemetryErrorKind::Engine,
        ),
    ] {
        if needles.iter().any(|n| hay.contains(n)) {
            return kind;
        }
    }
    rivulet_core::TelemetryErrorKind::Unknown
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

/// Compare two xcap monitors for identity, preferring the stable device id
/// and falling back to the display rect plus name where ids are not usable.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn same_monitor(a: &xcap::Monitor, b: &xcap::Monitor) -> bool {
    if let (Ok(ida), Ok(idb)) = (a.id(), b.id()) {
        return ida == idb;
    }
    let rect = |m: &xcap::Monitor| {
        (
            m.x().unwrap_or(0),
            m.y().unwrap_or(0),
            m.width().unwrap_or(0),
            m.height().unwrap_or(0),
            m.name().unwrap_or_default(),
        )
    };
    rect(a) == rect(b)
}

/// Keep only entries whose window resides on the selected monitor, preserving
/// the original (global) indices so callers keep storing a selection into the
/// unfiltered window list. When `selected_monitor_matched` is `false` (no
/// monitor selected or the index does not resolve) every entry is kept;
/// otherwise only entries for which `on_selected_monitor` returns `true` are
/// kept. Pure and unit-tested: the platform wrappers below only supply the
/// platform-specific monitor lookup.
fn keep_on_selected_monitor<T>(
    windows: Vec<T>,
    selected_monitor_matched: bool,
    on_selected_monitor: impl Fn(&T) -> bool,
) -> Vec<(usize, T)> {
    if !selected_monitor_matched {
        return windows.into_iter().enumerate().collect();
    }
    windows
        .into_iter()
        .enumerate()
        .filter(|(_, w)| on_selected_monitor(w))
        .collect()
}

/// Build the window entries for a monitor-scoped window dropdown.
///
/// When a monitor is selected only windows residing on that monitor are
/// returned (matched via `xcap::Window::current_monitor`). The returned
/// indices are positions in the *unfiltered* `windows` slice so callers can
/// keep `selected_window_idx` pointing into the unmodified list. With no
/// monitor selected every window is returned.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn windows_on_selected_monitor(
    windows: &[xcap::Window],
    monitors: &[xcap::Monitor],
    selected_monitor_idx: Option<usize>,
) -> Vec<(usize, xcap::Window)> {
    let selected_monitor = selected_monitor_idx.and_then(|idx| monitors.get(idx));
    keep_on_selected_monitor(windows.to_vec(), selected_monitor.is_some(), |w| {
        selected_monitor
            .and_then(|m| w.current_monitor().ok().map(|wm| same_monitor(&wm, m)))
            .unwrap_or(false)
    })
}

/// Compare two windows-capture monitors for identity, preferring the device
/// index and falling back to the friendly name where the index is unavailable.
#[cfg(target_os = "windows")]
fn same_backend_monitor(a: &Monitor, b: &Monitor) -> bool {
    match (a.index(), b.index()) {
        (Ok(a_idx), Ok(b_idx)) => a_idx == b_idx,
        _ => a
            .name()
            .ok()
            .zip(b.name().ok())
            .is_some_and(|(x, y)| x == y),
    }
}

/// Windows variant of `windows_on_selected_monitor` using the
/// windows-capture backend (`Window::monitor` + `Monitor::index`).
/// The returned indices are positions in the *unfiltered* `windows` slice so
/// callers can keep `selected_window_idx` pointing into the unmodified list.
#[cfg(target_os = "windows")]
fn windows_on_selected_monitor(
    windows: &[Window],
    monitors: &[Monitor],
    selected_monitor_idx: Option<usize>,
) -> Vec<(usize, Window)> {
    let selected_monitor = selected_monitor_idx.and_then(|idx| monitors.get(idx));
    keep_on_selected_monitor(windows.to_vec(), selected_monitor.is_some(), |w| {
        selected_monitor.is_some_and(|m| w.monitor().is_some_and(|wm| same_backend_monitor(m, &wm)))
    })
}

/// Detect the user's preferred language from the OS environment on first
/// launch (no persisted locale). Checks `LC_MESSAGES`, `LANG`, and on
/// Windows also `GetUserDefaultUILanguage` via the `LANG` env var that
/// modern Windows terminals export.
fn detect_os_locale() -> Locale {
    // Try LC_MESSAGES first (Linux/macOS standard).
    if let Ok(val) = std::env::var("LC_MESSAGES") {
        let locale = Locale::from_code(&val);
        tracing::info!(
            locale = locale.code(),
            "Detected OS locale from LC_MESSAGES"
        );
        return locale;
    }
    // Fallback: LANG (set by most Linux distros and Git Bash on Windows).
    if let Ok(val) = std::env::var("LANG") {
        let locale = Locale::from_code(&val);
        tracing::info!(locale = locale.code(), "Detected OS locale from LANG");
        return locale;
    }
    tracing::info!("No OS locale detected, defaulting to English");
    Locale::En
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
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

    // ── Usage telemetry (M5, opt-in) ──────────────────────────────

    #[test]
    fn telemetry_defaults_to_opt_out() {
        // The M5 privacy posture: telemetry is off unless the user explicitly
        // opts in, and the runtime reporter mirrors the persisted toggle.
        let app = RivuletApp::default();
        assert!(!app.telemetry_enabled, "telemetry must be opt-in");
        assert!(!app.telemetry.enabled(), "reporter must reflect the opt-in");
        assert!(!app.telemetry_startup_reported);
    }

    #[test]
    fn apply_telemetry_policy_mirrors_the_persisted_toggle() {
        let mut app = RivuletApp {
            telemetry_enabled: true,
            ..Default::default()
        };
        app.apply_telemetry_policy();
        assert!(app.telemetry.enabled(), "opted in => reporter enabled");
        assert!(app.telemetry_startup_reported);
        assert_eq!(
            app.telemetry.pending_len(),
            1,
            "Startup must be reported once per session"
        );
        // Re-applying (e.g. after restore) must not duplicate the Startup event.
        app.apply_telemetry_policy();
        assert_eq!(
            app.telemetry.pending_len(),
            1,
            "Startup must not be reported twice"
        );

        // Opting back out disables the reporter and clears everything captured.
        app.telemetry_enabled = false;
        app.apply_telemetry_policy();
        assert!(!app.telemetry.enabled());
        assert_eq!(app.telemetry.pending_len(), 0);
    }

    #[test]
    fn recording_stop_telemetry_reports_duration_and_health() {
        let mut app = RivuletApp {
            telemetry_enabled: true,
            ..Default::default()
        };
        app.apply_telemetry_policy();
        let captured = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let batch = captured.clone();
            app.telemetry
                .set_sink(Box::new(move |b: &rivulet_core::TelemetryBatch| {
                    batch.borrow_mut().push(b.clone())
                }));
        }
        app.last_error = None;
        app.record_started = std::time::Instant::now() - std::time::Duration::from_secs(90);
        app.complete_recording_session_telemetry();
        app.telemetry.flush();
        let batches = captured.borrow();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].events[1],
            rivulet_core::TelemetryEvent::RecordingStop {
                duration_secs: 90,
                healthy: true,
            }
        );
    }

    #[test]
    fn recording_stop_telemetry_marks_unhealthy_sessions() {
        let mut app = RivuletApp {
            telemetry_enabled: true,
            ..Default::default()
        };
        app.apply_telemetry_policy();
        let captured = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let batch = captured.clone();
            app.telemetry
                .set_sink(Box::new(move |b: &rivulet_core::TelemetryBatch| {
                    batch.borrow_mut().push(b.clone())
                }));
        }
        app.last_error = Some("capture failed".to_string());
        app.record_started = std::time::Instant::now() - std::time::Duration::from_secs(5);
        app.complete_recording_session_telemetry();
        app.telemetry.flush();
        let batches = captured.borrow();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].events[1],
            rivulet_core::TelemetryEvent::RecordingError {
                error: rivulet_core::TelemetryErrorKind::Capture,
            }
        );
        assert_eq!(
            batches[0].events[2],
            rivulet_core::TelemetryEvent::RecordingStop {
                duration_secs: 5,
                healthy: false,
            }
        );
    }

    #[test]
    fn recording_errors_classify_into_telemetry_categories() {
        // The raw, localized error text never enters the event stream: only the
        // deterministic category code is recorded. Text is locale-dependent and
        // free-form, so it must not leave the device even as a serialized batch.
        let classifications = [
            (
                "no source selected",
                rivulet_core::TelemetryErrorKind::Capture,
            ),
            (
                "Keine Aufnahmequelle ausgewählt.",
                rivulet_core::TelemetryErrorKind::Capture,
            ),
            (
                "recording produced no frames",
                rivulet_core::TelemetryErrorKind::Capture,
            ),
            (
                "pipeline failed to start",
                rivulet_core::TelemetryErrorKind::Engine,
            ),
            (
                "encoder rejected the format",
                rivulet_core::TelemetryErrorKind::Output,
            ),
            ("disk write failed", rivulet_core::TelemetryErrorKind::Io),
            (
                "access is denied",
                rivulet_core::TelemetryErrorKind::Permission,
            ),
            ("mystery failure", rivulet_core::TelemetryErrorKind::Unknown),
        ];
        for (error, expected) in classifications {
            assert_eq!(classify_record_error(error), expected, "classify {error:?}");
        }
    }

    #[test]
    fn scene_switches_report_telemetry_once_per_landed_switch() {
        let mut app = RivuletApp {
            telemetry_enabled: true,
            ..Default::default()
        };
        app.apply_telemetry_policy();
        let captured = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let batch = captured.clone();
            app.telemetry
                .set_sink(Box::new(move |b: &rivulet_core::TelemetryBatch| {
                    batch.borrow_mut().push(b.clone())
                }));
        }
        let _ = app.scenes.add(rivulet_core::Scene::new("A".to_owned()));
        let second = app.scenes.add(rivulet_core::Scene::new("B".to_owned()));

        assert!(app.switch_active_scene(second));
        // Unknown ids are ignored by SceneManager, so no telemetry either.
        assert!(!app.switch_active_scene(uuid::Uuid::new_v4()));
        assert!(app.switch_scene_back(), "back to the first scene must land");
        assert!(!app.switch_scene_back(), "empty history must stay silent");

        app.telemetry.flush();
        let batches = captured.borrow();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].events,
            vec![
                rivulet_core::TelemetryEvent::Startup,
                rivulet_core::TelemetryEvent::SceneSwitch,
                rivulet_core::TelemetryEvent::SceneSwitch,
            ]
        );
    }

    #[test]
    fn scene_switch_without_active_scene_stays_silent() {
        // A fresh app has no scenes: switching must not record anything.
        let mut app = RivuletApp {
            telemetry_enabled: true,
            ..Default::default()
        };
        app.apply_telemetry_policy();
        assert!(!app.switch_active_scene(uuid::Uuid::new_v4()));
        assert_eq!(app.telemetry.pending_len(), 1, "only the Startup event");
    }

    #[test]
    fn chat_connections_report_telemetry_on_each_transition() {
        let mut app = RivuletApp {
            telemetry_enabled: true,
            ..Default::default()
        };
        app.apply_telemetry_policy();
        let captured = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let batch = captured.clone();
            app.telemetry
                .set_sink(Box::new(move |b: &rivulet_core::TelemetryBatch| {
                    batch.borrow_mut().push(b.clone())
                }));
        }
        app.apply_chat_state(rivulet_core::ChatConnState::Connected);
        app.apply_chat_state(rivulet_core::ChatConnState::Connected);
        assert_eq!(
            app.telemetry.pending_len(),
            2,
            "a steady connected state must not flood the batch"
        );
        app.apply_chat_state(rivulet_core::ChatConnState::Disconnected);
        app.apply_chat_state(rivulet_core::ChatConnState::Off);

        app.telemetry.flush();
        let batches = captured.borrow();
        assert_eq!(batches.len(), 1);
        assert_eq!(
            batches[0].events,
            vec![
                rivulet_core::TelemetryEvent::Startup,
                rivulet_core::TelemetryEvent::ChatConnect { ok: true },
                rivulet_core::TelemetryEvent::ChatConnect { ok: false },
            ]
        );
    }

    #[test]
    fn telemetry_session_events_are_wired_into_the_stop_paths() {
        // Text-pins the M5 telemetry wiring: every platform recording stop
        // (Windows/Linux/macOS/aux) must classify the finished session, and the
        // opt-in policy must be applied on startup and from the Settings toggle.
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs"));
        let stops = [
            "fn stop_windows_recording",
            "fn stop_linux_recording",
            "fn stop_macos_recording",
            "fn stop_aux_recording",
        ];
        for stop in stops {
            assert!(
                source.contains(&format!("{stop}(&mut self)")),
                "stop path must exist for pinning: {stop}"
            );
        }
        assert!(
            source
                .matches("self.complete_recording_session_telemetry();")
                .count()
                >= 4,
            "every platform stop must classify its recording session"
        );
        assert!(
            source.contains("app.apply_telemetry_policy();"),
            "the opt-in policy must be applied after restore in new()"
        );
        assert!(
            source.contains(".checkbox(&mut self.telemetry_enabled, telemetry_enable)"),
            "Settings must wire the opt-in toggle"
        );
    }

    // ── Native alert ingestion (M5, follows/subs/donations/raids) ──

    #[test]
    fn alert_ingest_defaults_to_enabled_and_local() {
        let app = RivuletApp::default();
        assert!(
            app.alert_ingest_enabled,
            "local alert queue is on by default"
        );
        assert!(app.alert_ingest.enabled());
        assert!(app.alert_ingest.is_empty());
    }

    #[test]
    fn apply_alerts_policy_mirrors_toggle_and_clears_on_disable() {
        let mut app = RivuletApp::default();
        app.alert_ingest
            .push(rivulet_core::AlertEvent::sample_follow());
        assert_eq!(app.alert_ingest.len(), 1);

        app.alert_ingest_enabled = false;
        app.apply_alerts_policy();
        assert!(
            !app.alert_ingest.enabled(),
            "disabling must turn the runtime queue off"
        );
        assert!(
            app.alert_ingest.is_empty(),
            "disabling must discard pending entries"
        );

        // Re-enabling accepts new events again.
        app.alert_ingest_enabled = true;
        app.apply_alerts_policy();
        assert!(app.alert_ingest.enabled());
        assert!(app
            .alert_ingest
            .push(rivulet_core::AlertEvent::sample_follow()));
    }

    #[test]
    fn alert_events_surface_into_the_chat_dock() {
        let mut app = RivuletApp::default();
        app.queue_alert_preview();
        assert_eq!(
            app.alert_ingest.len(),
            5,
            "preview must queue one sample per alert kind"
        );
        app.reconcile_chat();
        assert_eq!(
            app.chat_messages.len(),
            5,
            "drained entries must land in the chat list"
        );
        let texts: Vec<&str> = app.chat_messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "PreviewViewer followed the channel",
                "SubFan subscribed (Tier 3)",
                "Gifter gifted 5 subs",
                "Donor donated 20.00 EUR",
                "RaidLeader raided with 42 viewers",
            ]
        );
        assert!(
            app.chat_messages.iter().all(|m| m.action && m.id.is_none()),
            "alert entries render as in-dock actions without a reply target"
        );
        assert!(
            app.chat_messages
                .iter()
                .all(|m| m.color.as_deref() == Some("#e0a458")),
            "alert entries carry a distinct accent color"
        );
    }

    #[test]
    fn alert_ingestion_is_wired_into_settings_and_chat_dock() {
        // Text-pins the M5 alert wiring: a persisted Settings toggle, an
        // honest local-only ingestion path draining into the bounded chat
        // list, a preview path for testing/settus without a live stream, and
        // the startup policy mirror.
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs"));
        for required in [
            "fn queue_alert_preview",
            "fn alert_event_to_chat_message",
            "fn apply_alerts_policy",
            "fn apply_alerts_receiver",
            "alerts_receiver_enabled",
            "alerts_twitch_secret",
            ".checkbox(&mut self.alert_ingest_enabled, alert_enable)",
            ".checkbox(&mut self.alerts_receiver_enabled, receiver_enable)",
            "self.alert_ingest.drain()",
            "app.apply_alerts_policy();",
            "receiver.events().try_recv()",
        ] {
            assert!(
                source.contains(required),
                "alert wiring must be present: {required}"
            );
        }
        assert!(
            source.contains("alert_preview_button"),
            "settings/chat-dock must expose the preview action"
        );
    }

    #[test]
    fn alerts_dock_accumulates_events_from_all_sources() {
        let mut app = RivuletApp::default();
        app.queue_alert_preview();
        app.reconcile_chat();

        assert_eq!(
            app.alert_events.len(),
            5,
            "every preview kind must land in the dedicated alerts dock"
        );
        let texts: Vec<&str> = app.alert_events.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "PreviewViewer followed the channel",
                "SubFan subscribed (Tier 3)",
                "Gifter gifted 5 subs",
                "Donor donated 20.00 EUR",
                "RaidLeader raided with 42 viewers",
            ],
            "dock lines use the same localized rendering as the chat entries"
        );
        // The dock renders each line with the accent color and the platform
        // badge from the event origin (sample events are Twitch-tagged).
        assert!(app
            .alert_events
            .iter()
            .all(|m| m.color.as_deref() == Some("#e0a458")));
        assert!(app
            .alert_events
            .iter()
            .all(|m| m.platform == Some(rivulet_core::ChatPlatform::Twitch)));
        // Surfacing in the dock must not remove the chat-dock mirror.
        assert_eq!(app.chat_messages.len(), 5);
    }

    #[test]
    fn alerts_dock_clear_empties_list_and_pending_queue() {
        let mut app = RivuletApp::default();
        app.queue_alert_preview();
        app.reconcile_chat();
        assert!(!app.alert_events.is_empty());

        // A pending (not yet drained) event must also be discarded by the
        // Clear action, so it cannot resurface on the next frame.
        app.alert_ingest
            .push(rivulet_core::AlertEvent::sample_follow());
        assert_eq!(app.alert_ingest.len(), 1);

        app.clear_alert_events();
        assert!(
            app.alert_events.is_empty(),
            "the visible dock list must be emptied"
        );
        assert!(
            app.alert_ingest.is_empty(),
            "pending undrained events must not resurface after Clear"
        );
    }

    #[test]
    fn alerts_dock_is_bounded_independently_of_the_chat_list() {
        let mut app = RivuletApp::default();
        // A burst far beyond the dock bound (and beyond the ingest default
        // capacity): the dock keeps the newest MAX_ALERT_EVENTS entries.
        app.alert_ingest.set_capacity(MAX_ALERT_EVENTS + 10);
        for _ in 0..MAX_ALERT_EVENTS + 10 {
            app.alert_ingest
                .push(rivulet_core::AlertEvent::sample_follow());
        }
        app.reconcile_chat();

        assert_eq!(
            app.alert_events.len(),
            MAX_ALERT_EVENTS,
            "the dock must evict its oldest entries, not grow unbounded"
        );
        assert_eq!(
            app.alert_events[0].user, "PreviewViewer",
            "entries render user names; oldest evicted keeps order intact"
        );
    }

    #[test]
    fn alerts_dock_source_contract_is_pinned() {
        // Source contract for the dedicated dock: it lives in the Stream
        // workspace as the middle column (stacking on narrow windows), is
        // bounded by its own constant, badges platforms and exposes a Clear
        // affordance through the testable helper.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        for marker in [
            "fn draw_alerts_dock",
            "fn clear_alert_events",
            "self.clear_alert_events()",
            "alerts_dock_title",
            "alerts_dock_hint",
            "alerts_dock_clear",
            "alerts_dock_empty",
            "self.draw_alerts_dock(&mut cols[1], chat_list_height)",
            "self.draw_alerts_dock(ui, chat_list_height)",
            "const MAX_ALERT_EVENTS: usize = 200;",
            "const STREAM_WORKSPACE_ALERTS_WIDTH: f32 = 1080.0;",
            "self.alert_events.push(message)",
        ] {
            assert!(
                source.contains(marker),
                "the alerts dock contract must be present: {marker}"
            );
        }
        let i18n = std::fs::read_to_string("../rivulet-core/src/i18n.rs").expect("i18n readable");
        for key in [
            "alerts_dock_title",
            "alerts_dock_hint",
            "alerts_dock_clear",
            "alerts_dock_empty",
        ] {
            let k = format!("\"{key}\"");
            assert!(
                i18n.matches(&k).count() >= 2,
                "{key} must exist in EN and DE"
            );
        }
    }

    /// Send a raw HTTP POST to a loopback listener and return the response head.
    fn post_loopback(
        addr: std::net::SocketAddr,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> String {
        use std::io::{Read, Write};
        let mut request = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n").into_bytes();
        for (name, value) in headers {
            request.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
        request.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
        request.extend_from_slice(body.as_bytes());

        let mut stream = std::net::TcpStream::connect(addr).expect("connect to loopback");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .expect("timeout");
        stream.write_all(&request).expect("write request");
        // The listener closes right after answering; on macOS that close can
        // surface as ConnectionReset instead of a clean EOF, so collect what
        // arrived and tolerate reset/timeout instead of panicking.
        let mut response = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => response.extend_from_slice(&chunk[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    break
                }
                Err(e) => panic!("read response: {e}"),
            }
        }
        String::from_utf8_lossy(&response).into_owned()
    }

    #[test]
    fn alerts_receiver_defaults_to_off_and_loopback_only() {
        let app = RivuletApp::default();
        assert!(
            !app.alerts_receiver_enabled,
            "the webhook receiver is opt-in and off by default"
        );
        assert!(app.alerts_receiver.is_none());
        assert_eq!(
            app.alerts_receiver_port,
            rivulet_core::DEFAULT_ALERTS_RECEIVER_PORT
        );
        assert!(app.alerts_twitch_secret.is_empty());
    }

    #[test]
    fn alerts_receiver_starts_and_surfaces_streamlabs_donation_in_the_chat_dock() {
        let mut app = RivuletApp {
            alerts_receiver_enabled: true,
            alerts_receiver_port: 0, // ephemeral loopback port for tests
            ..Default::default()
        };
        app.reconcile_chat();
        let addr = app
            .alerts_receiver
            .as_ref()
            .expect("receiver started on the ephemeral port")
            .addr();
        assert!(
            addr.ip().is_loopback(),
            "receiver must bind loopback only: {addr}"
        );

        let body = r#"{"type":"donation","message":[{"name":"Donor","amount":12.34,"currency":"EUR","message":"thank you"}]}"#;
        let response = post_loopback(
            addr,
            "/webhook/streamlabs",
            &[("Content-Type", "application/json")],
            body,
        );
        assert!(response.starts_with("HTTP/1.1 200"), "response: {response}");

        // One reconcile drains socket -> queue -> chat dock.
        app.reconcile_chat();
        let texts: Vec<&str> = app.chat_messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            texts,
            vec!["Donor donated 12.34 EUR"],
            "a Streamlabs donation must surface as a localized chat entry"
        );
        assert!(app.chat_messages[0].action && app.chat_messages[0].id.is_none());
        let stats = app.alerts_receiver.as_ref().expect("receiver").stats();
        assert_eq!(stats, (1, 1, 0), "one request received and accepted");
    }

    #[test]
    fn alerts_receiver_rejects_forged_twitch_signature_with_403() {
        let mut app = RivuletApp {
            alerts_receiver_enabled: true,
            alerts_receiver_port: 0,
            alerts_twitch_secret: "secret".to_owned(),
            ..Default::default()
        };
        app.reconcile_chat();
        let addr = app
            .alerts_receiver
            .as_ref()
            .expect("receiver started")
            .addr();

        let body = r#"{"subscription":{"type":"channel.follow"},"event":{"user_name":"Ada"}}"#;
        let response = post_loopback(
            addr,
            "/eventsub/twitch",
            &[
                ("Twitch-Webhook-Message-Id", "a1b2c3d4"),
                ("Twitch-Webhook-Message-Timestamp", "2026-09-09T12:00:00Z"),
                (
                    "Twitch-Webhook-Message-Signature",
                    "sha256=0000000000000000000000000000000000000000000000000000000000000000",
                ),
            ],
            body,
        );
        assert!(response.starts_with("HTTP/1.1 403"), "response: {response}");

        app.reconcile_chat();
        assert!(
            app.chat_messages.is_empty(),
            "a forged signature must never surface anything"
        );
        assert_eq!(
            app.alerts_receiver.as_ref().expect("receiver").stats(),
            (1, 0, 1)
        );
    }

    #[test]
    fn alerts_receiver_stops_and_clears_when_disabled() {
        let mut app = RivuletApp {
            alerts_receiver_enabled: true,
            alerts_receiver_port: 0,
            ..Default::default()
        };
        app.reconcile_chat();
        assert!(app.alerts_receiver.is_some());

        app.alerts_receiver_enabled = false;
        app.reconcile_chat();
        assert!(
            app.alerts_receiver.is_none(),
            "disabling must stop the loopback listener"
        );
        assert!(app.alerts_receiver_error.is_none());
        assert_eq!(
            app.alerts_receiver_applied,
            rivulet_core::AlertsReceiverConfig::default(),
            "the applied-config marker must reset after a stop"
        );
    }

    /// Puppet EventSub WebSocket server for GUI tests: accept one client,
    /// push a welcome then a follow notification, then close cleanly after the
    /// client disconnects (mirrors the rivulet-core fixture; a clean FIN avoids
    /// `WSAECONNRESET` on Windows).
    fn spawn_eventsub_ws_puppet(
        welcome: &'static str,
        notification: &'static str,
    ) -> std::net::SocketAddr {
        let (port_tx, port_rx) = std::sync::mpsc::sync_channel::<u16>(1);
        std::thread::Builder::new()
            .name("test-eventsub-gui-puppet".into())
            .spawn(move || {
                let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind puppet");
                let port = listener.local_addr().expect("addr").port();
                let _ = port_tx.try_send(port);
                let (stream, _) = listener.accept().expect("accept");
                let mut ws = tungstenite::accept(stream).expect("ws accept");
                ws.send(tungstenite::Message::Text(welcome.into()))
                    .expect("welcome");
                ws.send(tungstenite::Message::Text(notification.into()))
                    .expect("notification");
                std::thread::sleep(std::time::Duration::from_millis(200));
                while ws.read().is_ok() {}
            })
            .expect("spawn puppet");
        std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            port_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("puppet port"),
        )
    }

    #[test]
    fn alerts_eventsub_defaults_to_off_and_empty() {
        let app = RivuletApp::default();
        assert!(
            !app.alerts_eventsub_enabled,
            "the EventSub transport is opt-in and off by default"
        );
        assert!(app.alerts_eventsub.is_none());
        assert!(app.alerts_eventsub_client_id.is_empty());
        assert!(app.alerts_eventsub_token.is_empty());
        assert!(app.alerts_eventsub_broadcaster_id.is_empty());
    }

    #[test]
    fn alerts_eventsub_missing_credentials_never_start_a_worker() {
        let mut app = RivuletApp {
            alerts_eventsub_enabled: true,
            alerts_eventsub_client_id: String::new(),
            ..Default::default()
        };
        app.reconcile_chat();
        assert!(
            app.alerts_eventsub.is_none(),
            "empty credentials must keep the transport off, no dial-out"
        );
        assert_eq!(
            app.alerts_eventsub_applied,
            rivulet_core::EventsubWsConfig::default()
        );
    }

    #[test]
    fn alerts_raid_direction_defaults_to_out_and_flows_into_the_worker_config() {
        // The direction is persisted, defaults to raid-out (historical
        // behavior), and feeds `EventsubWsConfig` on every apply — so the
        // applied-config comparison in `apply_alerts_eventsub` sees a change
        // and restarts the worker when the user flips it.
        let app = RivuletApp::default();
        assert_eq!(
            app.alerts_raid_direction,
            rivulet_core::RaidAlertDirection::Out,
            "default must stay raid-out (from_ condition)"
        );

        let app = RivuletApp {
            alerts_raid_direction: rivulet_core::RaidAlertDirection::Both,
            ..Default::default()
        };
        // Directly reuse the config-construction the apply path uses.
        let desired = rivulet_core::EventsubWsConfig {
            raid_direction: app.alerts_raid_direction,
            ..Default::default()
        };
        assert_eq!(
            desired.raid_direction,
            rivulet_core::RaidAlertDirection::Both
        );
        assert_ne!(
            desired,
            rivulet_core::EventsubWsConfig::default(),
            "a changed direction must differ from the default config so the worker restarts"
        );
    }

    #[test]
    fn alerts_raid_direction_persists_and_labels_exist_in_both_locales() {
        // Persisted across sessions (serde on the enum via out/in/both) and
        // localized EN + DE.
        let app = RivuletApp {
            alerts_raid_direction: rivulet_core::RaidAlertDirection::In,
            ..Default::default()
        };
        let json = serde_json::to_string(&app).expect("serialize");
        assert!(json.contains("\"in\""), "direction must serialize: {json}");
        let restored: RivuletApp = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            restored.alerts_raid_direction,
            rivulet_core::RaidAlertDirection::In,
            "direction must survive the round-trip"
        );
        let i18n = std::fs::read_to_string("../rivulet-core/src/i18n.rs").expect("i18n readable");
        for key in [
            "alert_eventsub_raid_direction",
            "alert_raid_direction_out",
            "alert_raid_direction_in",
            "alert_raid_direction_both",
        ] {
            let k = format!("\"{key}\"");
            assert!(
                i18n.matches(&k).count() >= 2,
                "{key} must exist in EN and DE"
            );
        }
        assert!(!restored
            .raid_direction_label(rivulet_core::RaidAlertDirection::Both)
            .is_empty());
    }

    #[test]
    fn alerts_eventsub_starts_and_surfaces_a_follow_notification_in_the_chat_dock() {
        // Static payloads bound to the local puppet (a real Helix stub is not
        // needed for the GUI test: subscription POSTs hit api_base, which
        // points at a dead port, and the worker just counts those failures).
        const WELCOME: &str = r#"{
            "metadata": {"message_type": "session_welcome"},
            "payload": {"session": {"id": "SES_gui", "keepalive_timeout_seconds": 10}}
        }"#;
        const NOTIFICATION: &str = r#"{
            "metadata": {"message_type": "notification"},
            "payload": {"subscription": {"type": "channel.follow", "version": "2"},
                        "event": {"user_id": "42", "user_login": "ada",
                                  "user_name": "Ada", "followed_at": "2026-09-10T00:00:00Z"}}
        }"#;
        let ws_addr = spawn_eventsub_ws_puppet(WELCOME, NOTIFICATION);
        let mut app = RivuletApp {
            alerts_eventsub_enabled: true,
            alerts_eventsub_client_id: "test-client".to_owned(),
            alerts_eventsub_token: "test-token".to_owned(),
            alerts_eventsub_broadcaster_id: "123".to_owned(),
            alerts_eventsub_ws_endpoint: format!("ws://{ws_addr}"),
            alerts_eventsub_api_base: "http://127.0.0.1:9".to_owned(),
            ..Default::default()
        };
        app.reconcile_chat();
        assert!(app.alerts_eventsub.is_some(), "worker started");

        // The worker pushes asynchronously through the queue; poll reconcile
        // until the follow surfaces in the chat dock (or fail after 10 s).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let surfaced = loop {
            app.reconcile_chat();
            let texts: Vec<&str> = app.chat_messages.iter().map(|m| m.text.as_str()).collect();
            if texts.contains(&"Ada followed the channel") {
                break true;
            }
            if std::time::Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        assert!(
            surfaced,
            "an EventSub follow must surface as a localized chat entry: {:?}",
            app.chat_messages
                .iter()
                .map(|m| m.text.as_str())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn alerts_eventsub_stops_and_clears_when_disabled() {
        let mut app = RivuletApp {
            alerts_eventsub_enabled: true,
            alerts_eventsub_client_id: "client".to_owned(),
            alerts_eventsub_token: "token".to_owned(),
            alerts_eventsub_broadcaster_id: "123".to_owned(),
            alerts_eventsub_ws_endpoint: "ws://127.0.0.1:9".to_owned(),
            ..Default::default()
        };
        app.reconcile_chat();
        assert!(app.alerts_eventsub.is_some());

        app.alerts_eventsub_enabled = false;
        app.reconcile_chat();
        assert!(
            app.alerts_eventsub.is_none(),
            "disabling must stop the EventSub worker"
        );
        assert!(app.alerts_eventsub_error.is_none());
        assert_eq!(
            app.alerts_eventsub_applied,
            rivulet_core::EventsubWsConfig::default(),
            "the applied-config marker must reset after a stop"
        );
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
        // validator, so both warnings appear together. The Settings view is
        // rendered inline in `ui()` (no `draw_settings` function), so the
        // invariant is checked on the Apply handler pair instead.
        let apply = source
            .split_once("fn apply_discord_client_id")
            .map(|(_, rest)| rest)
            .expect("apply_discord_client_id must exist");
        assert!(apply.contains("fn apply_discord_payload_validation"));
        assert!(source.contains("self.apply_discord_client_id();"));
        assert!(source.contains("self.apply_discord_payload_validation();"));
        let client_id_call = source
            .find("self.apply_discord_client_id();")
            .unwrap_or(usize::MAX);
        let payload_call = source
            .find("self.apply_discord_payload_validation();")
            .unwrap_or(0);
        assert!(
            client_id_call < payload_call,
            "settings Apply must run the payload validator after the client id validator"
        );
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

    // ── VOD track settings UI (issue #78) ───────────────────────────────

    #[test]
    fn stream_vod_track_derivation_matches_vodtrack_contract() {
        // The GUI helper must map the two selectors onto the core model so
        // the leakage-safety contract (active() = enabled && recorded) holds.
        let app = RivuletApp::default();
        assert!(!app.stream_vod_track().active(), "default must be inactive");
    }

    #[test]
    fn stream_start_wires_vod_track_into_engine_settings() {
        // Source contract: both stream-start paths (button and OBS command)
        // attach the VodTrack derived from the selectors, so an active VOD
        // track actually reaches the dual-output recording branch.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        // Built at runtime so this test's own source text does not count as a
        // match site (self-reference would inflate the count by one).
        let needle = format!(".{}", "with_vod_track(self.stream_vod_track())");
        assert_eq!(
            source.matches(&needle).count(),
            2,
            "both stream-start construction sites must attach the VodTrack"
        );
        assert!(source.contains("fn stream_vod_track(&self) -> VodTrack"));
        assert!(source.contains("stream_vod_section"));
        assert!(source.contains("stream_vod_enabled"));
        assert!(source.contains("stream_vod_recorded"));
        assert!(source.contains("stream_vod_hint"));
        assert!(source.contains("stream_vod_inactive"));
    }

    #[test]
    fn stream_vod_fields_are_runtime_only() {
        // Runtime-only convention: the selectors are not persisted (like the
        // other stream fields); serde skip keeps old configs loadable.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let field_pos = source
            .find("stream_vod_enabled: bool,")
            .expect("field declaration exists");
        let skip_pos = source[..field_pos]
            .rfind("#[serde(skip)]")
            .expect("field is serde-skipped");
        let between = &source[skip_pos + "#[serde(skip)]".len()..field_pos];
        assert_eq!(
            between.matches("#[serde(skip)]").count(),
            0,
            "the skip attribute directly precedes the field"
        );
        // Defaults must be off; asserted on the live app rather than by
        // scanning source text (which would see this test's own literals).
        let app = RivuletApp::default();
        assert!(!app.stream_vod_enabled);
        assert!(!app.stream_vod_recorded);
    }

    #[test]
    fn stream_vod_i18n_keys_exist_in_both_locales() {
        let de = rivulet_core::Locale::De.tr("stream_vod_section");
        let en = rivulet_core::Locale::En.tr("stream_vod_section");
        assert_ne!(de, "stream_vod_section", "DE key must be translated");
        assert_ne!(en, "stream_vod_section", "EN key must be translated");
        for key in [
            "stream_vod_enabled",
            "stream_vod_recorded",
            "stream_vod_hint",
            "stream_vod_inactive",
        ] {
            assert_ne!(
                rivulet_core::Locale::En.tr(key),
                key,
                "EN key {key} must be translated"
            );
            assert_ne!(
                rivulet_core::Locale::De.tr(key),
                key,
                "DE key {key} must be translated"
            );
        }
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
    fn gui_recording_stop_never_blocks_the_ui_thread_on_teardown() {
        // Regression: the engine's synchronous stop runs the GStreamer EOS
        // wait (up to 10 s), pipeline teardown, auto-remux, and cloud upload
        // on the calling thread. When the GUI called it from the UI thread,
        // clicking "Stop" froze the interface for the whole duration. Every
        // GUI stop handler must go through the non-blocking
        // begin_background_stop() (which arms the engine's background stop
        // plus the visible "finalizing…" status).
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(head, _)| head)
            .unwrap_or(&source);
        let handlers = production.matches("self.begin_background_stop();").count();
        assert!(
            handlers >= 3,
            "each recording stop handler must arm the background stop, found {handlers}"
        );
        assert!(
            production.contains("fn begin_background_stop"),
            "begin_background_stop helper must exist"
        );
        assert!(
            production.contains("fn poll_stop_finalization"),
            "poll_stop_finalization helper must exist"
        );
        assert!(
            production.contains("self.engine.stop_recording_background()"),
            "the helper must use the engine's background stop"
        );
        let blocking = production.matches("self.engine.stop_recording();").count();
        assert_eq!(
            blocking, 0,
            "no GUI code may call the blocking engine.stop_recording()"
        );
    }

    #[test]
    fn background_stop_shows_finalizing_status_until_saved() {
        let mut app = RivuletApp::default();

        // Nothing recording: no finalizing status and no completion handle.
        app.begin_background_stop();
        assert!(app.stop_finalizing.is_none());
        assert_ne!(
            app.record_status.as_deref(),
            Some(app.tr("recording_finalizing")),
            "idle stop must not show a finalizing status"
        );

        // Active engine session (no frames needed for the stop path): the stop
        // arms the handle and shows the translated "finalizing…" status.
        let path =
            std::env::temp_dir().join(format!("rivulet_gui_bgstop_{}.mp4", std::process::id()));
        app.engine.start_local_recording(path.clone());
        app.begin_background_stop();
        assert!(
            app.stop_finalizing.is_some(),
            "an active recording must arm the completion handle"
        );
        assert_eq!(
            app.record_status.as_deref(),
            Some(app.tr("recording_finalizing"))
        );

        // The status flips to "Recording saved." once the background
        // finalization ran (polled once per frame like the real UI).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.record_status.as_deref() != Some(app.tr("recording_saved")) {
            assert!(
                std::time::Instant::now() < deadline,
                "status must flip to saved after the background finalization"
            );
            app.poll_stop_finalization();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            app.stop_finalizing.is_none(),
            "handle is dropped after completion"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stopping_a_pure_stream_session_ends_it_in_the_background() {
        let mut app = RivuletApp::default();
        app.engine
            .set_stream_settings(Some(StreamSettings::twitch("live_123-test")));
        app.engine.start_streaming();
        assert!(app.engine.is_streaming());
        assert!(app.engine.is_recording(), "a stream-only session is active");

        app.stop_streaming_session();

        // A pure-stream session has no local output to keep: stopping the
        // stream ends the engine session instead of leaking an active session
        // that would silently block the next recording/stream start.
        assert!(!app.engine.is_streaming());
        assert!(!app.engine.is_recording(), "session must not leak");
        assert!(
            app.stop_finalizing.is_some(),
            "the background stop must be armed"
        );
        assert_eq!(
            app.record_status.as_deref(),
            Some(app.tr("recording_finalizing"))
        );
        assert_eq!(
            app.stream_status_message.as_deref(),
            Some(app.tr("stopped"))
        );

        // The status flips to "Recording saved." once the background
        // finalization ran (polled once per frame like the real UI).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.record_status.as_deref() != Some(app.tr("recording_saved")) {
            assert!(
                std::time::Instant::now() < deadline,
                "status must flip to saved after the background finalization"
            );
            app.poll_stop_finalization();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(app.stop_finalizing.is_none());
    }

    #[test]
    fn stopping_the_stream_keeps_a_dual_recording_session_alive() {
        let mut app = RivuletApp::default();
        app.engine
            .set_stream_settings(Some(StreamSettings::twitch("live_123-test")));
        let path =
            std::env::temp_dir().join(format!("rivulet_gui_dual_stop_{}.mp4", std::process::id()));
        app.engine.start_local_recording(path.clone());
        assert!(app.engine.is_dual_output(), "dual session expected");

        app.stop_streaming_session();

        // Dual output cannot be reconfigured live: only the stream config is
        // dropped, the running recording (and its own non-blocking stop path)
        // stays untouched.
        assert!(!app.engine.is_streaming());
        assert!(app.engine.is_recording(), "recording must continue");
        assert!(
            app.stop_finalizing.is_none(),
            "no background stop may be armed for a running recording"
        );

        app.engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ndi_output_settings_apply_to_the_engine_and_warn_on_empty_name() {
        let mut app = RivuletApp::default();
        // Disabled by default: no NDI output configured on the engine.
        app.apply_ndi_output();
        assert!(app.engine.ndi_output().is_none());

        // Enabled with a name + group: the engine config mirrors the fields.
        app.ndi_output_enabled = true;
        app.ndi_output_name = "Rivulet Monitor".into();
        app.ndi_output_group = "studio".into();
        app.apply_ndi_output();
        let ndi = app.engine.ndi_output().expect("NDI config must be applied");
        assert!(ndi.active);
        assert_eq!(ndi.name, "Rivulet Monitor");
        assert_eq!(ndi.group.as_deref(), Some("studio"));
        assert!(app.ndi_warning.is_none());

        // An empty name cannot be enabled: the engine keeps no feed and the
        // Settings row shows the localized warning.
        app.ndi_output_name.clear();
        app.apply_ndi_output();
        assert!(
            app.engine.ndi_output().is_none(),
            "an empty name must not publish a feed"
        );
        assert_eq!(
            app.ndi_warning.as_deref(),
            Some(app.tr("ndi_name_required"))
        );

        // Disabling clears the engine feed again.
        app.ndi_output_name = "Rivulet Monitor".into();
        app.ndi_output_enabled = false;
        app.apply_ndi_output();
        assert!(app.engine.ndi_output().is_none());
        assert!(app.ndi_warning.is_none());
    }

    #[test]
    fn ndi_output_is_applied_before_every_session_start() {
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(head, _)| head)
            .unwrap_or(&source);
        assert!(
            production.contains("fn apply_ndi_output"),
            "apply_ndi_output helper must exist"
        );
        // Every recording start path and both streaming starts push the NDI
        // config right before the engine session begins.
        let applies = production.matches("self.apply_ndi_output();").count();
        assert!(
            applies >= 8,
            "expected the helper at every session start, found {applies}"
        );
        // Only the helper may touch the engine setter (2 calls: disable +
        // enable); call sites go through apply_ndi_output.
        let direct = production.matches("self.engine.set_ndi_output(").count();
        assert_eq!(
            direct, 2,
            "engine.set_ndi_output must live inside apply_ndi_output only"
        );
        // The Settings UI exposes the section with its localized labels.
        for needle in [
            "self.tr(\"ndi_section\")",
            "self.ndi_output_enabled",
            "ndi_plugin_missing",
        ] {
            assert!(production.contains(needle), "missing {needle} in Settings");
        }
    }

    // ── macOS recording (M5 Windows/macOS feature parity) ──────────

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_stop_without_recording_is_a_noop() {
        let mut app = RivuletApp::default();
        assert!(!app.is_recording);
        // An idle stop must not arm the finalizing status or touch the capture
        // handle (stop_recording_background returns None for no active session).
        app.stop_macos_recording();
        assert!(!app.is_recording);
        assert!(app.stop_finalizing.is_none());
        assert!(app.stop_signal.is_none());
        assert_eq!(
            app.record_status.as_deref(),
            None,
            "an idle stop must not show a status"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_stop_arms_the_background_finalization() {
        let mut app = RivuletApp::default();
        // Simulate an active session the way start_macos_recording leaves it:
        // the engine session is started (state-only, no GStreamer pipeline is
        // built until the first frame) and the app arms the capture channel.
        let path =
            std::env::temp_dir().join(format!("rivulet_gui_macos_stop_{}.mp4", std::process::id()));
        app.engine.start_local_recording(path);
        app.is_recording = true;
        app.raw_rx = Some(std_mpsc::channel::<RawFrame>().1);
        app.stop_signal = Some(Arc::new(AtomicBool::new(false)));

        app.stop_macos_recording();

        // The capture thread is signalled, the receiver is dropped, and the
        // UI shows the finalizing status until the background teardown ends.
        assert!(!app.is_recording);
        assert!(app.raw_rx.is_none());
        assert!(app.stop_signal.is_none());
        assert_eq!(
            app.record_status.as_deref(),
            Some(app.tr("recording_finalizing")),
            "an active stop must show the finalizing status"
        );

        // The completion handle fires once the (no-op) background finalization
        // ran, flipping the status to "Recording saved.".
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while app.record_status.as_deref() != Some(app.tr("recording_saved")) {
            assert!(
                std::time::Instant::now() < deadline,
                "status must flip to saved after the background finalization"
            );
            app.poll_stop_finalization();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(app.stop_finalizing.is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_source_refresh_resets_stale_selections() {
        let mut app = RivuletApp::default();
        // Selections pointing beyond the current lists must be reset by the
        // refresh (sources come and go, e.g. a window closes or a monitor is
        // unplugged).
        app.selected_monitor_idx = Some(7);
        app.selected_window_idx = Some(9);
        app.refresh_macos_sources();
        assert!(
            app.selected_monitor_idx
                .map_or(true, |idx| idx < app.monitors.len()),
            "stale monitor selection must be reset"
        );
        assert!(
            app.selected_window_idx
                .map_or(true, |idx| idx < app.windows.len()),
            "stale window selection must be reset"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_start_without_source_reports_it_without_a_dialog() {
        let mut app = RivuletApp::default();
        // No monitor/window selected: start must return before any file dialog
        // and surface the localized hint instead.
        app.start_macos_recording();
        assert!(!app.is_recording);
        assert_eq!(
            app.record_status.as_deref(),
            Some(app.tr("no_source_selected"))
        );
    }

    #[test]
    fn macos_record_view_is_wired_into_the_app() {
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(head, _)| head)
            .unwrap_or(&source);
        // The macOS impl block with the capture/stop/drain/view helpers exists.
        for needle in [
            "fn start_macos_recording",
            "fn stop_macos_recording",
            "fn drain_macos_frames",
            "fn draw_macos_record_view",
            "fn refresh_macos_sources",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
        // macOS audio capture (cpal): started with the recording, drained into
        // the engine's separate system/mic tracks, stopped on stop, and the
        // resolved loopback note surfaced in the Record view.
        for needle in [
            "fn start_macos_audio",
            "fn stop_macos_audio",
            "fn drain_macos_audio",
            "self.engine.set_audio_enabled(true)",
            "self.engine.set_separate_audio_tracks(true)",
            "self.engine.push_audio_track(&frame, AudioTrack::System)",
            "self.engine.push_audio_track(&frame, AudioTrack::Microphone)",
            "audio.system_source_note()",
        ] {
            assert!(production.contains(needle), "missing {needle}");
        }
        // The Record view dispatches to the macOS drawer (replacing the old
        // "screen recording unavailable" placeholder).
        assert!(
            production.contains("self.draw_macos_record_view(ui, &colors)"),
            "Record view must render the macOS drawer"
        );
        assert!(
            production.contains("self.refresh_macos_sources()"),
            "Record view must refresh the source lists"
        );
        // The record hotkey toggles the macOS path.
        assert!(
            production.contains("self.start_macos_recording()"),
            "hotkey must start the macOS recording"
        );
        // The view stays responsive: capture frames are drained per frame in
        // the ui() tick, not inside a blocking loop.
        assert!(
            production.contains("self.drain_macos_frames()"),
            "macOS frames must be drained from the per-frame update"
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

    // ── source delete hotkey (OBS 32.2 parity) ───────────────────

    #[test]
    fn delete_source_hotkey_defaults_to_delete_and_is_rebindable() {
        let mut hotkeys = HotkeyConfig::default();
        assert_eq!(
            hotkeys.delete_source,
            HotkeyBinding::plain(egui::Key::Delete)
        );
        assert_eq!(
            hotkeys.binding_for_action("delete_source"),
            Some(HotkeyBinding::plain(egui::Key::Delete))
        );
        assert_eq!(hotkeys.label_for("delete_source"), "Delete");
        let rebound = HotkeyBinding::plain(egui::Key::F8);
        assert!(hotkeys.set_binding_for_action("delete_source", rebound));
        assert_eq!(hotkeys.binding_for_action("delete_source"), Some(rebound));
        assert_eq!(hotkeys.label_for("delete_source"), "F8");
        assert_eq!(vk_code(egui::Key::Delete), Some(0x2E));
    }

    #[test]
    fn delete_source_hotkey_roundtrip_and_migrates_legacy_configs() {
        let hotkeys = HotkeyConfig::default();
        let json = serde_json::to_string(&hotkeys).expect("serialize hotkeys");
        let restored: HotkeyConfig = serde_json::from_str(&json).expect("deserialize hotkeys");
        assert_eq!(restored.delete_source, hotkeys.delete_source);
        assert_eq!(
            restored.delete_source,
            HotkeyBinding::plain(egui::Key::Delete)
        );
        // Configs saved before the field existed must migrate to Delete, not
        // to the generic placeholder binding.
        let mut legacy = serde_json::to_value(hotkeys).expect("serialize hotkeys");
        legacy
            .as_object_mut()
            .expect("hotkey config object")
            .remove("delete_source");
        let migrated: HotkeyConfig =
            serde_json::from_value(legacy).expect("deserialize legacy hotkeys");
        assert_eq!(
            migrated.delete_source,
            HotkeyBinding::plain(egui::Key::Delete)
        );
    }

    #[test]
    fn delete_source_is_deliberately_not_a_global_binding() {
        // Destructive and repeat-sensitive: never registered at the OS level,
        // so an unfocused Rivulet cannot delete sources while the user types
        // Delete in some other application.
        let app = RivuletApp::default();
        let bindings = app.current_global_bindings();
        assert!(
            !bindings.iter().any(|b| b.action == "delete_source"),
            "delete_source must not be OS-global"
        );
        assert_eq!(
            bindings.len(),
            4,
            "destructive actions are app-local; only record/pause/mute/save_replay are global"
        );
    }

    #[test]
    fn scene_device_picker_lists_webcam_game_and_monitor_devices() {
        let mut app = RivuletApp::default();
        app.camera_devices.push(rivulet_core::CameraDevice {
            name: "Facecam".to_owned(),
            element_factory: "vfsrc".to_owned(),
            device_path: "/dev/video0".to_owned(),
        });
        app.game_windows.push(rivulet_core::GameWindow {
            id: 42,
            title: "Elden Ring".to_owned(),
            width: 1920,
            height: 1080,
        });

        let webcam_entries = app.scene_device_entries(&rivulet_core::SourceKind::Webcam);
        assert_eq!(webcam_entries.len(), 1);
        assert_eq!(webcam_entries[0].0, "Facecam");
        assert_eq!(webcam_entries[0].1, "camera:/dev/video0");

        let game_entries = app.scene_device_entries(&rivulet_core::SourceKind::GameCapture);
        assert_eq!(game_entries.len(), 1);
        assert_eq!(game_entries[0].1, "game:42");

        // Non-capture kinds never offer a device list.
        assert!(app
            .scene_device_entries(&rivulet_core::SourceKind::Color)
            .is_empty());
        assert!(app
            .scene_device_entries(&rivulet_core::SourceKind::Browser)
            .is_empty());
    }

    #[test]
    fn add_scene_source_applies_selected_device_id() {
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        app.camera_devices.push(rivulet_core::CameraDevice {
            name: "Facecam".to_owned(),
            element_factory: "vfsrc".to_owned(),
            device_path: "/dev/video0".to_owned(),
        });
        app.selected_scene_device_idx = Some(0);

        let id = app.add_scene_source_from_dialog(scene_id, &rivulet_core::SourceKind::Webcam);

        let source = app.source_manager.get_source(id).expect("source added");
        assert_eq!(source.device_id, "camera:/dev/video0");
        assert_eq!(
            app.scene_status.as_deref(),
            Some("Source added with device \"camera:/dev/video0\".")
        );
        assert_eq!(app.selected_composition_source, Some(id));
    }

    #[test]
    fn add_scene_source_without_device_selection_keeps_device_empty() {
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);

        let id = app.add_scene_source_from_dialog(scene_id, &rivulet_core::SourceKind::Color);

        let source = app.source_manager.get_source(id).expect("source added");
        assert_eq!(source.device_id, "");
        assert_eq!(app.scene_status, None);
    }

    #[test]
    fn delete_source_dispatch_stages_confirmation_before_deletion() {
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Cam".to_owned(),
            rivulet_core::SourceKind::Webcam,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        app.selected_composition_source = Some(source_id);

        app.dispatch_hotkey_action("delete_source");

        // The delete hotkey must only stage the action behind the confirmation
        // dialog; the source survives until the user confirms.
        assert!(
            app.source_manager.get_source(source_id).is_some(),
            "source must survive the hotkey until confirmed"
        );
        assert!(matches!(
            app.pending_confirmation,
            Some(PendingConfirmation::DeleteCompositionSource { .. })
        ));
        assert_eq!(app.selected_composition_source, Some(source_id));

        app.confirm_pending_confirmation();

        assert!(app.source_manager.get_source(source_id).is_none());
        assert!(app.source_manager.sources().is_empty());
        assert_eq!(app.selected_composition_source, None);
        assert_eq!(app.scene_status.as_deref(), Some("Source \"Cam\" deleted."));
        assert_eq!(app.pending_confirmation, None);
    }

    #[test]
    fn delete_source_confirmation_cancel_discards_the_staged_action() {
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Cam".to_owned(),
            rivulet_core::SourceKind::Webcam,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        app.selected_composition_source = Some(source_id);

        app.dispatch_hotkey_action("delete_source");
        assert!(app.pending_confirmation.is_some());

        app.cancel_pending_confirmation();

        assert!(app.source_manager.get_source(source_id).is_some());
        assert_eq!(app.pending_confirmation, None);
        assert_eq!(app.selected_composition_source, Some(source_id));
        assert_eq!(app.scene_status, None);
    }

    #[test]
    fn delete_source_confirmation_guard_reruns_on_confirm() {
        // If the source becomes locked (or gone) between staging and confirm,
        // the confirmation path must re-run the guards instead of deleting.
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Banner".to_owned(),
            rivulet_core::SourceKind::Image,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        app.selected_composition_source = Some(source_id);

        app.dispatch_hotkey_action("delete_source");
        // Lock moves in while the dialog is open — the confirm path repects it.
        app.source_manager.set_locked(source_id, scene_id, true);
        app.confirm_pending_confirmation();

        assert!(
            app.source_manager.get_source(source_id).is_some(),
            "the confirm path must re-check the scene lock"
        );
        assert_eq!(app.scene_status.as_deref(), Some("Source is locked."));
    }

    #[test]
    fn composition_copy_carries_every_scene_item_property_to_the_clipboard() {
        // Issue #192: copy must capture the source AND its binding verbatim,
        // so paste reproduces transforms, crops, lock and visibility.
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Banner".to_owned(),
            rivulet_core::SourceKind::Image,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        let transform = rivulet_core::Transform {
            width: 320.0,
            ..rivulet_core::Transform::default()
        };
        app.source_manager
            .set_transform(source_id, scene_id, transform);
        app.source_manager.set_locked(source_id, scene_id, true);
        app.selected_composition_source = Some(source_id);

        app.copy_selected_composition_source();

        let clipboard = app
            .scene_item_clipboard
            .expect("copy populated the clipboard");
        assert_eq!(clipboard.source.id, source_id);
        assert_eq!(clipboard.source.name, "Banner");
        let binding = &clipboard.binding;
        assert_eq!(binding.source_id, source_id);
        assert_eq!(binding.scene_id, scene_id);
        assert_eq!(
            binding
                .transform_override
                .as_ref()
                .map(|t| t.width)
                .unwrap_or_default(),
            320.0
        );
        assert!(binding.locked, "lock state travels with the item");
        assert_eq!(app.scene_status.as_deref(), Some("Scene item copied"));
    }

    #[test]
    fn composition_paste_duplicates_across_scenes_without_touching_the_origin() {
        // Paste into another scene must create a NEW source id (no shared
        // identity), keep the original untouched, and select the duplicate.
        let mut app = RivuletApp::default();
        let origin = app
            .scenes
            .add(rivulet_core::Scene::new("Origin".to_owned()));
        app.scenes.switch_to(origin);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Banner".to_owned(),
            rivulet_core::SourceKind::Image,
        ));
        app.source_manager.bind_source(source_id, origin, None);
        app.selected_composition_source = Some(source_id);
        app.copy_selected_composition_source();

        let target = app
            .scenes
            .add(rivulet_core::Scene::new("Target".to_owned()));
        app.scenes.switch_to(target);
        app.paste_scene_item_clipboard();

        let new_id = app
            .selected_composition_source
            .expect("paste selected the new item");
        assert_ne!(new_id, source_id, "paste must duplicate, not rebind");
        assert!(app.source_manager.get_source(new_id).is_some());
        assert!(
            app.source_manager
                .scene_sources(target)
                .iter()
                .any(|b| b.source_id == new_id),
            "the duplicate is bound to the target scene"
        );
        assert!(app.source_manager.get_source(source_id).is_some());
        assert!(
            app.source_manager
                .scene_sources(origin)
                .iter()
                .any(|b| b.source_id == source_id),
            "the origin scene keeps its item"
        );
        assert_eq!(
            app.source_manager
                .get_source(new_id)
                .map(|s| s.name.as_str()),
            Some("Banner copy"),
            "the duplicate gets the OBS-style copy suffix"
        );
    }

    #[test]
    fn composition_paste_undo_removes_the_duplicate() {
        // Each paste lands on the SourceManager paste-undo stack; Ctrl+Z must
        // remove the pasted item again (issue #192 undo integration).
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Banner".to_owned(),
            rivulet_core::SourceKind::Image,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        app.selected_composition_source = Some(source_id);
        app.copy_selected_composition_source();
        app.paste_scene_item_clipboard();
        let pasted_id = app.selected_composition_source.expect("paste selected");
        assert!(app.source_manager.get_source(pasted_id).is_some());

        assert!(app.source_manager.can_undo_paste(), "a paste is undoable");
        assert!(app.source_manager.undo_paste());

        assert!(
            app.source_manager.get_source(pasted_id).is_none(),
            "undo removed the pasted duplicate"
        );
        assert!(app.source_manager.get_source(source_id).is_some());
        assert!(!app.source_manager.can_undo_paste());
    }

    #[test]
    fn composition_paste_with_empty_clipboard_reports_status_and_changes_nothing() {
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let before = app.source_manager.scene_sources(scene_id).len();

        app.paste_scene_item_clipboard();

        assert_eq!(
            app.scene_status.as_deref(),
            Some("Clipboard is empty — copy a scene item first")
        );
        assert_eq!(app.source_manager.scene_sources(scene_id).len(), before);
        assert_eq!(app.selected_composition_source, None);
    }

    #[test]
    fn delete_source_dispatch_respects_the_scene_lock() {
        let mut app = RivuletApp::default();
        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Banner".to_owned(),
            rivulet_core::SourceKind::Image,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        app.source_manager.set_locked(source_id, scene_id, true);
        app.selected_composition_source = Some(source_id);

        app.dispatch_hotkey_action("delete_source");

        assert!(
            app.source_manager.get_source(source_id).is_some(),
            "locked sources must not even be staged, let alone deleted"
        );
        assert_eq!(app.selected_composition_source, Some(source_id));
        assert_eq!(app.scene_status.as_deref(), Some("Source is locked."));
        assert_eq!(app.pending_confirmation, None);
    }

    #[test]
    fn delete_source_dispatch_ignored_without_a_scene_or_selection() {
        let mut app = RivuletApp::default();
        app.dispatch_hotkey_action("delete_source");
        assert!(app.source_manager.sources().is_empty());
        assert_eq!(app.scene_status, None);

        let scene_id = app.scenes.add(rivulet_core::Scene::new("Main".to_owned()));
        app.scenes.switch_to(scene_id);
        let source_id = app.source_manager.add_source(rivulet_core::Source::new(
            "Cam".to_owned(),
            rivulet_core::SourceKind::Webcam,
        ));
        app.source_manager.bind_source(source_id, scene_id, None);
        app.selected_composition_source = None;
        app.dispatch_hotkey_action("delete_source");
        assert!(
            app.source_manager.get_source(source_id).is_some(),
            "no selection means nothing to delete"
        );
    }

    #[test]
    fn delete_source_hotkey_is_wired_and_guarded_against_text_input() {
        // The action must be rebindable through the Settings hotkey list, live
        // in the shared dispatch, and fire only outside the text-input guard so
        // Delete keeps working for ordinary text editing.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        assert!(
            source.contains("\"record\", \"pause\", \"mute\", \"save_replay\", \"delete_source\"]")
        );
        let in_app_delete = source
            .find("self.hotkeys.delete_source.pressed_in(i)")
            .unwrap_or(0);
        let text_guard = source
            .find("if !wants_keyboard_input {")
            .unwrap_or(usize::MAX);
        assert!(
            text_guard < in_app_delete,
            "in-app delete dispatch must live inside the text-input guard"
        );
        assert!(source.contains("self.delete_selected_composition_source()"));
    }

    #[test]
    fn chat_account_removal_is_staged_behind_confirmation() {
        let mut app = RivuletApp::default();
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Twitch,
            "rivulet_test".to_owned(),
        ));
        assert_eq!(app.chat_accounts.len(), 1);

        app.remove_chat_account(0);

        assert_eq!(
            app.chat_accounts.len(),
            1,
            "staging must not remove the account yet"
        );
        assert!(matches!(
            app.pending_confirmation,
            Some(PendingConfirmation::RemoveChatAccount { .. })
        ));
        assert_eq!(app.chat_action_pending, None);

        app.confirm_pending_confirmation();

        assert!(app.chat_accounts.is_empty());
        assert_eq!(app.pending_confirmation, None);
    }

    #[test]
    fn chat_account_removal_cancel_keeps_the_account() {
        let mut app = RivuletApp::default();
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Kick,
            "anchovies_channel".to_owned(),
        ));
        app.remove_chat_account(0);
        assert!(app.pending_confirmation.is_some());

        app.cancel_pending_confirmation();

        assert_eq!(app.chat_accounts.len(), 1);
        assert_eq!(app.chat_accounts[0].channel, "anchovies_channel");
        assert_eq!(app.pending_confirmation, None);
        assert_eq!(app.chat_action_pending, None);
    }

    #[test]
    fn chat_account_removal_out_of_bounds_is_a_no_op() {
        let mut app = RivuletApp::default();
        app.remove_chat_account(0);
        assert_eq!(app.pending_confirmation, None);
        assert!(app.chat_accounts.is_empty());
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
        // The Window dropdown owns the window title; the Source dropdown shows
        // its "Select Monitor" fallback when no monitor is selected.
        assert_eq!(resolve_monitor_label(None, None), None);
        assert_eq!(resolve_monitor_label(None, Some("ghost")), None);
    }

    #[test]
    fn monitor_label_keeps_monitor_name_when_window_also_selected() {
        // Regression: selecting a window keeps the monitor selected, so the
        // Source dropdown must still show the monitor name.
        assert_eq!(
            resolve_monitor_label(Some(1), Some("Display 2")),
            Some("Display 2".to_string())
        );
    }

    #[test]
    fn monitor_label_is_none_when_empty_name() {
        assert_eq!(
            resolve_monitor_label(Some(0), Some("")),
            Some(String::new())
        );
    }

    // ── Source window list filtering ───────────────────────────────

    #[test]
    fn keep_on_selected_monitor_keeps_all_when_no_monitor_selected() {
        let kept = keep_on_selected_monitor(vec!["a", "b", "c"], false, |_| true);
        assert_eq!(kept, vec![(0, "a"), (1, "b"), (2, "c")]);
    }

    #[test]
    fn keep_on_selected_monitor_keeps_only_matching_windows_preserving_indices() {
        let kept =
            keep_on_selected_monitor(vec!["a", "b", "c", "d"], true, |w| *w == "b" || *w == "d");
        assert_eq!(kept, vec![(1, "b"), (3, "d")]);
    }

    #[test]
    fn keep_on_selected_monitor_returns_empty_when_nothing_matches() {
        let kept = keep_on_selected_monitor(vec!["a", "b"], true, |_| false);
        assert!(kept.is_empty());
    }

    #[test]
    fn keep_on_selected_monitor_handles_empty_list() {
        let kept = keep_on_selected_monitor::<&str>(Vec::new(), true, |_| true);
        assert!(kept.is_empty());
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

    // ── Motion preference (ui-005, WCAG 2.3.3) ─────────────────

    #[test]
    fn motion_preference_survives_serde_round_trip() {
        // The motion preference must persist across restarts like the theme
        // preference does.
        let app = RivuletApp {
            motion_preference: theme::MotionPreference::Reduced,
            ..Default::default()
        };
        let json = serde_json::to_value(&app).expect("serialize app");
        assert_eq!(json["motion_preference"], "Reduced");
        let restored: RivuletApp = serde_json::from_value(json).expect("deserialize app");
        assert_eq!(restored.motion_preference, theme::MotionPreference::Reduced);
    }

    #[test]
    fn motion_preference_defaults_to_system() {
        // Fresh installs follow the OS reduce-motion setting.
        let app = RivuletApp::default();
        assert_eq!(
            app.motion_preference,
            theme::MotionPreference::System,
            "default must be System (follow the OS)"
        );
        // Runtime-only fields stay out of the persisted JSON.
        let json = serde_json::to_value(RivuletApp::default()).expect("serialize");
        assert!(json.get("motion_applied").is_none());
        assert!(json.get("os_reduced_motion").is_none());
        assert!(json.get("os_reduced_motion_probed_at").is_none());
    }

    #[test]
    fn scene_transition_collapses_fade_to_cut_when_reduced() {
        // Under applied reduced motion, a configured Fade must degrade to a
        // Cut (progress_at() == 1.0 immediately) so scene switches are
        // instant (WCAG 2.3.3).
        let mut app = RivuletApp {
            transition_kind: rivulet_core::TransitionKind::Fade,
            transition_duration_ms: 1000,
            ..Default::default()
        };
        let from = uuid::Uuid::new_v4();
        let to = uuid::Uuid::new_v4();

        app.motion_applied = Some(false);
        let animated = app.scene_transition_for_switch(Some(from), Some(to), Instant::now());
        assert_eq!(animated.kind, rivulet_core::TransitionKind::Fade);
        assert!(animated.progress_at(Instant::now()) < 1.0);

        app.motion_applied = Some(true);
        let reduced = app.scene_transition_for_switch(Some(from), Some(to), Instant::now());
        assert_eq!(reduced.kind, rivulet_core::TransitionKind::Cut);
        assert!(reduced.progress_at(Instant::now()) >= 1.0);
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

    // ── Mobile & HTTP remote companion ─────────────────────────────

    #[test]
    fn remote_companion_defaults_are_opt_in_loopback_and_secure() {
        let app = RivuletApp::default();
        assert!(
            !app.remote_companion_enabled,
            "companion page must be opt-in"
        );
        assert!(!app.remote_companion_bind_lan, "LAN bind must be opt-in");
        assert!(
            !app.remote_allow_stream_control,
            "remote stream control must be explicit permission"
        );
        assert_eq!(
            app.remote_companion_port,
            rivulet_obs_websocket::COMPANION_DEFAULT_PORT
        );
        assert!(app.remote_companion_server.is_none());
        assert!(!app.companion_reaches_lan());
    }

    #[test]
    fn companion_reaches_lan_reflects_enable_and_bind_toggles() {
        let app = RivuletApp::default();
        assert!(!app.companion_reaches_lan());
        let app = RivuletApp {
            remote_companion_enabled: true,
            ..Default::default()
        };
        assert!(
            !app.companion_reaches_lan(),
            "enabled alone is still loopback"
        );
        let app = RivuletApp {
            remote_companion_enabled: true,
            remote_companion_bind_lan: true,
            ..Default::default()
        };
        assert!(
            app.companion_reaches_lan(),
            "enabled + LAN bind reaches the LAN"
        );
    }

    #[test]
    fn remote_companion_settings_persist_across_restarts() {
        // The RivuletApp serde round-trip must carry the companion settings
        // (toggle + port + bind + permission) across an app restart, while the
        // running server handle and status lines stay runtime-only.
        let app = RivuletApp {
            remote_companion_enabled: true,
            remote_companion_port: 4466,
            remote_companion_bind_lan: true,
            remote_allow_stream_control: true,
            remote_companion_server: Some(rivulet_obs_websocket::CompanionServerHandle::unused()),
            remote_companion_status: Some("running".to_owned()),
            remote_companion_url: Some("http://127.0.0.1:4466".to_owned()),
            ..Default::default()
        };
        let json = serde_json::to_value(&app).expect("serialize app");
        assert_eq!(json["remote_companion_enabled"], true);
        assert_eq!(json["remote_companion_port"], 4466);
        assert_eq!(json["remote_companion_bind_lan"], true);
        assert_eq!(json["remote_allow_stream_control"], true);
        // Runtime-only state must not be persisted.
        assert!(json.get("remote_companion_server").is_none());
        assert!(json.get("remote_companion_status").is_none());
        assert!(json.get("remote_companion_url").is_none());

        let restored: RivuletApp = serde_json::from_value(json).expect("deserialize app");
        assert!(restored.remote_companion_enabled);
        assert_eq!(restored.remote_companion_port, 4466);
        assert!(restored.remote_companion_bind_lan);
        assert!(restored.remote_allow_stream_control);
        assert!(restored.remote_companion_server.is_none());
        assert!(restored.remote_companion_status.is_none());
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
            chat_worker_multi: Some(rivulet_core::MultiChat::from_configs(&[
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::Twitch,
                    twitch_endpoint: "127.0.0.1:1".to_owned(),
                    channel: "rivulet".to_owned(),
                    token: "oauth:abc123".to_owned(),
                    ..Default::default()
                },
            ])),
            chat_state: rivulet_core::ChatConnState::Connected,
            ..Default::default()
        };
        assert!(app.send_chat_message(" hello there ".to_owned()));
        // The broadcast outcome is reported per platform so partial failures
        // (read-only YouTube, rate-limited accounts) stay visible.
        assert_eq!(
            std::mem::take(&mut app.chat_last_send_outcomes),
            vec![(rivulet_core::ChatPlatform::Twitch, true)]
        );
    }

    #[test]
    fn kick_engagement_events_drain_into_both_docks_with_badge() {
        // The GUI drain path (alert_ingest -> both docks, kick badge on the
        // rendered line) fed through the real queue with a kick-tagged sub
        // event, exactly as the chat-worker drain produces it.
        let mut app = RivuletApp::default();
        app.alert_ingest
            .push(rivulet_core::AlertEvent {
                kind: rivulet_core::AlertKind::Subscribe,
                user: "KickFan".to_owned(),
                recipient: None,
                count: 0,
                tier: Some("Tier 2".to_owned()),
                amount: None,
                currency: None,
                message: None,
                timestamp: 1_700_000_000,
                platform: Some(rivulet_core::ChatPlatform::Kick),
            })
            .then_some(())
            .expect("queue accepts while enabled");

        app.reconcile_chat();

        let chat_texts: Vec<&str> = app.chat_messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            chat_texts,
            vec!["KickFan subscribed (Tier 2)"],
            "the kick subscription must surface as a localized chat entry"
        );
        assert_eq!(
            app.chat_messages[0].platform,
            Some(rivulet_core::ChatPlatform::Kick),
            "the line must carry the kick badge"
        );
        let dock_texts: Vec<&str> = app.alert_events.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            dock_texts,
            vec!["KickFan subscribed (Tier 2)"],
            "the dedicated alerts dock accumulates the same event"
        );
    }

    #[test]
    fn youtube_super_chat_renders_as_a_donation_line_with_badge() {
        // A YouTube Super Chat event (Donation kind, YouTube-tagged, exactly
        // as parse_youtube_alert_events produces it) must surface in both
        // docks as the localized donation line carrying the [YouTube] badge.
        let mut app = RivuletApp::default();
        app.alert_ingest
            .push(rivulet_core::AlertEvent {
                kind: rivulet_core::AlertKind::Donation,
                user: "YTDonor".to_owned(),
                recipient: None,
                count: 0,
                tier: None,
                amount: Some(19.99),
                currency: Some("USD".to_owned()),
                message: Some("love the stream".to_owned()),
                timestamp: 1_700_000_000,
                platform: Some(rivulet_core::ChatPlatform::YouTube),
            })
            .then_some(())
            .expect("queue accepts while enabled");

        app.reconcile_chat();

        assert_eq!(
            app.chat_messages[0].text, "YTDonor donated 19.99 USD",
            "the super chat must render as a localized donation line"
        );
        assert_eq!(
            app.chat_messages[0].platform,
            Some(rivulet_core::ChatPlatform::YouTube),
            "the line must carry the youtube badge"
        );
        assert_eq!(
            app.alert_events[0].text, "YTDonor donated 19.99 USD",
            "the dedicated alerts dock accumulates the same event"
        );
    }

    #[test]
    fn chat_dock_drain_covers_chat_worker_alert_receivers() {
        // Source-pin the platform drain so the combined dock keeps consuming
        // the chat workers' engagement receivers alongside EventSub/Streamlabs,
        // attributed per platform for the per-source rate limiter.
        let source = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/app.rs"));
        for required in [
            "multi.alert_receivers_by_platform()",
            "while let Ok(event) = rx.try_recv()",
            "self.alert_ingest.push_from(source, event);",
        ] {
            assert!(
                source.contains(required),
                "chat-worker alert drain must stay wired: {required}"
            );
        }
    }

    #[test]
    fn spam_burst_surfaces_one_localized_suppression_line() {
        // A source exceeding its per-window budget must have its excess
        // dropped and surface exactly one localized suppression line in both
        // docks — throttling is visible, not silent.
        let mut app = RivuletApp::default();
        for i in 0..rivulet_core::ALERT_SOURCE_WINDOW_CAPACITY + 5 {
            let event = rivulet_core::AlertEvent {
                user: format!("Spammer{i}"),
                ..rivulet_core::AlertEvent::sample_follow()
            };
            app.alert_ingest
                .push_from(rivulet_core::AlertSource::TwitchEventSub, event);
        }

        app.reconcile_chat();

        let dock_texts: Vec<&str> = app.alert_events.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(
            dock_texts.len(),
            rivulet_core::ALERT_SOURCE_WINDOW_CAPACITY + 1,
            "accepted events plus exactly one suppression notice"
        );
        assert_eq!(
            dock_texts.last().copied(),
            Some("Twitch EventSub: 5 further alerts suppressed (rate limit)"),
            "the suppression line names the source and the dropped count"
        );
        assert_eq!(
            app.alert_events.last().map(|m| m.platform),
            Some(None),
            "the notice carries no platform badge"
        );
    }

    #[test]
    fn chat_send_reply_requires_token_parent_id_and_worker() {
        // Threaded replies need the same authenticated connection as plain
        // sends plus a parent Twitch message id.
        let mut app = RivuletApp::default();
        let twitch = rivulet_core::ChatPlatform::Twitch;
        let youtube = rivulet_core::ChatPlatform::YouTube;
        assert!(
            !app.send_chat_reply("hello".to_owned(), "parent-1".to_owned(), twitch),
            "no worker must block replies"
        );
        assert!(
            !app.send_chat_reply("   ".to_owned(), "parent-1".to_owned(), twitch),
            "whitespace text must be rejected"
        );
        assert!(
            !app.send_chat_reply("hello".to_owned(), "   ".to_owned(), twitch),
            "missing parent id must be rejected"
        );
        assert!(
            !app.send_chat_reply("hello".to_owned(), "parent-1".to_owned(), youtube),
            "the read-only platform must reject replies"
        );
    }

    #[test]
    fn chat_send_reply_enqueues_with_a_running_worker() {
        // Mirror of the plain-send enqueue test: with a token, a worker and a
        // parent id the reply is handed to the worker non-blocking.
        let mut app = RivuletApp {
            chat_worker_multi: Some(rivulet_core::MultiChat::from_configs(&[
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::Twitch,
                    twitch_endpoint: "127.0.0.1:1".to_owned(),
                    channel: "rivulet".to_owned(),
                    token: "oauth:abc123".to_owned(),
                    ..Default::default()
                },
            ])),
            chat_state: rivulet_core::ChatConnState::Connected,
            ..Default::default()
        };
        assert!(
            app.send_chat_reply(
                " thanks! ".to_owned(),
                "parent-1".to_owned(),
                rivulet_core::ChatPlatform::Twitch
            ),
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

    // ── Global status surface (sidebar footer, ui-007) ──────────────

    #[test]
    fn global_issue_aggregates_view_local_errors() {
        // A failure raised in a background tab (Settings) must surface
        // globally even while the user is on a different view.
        let app = RivuletApp {
            chat_add_error: Some("vault refused".to_owned()),
            ..Default::default()
        };
        let issue = app.collect_global_issue();
        assert_eq!(
            issue.as_ref().map(|(_, view, _)| *view),
            Some(AppView::Settings)
        );
        assert_eq!(
            issue.as_ref().map(|(kind, _, _)| *kind),
            Some(GlobalIssueKind::Error)
        );
        assert_eq!(
            issue.as_ref().map(|(_, _, text)| text.as_str()),
            Some("vault refused")
        );
    }

    #[test]
    fn global_issue_prefers_the_highest_severity_then_earliest_view() {
        let app = RivuletApp {
            scene_status: Some("Could not export scene collection: disk full".to_owned()), // Error, Scenes
            stream_key_store_status: Some(RivuletApp::static_tr("stream_key_store_unavailable")), // Warning, Stream
            ..Default::default()
        };
        assert_eq!(
            app.collect_global_issue()
                .as_ref()
                .map(|(_, view, text)| (*view, text.as_str())),
            Some((
                AppView::Scenes,
                "Could not export scene collection: disk full"
            )),
            "error must beat warning"
        );

        // Two errors on different views: the earliest sidebar view wins.
        let app = RivuletApp {
            chat_add_error: Some("settings-fail".to_owned()), // Settings
            last_error: Some("record-fail".to_owned()),       // Record
            ..Default::default()
        };
        assert_eq!(
            app.collect_global_issue()
                .as_ref()
                .map(|(_, view, text)| (*view, text.as_str())),
            Some((AppView::Record, "record-fail")),
            "ties must resolve to the earliest view in sidebar order"
        );
    }

    #[test]
    fn global_issue_ignores_positive_and_idle_statuses() {
        // Confirmations and idle service states must not be surfaced.
        let app = RivuletApp {
            scene_status: Some(RivuletApp::static_tr("scenes_switched").replace("{0}", "Game")),
            stream_key_store_status: Some(RivuletApp::static_tr("stream_key_saved")),
            obs_ws_status: Some(RivuletApp::static_tr("obs_ws_stopped")),
            remote_companion_status: Some(RivuletApp::static_tr("remote_companion_stopped")),
            ..Default::default()
        };
        assert!(
            app.collect_global_issue().is_none(),
            "confirmations/idle states must stay view-local"
        );
    }

    #[test]
    fn global_issue_surfaces_service_failures_as_warnings() {
        let app = RivuletApp {
            obs_ws_status: Some(RivuletApp::static_tr("obs_ws_error").replace("{0}", "port busy")),
            ..Default::default()
        };
        assert_eq!(
            app.collect_global_issue()
                .as_ref()
                .map(|(kind, view, _)| (*kind, *view)),
            Some((GlobalIssueKind::Warning, AppView::Settings)),
            "the obs-websocket start failure must be mirrored globally"
        );
    }

    #[test]
    fn global_issue_footers_issue_stamp_renews_on_issue_change() {
        let mut app = RivuletApp::default();
        assert!(app.global_issue_stamp.is_none());

        // First issue arms the stamp + view.
        app.chat_add_error = Some("first".to_owned());
        let first_view = app.collect_global_issue().map(|(_, view, _)| view);
        assert_eq!(first_view, Some(AppView::Settings));
        app.global_issue_view = first_view;
        app.global_issue_stamp = Some(Instant::now());

        // A *different* issue (different view) renews the stamp.
        app.chat_add_error = None;
        app.last_error = Some("record fail".to_owned());
        let second = app.collect_global_issue();
        assert_eq!(
            second.as_ref().map(|(_, view, _)| *view),
            Some(AppView::Record)
        );
        let prev_stamp = app.global_issue_stamp.unwrap();
        app.global_issue_view = second.as_ref().map(|(_, view, _)| *view);
        app.global_issue_stamp = Some(Instant::now());
        assert!(
            app.global_issue_stamp.unwrap() >= prev_stamp,
            "switching issues must renew the stamp (fresh TTL window)"
        );
    }

    #[test]
    fn chat_submit_arms_threaded_reply_and_clears_reply_target() {
        let mut app = RivuletApp {
            chat_input: "thanks!".to_owned(),
            chat_reply_target: Some((
                "ViewerOne".to_owned(),
                rivulet_core::ChatPlatform::Twitch,
                "msg-1".to_owned(),
            )),
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
                "msg-1".to_owned(),
                rivulet_core::ChatPlatform::Twitch
            ))
        );
    }

    #[test]
    fn chat_submit_rejects_whitespace_and_keeps_reply_target() {
        let mut app = RivuletApp {
            chat_reply_target: Some((
                "ViewerOne".to_owned(),
                rivulet_core::ChatPlatform::Twitch,
                "msg-1".to_owned(),
            )),
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
            chat_worker_multi: Some(rivulet_core::MultiChat::from_configs(&[
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::Twitch,
                    twitch_endpoint: "127.0.0.1:1".to_owned(), // worker backs off
                    channel: "rivulet".to_owned(),
                    token: "oauth:abc123".to_owned(),
                    ..Default::default()
                },
            ])),
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
            chat_worker_multi: Some(rivulet_core::MultiChat::from_configs(&[
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::Twitch,
                    twitch_endpoint: "127.0.0.1:1".to_owned(), // worker backs off
                    channel: "rivulet".to_owned(),
                    token: "oauth:abc123".to_owned(),
                    ..Default::default()
                },
            ])),
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
            chat_reply_target: Some((
                "ViewerOne".to_owned(),
                rivulet_core::ChatPlatform::Twitch,
                "msg-1".to_owned(),
            )),
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
    fn chat_add_account_rejects_empty_channel_and_duplicates() {
        let mut app = RivuletApp {
            chat_add_platform: rivulet_core::ChatPlatform::Twitch,
            ..RivuletApp::default()
        };
        // An empty channel is refused with the dedicated hint.
        app.apply_chat_add_account();
        assert!(
            app.chat_add_error.is_some(),
            "empty channel must be refused"
        );
        assert!(app.chat_accounts.is_empty());
        assert!(app.chat_action_pending.is_none());

        // A valid account lands in the list and clears the draft + error.
        app.chat_add_channel = " rivulet ".to_owned();
        app.chat_add_token = "oauth:abc".to_owned();
        app.apply_chat_add_account();
        if app.chat_add_error.is_none() {
            app.store_chat_account_token();
        }
        assert!(app.chat_add_error.is_none());
        assert_eq!(app.chat_accounts.len(), 1);
        assert_eq!(
            app.chat_accounts[0].platform,
            rivulet_core::ChatPlatform::Twitch
        );
        assert_eq!(app.chat_accounts[0].channel, "rivulet");
        // The roster is token-free (CodeQL rust/cleartext-logging root fix):
        // the entered token went straight into the OS credential vault.
        assert!(
            app.chat_add_token.is_empty(),
            "draft token must be consumed into the vault (never linger)"
        );
        let stored = rivulet_core::ChatTokenStore::default()
            .load(rivulet_core::ChatPlatform::Twitch, "rivulet")
            .ok()
            .flatten();
        assert_eq!(
            stored.as_deref(),
            Some("oauth:abc"),
            "the add flow must persist the token in the vault"
        );
        let _ = rivulet_core::ChatTokenStore::default()
            .delete(rivulet_core::ChatPlatform::Twitch, "rivulet");
        assert_eq!(
            app.chat_action_pending,
            Some(ChatAction::Disconnect),
            "roster changes must re-arm the worker rebuild"
        );

        // A second account on the same platform is refused as a duplicate.
        app.chat_add_platform = rivulet_core::ChatPlatform::Twitch;
        app.chat_add_channel = "other".to_owned();
        app.apply_chat_add_account();
        assert!(
            app.chat_add_error.is_some(),
            "duplicate platform must be refused"
        );
        assert_eq!(app.chat_accounts.len(), 1);

        // A different platform is fine — that is the whole point.
        app.chat_add_platform = rivulet_core::ChatPlatform::Kick;
        app.chat_add_channel = "rivulet".to_owned();
        app.apply_chat_add_account();
        assert!(app.chat_add_error.is_none());
        assert_eq!(app.chat_accounts.len(), 2);
        assert_eq!(
            app.chat_accounts[1].platform,
            rivulet_core::ChatPlatform::Kick
        );
    }

    #[test]
    fn chat_broadcast_reports_per_platform_outcomes() {
        // Twitch (capable) + YouTube (read-only): the broadcast must enqueue
        // on the Twitch leg and record the YouTube refusal for the dock.
        let mut app = RivuletApp {
            chat_worker_multi: Some(rivulet_core::MultiChat::from_configs(&[
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::Twitch,
                    twitch_endpoint: "127.0.0.1:1".to_owned(),
                    channel: "rivulet".to_owned(),
                    token: "oauth:abc".to_owned(),
                    ..Default::default()
                },
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::YouTube,
                    channel: "abc123".to_owned(),
                    ..Default::default()
                },
            ])),
            chat_state: rivulet_core::ChatConnState::Connected,
            ..Default::default()
        };
        assert!(app.send_chat_message("hello all".to_owned()));
        let outcomes = std::mem::take(&mut app.chat_last_send_outcomes);
        assert_eq!(
            outcomes,
            vec![
                (rivulet_core::ChatPlatform::Twitch, true),
                (rivulet_core::ChatPlatform::YouTube, false),
            ]
        );
    }

    #[test]
    fn chat_accounts_persist_and_legacy_configs_migrate() {
        // Round-trip: a configured account list survives save/restore. The
        // token is NOT part of the persisted JSON (serde(skip)) — it lives
        // in the OS credential vault. The save below therefore proves the
        // secret never reaches the config file.
        let mut app = RivuletApp {
            chat_accounts: vec![
                rivulet_core::ChatAccount::new(
                    rivulet_core::ChatPlatform::Twitch,
                    "rivulet".to_owned(),
                ),
                rivulet_core::ChatAccount::new(
                    rivulet_core::ChatPlatform::Kick,
                    "rivulet".to_owned(),
                ),
            ],
            ..Default::default()
        };
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);
        let stored_json = storage
            .values
            .get(eframe::APP_KEY)
            .cloned()
            .unwrap_or_default();
        eprintln!("DEBUG chat_accounts fragment: {:?}", {
            let start = stored_json.find("chat_accounts").unwrap_or(0);
            let end = (start + 300).min(stored_json.len());
            &stored_json[start..end]
        });
        // The whole-app storage string legitimately mentions other token
        // fields (the legacy chat field, the EventSub credential), so a
        // blanket substring check would be meaningless. Instead assert the
        // exact serialized roster fragment: `ChatAccount` has exactly two
        // fields, so a token-bearing entry could not produce this string.
        assert!(
            stored_json.contains(
                "chat_accounts:[(platform:Twitch,channel:\"rivulet\"),(platform:Kick,channel:\"rivulet\")]"
            ),
            "roster entries must persist as exactly platform + channel"
        );
        assert!(
            !stored_json.contains("round-trip-secret"),
            "the fixture secret must never be written to persisted state"
        );

        let restored = RivuletApp::restore_from_storage(Some(&storage))
            .expect("persisted app state must be restored");
        assert_eq!(restored.chat_accounts.len(), 2);
        assert_eq!(
            restored.chat_accounts[0].platform,
            rivulet_core::ChatPlatform::Twitch
        );
        assert_eq!(restored.chat_accounts[0].channel, "rivulet");
        assert_eq!(
            restored.chat_accounts[1].platform,
            rivulet_core::ChatPlatform::Kick
        );

        // Migration: a legacy single-platform config (channel set) becomes
        // the first account entry; a fully empty config is left alone. The
        // legacy token moves to the OS credential vault (best effort) and
        // the in-memory copy is consumed so the next save is token-free.
        let mut legacy = RivuletApp {
            chat_platform: rivulet_core::ChatPlatform::Kick,
            chat_channel: "legacychan".to_owned(),
            chat_oauth_token: "legacy-migrate-secret".to_owned(),
            ..Default::default()
        };
        // The legacy config itself was saved with its token (that is the
        // pre-migration reality), so the storage JSON may contain it here.
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut legacy, &mut storage);
        let mut restored =
            RivuletApp::restore_from_storage(Some(&storage)).expect("legacy state must restore");
        assert_eq!(
            restored.chat_accounts.len(),
            1,
            "legacy config must migrate"
        );
        assert_eq!(
            restored.chat_accounts[0].platform,
            rivulet_core::ChatPlatform::Kick
        );
        assert_eq!(restored.chat_accounts[0].channel, "legacychan");
        assert!(
            restored.chat_channel.is_empty() && restored.chat_oauth_token.is_empty(),
            "the legacy fields must be consumed by the migration"
        );
        // The next save after the migration must be token-free: the in-memory
        // token was moved to the OS credential vault and wiped.
        let mut post = MemoryStorage::default();
        eframe::App::save(&mut restored, &mut post);
        let post_json = post
            .values
            .get(eframe::APP_KEY)
            .cloned()
            .unwrap_or_default();
        assert!(
            !post_json.contains("legacy-migrate-secret"),
            "post-migration state must not contain the legacy token"
        );
        // Clean the vault entry the migration created.
        let _ = rivulet_core::ChatTokenStore::default()
            .delete(rivulet_core::ChatPlatform::Kick, "legacychan");

        let mut fresh = RivuletApp::default();
        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut fresh, &mut storage);
        let restored =
            RivuletApp::restore_from_storage(Some(&storage)).expect("fresh state must restore");
        assert!(
            restored.chat_accounts.is_empty(),
            "a fresh config must not grow a phantom account"
        );
    }

    #[test]
    fn chat_token_store_round_trips_through_the_os_vault() {
        // Real keyring round trip (Windows credential vault in CI). Proves
        // save/load/delete for chat tokens, and that the hydrated value
        // matches what the add-account flow stored.
        let store = rivulet_core::ChatTokenStore::default();
        let channel = "rivulet-vault-test";
        // Clean any stale entry from an earlier run.
        let _ = store.delete(rivulet_core::ChatPlatform::Twitch, channel);
        assert_eq!(
            store.load(rivulet_core::ChatPlatform::Twitch, channel),
            Ok(None),
            "no entry yet"
        );
        store
            .save(
                rivulet_core::ChatPlatform::Twitch,
                channel,
                "oauth:vault-secret",
            )
            .expect("save token to vault");
        assert_eq!(
            store.load(rivulet_core::ChatPlatform::Twitch, channel),
            Ok(Some("oauth:vault-secret".to_owned())),
        );
        store
            .delete(rivulet_core::ChatPlatform::Twitch, channel)
            .expect("delete token from vault");
        assert_eq!(
            store.load(rivulet_core::ChatPlatform::Twitch, channel),
            Ok(None),
        );
    }

    #[test]
    fn chat_message_platform_tags_reach_the_dock_list() {
        // The parsers tag every message with its platform; the dock badges
        // lines from that tag. A synthetic alert entry keeps the platform
        // its ingestion source provided (None for provider-agnostic ones).
        let mut app = RivuletApp::default();
        app.chat_messages.push(rivulet_core::ChatMessage {
            user: "ViewerOne".to_owned(),
            text: "hi".to_owned(),
            action: false,
            color: None,
            badges: Vec::new(),
            broadcaster: false,
            id: Some("1".to_owned()),
            timestamp: 1,
            platform: Some(rivulet_core::ChatPlatform::Kick),
            source_room_id: None,
        });
        app.chat_messages.push(rivulet_core::ChatMessage {
            user: "PreviewViewer".to_owned(),
            text: "followed".to_owned(),
            action: true,
            color: None,
            badges: Vec::new(),
            broadcaster: false,
            id: None,
            timestamp: 2,
            platform: None,
            source_room_id: None,
        });
        assert_eq!(
            app.chat_messages[0].platform,
            Some(rivulet_core::ChatPlatform::Kick)
        );
        assert_eq!(app.chat_messages[1].platform, None);
        // The alert conversion carries the AlertEvent platform through.
        let event = rivulet_core::AlertEvent::sample_follow();
        let line = app.alert_event_to_chat_message(&event);
        assert_eq!(
            line.platform,
            Some(rivulet_core::ChatPlatform::Twitch),
            "the sample alert is Twitch EventSub-shaped"
        );
    }

    #[test]
    fn chat_reply_routes_through_the_armed_platform() {
        // A Kick message cannot be replied to (no threading); a Twitch one
        // is enqueued on the Twitch leg. The platform comes with the armed
        // target, never guessed.
        let mut app = RivuletApp {
            chat_worker_multi: Some(rivulet_core::MultiChat::from_configs(&[
                rivulet_core::ChatConfig {
                    platform: rivulet_core::ChatPlatform::Twitch,
                    twitch_endpoint: "127.0.0.1:1".to_owned(),
                    channel: "rivulet".to_owned(),
                    token: "oauth:abc".to_owned(),
                    ..Default::default()
                },
            ])),
            chat_state: rivulet_core::ChatConnState::Connected,
            ..Default::default()
        };
        assert!(!app.send_chat_reply(
            "hi".to_owned(),
            "parent-1".to_owned(),
            rivulet_core::ChatPlatform::Kick,
        ));
        assert!(app.send_chat_reply(
            "hi".to_owned(),
            "parent-1".to_owned(),
            rivulet_core::ChatPlatform::Twitch,
        ));
    }

    #[test]
    fn chat_combined_dock_contract_is_pinned_in_source() {
        // Source contract for the combined dock: the account list drives the
        // workers, the message list badges platforms, the broadcast reports
        // per-platform outcomes and the migration keeps old configs working.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_chat_dock")
            .map(|(_, rest)| rest)
            .expect("draw_chat_dock must exist in the Stream workspace");
        for marker in [
            "self.chat_accounts",
            "MultiChat",
            "message.platform",
            "platform.label()",
            "chat_last_send_outcomes",
            "chat_send_partial",
            "apply_chat_add_account()",
            "chat_multi_hint",
        ] {
            assert!(
                draw.contains(marker),
                "the combined dock must reference {marker}"
            );
        }
        // The old single-platform selector must be gone from the dock.
        assert!(
            !draw.contains("\"chat_platform\""),
            "the platform ComboBox must be replaced by the account list"
        );
        let i18n = std::fs::read_to_string("../rivulet-core/src/i18n.rs").expect("i18n readable");
        for key in [
            "chat_multi_hint",
            "chat_account_add",
            "chat_account_remove",
            "chat_account_duplicate",
            "chat_account_channel_required",
            "chat_channel_empty",
            "chat_send_partial",
        ] {
            let k = format!("\"{key}\"");
            assert!(
                i18n.matches(&k).count() >= 2,
                "{key} must exist in EN and DE"
            );
        }
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
    fn chat_dock_badges_shared_chat_source_in_source() {
        // Shared-Chat attribution contract: the dock must render a per-source
        // badge for messages carrying `source_room_id` (Twitch Shared Chat),
        // before the user name, with the translated hover explanation. Plain
        // messages (no source room) must not hit the badge branch.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        let draw = source
            .split_once("fn draw_chat_dock")
            .map(|(_, rest)| rest)
            .expect("draw_chat_dock must exist in the Stream workspace");
        assert!(
            draw.contains("message.source_room_id"),
            "the dock must branch on the shared-chat source tag"
        );
        let badge = draw
            .find("chat_shared_chat_source_tooltip")
            .expect("shared-chat badge in dock");
        let name = draw.find("ui.label(text)").expect("user label in dock");
        assert!(badge < name, "the badge must render before the user label");
        let i18n = std::fs::read_to_string("../rivulet-core/src/i18n.rs").expect("i18n readable");
        let k = "\"chat_shared_chat_source_tooltip\"";
        assert!(
            i18n.matches(k).count() >= 2,
            "shared-chat tooltip must exist in EN and DE"
        );
    }

    #[test]
    fn alert_lines_never_claim_a_shared_chat_source() {
        // Alert previews are host-local synthetic entries; they must never
        // grow a shared-chat badge.
        let app = RivuletApp::default();
        let line = app.alert_event_to_chat_message(&rivulet_core::AlertEvent::sample_follow());
        assert!(line.source_room_id.is_none());
    }

    #[test]
    fn shared_room_badge_falls_back_and_retries_without_credentials() {
        // Without a client id / Twitch token the badge stays on the raw id,
        // nothing dials out, and the in-flight marker is released so a later
        // frame retries instead of sticking. A landed resolution flips the
        // label to the cached login.
        let mut app = RivuletApp::default();
        assert_eq!(app.shared_room_badge("12826"), Some("12826".to_owned()));
        assert!(app.chat_room_name_rx.is_none(), "no lookup without creds");
        assert_eq!(app.shared_room_badge("12826"), Some("12826".to_owned()));
        app.chat_room_names
            .apply_results(vec![("12826".to_owned(), "twitch".to_owned())]);
        assert_eq!(app.shared_room_badge("12826"), Some("twitch".to_owned()));
    }

    #[test]
    fn chat_dock_resolves_shared_chat_rooms_in_source() {
        // Source contract: the badge helper drives the cached Helix lookup
        // (collapsed in-flight, background dispatch, raw-id fallback) and the
        // reconcile loop drains the finished lookup into the cache.
        let source = std::fs::read_to_string("src/app.rs").expect("GUI source readable");
        assert!(
            source.contains("fn shared_room_badge"),
            "the badge helper must exist"
        );
        let draw = source
            .split_once("fn draw_chat_dock")
            .map(|(_, rest)| rest)
            .expect("draw_chat_dock must exist");
        assert!(
            draw.contains("shared_room_badge"),
            "the dock must render through the badge helper"
        );
        let helper = source
            .split_once("fn shared_room_badge")
            .map(|(_, rest)| rest)
            .expect("shared_room_badge must exist");
        for marker in [
            "RoomNameStep::Known",
            "RoomNameStep::Pending",
            "release_in_flight",
            "ChatTokenStore::default()",
            "HelixRoomResolver::default()",
            "helix_users_by_id",
            "thread::spawn",
        ] {
            assert!(
                helper.contains(marker),
                "the badge helper must use {marker}"
            );
        }
        let reconcile = source
            .split_once("fn reconcile_chat")
            .map(|(_, rest)| rest)
            .expect("reconcile_chat must exist");
        assert!(
            reconcile.contains("chat_room_name_rx") && reconcile.contains("apply_results"),
            "reconcile must drain finished room-name lookups into the cache"
        );
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

    #[test]
    fn detect_os_locale_returns_valid_locale() {
        // detect_os_locale must always return a valid Locale variant,
        // even when no env vars are set (defaults to English).
        let locale = super::detect_os_locale();
        assert!(Locale::all().contains(&locale));
    }

    #[test]
    fn restream_target_config_converts_to_stream_target() {
        let config = super::RestreamTargetConfig {
            name: "My Twitch".to_owned(),
            platform: StreamPlatform::Twitch,
            ingest_url: "rtmps://live.twitch.tv/app".to_owned(),
            stream_key: "test_key_123".to_owned(),
            enabled: true,
        };
        let target = config.to_stream_target().expect("should produce a target");
        assert_eq!(target.name, "My Twitch");
        assert_eq!(target.settings.platform, StreamPlatform::Twitch);
        assert_eq!(target.settings.stream_key, "test_key_123");
    }

    #[test]
    fn restream_target_config_disabled_produces_none() {
        let config = super::RestreamTargetConfig {
            enabled: false,
            stream_key: "key".to_owned(),
            ..Default::default()
        };
        assert!(config.to_stream_target().is_none());
    }

    #[test]
    fn restream_target_config_empty_key_produces_none() {
        let config = super::RestreamTargetConfig {
            enabled: true,
            stream_key: String::new(),
            ..Default::default()
        };
        assert!(config.to_stream_target().is_none());
    }

    #[test]
    fn restream_target_config_uses_platform_default_url_when_empty() {
        let config = super::RestreamTargetConfig {
            name: "YT".to_owned(),
            platform: StreamPlatform::YouTube,
            ingest_url: String::new(),
            stream_key: "yt_key".to_owned(),
            enabled: true,
        };
        let target = config.to_stream_target().expect("should produce target");
        assert!(target.settings.ingest_url.contains("youtube.com"));
    }

    // ── Plugin Phase 3: approval flow contract (plugin-system RFC) ──────

    /// A discovered plugin fixture with the given capabilities, parsed
    /// through the real manifest parser.
    fn discovered_plugin(caps: &str) -> rivulet_core::DiscoveredPlugin {
        let toml = format!(
            r#"
[plugin]
id = "com.example.demo"
version = "1.2.0"
api_version = {{ min = "1.0", max = "2.0" }}
name = "Demo"
author = "Demo Author"
type = {{ kind = "ui_panel", entry_point = "plugin.wasm" }}
[plugin.capabilities]
{caps}
"#
        );
        let manifest = rivulet_core::parse_manifest(&toml).expect("manifest parses");
        rivulet_core::DiscoveredPlugin {
            manifest,
            bundle_dir: std::path::PathBuf::from("/plugins/com.example.demo"),
            binary_path: std::path::PathBuf::from("/plugins/com.example.demo/plugin.wasm"),
        }
    }

    #[test]
    fn plugin_review_state_reflects_decisions_and_enable_flag() {
        let mut app = RivuletApp {
            ..Default::default()
        };
        let plugin = discovered_plugin("ui = true\naudio_in = true");
        app.plugins_discovered.push(plugin.clone());

        // Nothing decided yet → Pending (enable blocked).
        assert_eq!(app.plugin_review_state(&plugin), PluginReviewState::Pending);

        // One capability approved, one denied → fully decided + disabled.
        app.plugin_approvals.approve("com.example.demo", "ui");
        app.plugin_approvals.deny("com.example.demo", "audio_in");
        assert_eq!(
            app.plugin_review_state(&plugin),
            PluginReviewState::Disabled
        );

        // Enabling flips the state to Enabled.
        app.set_plugin_enabled("com.example.demo", true);
        assert_eq!(app.plugin_review_state(&plugin), PluginReviewState::Enabled);
        assert!(app
            .plugin_approvals
            .record("com.example.demo")
            .is_some_and(|r| r.enabled));
    }

    #[test]
    fn set_plugin_enabled_is_blocked_until_fully_decided() {
        let mut app = RivuletApp {
            ..Default::default()
        };
        let plugin = discovered_plugin("ui = true");
        // Review pending: enabling must be a no-op (gate before activation).
        app.set_plugin_enabled("com.example.demo", true);
        assert!(app.plugin_approvals.record("com.example.demo").is_none());
        let _ = &plugin; // fixtures share the manifest id
    }

    #[test]
    fn plugin_review_round_trip_draft_to_store() {
        let mut app = RivuletApp {
            ..Default::default()
        };
        let plugin = discovered_plugin("ui = true\nsecrets = true");
        app.plugins_discovered.push(plugin);

        // Open the dialog: the draft mirrors the persisted decisions.
        app.open_plugin_review("com.example.demo");
        assert_eq!(app.plugin_review_open.as_deref(), Some("com.example.demo"));
        assert!(app.plugin_review_draft.contains_key("ui"));
        assert!(app.plugin_review_draft.contains_key("secrets"));

        // Approve both in the draft and apply.
        app.plugin_review_draft.insert("ui".to_owned(), true);
        app.plugin_review_draft.insert("secrets".to_owned(), true);
        app.apply_plugin_review();

        // Dialog closed; both decisions persisted (secrets stays recorded —
        // but the runtime never grants it to WASM, RFC §6.3).
        assert!(app.plugin_review_open.is_none());
        assert!(app.plugin_review_draft.is_empty());
        let record = app
            .plugin_approvals
            .record("com.example.demo")
            .expect("record exists");
        assert_eq!(
            record.capabilities.get("ui"),
            Some(&rivulet_core::CapabilityDecision::Approved)
        );
        assert_eq!(
            record.capabilities.get("secrets"),
            Some(&rivulet_core::CapabilityDecision::Approved)
        );
        // Enable stays off until the user explicitly enables.
        assert!(!record.enabled);
    }

    #[test]
    fn apply_plugin_review_denies_unreviewed_capabilities() {
        let mut app = RivuletApp {
            ..Default::default()
        };
        app.plugins_discovered.push(discovered_plugin(
            "ui = true\naudio_in = true\nvideo_out = true",
        ));

        app.open_plugin_review("com.example.demo");
        // Approve only ui in the draft.
        app.plugin_review_draft.insert("ui".to_owned(), true);
        app.plugin_review_draft.insert("audio_in".to_owned(), false);
        app.plugin_review_draft
            .insert("video_out".to_owned(), false);
        app.apply_plugin_review();

        let record = app
            .plugin_approvals
            .record("com.example.demo")
            .expect("record exists");
        assert_eq!(
            record.capabilities.get("ui"),
            Some(&rivulet_core::CapabilityDecision::Approved)
        );
        assert_eq!(
            record.capabilities.get("audio_in"),
            Some(&rivulet_core::CapabilityDecision::Denied)
        );
        assert_eq!(
            record.capabilities.get("video_out"),
            Some(&rivulet_core::CapabilityDecision::Denied)
        );
    }

    #[test]
    fn open_plugin_review_seeds_draft_from_previous_decisions() {
        let mut app = RivuletApp {
            ..Default::default()
        };
        app.plugins_discovered
            .push(discovered_plugin("ui = true\naudio_in = true"));
        app.plugin_approvals.approve("com.example.demo", "ui");
        app.plugin_approvals.deny("com.example.demo", "audio_in");

        app.open_plugin_review("com.example.demo");
        assert_eq!(app.plugin_review_draft.get("ui"), Some(&true));
        assert_eq!(app.plugin_review_draft.get("audio_in"), Some(&false));
    }

    #[test]
    fn plugin_approvals_persist_through_serde_round_trip() {
        // The approvals live inside RivuletApp's serde state (eframe storage);
        // a round trip through serde must preserve every decision + flag.
        let mut app = RivuletApp {
            ..Default::default()
        };
        app.plugins_discovered
            .push(discovered_plugin("ui = true\nsecrets = true"));
        app.open_plugin_review("com.example.demo");
        app.plugin_review_draft.insert("ui".to_owned(), true);
        app.plugin_review_draft.insert("secrets".to_owned(), true);
        app.apply_plugin_review();
        app.set_plugin_enabled("com.example.demo", true);

        let json = serde_json::to_string(&app.plugin_approvals).unwrap();
        let restored: rivulet_core::PluginApprovals = serde_json::from_str(&json).unwrap();
        let record = restored.record("com.example.demo").expect("record exists");
        assert!(record.enabled);
        assert_eq!(
            record.capabilities.get("ui"),
            Some(&rivulet_core::CapabilityDecision::Approved)
        );
    }

    #[test]
    fn sensitive_capability_never_granted_to_wasm_even_after_approval() {
        let mut app = RivuletApp {
            ..Default::default()
        };
        app.plugins_discovered
            .push(discovered_plugin("secrets = true"));
        app.open_plugin_review("com.example.demo");
        app.plugin_review_draft.insert("secrets".to_owned(), true);
        app.apply_plugin_review();
        app.set_plugin_enabled("com.example.demo", true);

        let plugin = &app.plugins_discovered[0];
        assert!(!app.plugin_approvals.effective_grant(
            plugin.id(),
            plugin.capabilities(),
            "secrets",
            true,
        ));
    }

    // ── Multi-track audio routing, Phase 2 GUI (issue #154) ───────────

    #[test]
    fn audio_routing_badges_cover_all_four_routing_states() {
        assert_eq!(RivuletApp::audio_routing_badges(AudioRouting::BOTH), "R·S");
        assert_eq!(
            RivuletApp::audio_routing_badges(AudioRouting::RECORD_ONLY),
            "R"
        );
        assert_eq!(
            RivuletApp::audio_routing_badges(AudioRouting::STREAM_ONLY),
            "S"
        );
        assert_eq!(RivuletApp::audio_routing_badges(AudioRouting::NONE), "—");
    }

    #[test]
    fn audio_sources_persist_through_eframe_storage_round_trip() {
        // audio_routing_v1: sources with volume, mute, routing and filters
        // must survive save -> restore exactly (the design-doc gate).
        let mut app = RivuletApp {
            audio_sources: vec![
                AudioSource::system_default().with_routing(AudioRouting::RECORD_ONLY),
                AudioSource::microphone_default(),
            ],
            ..Default::default()
        };
        app.audio_sources[0].volume = 0.75;
        app.audio_sources[0].muted = true;
        app.audio_sources[1].routing = AudioRouting::STREAM_ONLY;

        let mut storage = MemoryStorage::default();
        eframe::App::save(&mut app, &mut storage);

        let restored = RivuletApp::restore_from_storage(Some(&storage))
            .expect("persisted app state must be restored");
        assert_eq!(restored.audio_sources.len(), 2);
        assert_eq!(restored.audio_sources[0].volume, 0.75);
        assert!(restored.audio_sources[0].muted);
        assert_eq!(restored.audio_sources[0].routing, AudioRouting::RECORD_ONLY);
        assert_eq!(restored.audio_sources[1].routing, AudioRouting::STREAM_ONLY);
        // The restore path must seed the engine so the first session starts
        // with the persisted routing, and the sync flag must be cleared.
        assert_eq!(restored.engine.audio_sources().len(), 2);
        assert!(!restored.audio_mixer_needs_sync);
    }

    #[test]
    fn audio_sources_with_legacy_capture_stay_empty_by_default() {
        // Empty config = legacy System/Microphone capture (backward compat).
        let app = RivuletApp::default();
        assert!(app.audio_sources.is_empty());
        assert!(app.engine.audio_sources().is_empty());
    }

    #[test]
    fn audio_mixer_add_and_remove_sync_the_engine() {
        let mut app = RivuletApp::default();
        let source = AudioSource::application("Discord", "pending_app");
        let id = app.engine.add_audio_source(source.clone());
        app.audio_sources.push(source);
        app.audio_mixer_needs_sync = true;
        app.sync_audio_routing();
        assert_eq!(app.engine.audio_sources().len(), 1);
        assert!(!app.audio_mixer_needs_sync, "sync must clear the flag");

        // Removal is staged behind the confirmation dialog first.
        app.remove_audio_source(id);
        assert!(!app.engine.audio_sources().is_empty());
        assert!(matches!(
            app.pending_confirmation,
            Some(PendingConfirmation::RemoveAudioSource { .. })
        ));

        app.confirm_pending_confirmation();
        app.sync_audio_routing();
        assert!(app.engine.audio_sources().is_empty());
        assert!(app.audio_sources.is_empty());
        assert_eq!(app.pending_confirmation, None);
    }

    #[test]
    fn audio_source_removal_cancel_keeps_the_engine_source() {
        let mut app = RivuletApp::default();
        let source = AudioSource::application("Discord", "pending_app");
        let id = app.engine.add_audio_source(source.clone());
        app.audio_sources.push(source);

        app.remove_audio_source(id);
        assert!(app.pending_confirmation.is_some());
        app.cancel_pending_confirmation();

        assert!(
            app.engine.audio_sources().iter().any(|s| s.id == id),
            "cancelled removal must leave the engine source intact"
        );
        assert!(app.audio_sources.iter().any(|s| s.id == id));
        assert_eq!(app.pending_confirmation, None);
    }

    #[test]
    fn audio_mixer_volume_and_mute_changes_reach_the_engine() {
        // The strip writes through the engine API so the values land in the
        // engine list (and would apply live when a session is running).
        let mut app = RivuletApp::default();
        let id = app
            .engine
            .add_audio_source(AudioSource::microphone_default());
        app.audio_sources
            .push(app.engine.audio_sources()[0].clone());

        let _ = app.engine.set_audio_source_volume(id, 0.5);
        let _ = app.engine.set_audio_source_muted(id, true);
        assert_eq!(app.engine.audio_sources()[0].effective_volume(), 0.0);
        let _ = app.engine.set_audio_source_muted(id, false);
        assert_eq!(app.engine.audio_sources()[0].effective_volume(), 0.5);

        // Mirror the engine edit into the GUI list (what the strip does).
        app.audio_sources[0].volume = 0.5;
        app.audio_sources[0].muted = false;
        assert_eq!(app.audio_sources[0].volume, 0.5);
        assert!(!app.audio_sources[0].muted);
    }

    #[test]
    fn audio_mixer_routing_change_flows_into_engine_config() {
        let mut app = RivuletApp::default();
        app.engine
            .set_audio_sources(vec![AudioSource::system_default()]);
        app.audio_sources = app.engine.audio_sources().to_vec();
        let id = app.audio_sources[0].id;

        let _ = app
            .engine
            .set_audio_source_routing(id, AudioRouting::STREAM_ONLY);
        app.audio_sources[0].routing = AudioRouting::STREAM_ONLY;
        assert_eq!(
            app.engine.audio_sources()[0].routing,
            AudioRouting::STREAM_ONLY
        );
    }

    #[test]
    fn audio_mixer_i18n_keys_translate_in_both_locales() {
        use rivulet_core::Locale;
        for key in [
            "audio_source_add",
            "audio_source_remove",
            "audio_source_name",
            "audio_source_mute",
            "audio_routing_record",
            "audio_routing_stream",
            "audio_routing_hint",
            "audio_routing_inline",
            "audio_routing_sources",
            "audio_routing_legacy_active",
            "audio_filter_per_source",
            "filter_compressor",
            "filter_limiter",
        ] {
            let en = Locale::En.tr(key);
            let de = Locale::De.tr(key);
            assert_ne!(en, key, "EN key {key} must exist");
            assert_ne!(de, key, "DE key {key} must exist");
        }
        assert_eq!(Locale::En.tr("audio_source_add"), "Add Source");
        assert_eq!(Locale::De.tr("audio_source_add"), "Quelle hinzufügen");
    }

    #[test]
    fn audio_mixer_ui_is_wired_into_all_three_placements() {
        // The Mixer view hosts the full matrix; the Record and Stream views
        // embed the same shared strip (single implementation, three
        // placements - the design-doc contract).
        let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
        assert!(source.contains("fn draw_mixer_sources"));
        assert!(source.contains("fn draw_audio_source_strip"));
        assert!(source.contains("fn draw_inline_audio_mixer"));
        // Exactly two inline call sites (Record + Stream); the third
        // string match is this test's own literal, so expect three.
        let inline_calls = source.matches("self.draw_inline_audio_mixer(ui);").count();
        assert_eq!(
            inline_calls, 3,
            "the inline strip must appear in the Record and Stream views"
        );
        // The Mixer-view block hosts the matrix behind the sync hook.
        assert!(source.contains("self.sync_audio_routing();"));
        assert!(source.contains("self.draw_mixer_sources(ui);"));
    }

    #[test]
    fn audio_mixer_application_sources_without_pid_stay_pending() {
        // An Application-kind source without a process selection keeps the
        // `pending_app` device id: the pipeline phase must never mistake it
        // for a resolved capture target (and the WASAPI backend must never
        // try to activate pid-less sources).
        let source = AudioSource::application("Discord", "pending_app");
        assert_eq!(source.device_id, "pending_app");
        assert_eq!(source.kind, rivulet_core::AudioSourceKind::Application);
        assert_eq!(source.device_pid(), None);
    }

    // ── Multi-track audio routing, Phase 3 (issue #154, Windows WASAPI) ──

    #[test]
    fn routed_application_targets_selects_only_routed_pid_sources() {
        // Exactly the contract `sync_app_audio_captures` implements: routed
        // Application sources with a pid, nothing else.
        let routed_app = AudioSource::application("Game", "pid:111");
        let unrouted_app =
            AudioSource::application("Updater", "pid:222").with_routing(AudioRouting::NONE);
        let pending_app = AudioSource::application("Discord", "pending_app");
        let system = AudioSource::system_default();
        let sources = vec![routed_app, unrouted_app, pending_app, system];

        let targets = routed_application_targets(&sources);
        assert_eq!(targets.len(), 1, "only the routed pid source is captured");
        assert_eq!(targets[0].1, 111);
    }

    #[test]
    fn routed_application_targets_is_empty_without_sources() {
        // The legacy capture path stays untouched when no routed sources exist.
        assert!(routed_application_targets(&[]).is_empty());
    }

    // ── Device capture (issue #229 WASAPI, issue #231 PipeWire) ─────

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn routed_device_targets_selects_only_routed_device_sources() {
        // Exactly the contract `sync_device_audio_captures` implements:
        // routed Input/Output sources with a parseable WASAPI device id.
        let routed_out = AudioSource::output_device("Game", "wasapi-out:{0.0.0.00000000}.{abc}");
        let routed_in = AudioSource::input_device("Mic", "wasapi-in:{0.0.1.00000000}.{def}");
        let unrouted = AudioSource::output_device("Idle", "wasapi-out:{0.0.0.00000000}.{ghi}")
            .with_routing(AudioRouting::NONE);
        let legacy = AudioSource::output_device("System", "system_loopback");
        let sources = vec![routed_out, routed_in, unrouted, legacy];

        let targets = routed_device_targets(&sources);
        assert_eq!(
            targets.len(),
            2,
            "only routed WASAPI device sources are captured"
        );
        assert_eq!(targets[0].1, "wasapi-out:{0.0.0.00000000}.{abc}");
        assert_eq!(targets[1].1, "wasapi-in:{0.0.1.00000000}.{def}");
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn routed_device_targets_is_empty_without_device_sources() {
        // Legacy placeholders and pid sources never reach the device backend.
        let sources = vec![
            AudioSource::output_device("System", "system_loopback"),
            AudioSource::input_device("Mic", "default_input"),
            AudioSource::application("App", "pid:42"),
        ];
        assert!(routed_device_targets(&sources).is_empty());
        assert!(routed_device_targets(&[]).is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn routed_device_targets_selects_routed_pipewire_sources() {
        // Issue #231: the Linux mirror of the WASAPI target contract —
        // routed sources with `pw-src:`/`pw-mon:` ids go to the PipeWire
        // device backend, placeholders and pids do not.
        let sources = vec![
            AudioSource::input_device("Mic", "pw-src:42"),
            AudioSource::output_device("Desktop", "pw-mon:7"),
            AudioSource::output_device("Idle", "pw-mon:9").with_routing(AudioRouting::NONE),
            AudioSource::output_device("System", "system_loopback"),
        ];
        let targets = routed_device_targets(&sources);
        assert_eq!(
            targets,
            vec![
                (sources[0].id, "pw-src:42".to_owned()),
                (sources[1].id, "pw-mon:7".to_owned()),
            ],
            "only routed PipeWire device sources are captured"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pipewire_device_source_shows_friendly_name_with_id_fallback() {
        // Strips resolve a `pw-` device source through the node list; a
        // vanished node falls back to the raw device id.
        #[allow(clippy::needless_update)]
        let devices = vec![rivulet_audio::AudioDeviceInfo {
            endpoint_id: "42".to_owned(),
            name: "Built-in Speakers".to_owned(),
            node_name: "alsa_output.pci-0000_00_1f.3.analog-stereo".to_owned(),
            is_output: true,
            is_default: false,
        }];
        let source = AudioSource::output_device("Desktop", "pw-mon:42");
        assert_eq!(
            audio_device_strip_label(&source, &devices),
            "Built-in Speakers"
        );
        assert_eq!(
            audio_device_strip_label(&AudioSource::output_device("Gone", "pw-mon:999"), &devices),
            "pw-mon:999",
            "a vanished node must not render as a different device"
        );
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn device_source_shows_friendly_name_with_id_fallback() {
        // Strips resolve a device source through the device list; a vanished
        // device falls back to the raw device id, and legacy sources keep
        // their stored name. Uses the platform-agnostic fields; the id shape
        // differs per platform but the resolution contract is shared.
        #[allow(clippy::needless_update)]
        let devices = vec![rivulet_audio::AudioDeviceInfo {
            endpoint_id: "{0.0.0.00000000}.{abc}".to_owned(),
            name: "Speakers (Sonar Stream)".to_owned(),
            #[cfg(target_os = "linux")]
            node_name: String::new(),
            is_output: true,
            is_default: false,
        }];
        let source = AudioSource::output_device("Game", "wasapi-out:{0.0.0.00000000}.{abc}");
        assert_eq!(
            audio_device_strip_label(&source, &devices),
            "Speakers (Sonar Stream)"
        );
        assert_eq!(
            audio_device_strip_label(
                &AudioSource::output_device("Gone", "wasapi-out:{0.0.0.00000000}.{gone}"),
                &devices
            ),
            "wasapi-out:{0.0.0.00000000}.{gone}",
            "a vanished endpoint must not render as a different device"
        );
        let legacy = AudioSource::system_default();
        assert_eq!(
            audio_device_strip_label(&legacy, &devices),
            legacy.name,
            "legacy sources keep their stored name"
        );
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn device_picker_label_marks_the_default_device() {
        #[allow(clippy::needless_update)]
        let device = rivulet_audio::AudioDeviceInfo {
            endpoint_id: "{0.0.0.00000000}.{abc}".to_owned(),
            name: "Speakers".to_owned(),
            #[cfg(target_os = "linux")]
            node_name: String::new(),
            is_output: true,
            is_default: true,
        };
        assert_eq!(
            audio_device_picker_label(&device, "(default)"),
            "Speakers (default)"
        );
        let mut other = device.clone();
        other.is_default = false;
        assert_eq!(audio_device_picker_label(&other, "(default)"), "Speakers");
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn audio_devices_refresh_due_is_bounded_by_interval() {
        // Same bounded-polling contract as the game-window list: a never
        // enumerated cache is due, a fresh one is not, and the interval
        // elapsed since the last enumeration makes it due again.
        let app = RivuletApp::default();
        assert!(
            app.audio_devices_refresh_due(std::time::Instant::now()),
            "never enumerated => due"
        );
        let app = RivuletApp {
            audio_device_list_last_refresh: Some(std::time::Instant::now()),
            ..RivuletApp::default()
        };
        assert!(
            !app.audio_devices_refresh_due(std::time::Instant::now()),
            "a fresh enumeration is not due"
        );
        let app = RivuletApp {
            audio_device_list_last_refresh: Some(
                std::time::Instant::now() - AUDIO_DEVICES_REFRESH_INTERVAL,
            ),
            ..RivuletApp::default()
        };
        assert!(
            app.audio_devices_refresh_due(std::time::Instant::now()),
            "interval elapsed => due again"
        );
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn refresh_audio_devices_live_keeps_selection_and_clears_vanished() {
        // A selection whose endpoint still exists survives the live refresh;
        // a vanished one is cleared so the pending-device hint shows instead
        // of silently adding a dead device id.
        let devices = RivuletApp::default().audio_device_list();
        let existing = devices
            .first()
            .map(|d| d.device_id())
            .unwrap_or_else(|| "wasapi-out:{0.0.0.00000000}.{ci}".to_owned());
        let mut app = RivuletApp {
            audio_mixer_new_source_device_id: Some(existing.clone()),
            ..RivuletApp::default()
        };
        app.refresh_audio_devices_live();
        if devices.is_empty() {
            // Endpoint-less host (hosted CI runner): nothing can survive.
            assert!(app.audio_mixer_new_source_device_id.is_none());
        } else {
            assert_eq!(
                app.audio_mixer_new_source_device_id.as_deref(),
                Some(existing.as_str()),
                "a live endpoint selection must survive the refresh"
            );
        }
        let mut app = RivuletApp {
            audio_mixer_new_source_device_id: Some("wasapi-out:{0.0.0.00000000}.{gone}".to_owned()),
            ..RivuletApp::default()
        };
        app.refresh_audio_devices_live();
        assert!(
            app.audio_mixer_new_source_device_id.is_none(),
            "a vanished endpoint must not survive the refresh"
        );
        assert!(
            app.audio_device_list.is_some(),
            "the refresh populated the cache"
        );
        assert!(app.audio_device_list_last_refresh.is_some());
    }

    #[cfg(any(target_os = "windows", target_os = "linux"))]
    #[test]
    fn device_picker_ui_is_wired() {
        // The Mixer's Input/Output rows show the device picker; the selected
        // endpoint becomes the source's `wasapi-out:`/`wasapi-in:` device id
        // on add, and routed device sources get capture threads + drains.
        let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
        for needle in [
            "audio_mixer_new_source_device_id",
            "audio_device_list",
            "audio_device_list_last_refresh",
            "fn audio_devices_refresh_due",
            "fn refresh_audio_devices_live",
            "AUDIO_DEVICES_REFRESH_INTERVAL",
            "fn routed_device_targets",
            "fn audio_device_picker_label",
            "fn audio_device_strip_label",
            "fn sync_device_audio_captures",
            "fn drain_device_audio_frames",
            "self.sync_device_audio_captures();",
            "self.drain_device_audio_frames();",
            "AudioDeviceCapture::start_for_target",
            "push_audio_source",
        ] {
            assert!(
                source.contains(needle),
                "app.rs must pin the device-capture surface: {needle}"
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn app_audio_process_picker_ui_is_wired() {
        // The Mixer's Application-kind row shows the process picker; the
        // selected pid becomes the source's `pid:<n>` device id on add.
        let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
        assert!(source.contains("fn list_audio_processes"));
        assert!(source.contains("audio_mixer_new_source_pid"));
        assert!(source.contains("fn sync_app_audio_captures"));
        assert!(source.contains("fn drain_app_audio_frames"));
        assert!(source.contains("self.sync_app_audio_captures();"));
        assert!(source.contains("self.drain_app_audio_frames();"));
        assert!(source.contains("AppAudioCapture::start"));
        assert!(source.contains("push_audio_source"));
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[test]
    fn app_audio_processes_refresh_due_is_bounded_by_interval() {
        // Same bounded-polling contract as the device picker: a never
        // enumerated cache is due, a fresh one is not, and the interval
        // elapsed since the last enumeration makes it due again.
        let app = RivuletApp::default();
        assert!(
            app.app_audio_processes_refresh_due(std::time::Instant::now()),
            "never enumerated => due"
        );
        let app = RivuletApp {
            app_audio_processes_last_refresh: Some(std::time::Instant::now()),
            ..RivuletApp::default()
        };
        assert!(
            !app.app_audio_processes_refresh_due(std::time::Instant::now()),
            "a fresh enumeration is not due"
        );
        let app = RivuletApp {
            app_audio_processes_last_refresh: Some(
                std::time::Instant::now() - APP_AUDIO_PROCESSES_REFRESH_INTERVAL,
            ),
            ..RivuletApp::default()
        };
        assert!(
            app.app_audio_processes_refresh_due(std::time::Instant::now()),
            "interval elapsed => due again"
        );
    }

    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    #[test]
    fn refresh_app_audio_processes_live_keeps_selection_and_clears_exited() {
        // A selection whose process still exists survives the live refresh;
        // an exited one is cleared so the pending-process hint shows instead
        // of silently adding a dead pid. The survival pid comes from the
        // enumerated list itself (macOS lists devices under the shared
        // fallback pid, not per-process pids; headless CI hosts may
        // enumerate nothing at all), `u32::MAX` is a safe stand-in for an
        // exited process on every platform.
        let enumerated = rivulet_audio::list_audio_processes();
        if let Some(alive) = enumerated.first().map(|p| p.pid) {
            let mut app = RivuletApp {
                audio_mixer_new_source_pid: Some(alive),
                ..RivuletApp::default()
            };
            app.refresh_app_audio_processes_live();
            assert_eq!(
                app.audio_mixer_new_source_pid,
                Some(alive),
                "a live selection must survive the refresh"
            );
            assert!(
                app.app_audio_processes.is_some(),
                "the refresh populated the cache"
            );
            assert!(app.app_audio_processes_last_refresh.is_some());
        }

        let mut app = RivuletApp {
            audio_mixer_new_source_pid: Some(u32::MAX),
            ..RivuletApp::default()
        };
        app.refresh_app_audio_processes_live();
        assert!(
            app.audio_mixer_new_source_pid.is_none(),
            "an exited selection must not survive the refresh"
        );
    }

    #[test]
    fn per_app_capture_gating_covers_all_backends() {
        // Phase 5 (issue #154): the per-app capture wiring must be compiled
        // on all three desktop platforms — Windows WASAPI loopback, Linux
        // PipeWire, and the macOS system-loopback fallback. Every cfg gate
        // on the picker/lifecycle/drain surface must name all three, so a
        // new backend cannot silently regress to Windows-only (the Phase-4
        // field-gating CI failure).
        let source = fs::read_to_string("src/app.rs").expect("GUI source must be readable");
        let triple = "any(target_os = \"windows\", target_os = \"linux\", target_os = \"macos\")";
        let wiring_markers = [
            "fn sync_app_audio_captures(&mut self)",
            "fn drain_app_audio_frames(&mut self)",
            "fn refresh_app_audio_processes_live(&mut self)",
            "fn app_audio_processes_refresh_due(&self",
            "app_audio_processes_last_refresh: Option<std::time::Instant>",
            "app_audio_processes_last_refresh: None,",
            "audio_mixer_new_source_pid: Option<u32>",
            "audio_mixer_new_source_pid: None,",
        ];
        for marker in wiring_markers {
            // The definition site is the first occurrence; the attribute
            // gating it is the nearest `#[cfg(` before it.
            let pos = source
                .find(marker)
                .unwrap_or_else(|| panic!("wiring marker {marker:?} must exist"));
            let from = source[..pos]
                .rfind("#[cfg(")
                .unwrap_or_else(|| panic!("wiring marker {marker:?} must carry a cfg gate"));
            let gate = &source[from..pos];
            assert!(
                gate.contains(triple),
                "per-app wiring marker {marker:?} must be gated on all three platforms, found gate: {gate}"
            );
        }
    }
    // ── stream-info editor (title/game, per-platform + all) ──────────

    #[test]
    fn chat_info_apply_rejects_empty_drafts_without_thread() {
        let mut app = RivuletApp::default();
        app.apply_chat_stream_info(None);
        assert_eq!(
            app.chat_info_error.as_deref(),
            Some("Enter a title or a game to apply."),
            "empty drafts must be rejected up front"
        );
        assert!(app.chat_info_rx.is_none(), "no thread may be spawned");
        assert!(!app.chat_info_busy);
    }

    #[test]
    fn chat_info_apply_spawns_thread_and_clears_error() {
        let mut app = RivuletApp::default();
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Kick,
            "somechannel".to_owned(),
        ));
        app.chat_info_title[rivulet_core::InfoPlatform::Kick.index()] = "New title".to_owned();
        app.chat_info_error = Some("stale".to_owned());
        app.apply_chat_stream_info(None);
        assert!(app.chat_info_busy, "apply must mark the editor busy");
        assert!(app.chat_info_rx.is_some(), "a result channel must exist");
        assert!(
            app.chat_info_error.is_none(),
            "a successful apply clears the error"
        );
        // Drain via the reconcile path (thread finishes quickly; retry a
        // few times to avoid flakiness).
        for _ in 0..100 {
            app.reconcile_chat();
            if !app.chat_info_busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.chat_info_busy, "reconcile must drain the outcome");
        assert_eq!(app.chat_info_outcomes.len(), 1, "one configured platform");
        assert_eq!(
            app.chat_info_outcomes[0].0,
            rivulet_core::InfoPlatform::Kick,
            "Kick outcome must be reported"
        );
        assert!(
            !app.chat_info_outcomes[0].1.is_ok(),
            "no real network in tests: the Kick call must fail honestly"
        );
    }

    #[test]
    fn chat_info_apply_only_targets_the_selected_platform() {
        let mut app = RivuletApp::default();
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Twitch,
            "chan".to_owned(),
        ));
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Kick,
            "chan".to_owned(),
        ));
        app.chat_info_game[rivulet_core::InfoPlatform::Twitch.index()] = "Just Chatting".to_owned();
        app.apply_chat_stream_info(Some(rivulet_core::InfoPlatform::Twitch));
        assert!(app.chat_info_busy);
        for _ in 0..100 {
            app.reconcile_chat();
            if !app.chat_info_busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            app.chat_info_outcomes.len(),
            1,
            "a per-platform apply must touch exactly that platform"
        );
        assert_eq!(
            app.chat_info_outcomes[0].0,
            rivulet_core::InfoPlatform::Twitch
        );
    }

    #[test]
    fn chat_info_apply_all_covers_every_configured_platform() {
        let mut app = RivuletApp::default();
        for (platform, channel) in [
            (rivulet_core::ChatPlatform::Twitch, "chan"),
            (rivulet_core::ChatPlatform::Kick, "chan"),
            (rivulet_core::ChatPlatform::YouTube, "abcVideo"),
        ] {
            app.chat_accounts
                .push(rivulet_core::ChatAccount::new(platform, channel.to_owned()));
        }
        for platform in rivulet_core::InfoPlatform::ALL {
            let idx = platform.index();
            app.chat_info_title[idx] = format!("{label} title", label = platform.label());
            app.chat_info_game[idx] = "Software".to_owned();
        }
        app.apply_chat_stream_info(None);
        for _ in 0..100 {
            app.reconcile_chat();
            if !app.chat_info_busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            app.chat_info_outcomes.len(),
            3,
            "apply-to-all must reach every configured platform"
        );
        let platforms: Vec<_> = app.chat_info_outcomes.iter().map(|(p, _)| *p).collect();
        assert_eq!(
            platforms,
            vec![
                rivulet_core::InfoPlatform::Twitch,
                rivulet_core::InfoPlatform::Kick,
                rivulet_core::InfoPlatform::YouTube,
            ]
        );
    }

    #[test]
    fn chat_info_busy_blocks_reapply_and_failed_thread_unblocks() {
        let mut app = RivuletApp::default();
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Kick,
            "chan".to_owned(),
        ));
        app.chat_info_title[rivulet_core::InfoPlatform::Kick.index()] = "x".to_owned();
        app.apply_chat_stream_info(None);
        assert!(app.chat_info_busy);
        app.apply_chat_stream_info(None);
        assert!(
            app.chat_info_rx.is_some() && app.chat_info_busy,
            "a second apply while busy must be a no-op"
        );
        // Simulate a crashed worker: the receiver is still registered but
        // its sender is gone (thread panicked without sending); the
        // reconcile must unblock the editor and surface it.
        let (tx, rx) = std::sync::mpsc::channel::<
            Vec<(rivulet_core::InfoPlatform, rivulet_core::InfoUpdateOutcome)>,
        >();
        drop(tx);
        app.chat_info_rx = Some(rx);
        app.chat_info_busy = true;
        app.reconcile_chat();
        assert!(!app.chat_info_busy, "a disconnected channel must unblock");
        assert_eq!(
            app.chat_info_error.as_deref(),
            Some("Update failed (worker thread died)"),
        );
    }

    #[test]
    fn chat_info_per_platform_drafts_are_independent() {
        let mut app = RivuletApp::default();
        for (platform, channel) in [
            (rivulet_core::ChatPlatform::Twitch, "chan"),
            (rivulet_core::ChatPlatform::Kick, "chan"),
        ] {
            app.chat_accounts
                .push(rivulet_core::ChatAccount::new(platform, channel.to_owned()));
        }
        // Only Twitch has a draft; Kick stays empty. An apply-to-all must
        // reach exactly the platform with content (#214 core behavior).
        app.chat_info_title[rivulet_core::InfoPlatform::Twitch.index()] = "Twitch only".to_owned();
        app.apply_chat_stream_info(None);
        for _ in 0..100 {
            app.reconcile_chat();
            if !app.chat_info_busy {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            app.chat_info_outcomes.len(),
            1,
            "empty per-platform drafts must be skipped, not failed"
        );
        assert_eq!(
            app.chat_info_outcomes[0].0,
            rivulet_core::InfoPlatform::Twitch
        );
    }

    #[test]
    fn chat_info_scope_tabs_switch_the_edited_drafts() {
        let mut app = RivuletApp::default();
        app.chat_accounts.push(rivulet_core::ChatAccount::new(
            rivulet_core::ChatPlatform::Kick,
            "chan".to_owned(),
        ));
        // Default scope: all platforms.
        assert_eq!(app.chat_info_scope, None);
        // Switching the scope does not copy values between platforms —
        // each platform's drafts stay independent.
        app.chat_info_scope = Some(rivulet_core::InfoPlatform::Kick);
        app.chat_info_title[rivulet_core::InfoPlatform::Kick.index()] = "Kick draft".to_owned();
        app.chat_info_scope = Some(rivulet_core::InfoPlatform::Twitch);
        app.chat_info_title[rivulet_core::InfoPlatform::Twitch.index()] = "Twitch draft".to_owned();
        assert_eq!(
            app.chat_info_title[rivulet_core::InfoPlatform::Kick.index()],
            "Kick draft",
            "drafts must not leak between platform scopes"
        );
        // Back to all: drafts remain per platform.
        app.chat_info_scope = None;
        assert_eq!(
            app.chat_info_title[rivulet_core::InfoPlatform::Kick.index()],
            "Kick draft"
        );
    }

    #[test]
    fn chat_info_fields_are_not_persisted() {
        let json = serde_json::json!({});
        assert!(json.get("chat_info_title").is_none());
        let app = RivuletApp::default();
        let serialized = serde_json::to_value(&app).expect("app must serialize");
        for key in [
            "chat_info_title",
            "chat_info_game",
            "chat_info_scope",
            "chat_info_busy",
            "chat_info_rx",
            "chat_info_outcomes",
            "chat_info_error",
        ] {
            assert!(
                serialized.get(key).is_none(),
                "{key} must be #[serde(skip)] (pure draft state)"
            );
        }
    }

    // ── browser-source backend wiring (issue #227) ───────────────────

    /// Build an app with a synthetic backend injected, so the tick drives
    /// real sync/input/poll logic without touching a native webview.
    fn app_with_synthetic_browser_backend() -> RivuletApp {
        RivuletApp {
            browser_backend: Some(Box::new(rivulet_core::SyntheticBrowserBackend::default())),
            ..RivuletApp::default()
        }
    }

    #[test]
    fn browser_tick_syncs_settings_polls_and_submits_a_frame() {
        let mut app = app_with_synthetic_browser_backend();
        let config = rivulet_core::BrowserSource::new("https://example.com", 640, 480)
            .expect("valid source");
        app.browser_source = config;

        app.tick_browser_source().expect("tick must not fail");

        // The synthetic backend paints a deterministic frame for the synced
        // viewport; the tick must have submitted it back into `browser_source`.
        let frame = app.browser_source.latest_frame().expect("a frame");
        assert_eq!((frame.width, frame.height), (640, 480));
        // Applied state now tracks the source, so a second tick is a no-op.
        assert_eq!(
            app.browser_applied.as_ref().expect("applied state"),
            &rivulet_core::BrowserAppliedState::from_source(&app.browser_source)
        );
        let sequence = frame.sequence;
        app.tick_browser_source().expect("tick must not fail");
        // Idle backend (repaint flag cleared) → no new frame, same sequence.
        assert_eq!(
            app.browser_source.latest_frame().map(|f| f.sequence),
            Some(sequence)
        );
    }

    #[test]
    fn browser_tick_forward_diffs_only_after_changes() {
        let mut app = app_with_synthetic_browser_backend();
        app.tick_browser_source().expect("initial sync");
        let first = app
            .browser_source
            .latest_frame()
            .expect("frame")
            .rgba
            .clone();

        // Navigating invalidates the frame cache and repaints with a new,
        // URL-derived colour (the synthetic backend's deterministic fill).
        app.browser_source
            .navigate("https://other.example/page")
            .expect("nav");
        app.tick_browser_source().expect("tick");
        let frame = app.browser_source.latest_frame().expect("frame");
        assert_ne!(frame.rgba, first, "navigation repaints new content");
        assert_eq!(frame.width, app.browser_source.width);
    }

    #[test]
    fn browser_tick_drains_queued_input_to_backend() {
        let mut app = app_with_synthetic_browser_backend();
        assert!(app
            .browser_source
            .enqueue_input(rivulet_core::BrowserInput::MouseMove { x: 12.0, y: 34.0 }));

        app.tick_browser_source().expect("tick");
        assert_eq!(
            app.browser_source.pending_input_count(),
            0,
            "queued input must be forwarded to the backend"
        );
    }

    #[test]
    fn browser_tick_skips_input_when_interaction_disabled() {
        let mut app = app_with_synthetic_browser_backend();
        app.browser_source.interaction_enabled = false;
        assert!(!app
            .browser_source
            .enqueue_input(rivulet_core::BrowserInput::MouseMove { x: 1.0, y: 1.0 }));
        assert_eq!(app.browser_source.pending_input_count(), 0);
    }

    #[test]
    fn browser_tick_without_backend_is_a_noop() {
        let mut app = RivuletApp {
            browser_backend_failed: true,
            ..RivuletApp::default()
        };
        app.tick_browser_source().expect("no backend, no error");
        assert!(app.browser_source.latest_frame().is_none());
    }

    #[test]
    fn browser_backend_fields_are_not_persisted() {
        let mut app = app_with_synthetic_browser_backend();
        app.tick_browser_source().expect("tick");
        let serialized = serde_json::to_value(&app).expect("app must serialize");
        for key in [
            "browser_backend",
            "browser_applied",
            "browser_preview_texture",
        ] {
            assert!(
                serialized.get(key).is_none(),
                "{key} must be #[serde(skip)] (transient backend state)"
            );
        }
    }
}
