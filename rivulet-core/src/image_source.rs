use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Supported image file extensions (case-insensitive).
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "bmp", "webp"];

/// An image source — either a single file or a folder used as a slideshow.
#[derive(Debug, Clone)]
pub struct ImageSource {
    /// Path to a single image file, or a directory for folder-mode.
    pub path: PathBuf,
    /// `true` when `path` points to a directory and we cycle through images.
    pub folder_mode: bool,
    /// Slideshow interval (only used when `folder_mode == true`).
    pub interval: Duration,
    /// Cached list of image files discovered inside the folder.
    /// Populated by [`ImageSource::refresh_folder`].
    files: Vec<PathBuf>,
    /// Index of the currently displayed image in `files`.
    current_index: usize,
    /// Timestamp of the last advance (for slideshow timing).
    last_advance: Instant,
}

impl ImageSource {
    /// Create a new single-image source.
    pub fn single(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            folder_mode: false,
            interval: Duration::from_secs(5),
            files: Vec::new(),
            current_index: 0,
            last_advance: Instant::now(),
        }
    }

    /// Create a new folder/slideshow source.
    pub fn folder(path: impl Into<PathBuf>, interval: Duration) -> Self {
        let mut src = Self {
            path: path.into(),
            folder_mode: true,
            interval,
            files: Vec::new(),
            current_index: 0,
            last_advance: Instant::now(),
        };
        src.refresh_folder();
        src
    }

    /// Re-scan the folder for image files. Returns the number of images found.
    pub fn refresh_folder(&mut self) -> usize {
        self.files.clear();
        if self.folder_mode && self.path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&self.path) {
                let mut paths: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| is_image_file(p))
                    .collect();
                paths.sort();
                self.files = paths;
            }
        }
        self.files.len()
    }

    /// Returns the path of the currently active image.
    ///
    /// - For single-file mode this is always `self.path`.
    /// - For folder mode this cycles through the discovered files based on
    ///   the elapsed time since the last advance.
    pub fn current_path(&mut self) -> Option<&Path> {
        if !self.folder_mode {
            return if self.path.is_file() {
                Some(self.path.as_path())
            } else {
                None
            };
        }

        // Slideshow: advance when the interval has elapsed.
        if !self.files.is_empty() && self.last_advance.elapsed() >= self.interval {
            self.current_index = (self.current_index + 1) % self.files.len();
            self.last_advance = Instant::now();
        }

        self.files.get(self.current_index).map(|p| p.as_path())
    }

    /// Returns the number of images in a folder-mode source.
    pub fn image_count(&self) -> usize {
        self.files.len()
    }

    /// Returns the list of discovered files (read-only).
    pub fn files(&self) -> &[PathBuf] {
        &self.files
    }
}

/// Check if a file path has a supported image extension.
pub fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Supported image extensions (for UI display).
pub fn supported_image_extensions() -> &'static [&'static str] {
    IMAGE_EXTENSIONS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn single_image_source_returns_path() {
        let tmp = TempDir::new().unwrap();
        let img = tmp.path().join("test.png");
        fs::write(&img, b"fake").unwrap();

        let mut src = ImageSource::single(&img);
        assert!(!src.folder_mode);
        assert_eq!(src.current_path(), Some(img.as_path()));
    }

    #[test]
    fn single_image_missing_file_returns_none() {
        let mut src = ImageSource::single("/nonexistent/image.png");
        assert!(!src.folder_mode);
        assert_eq!(src.current_path(), None);
    }

    #[test]
    fn folder_mode_discovers_images() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.png"), b"a").unwrap();
        fs::write(tmp.path().join("b.jpg"), b"b").unwrap();
        fs::write(tmp.path().join("c.txt"), b"c").unwrap(); // not an image
        fs::write(tmp.path().join("d.gif"), b"d").unwrap();

        let src = ImageSource::folder(tmp.path(), Duration::from_secs(5));
        assert!(src.folder_mode);
        assert_eq!(src.image_count(), 3); // a.png, b.jpg, d.gif
        assert_eq!(src.files()[0].file_name().unwrap(), "a.png");
    }

    #[test]
    fn folder_mode_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mut src = ImageSource::folder(tmp.path(), Duration::from_secs(1));
        assert_eq!(src.image_count(), 0);
        assert!(src.current_path().is_none());
    }

    #[test]
    fn folder_mode_nonexistent_dir() {
        let mut src = ImageSource::folder("/nonexistent/dir", Duration::from_secs(1));
        assert_eq!(src.image_count(), 0);
        assert!(src.current_path().is_none());
    }

    #[test]
    fn refresh_folder_updates_list() {
        let tmp = TempDir::new().unwrap();
        let mut src = ImageSource::folder(tmp.path(), Duration::from_secs(5));
        assert_eq!(src.image_count(), 0);

        fs::write(tmp.path().join("new.png"), b"x").unwrap();
        let count = src.refresh_folder();
        assert_eq!(count, 1);
        assert_eq!(src.image_count(), 1);
    }

    #[test]
    fn slideshow_advances_on_interval() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("1.png"), b"1").unwrap();
        fs::write(tmp.path().join("2.png"), b"2").unwrap();

        let mut src = ImageSource::folder(tmp.path(), Duration::from_millis(10));
        assert_eq!(src.image_count(), 2);

        let first = src.current_path().map(|p| p.to_path_buf());

        // Wait for interval
        std::thread::sleep(Duration::from_millis(20));

        let second = src.current_path().map(|p| p.to_path_buf());
        assert_ne!(first, second);
    }

    #[test]
    fn is_image_file_extension_check() {
        assert!(is_image_file(Path::new("photo.png")));
        assert!(is_image_file(Path::new("photo.PNG")));
        assert!(is_image_file(Path::new("photo.jpg")));
        assert!(is_image_file(Path::new("photo.jpeg")));
        assert!(is_image_file(Path::new("anim.gif")));
        assert!(is_image_file(Path::new("pic.webp")));
        assert!(!is_image_file(Path::new("doc.txt")));
        assert!(!is_image_file(Path::new("no-ext")));
    }

    #[test]
    fn supported_extensions_list() {
        let exts = supported_image_extensions();
        assert!(exts.contains(&"png"));
        assert!(exts.contains(&"jpg"));
        assert!(exts.contains(&"gif"));
    }

    #[test]
    fn default_interval_is_five_seconds() {
        let src = ImageSource::single("/tmp/x.png");
        assert_eq!(src.interval, Duration::from_secs(5));
    }
}
