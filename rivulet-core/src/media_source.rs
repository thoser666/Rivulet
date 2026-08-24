use std::path::PathBuf;
use std::time::Duration;

/// Supported media file types for playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Video file (MP4, MKV, WebM, AVI, MOV).
    Video,
    /// Audio file (MP3, WAV, FLAC, AAC, OGG).
    Audio,
    /// Image file used as video source (PNG, JPEG, GIF, BMP, WebP).
    Image,
}

impl MediaType {
    pub fn label(&self) -> &'static str {
        match self {
            MediaType::Video => "Video",
            MediaType::Audio => "Audio",
            MediaType::Image => "Image",
        }
    }

    /// Guess the media type from a file extension.
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            // Video
            "mp4" | "mkv" | "webm" | "avi" | "mov" | "wmv" | "flv" | "ts" => Some(MediaType::Video),
            // Audio
            "mp3" | "wav" | "flac" | "aac" | "ogg" | "opus" | "wma" => Some(MediaType::Audio),
            // Image
            "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tiff" => Some(MediaType::Image),
            _ => None,
        }
    }
}

/// Playback mode for media sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackMode {
    /// Play once and stop.
    #[default]
    Once,
    /// Loop indefinitely.
    Loop,
    /// Play once, then pause on the last frame (video) or silence (audio).
    Freeze,
}

impl PlaybackMode {
    pub fn label(&self) -> &'static str {
        match self {
            PlaybackMode::Once => "Once",
            PlaybackMode::Loop => "Loop",
            PlaybackMode::Freeze => "Freeze",
        }
    }
}

/// A media source — plays a video, audio, or image file in a scene.
///
/// Supports playback modes (once, loop, freeze), speed control, volume,
/// and restart-on-scene-entry behavior.
#[derive(Debug, Clone)]
pub struct MediaSource {
    /// Path to the media file.
    pub path: PathBuf,
    /// Detected media type.
    pub media_type: MediaType,
    /// Playback mode.
    pub playback_mode: PlaybackMode,
    /// Playback speed multiplier (1.0 = normal, 0.5 = half speed, 2.0 = double).
    pub speed: f32,
    /// Volume for audio/video playback (0.0–1.0).
    pub volume: f32,
    /// Whether to restart playback when the scene becomes active.
    pub restart_on_scene_entry: bool,
    /// Whether the source is currently playing.
    pub playing: bool,
    /// Current playback position (approximate).
    pub position: Duration,
    /// Total duration of the media file (if known).
    pub duration: Option<Duration>,
    /// Mute audio output (video still plays silently).
    pub muted: bool,
}

impl MediaSource {
    /// Create a new media source from a file path.
    ///
    /// Returns `None` if the file extension is not recognized.
    pub fn new(path: impl Into<PathBuf>) -> Option<Self> {
        let path = path.into();
        let ext = path.extension()?.to_str()?;
        let media_type = MediaType::from_extension(ext)?;

        Some(Self {
            path,
            media_type,
            playback_mode: PlaybackMode::default(),
            speed: 1.0,
            volume: 1.0,
            restart_on_scene_entry: true,
            playing: false,
            position: Duration::ZERO,
            duration: None,
            muted: false,
        })
    }

    /// Returns true if the source has a valid file path.
    pub fn has_file(&self) -> bool {
        self.path.exists()
    }

    /// Returns the file name (without directory).
    pub fn file_name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
    }

    /// Returns the file extension.
    pub fn extension(&self) -> &str {
        self.path.extension().and_then(|e| e.to_str()).unwrap_or("")
    }

    /// Returns a summary string for UI display.
    pub fn summary(&self) -> String {
        let status = if self.playing { "playing" } else { "stopped" };
        let speed = if (self.speed - 1.0).abs() > 0.01 {
            format!(" {}×", self.speed)
        } else {
            String::new()
        };
        let vol = if self.muted {
            " (muted)".to_string()
        } else if (self.volume - 1.0).abs() > 0.01 {
            format!(" vol={:.0}%", self.volume * 100.0)
        } else {
            String::new()
        };
        format!(
            "[{}] {}{}{} {}",
            self.media_type.label(),
            self.file_name(),
            speed,
            vol,
            status,
        )
    }

    /// Returns the duration formatted as MM:SS or HH:MM:SS.
    pub fn format_duration(dur: Duration) -> String {
        let total_secs = dur.as_secs();
        let hours = total_secs / 3600;
        let mins = (total_secs % 3600) / 60;
        let secs = total_secs % 60;
        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, mins, secs)
        } else {
            format!("{:02}:{:02}", mins, secs)
        }
    }

    /// Returns the position formatted as MM:SS / MM:SS.
    pub fn position_display(&self) -> String {
        let pos = Self::format_duration(self.position);
        match self.duration {
            Some(dur) => format!("{}/{}", pos, Self::format_duration(dur)),
            None => pos,
        }
    }
}

impl Default for MediaSource {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            media_type: MediaType::Video,
            playback_mode: PlaybackMode::default(),
            speed: 1.0,
            volume: 1.0,
            restart_on_scene_entry: true,
            playing: false,
            position: Duration::ZERO,
            duration: None,
            muted: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MediaType ────────────────────────────────────────────────

    #[test]
    fn media_type_labels() {
        assert_eq!(MediaType::Video.label(), "Video");
        assert_eq!(MediaType::Audio.label(), "Audio");
        assert_eq!(MediaType::Image.label(), "Image");
    }

    #[test]
    fn media_type_from_extension_video() {
        assert_eq!(MediaType::from_extension("mp4"), Some(MediaType::Video));
        assert_eq!(MediaType::from_extension("mkv"), Some(MediaType::Video));
        assert_eq!(MediaType::from_extension("webm"), Some(MediaType::Video));
        assert_eq!(MediaType::from_extension("MP4"), Some(MediaType::Video));
    }

