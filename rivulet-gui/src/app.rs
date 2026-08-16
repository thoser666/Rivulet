#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(unused_imports, dead_code, unused_variables)]

use eframe::egui;
use rivulet_core::{Locale, RivuletEngine};

// --- Linux-specific imports ---
#[cfg(target_os = "linux")]
use {
    ashpd::desktop::screencast::{Screencast, Stream},
    ashpd::desktop::Session,
    once_cell::sync::Lazy,
    pipewire::spa::utils::Fd,
    rivulet_audio::{AudioCapture, AudioConfig},
    rivulet_core::{AudioFrame, AudioTrack},
    std::sync::mpsc as std_mpsc,
    std::sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc,
    },
    std::thread,
    std::time::Instant,
    tokio::runtime::Runtime,
};

// --- Windows imports (for windows-capture v1.5.0) ---
#[cfg(target_os = "windows")]
use {
    rfd,
    std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
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
#[derive(Debug)]
struct RawFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
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
            println!("Stop signal received, ending recording.");
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
            println!("GUI channel closed, ending recording.");
            capture_control.stop();
        }

        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        println!("Recording session was ended by the system.");
        self.stop_signal.store(true, Ordering::SeqCst);
        Ok(())
    }
}

// --- Linux-specific types ---
#[cfg(target_os = "linux")]
static TOKIO_RT: Lazy<Runtime> = Lazy::new(|| tokio::runtime::Runtime::new().unwrap());

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
    audio_peak: Arc<AtomicU32>,
    #[cfg(target_os = "linux")]
    capture_system: bool,
    #[cfg(target_os = "linux")]
    capture_mic: bool,
    #[cfg(target_os = "linux")]
    separate_tracks: bool,
    #[cfg(target_os = "linux")]
    system_volume: f32,
    #[cfg(target_os = "linux")]
    mic_volume: f32,

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
    stop_signal: Option<Arc<AtomicBool>>,
    #[cfg(target_os = "linux")]
    #[serde(skip)]
    record_started: Instant,
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
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    stop_signal: Option<Arc<AtomicBool>>,
    #[cfg(target_os = "windows")]
    #[serde(skip)]
    last_error: Option<String>,
}

