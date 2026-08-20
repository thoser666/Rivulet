use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Describes a game-like window that can be captured.
#[derive(Debug, Clone)]
pub struct GameWindow {
    /// Platform window handle (HWND on Windows, Window ID on X11).
    pub id: u64,
    /// Window title (e.g. game name).
    pub title: String,
    /// Window width in pixels.
    pub width: u32,
    /// Window height in pixels.
    pub height: u32,
}

/// Configuration for game capture.
pub struct GameCaptureConfig {
    /// Target FPS (0 = match game FPS, fallback 60).
    pub fps: u32,
    /// Whether to capture the mouse cursor.
    pub capture_cursor: bool,
}

impl Default for GameCaptureConfig {
    fn default() -> Self {
        Self {
            fps: 0,
            capture_cursor: true,
        }
    }
}

/// A raw frame produced by game capture.
#[derive(Debug, Clone)]
pub struct GameCaptureFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// List windows that are likely game windows.
///
/// Heuristic: windows with a non-empty title that are reasonably large
/// (>640x480) and not the desktop/taskbar.
pub fn list_game_windows() -> Vec<GameWindow> {
    #[cfg(target_os = "linux")]
    {
        list_game_windows_linux()
    }
    #[cfg(target_os = "windows")]
    {
        list_game_windows_windows()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
fn list_game_windows_linux() -> Vec<GameWindow> {
    use std::process::Command;

    let output = Command::new("xdotool")
        .args(["search", "--name", ".*"])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut windows = Vec::new();

    for line in stdout.lines() {
        let id: u64 = match line.trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Get window title
        let title_output = Command::new("xdotool")
            .args(["getwindowname", &id.to_string()])
            .output();
        let title = match title_output {
            Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
            Err(_) => continue,
        };
        if title.is_empty() {
            continue;
        }
        // Get geometry
        let geom_output = Command::new("xdotool")
            .args(["getwindowgeometry", "--shell", &id.to_string()])
            .output();
        if let Ok(o) = geom_output {
            let geom = String::from_utf8_lossy(&o.stdout);
            let mut w = 0u32;
            let mut h = 0u32;
            for line in geom.lines() {
                if let Some(val) = line.strip_prefix("WIDTH=") {
                    w = val.parse().unwrap_or(0);
                }
                if let Some(val) = line.strip_prefix("HEIGHT=") {
                    h = val.parse().unwrap_or(0);
                }
            }
            if w > 640 && h > 480 {
                windows.push(GameWindow {
                    id,
                    title,
                    width: w,
                    height: h,
                });
            }
        }
    }
    windows
}

#[cfg(target_os = "windows")]
fn list_game_windows_windows() -> Vec<GameWindow> {
    // On Windows, list visible windows with reasonable dimensions
    // using the windows-capture crate's Window::all() is not available
    // without the GUI crate. Return empty for now; the GUI will provide
    // its own window list from the windows-capture crate.
    Vec::new()
}

/// Spawn a capture thread that captures frames from a game window.
///
/// On Linux, uses xcap::Window::capture_image() in a loop.
/// On Windows, the GUI uses windows-capture directly; this function
/// provides the thread infrastructure for the Linux path.
#[allow(unused_variables)]
pub fn start_game_capture(
    window: &GameWindow,
    config: &GameCaptureConfig,
) -> (mpsc::Receiver<GameCaptureFrame>, GameCaptureHandle) {
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(Mutex::new(false));
    let stop_clone = stop.clone();

    let window_id = window.id;
    let fps = if config.fps > 0 { config.fps } else { 60 };

    let handle = GameCaptureHandle { stop };

    thread::spawn(move || {
        #[cfg(target_os = "linux")]
        {
            use xcap::Window;
            let frame_duration = Duration::from_millis(1000 / fps as u64);
            loop {
                if *stop_clone.lock().unwrap() {
                    break;
                }
                if let Ok(windows) = Window::all() {
                    if let Some(w) = windows
                        .iter()
                        .find(|w| w.id().map(|id| id as u64).unwrap_or(0) == window_id)
                    {
                        if let Ok(image) = w.capture_image() {
                            // xcap 0.9 returns an RGBA8 ImageBuffer directly.
                            let (width, height) = (image.width(), image.height());
                            let raw = image.into_raw();
                            let _ = tx.send(GameCaptureFrame {
                                data: raw,
                                width,
                                height,
                            });
                        }
                    }
                }
                thread::sleep(frame_duration);
            }
        }
        #[cfg(target_os = "windows")]
        {
            // On Windows, game capture is handled by the windows-capture crate
            // in the GUI layer. This thread just waits until stopped.
            loop {
                if *stop_clone.lock().unwrap() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    });

    (rx, handle)
}

/// Handle to a running game capture session. Dropping this stops the capture.
pub struct GameCaptureHandle {
    stop: Arc<Mutex<bool>>,
}

impl Drop for GameCaptureHandle {
    fn drop(&mut self) {
        *self.stop.lock().unwrap() = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_capture_config_default() {
        let cfg = GameCaptureConfig::default();
        assert_eq!(cfg.fps, 0);
        assert!(cfg.capture_cursor);
    }

    #[test]
    fn list_game_windows_does_not_panic() {
        let _windows = list_game_windows();
    }

    /// The Linux CI job starts Xvfb and an xmessage window before running this
    /// opt-in test. Keeping it opt-in lets developers run the normal test
    /// suite without requiring an X11 session or xdotool locally.
    #[cfg(target_os = "linux")]
    #[test]
    fn list_game_windows_finds_the_ci_window() {
        if std::env::var_os("RIVULET_TEST_XDOTOOL").is_none() {
            return;
        }

        let windows = list_game_windows();
        assert!(
            !windows.is_empty(),
            "list_game_windows() returned no windows; verify DISPLAY, Xvfb and xdotool"
        );
        assert!(
            windows
                .iter()
                .any(|window| window.title.contains("RivuletGameCaptureTest")),
            "the CI test window was not enumerated: {windows:?}"
        );
    }
}