    #[test]
    fn media_type_from_extension_audio() {
        assert_eq!(MediaType::from_extension("mp3"), Some(MediaType::Audio));
        assert_eq!(MediaType::from_extension("wav"), Some(MediaType::Audio));
        assert_eq!(MediaType::from_extension("flac"), Some(MediaType::Audio));
    }

    #[test]
    fn media_type_from_extension_image() {
        assert_eq!(MediaType::from_extension("png"), Some(MediaType::Image));
        assert_eq!(MediaType::from_extension("jpg"), Some(MediaType::Image));
        assert_eq!(MediaType::from_extension("gif"), Some(MediaType::Image));
    }

    #[test]
    fn media_type_from_extension_unknown() {
        assert_eq!(MediaType::from_extension("xyz"), None);
        assert_eq!(MediaType::from_extension("txt"), None);
    }

    // ── PlaybackMode ─────────────────────────────────────────────

    #[test]
    fn playback_mode_labels() {
        assert_eq!(PlaybackMode::Once.label(), "Once");
        assert_eq!(PlaybackMode::Loop.label(), "Loop");
        assert_eq!(PlaybackMode::Freeze.label(), "Freeze");
    }

    #[test]
    fn playback_mode_default_is_once() {
        assert_eq!(PlaybackMode::default(), PlaybackMode::Once);
    }

    // ── MediaSource creation ─────────────────────────────────────

    #[test]
    fn media_source_new_mp4() {
        let ms = MediaSource::new("/tmp/video.mp4").unwrap();
        assert_eq!(ms.media_type, MediaType::Video);
        assert_eq!(ms.path, PathBuf::from("/tmp/video.mp4"));
        assert!(!ms.playing);
        assert_eq!(ms.speed, 1.0);
        assert_eq!(ms.volume, 1.0);
        assert!(ms.restart_on_scene_entry);
    }

    #[test]
    fn media_source_new_mp3() {
        let ms = MediaSource::new("/tmp/audio.mp3").unwrap();
        assert_eq!(ms.media_type, MediaType::Audio);
    }

    #[test]
    fn media_source_new_png() {
        let ms = MediaSource::new("/tmp/image.png").unwrap();
        assert_eq!(ms.media_type, MediaType::Image);
    }

    #[test]
    fn media_source_new_unknown_ext() {
        assert!(MediaSource::new("/tmp/file.xyz").is_none());
    }

    #[test]
    fn media_source_default() {
        let ms = MediaSource::default();
        assert!(ms.path.as_os_str().is_empty());
        assert_eq!(ms.media_type, MediaType::Video);
        assert_eq!(ms.playback_mode, PlaybackMode::Once);
        assert!(!ms.playing);
    }

    // ── File info ────────────────────────────────────────────────

    #[test]
    fn file_name_from_path() {
        let ms = MediaSource::new("/home/user/video.mp4").unwrap();
        assert_eq!(ms.file_name(), "video.mp4");
    }

    #[test]
    fn extension_from_path() {
        let ms = MediaSource::new("/home/user/video.mp4").unwrap();
        assert_eq!(ms.extension(), "mp4");
    }

    // ── Summary ──────────────────────────────────────────────────

    #[test]
    fn summary_stopped() {
        let ms = MediaSource::new("test.mp4").unwrap();
        let s = ms.summary();
        assert!(s.contains("[Video]"));
        assert!(s.contains("test.mp4"));
        assert!(s.contains("stopped"));
    }

    #[test]
    fn summary_playing() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.playing = true;
        let s = ms.summary();
        assert!(s.contains("playing"));
    }

    #[test]
    fn summary_with_speed() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.speed = 2.0;
        let s = ms.summary();
        assert!(s.contains("2×"));
    }

    #[test]
    fn summary_muted() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.muted = true;
        let s = ms.summary();
        assert!(s.contains("muted"));
    }

    #[test]
    fn summary_custom_volume() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.volume = 0.75;
        let s = ms.summary();
        assert!(s.contains("vol=75%"));
    }

    // ── Duration formatting ──────────────────────────────────────

    #[test]
    fn format_duration_zero() {
        assert_eq!(MediaSource::format_duration(Duration::ZERO), "00:00");
    }

    #[test]
    fn format_duration_minutes() {
        assert_eq!(
            MediaSource::format_duration(Duration::from_secs(125)),
            "02:05"
        );
    }

    #[test]
    fn format_duration_hours() {
        assert_eq!(
            MediaSource::format_duration(Duration::from_secs(3661)),
            "01:01:01"
        );
    }

    #[test]
    fn position_display_no_duration() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.position = Duration::from_secs(90);
        assert_eq!(ms.position_display(), "01:30");
    }

    #[test]
    fn position_display_with_duration() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.position = Duration::from_secs(90);
        ms.duration = Some(Duration::from_secs(300));
        assert_eq!(ms.position_display(), "01:30/05:00");
    }

    // ── Clone, Debug ─────────────────────────────────────────────

    #[test]
    fn media_source_clone() {
        let mut ms = MediaSource::new("test.mp4").unwrap();
        ms.speed = 1.5;
        ms.playing = true;
        let ms2 = ms.clone();
        assert_eq!(ms2.speed, 1.5);
        assert!(ms2.playing);
    }

    #[test]
    fn media_source_debug() {
        let ms = MediaSource::new("test.mp4").unwrap();
        let dbg = format!("{ms:?}");
        assert!(dbg.contains("MediaSource"));
        assert!(dbg.contains("test.mp4"));
    }
}
