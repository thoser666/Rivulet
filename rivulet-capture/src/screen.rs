use crate::{CaptureSource, CapturedFrame};
use anyhow::{Context, Result};
use rivulet_core::CaptureRegion;
use xcap::Monitor;

pub struct XCapScreenCapture {
    monitor: Monitor,
    width: u32,
    height: u32,
    /// Optional rectangular sub-region of the monitor to capture, in
    /// monitor-local pixels. `None` captures the full monitor.
    region: Option<CaptureRegion>,
    capturing: bool,
}

impl XCapScreenCapture {
    /// Create a capture for the monitor at `display_index`, capturing the
    /// full monitor.
    pub fn new(display_index: u32) -> Result<Self> {
        Self::new_with_region(display_index, None)
    }

    /// Create a capture for the monitor at `display_index`, restricted to the
    /// given rectangular region (monitor-local pixels). The region is clamped
    /// to the monitor bounds and even-aligned at capture time, so a stale
    /// region can never panic or read out of bounds.
    pub fn new_with_region(display_index: u32, region: Option<CaptureRegion>) -> Result<Self> {
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
        if let Some(region) = region {
            tracing::info!(
                "Capture region: {}x{} at ({}, {})",
                region.width,
                region.height,
                region.x,
                region.y
            );
        }

        Ok(Self {
            monitor,
            width,
            height,
            region,
            capturing: false,
        })
    }

    /// Replace the capture region. `None` captures the full monitor again.
    pub fn set_region(&mut self, region: Option<CaptureRegion>) {
        self.region = region;
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

        let frame = match self.region {
            Some(region) => match region.crop_rgba(&rgba_data, width, height, stride) {
                // The region is clamped and even-aligned inside crop_rgba, so
                // a valid region always yields a crop; only an empty region
                // (e.g. after clamping) falls back to the full frame.
                Some((data, cropped_width, cropped_height)) => {
                    let cropped_stride = cropped_width * 4;
                    CapturedFrame::new(data, cropped_width, cropped_height, cropped_stride)
                }
                None => CapturedFrame::new(rgba_data, width, height, stride),
            },
            None => CapturedFrame::new(rgba_data, width, height, stride),
        };

        Ok(Some(frame))
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_capturing(&self) -> bool {
        self.capturing
    }
}
