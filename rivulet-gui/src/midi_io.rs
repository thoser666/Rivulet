//! MIDI device input bridge (M5: map controllers like the Korg NanoKontrol
//! to scene switches, volume faders, and filter toggles).
//!
//! This module owns the only hardware-dependent part of MIDI support: it
//! enumerates input ports through `midir` and, when connected, runs the
//! library's input thread, which parses raw bytes with the hardware-free
//! `rivulet_core::parse_midi` and forwards [`MidiMessage`]s over an mpsc
//! channel. The GUI polls [`MidiListener::try_recv`] every frame and applies
//! the mapped actions — exactly like the global-hotkey bridge.

use std::sync::mpsc;

use rivulet_core::{parse_midi, MidiMessage};

/// Enumerate the names of all MIDI input ports (for the Settings dropdown).
pub fn list_devices() -> Vec<String> {
    let Ok(midi_in) = midir::MidiInput::new("rivulet-midi") else {
        return Vec::new();
    };
    midi_in
        .ports()
        .iter()
        .filter_map(|port| midi_in.port_name(port).ok())
        .collect()
}

/// A live MIDI input connection. Dropping it stops the input thread and
/// disconnects from the device.
pub struct MidiListener {
    rx: mpsc::Receiver<MidiMessage>,
    // The connection owns the midir input thread; dropping it closes the
    // port. Kept only for its Drop side effect.
    _conn: midir::MidiInputConnection<()>,
}

impl MidiListener {
    /// Connect to the `device_index`-th enumerated input port.
    pub fn start(device_index: usize) -> Result<Self, String> {
        let midi_in = midir::MidiInput::new("rivulet-midi").map_err(|e| e.to_string())?;
        let ports = midi_in.ports();
        let port = ports
            .get(device_index)
            .ok_or_else(|| format!("no MIDI input at index {device_index}"))?;
        let port_name = midi_in.port_name(port).map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel::<MidiMessage>();
        let conn = midi_in
            .connect(
                port,
                "rivulet-midi",
                move |_stamp, bytes, _data| {
                    if let Some(msg) = parse_midi(bytes) {
                        let _ = tx.send(msg);
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        tracing::info!(port = %port_name, "MIDI listener connected");
        Ok(Self { rx, _conn: conn })
    }

    /// Drain a pending message, if any (called once per UI frame).
    pub fn try_recv(&mut self) -> Option<MidiMessage> {
        self.rx.try_recv().ok()
    }
}
