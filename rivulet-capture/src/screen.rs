use crate::{CaptureSource, CapturedFrame};
use anyhow::{Context, Result};
use xcap::Monitor;

pub struct XCapScreenCapture {
    monitor: Monitor,
    width: u32,
    height: u32,
    capturing: bool,
}

impl XCapScreenCapture {
    pub fn new(display_index: u32) -> Result<Self> {
        tracing::info!(
            "Initializing xcap screen capture for display {}",
            display_index
        );

        let monitors = Monitor::all().context("Failed to get monitors")?;

        let monitor = monitors
            .get(display_index as usize)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Monitor {} not found", display_index))?;

        let width = monitor.width().context("Failed to get monitor width")?;
        let height = monitor.height().context("Failed to get monitor height")?;

        tracing::info!(
            "Monitor: {}x{} - {}",
            width,
            height,
            monitor.name().unwrap_or_default()
        );

        Ok(Self {
            monitor,
            width,
            height,
            capturing: false,
        })
    }

    /// List all available monitors
    pub fn list_monitors() -> Result<Vec<MonitorInfo>> {
        let monitors = Monitor::all().context("Failed to get monitors")?;

        Ok(monitors
            .iter()
            .enumerate()
            .map(|(idx, m)| MonitorInfo {
                index: idx as u32,
                name: m.name().unwrap_or_default(),
                width: m.width().unwrap_or(0),
                height: m.height().unwrap_or(0),
                x: m.x().unwrap_or(0),
                y: m.y().unwrap_or(0),
                is_primary: m.is_primary().unwrap_or(false),
            })
            .collect())
    }
}

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

impl CaptureSource for XCapScreenCapture {
    fn start(&mut self) -> Result<()> {
        tracing::info!("Starting screen capture");
        self.capturing = true;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        tracing::info!("Stopping screen capture");
        self.capturing = false;
        Ok(())
    }

    fn capture_frame(&mut self) -> Result<Option<CapturedFrame>> {
        if !self.capturing {
            return Ok(None);
        }

        // Capture screenshot
        let image = self
            .monitor
            .capture_image()
            .context("Failed to capture screen")?;

        let width = image.width();
        let height = image.height();
        let rgba_data = image.into_raw();
        let stride = width * 4;

        let frame = CapturedFrame::new(rgba_data, width, height, stride);

        Ok(Some(frame))
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_capturing(&self) -> bool {
        self.capturing
    }
}