impl Default for RivuletApp {
    fn default() -> Self {
        #[cfg(target_os = "linux")]
        let (sender, receiver) = std_mpsc::channel();

        Self {
            engine: Default::default(),
            locale: Locale::default(),

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
            audio_peak: Arc::new(AtomicU32::new(0)),
            #[cfg(target_os = "linux")]
            capture_system: true,
            #[cfg(target_os = "linux")]
            capture_mic: true,
            #[cfg(target_os = "linux")]
            separate_tracks: false,
            #[cfg(target_os = "linux")]
            system_volume: 0.8,
            #[cfg(target_os = "linux")]
            mic_volume: 1.0,

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
            stop_signal: None,
            #[cfg(target_os = "linux")]
            record_started: Instant::now(),
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
            frame_receiver: None,
            #[cfg(target_os = "windows")]
            stop_signal: None,
            #[cfg(target_os = "windows")]
            last_error: None,
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

    fn start_windows_recording(&mut self) {
        let file_path = rfd::FileDialog::new()
            .add_filter("MP4 Video", &["mp4", "mov"])
            .set_file_name(&format!(
                "rivulet-recording-{}.mp4",
                chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S")
            ))
            .save_file();

        let Some(path) = file_path else {
            println!("File selection cancelled.");
            return;
        };

        // Determine the capture source (monitor or window) and start accordingly
        if let Some(idx) = self.selected_monitor_idx {
            let Some(monitor) = self.monitors.get(idx).cloned() else {
                self.last_error = Some(self.tr("invalid_source").to_string());
                return;
            };

            self.engine.start_local_recording(path.clone());

            let (sender, receiver) = mpsc::channel();
            self.frame_receiver = Some(receiver);

            let stop_signal = Arc::new(AtomicBool::new(false));
            self.stop_signal = Some(stop_signal.clone());

            let flags = (sender, stop_signal);

            self.is_windows_recording = true;
            self.last_error = None;

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
                println!("Starting capture thread (Monitor)...");
                if let Err(e) = CaptureHandler::start(settings) {
                    if !e.to_string().contains("user stopped")
                        && !e.to_string().contains("GUI channel closed")
                    {
                        eprintln!("Error in capture thread: {}", e);
                    }
                }
                println!("Capture thread stopped.");
            });
        } else if let Some(idx) = self.selected_window_idx {
            let Some(window) = self.windows.get(idx).cloned() else {
                self.last_error = Some(self.tr("invalid_source").to_string());
                return;
            };

            self.engine.start_local_recording(path.clone());

            let (sender, receiver) = mpsc::channel();
            self.frame_receiver = Some(receiver);

            let stop_signal = Arc::new(AtomicBool::new(false));
            self.stop_signal = Some(stop_signal.clone());

            let flags = (sender, stop_signal);

            self.is_windows_recording = true;
            self.last_error = None;

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
                println!("Starting capture thread (Window)...");
                if let Err(e) = CaptureHandler::start(settings) {
                    if !e.to_string().contains("user stopped")
                        && !e.to_string().contains("GUI channel closed")
                    {
                        eprintln!("Error in capture thread: {}", e);
                    }
                }
                println!("Capture thread stopped.");
            });
        } else {
            self.last_error = Some(self.tr("no_source_selected").to_string());
            return;
        }
    }

    fn stop_windows_recording(&mut self) {
        println!("Sending stop signal.");
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        self.is_windows_recording = false;
        self.engine.stop_recording();
        self.frame_receiver = None;
        self.stop_signal = None;
    }
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
            separate_tracks: self.separate_tracks,
            ..Default::default()
        };

        let mut audio = match AudioCapture::new(config) {
            Ok(audio) => audio,
            Err(e) => {
                self.audio_status =
                    Some(self.tr_fmt("audio_capture_unavailable", &[e.to_string()]));
                return;
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
                }
                Err(e) => {
                    self.audio_status = Some(self.tr_fmt("audio_start_failed", &[e.to_string()]));
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
                }
                Err(e) => {
                    self.audio_status = Some(self.tr_fmt("audio_start_failed", &[e.to_string()]));
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
        self.audio_peak.store(0.0f32.to_bits(), Ordering::SeqCst);
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

    fn start_linux_recording(&mut self) {
        if self.is_recording {
            return;
        }

        // Capture source: window takes precedence over monitor.
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

        let Some(path) = rfd::FileDialog::new()
            .add_filter("MP4 Video", &["mp4"])
            .set_file_name(format!(
                "rivulet-recording-{}.mp4",
                chrono::Utc::now().format("%Y-%m-%d_%H-%M-%S")
            ))
            .save_file()
        else {
            println!("File selection cancelled.");
            return;
        };

        if (self.capture_system || self.capture_mic) && !self.audio_preview {
            self.start_audio_capture();
        }
        self.engine.set_audio_enabled(self.audio_preview);
        self.engine
            .set_separate_audio_tracks(self.separate_tracks && self.audio_preview);
        if self.engine.separate_audio_tracks() {
            self.engine
                .set_audio_track_enabled(rivulet_core::AudioTrack::System, self.capture_system);
            self.engine
                .set_audio_track_enabled(rivulet_core::AudioTrack::Microphone, self.capture_mic);
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

        let fps = 30u64;
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
                        eprintln!("Capture error: {e}");
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
        self.record_status = None;
        println!("Linux recording started: {source_desc}");
    }

    fn stop_linux_recording(&mut self) {
        if !self.is_recording {
            return;
        }
        if let Some(signal) = &self.stop_signal {
            signal.store(true, Ordering::SeqCst);
        }
        self.engine.stop_recording();
        self.raw_rx = None;
        self.stop_signal = None;
        self.stop_audio_capture();
        self.is_recording = false;
        self.record_status = Some(self.tr("recording_saved").to_string());
    }

    fn drain_linux_frames(&mut self) {
        if let Some(rx) = &self.raw_rx {
            while let Ok(raw) = rx.try_recv() {
                self.engine
                    .process_raw_frame(&raw.data, raw.width, raw.height);
            }
        }
        if self.separate_tracks && self.audio_preview {
            if self.is_recording {
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
            if self.is_recording {
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
    /// Translate a UI string into the currently selected locale.
    fn tr<'a>(&self, key: &'a str) -> &'a str {
        self.locale.tr(key)
    }

    /// Translate a UI string and substitute positional placeholders.
    fn tr_fmt(&self, key: &str, args: &[String]) -> String {
        self.locale.tr_fmt(key, args)
    }

    pub fn new(cc: &eframe::CreationContext<'_>, engine: RivuletEngine) -> Self {
        #[allow(unused_mut)]
        let mut app = Self {
            engine,
            ..Default::default()
        };
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

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "windows")]
        {
            if self.is_windows_recording {
                if self.frame_receiver.is_some() {
                    if self.frame_receiver.as_ref().unwrap().try_recv().is_err() {
                        if let Some(signal) = &self.stop_signal {
                            if !signal.load(Ordering::SeqCst) {
                                println!("Recording ended unexpectedly.");
                                self.stop_windows_recording();
                            }
                        }
                    }
                }
            }
            if let Some(receiver) = &self.frame_receiver {
                while let Ok(raw_frame) = receiver.try_recv() {
                    self.engine.process_raw_frame(
                        &raw_frame.data,
                        raw_frame.width,
                        raw_frame.height,
                    );
                }
            }
        }

        egui::Panel::top("top_panel").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button(self.tr("language"), |ui| {
                    for locale in Locale::all() {
                        if ui
                            .selectable_label(self.locale == *locale, locale.name())
                            .clicked()
                        {
                            self.locale = *locale;
                        }
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading("Welcome to Rivulet");
            ui.separator();

            #[cfg(target_os = "windows")]
            {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(self.tr("windows_screen_recording")).strong());
                if self.is_windows_recording {
                    if ui.button("⏹ Stop Recording").clicked() {
                        self.stop_windows_recording();
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.label(self.tr("source"));
                        egui::ComboBox::from_id_salt("monitor_select")
                            .selected_text(
                                self.selected_monitor_idx
                                    .and_then(|idx| self.monitors.get(idx))
                                    .map(|m| {
                                        m.name().unwrap_or_else(|_| {
                                            self.tr("unknown_monitor").to_string()
                                        })
                                    })
                                    .unwrap_or_else(|| self.tr("select_monitor").to_string()),
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
                                    .unwrap_or_else(|| self.tr("select_window").to_string()),
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
                                    }
                                }
                            });
                        if ui
                            .button("🔄")
                            .on_hover_text(self.tr("refresh_sources"))
                            .clicked()
                        {
                            self.refresh_capture_sources();
                        }
                    });
                    let source_selected =
                        self.selected_monitor_idx.is_some() || self.selected_window_idx.is_some();
                    if ui
                        .add_enabled(
                            source_selected,
                            egui::Button::new(format!("⏺ {}", self.tr("start_recording"))),
                        )
                        .clicked()
                    {
                        self.start_windows_recording();
                    };
                }
                if let Some(err) = &self.last_error {
                    ui.colored_label(egui::Color32::RED, err);
                }
            }

            #[cfg(target_os = "linux")]
            {
                self.drain_linux_frames();

                ui.add_space(10.0);
                ui.label(egui::RichText::new(self.tr("screen_recording")).strong());

                if self.is_recording {
                    ui.horizontal(|ui| {
                        let elapsed = self.record_started.elapsed().as_secs();
                        ui.label(
                            egui::RichText::new(
                                self.tr_fmt("recording_in_progress", &[elapsed.to_string()]),
                            )
                            .color(egui::Color32::LIGHT_RED),
                        );
                        if ui
                            .button(format!("⏹ {}", self.tr("stop_recording")))
                            .clicked()
                        {
                            self.stop_linux_recording();
                        }
                    });
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
                                    .unwrap_or_else(|| self.tr("select_monitor").to_string()),
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
                                    .unwrap_or_else(|| self.tr("select_window").to_string()),
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
                                    }
                                }
                            });
                        if ui
                            .button("🔄")
                            .on_hover_text(self.tr("refresh_sources"))
                            .clicked()
                        {
                            self.refresh_linux_sources();
                        }
                    });
                    let source_selected =
                        self.selected_monitor_idx.is_some() || self.selected_window_idx.is_some();
                    if ui
                        .add_enabled(
                            source_selected,
                            egui::Button::new(format!("⏺ {}", self.tr("start_recording"))),
                        )
                        .clicked()
                    {
                        self.start_linux_recording();
                    }
                }
                if let Some(status) = &self.record_status {
                    ui.colored_label(egui::Color32::LIGHT_BLUE, status);
                }

                ui.separator();
                ui.label(egui::RichText::new(self.tr("audio_mixer")).strong());

                ui.horizontal(|ui| {
                    if self.audio_preview {
                        if ui.button(format!("⏹ {}", self.tr("stop_audio"))).clicked() {
                            self.stop_audio_capture();
                        }
                        ui.label(
                            egui::RichText::new(format!("● {}", self.tr("running")))
                                .color(egui::Color32::LIGHT_GREEN),
                        );
                    } else if ui.button(format!("▶ {}", self.tr("start_audio"))).clicked() {
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

                let mut sys_vol = self.system_volume;
                let mut mic_vol = self.mic_volume;
                if ui
                    .add(egui::Slider::new(&mut sys_vol, 0.0..=1.0).text(self.tr("system_audio")))
                    .changed()
                {
                    self.system_volume = sys_vol;
                    if let Some(audio) = &self.audio {
                        audio.set_system_volume(sys_vol);
                    }
                }
                if ui
                    .add(egui::Slider::new(&mut mic_vol, 0.0..=1.0).text(self.tr("microphone")))
                    .changed()
                {
                    self.mic_volume = mic_vol;
                    if let Some(audio) = &self.audio {
                        audio.set_mic_volume(mic_vol);
                    }
                }

                let peak = f32::from_bits(self.audio_peak.load(Ordering::SeqCst)).clamp(0.0, 1.0);
                let db = if peak > 0.0 {
                    20.0 * peak.log10()
                } else {
                    -96.0
                };
                ui.add(
                    egui::ProgressBar::new(peak)
                        .desired_width(f32::INFINITY)
                        .text(self.tr_fmt("peak", &[format!("{db:.1}")])),
                );

                if let Some(status) = &self.audio_status {
                    ui.colored_label(egui::Color32::RED, status);
                }
            }

            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            {
                ui.add_space(20.0);
                ui.label(egui::RichText::new("ℹ️ Screen Recording Feature").strong());
                ui.label("This feature is currently only available on Linux and Windows.");
            }

            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label("powered by ");
                    ui.hyperlink_to("egui", "https://github.com/emilk/egui");
                    ui.label(" and ");
                    ui.hyperlink_to(
                        "eframe",
                        "https://github.com/emilk/egui/tree/master/crates/eframe",
                    );
                    ui.label(".");
                });
            });
        });
    }
}
