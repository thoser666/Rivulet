use anyhow::{Context, Result};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};

use crate::{EncodableFrame, Encoder, RecordingSettings};

pub struct Recorder {
    settings: RecordingSettings,
    frame_sender: Option<Sender<EncodableFrame>>,
    encoder_thread: Option<JoinHandle<Result<()>>>,
    is_recording: Arc<AtomicBool>,
}

impl Recorder {
    pub fn new(settings: RecordingSettings) -> Result<Self> {
        Ok(Self {
            settings,
            frame_sender: None,
            encoder_thread: None,
            is_recording: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn start(&mut self) -> Result<()> {
        if self.is_recording.load(Ordering::SeqCst) {
            anyhow::bail!("Recording already in progress");
        }

        let (tx, rx) = bounded::<EncodableFrame>(100);
        self.frame_sender = Some(tx);

        let settings = self.settings.clone();
        let is_recording = Arc::clone(&self.is_recording);

        // Set the flag BEFORE spawning: the thread checks its run condition
        // immediately; if the flag is only set after spawning, the thread can
        // skip the loop on CI, drop the receiver, and send_frame fails with
        // "Disconnected".
        self.is_recording.store(true, Ordering::SeqCst);

        // Start the encoder thread
        let handle = thread::spawn(move || Self::encoder_thread_fn(rx, settings, is_recording));

        self.encoder_thread = Some(handle);

        println!("Recording started: {:?}", self.settings.output_path);
        Ok(())
    }

    pub fn send_frame(&self, frame: EncodableFrame) -> Result<()> {
        if let Some(sender) = &self.frame_sender {
            sender
                .send(frame)
                .context("Failed to send frame to encoder")?;
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        if !self.is_recording.load(Ordering::SeqCst) {
            return Ok(());
        }

        self.is_recording.store(false, Ordering::SeqCst);

        // Close the channel
        drop(self.frame_sender.take());

        // Wait for the encoder thread
        if let Some(handle) = self.encoder_thread.take() {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("Encoder thread panicked"))??;
        }

        println!("Recording stopped");
        Ok(())
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording.load(Ordering::SeqCst)
    }

    fn encoder_thread_fn(
        rx: Receiver<EncodableFrame>,
        settings: RecordingSettings,
        is_recording: Arc<AtomicBool>,
    ) -> Result<()> {
        let mut encoder = Encoder::new(settings)?;

        while is_recording.load(Ordering::SeqCst) || !rx.is_empty() {
            match rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(frame) => {
                    encoder.encode_frame(&frame)?;
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }

        encoder.finalize()?;
        Ok(())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EncodableFrame;
    use std::path::PathBuf;

    fn make_settings() -> RecordingSettings {
        RecordingSettings {
            output_path: PathBuf::from("out.mp4"),
            width: 1280,
            height: 720,
            fps: 30,
            bitrate: 2_000_000,
            codec: "h264".to_string(),
        }
    }

    fn make_frame() -> EncodableFrame {
        EncodableFrame {
            data: vec![0; 100],
            width: 1280,
            height: 720,
            stride: 1280 * 4,
            timestamp: std::time::Instant::now(),
        }
    }

    #[test]
    fn recorder_start_send_stop_lifecycle() {
        let mut recorder = Recorder::new(make_settings()).unwrap();
        assert!(!recorder.is_recording());

        recorder.start().unwrap();
        assert!(recorder.is_recording());

        for _ in 0..5 {
            recorder.send_frame(make_frame()).unwrap();
        }

        recorder.stop().unwrap();
        assert!(!recorder.is_recording());
    }

    #[test]
    fn recorder_rejects_double_start() {
        let mut recorder = Recorder::new(make_settings()).unwrap();
        recorder.start().unwrap();
        assert!(recorder.start().is_err());
        recorder.stop().unwrap();
    }

    #[test]
    fn stop_without_start_is_noop() {
        let mut recorder = Recorder::new(make_settings()).unwrap();
        assert!(recorder.stop().is_ok());
    }
}
