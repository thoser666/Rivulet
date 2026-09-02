//! MIDI controller mapping (Korg NanoKontrol & friends).
//!
//! This module is platform-independent and hardware-free: it parses raw MIDI
//! bytes into [`MidiMessage`]s, holds the user's [`MidiMapping`] (channel +
//! kind + note/CC number → action), and dispatches incoming messages to the
//! actions they are bound to. The device I/O itself lives in the GUI (a thin
//! `midir` listener thread) so this logic stays fully unit-testable in CI.
//!
//! Supported message kinds: Note On, Note Off (including Note On with velocity
//! 0, which the MIDI spec treats as Note Off), and Control Change. The parser
//! handles complete 3-byte messages; running status is not supported and such
//! frames are ignored (documented limitation).

use serde::{Deserialize, Serialize};

/// Which MIDI message kind a binding listens for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiKind {
    /// Note On (status 0x90..0x9F). Velocity 0 is normalized to Note Off.
    NoteOn,
    /// Note Off (status 0x80..0x8F, or Note On with velocity 0).
    NoteOff,
    /// Control Change (status 0xB0..0xBF).
    ControlChange,
}

impl Default for MidiKind {
    /// Control Change is the kind most controllers use for faders/knobs.
    fn default() -> Self {
        MidiKind::ControlChange
    }
}

impl MidiKind {
    /// The short label used in the mapping table and for serialization tests.
    pub fn label(self) -> &'static str {
        match self {
            MidiKind::NoteOn => "NoteOn",
            MidiKind::NoteOff => "NoteOff",
            MidiKind::ControlChange => "CC",
        }
    }
}

/// A parsed, normalized MIDI message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiMessage {
    /// MIDI channel 0..15 (as sent; the GUI shows 1-based numbers).
    pub channel: u8,
    pub kind: MidiKind,
    /// Note number or CC number (data byte 1).
    pub number: u8,
    /// Velocity or CC value (data byte 2).
    pub value: u8,
}

/// Parse a complete 3-byte MIDI message. Returns `None` for anything that is
/// not a Note On/Off/CC frame we understand (SysEx, program change, running
/// status, malformed frames).
pub fn parse_midi(bytes: &[u8]) -> Option<MidiMessage> {
    if bytes.len() < 3 {
        return None;
    }
    let status = bytes[0];
    let (kind, channel) = match status & 0xF0 {
        0x90 => (MidiKind::NoteOn, status & 0x0F),
        0x80 => (MidiKind::NoteOff, status & 0x0F),
        0xB0 => (MidiKind::ControlChange, status & 0x0F),
        _ => return None,
    };
    let number = bytes[1];
    let mut value = bytes[2];
    // Note On with velocity 0 is a Note Off per the MIDI spec.
    let kind = if kind == MidiKind::NoteOn && value == 0 {
        value = 127;
        MidiKind::NoteOff
    } else {
        kind
    };
    Some(MidiMessage {
        channel,
        kind,
        number,
        value,
    })
}

/// An action a MIDI binding can trigger. `SwitchScene` targets a scene by its
/// stable id so renames do not break bindings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiAction {
    /// Switch the program scene to the given scene id.
    SwitchScene(uuid::Uuid),
    /// Toggle recording (start/stop the active capture path).
    ToggleRecord,
    /// Toggle streaming (start/stop the configured ingest).
    ToggleStream,
    /// Toggle the mute state of the active output.
    ToggleMute,
    /// Set the master volume from the CC value (0..127 → 0.0..1.0).
    SetMasterVolume,
    /// Toggle the chroma key of the currently selected source.
    ToggleChromaKey,
}

impl MidiAction {
    /// Stable wire name used for serialization and for the mapping table.
    pub fn name(&self) -> &'static str {
        match self {
            MidiAction::SwitchScene(_) => "SwitchScene",
            MidiAction::ToggleRecord => "ToggleRecord",
            MidiAction::ToggleStream => "ToggleStream",
            MidiAction::ToggleMute => "ToggleMute",
            MidiAction::SetMasterVolume => "SetMasterVolume",
            MidiAction::ToggleChromaKey => "ToggleChromaKey",
        }
    }
}

/// One entry of a [`MidiMapping`]: which message triggers which action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiBinding {
    /// MIDI channel 0..15.
    pub channel: u8,
    pub kind: MidiKind,
    /// Note number or CC number.
    pub number: u8,
    pub action: MidiAction,
}

impl MidiBinding {
    /// Convenience constructor used by the GUI "learn/add" flow and tests.
    pub fn new(channel: u8, kind: MidiKind, number: u8, action: MidiAction) -> Self {
        Self {
            channel,
            kind,
            number,
            action,
        }
    }

    /// True when this binding matches the given message.
    pub fn matches(&self, msg: &MidiMessage) -> bool {
        self.channel == msg.channel && self.kind == msg.kind && self.number == msg.number
    }
}

/// The full set of user-configured MIDI bindings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiMapping {
    pub bindings: Vec<MidiBinding>,
}

impl MidiMapping {
    /// Dispatch a parsed message: returns the actions of every binding that
    /// matches (normally zero or one). A CC binding used as a fader returns
    /// [`MidiAction::SetMasterVolume`] with the value carried implicitly —
    /// the GUI scales `msg.value` to a volume ratio at apply time.
    pub fn dispatch(&self, msg: &MidiMessage) -> Vec<&MidiAction> {
        self.bindings
            .iter()
            .filter(|b| b.matches(msg))
            .map(|b| &b.action)
            .collect()
    }

    /// The CC value 0..127 normalized to a volume ratio 0.0..1.0 (fader use).
    pub fn volume_ratio(value: u8) -> f32 {
        (value as f32 / 127.0).clamp(0.0, 1.0)
    }
}

/// A named snapshot of a full [`MidiMapping`] that can be saved per device.
/// Presets are keyed by the device name as enumerated by the OS (e.g.
/// `nanoKONTROL2 1 SLIDER/KNOB`), not by index — indices shift when devices
/// are hot-plugged, names are stable identifiers the user recognises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiPreset {
    /// The device name the preset belongs to.
    pub device: String,
    /// User-chosen preset name, unique per device.
    pub name: String,
    /// The complete binding set saved in this preset.
    pub mapping: MidiMapping,
}

/// The per-device preset library. Stored as a plain list so serialization is
/// deterministic (insertion order) and tests can compare round-trips exactly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiPresetLibrary {
    pub presets: Vec<MidiPreset>,
}

impl MidiPresetLibrary {
    /// Save or replace the preset `name` for `device` with `mapping`.
    /// Returns `true` when an existing preset was replaced.
    pub fn save(&mut self, device: &str, name: &str, mapping: MidiMapping) -> bool {
        if let Some(existing) = self
            .presets
            .iter_mut()
            .find(|p| p.device == device && p.name == name)
        {
            existing.mapping = mapping;
            true
        } else {
            self.presets.push(MidiPreset {
                device: device.to_owned(),
                name: name.to_owned(),
                mapping,
            });
            false
        }
    }

    /// Load the mapping of the preset `name` for `device`, if present.
    pub fn load(&self, device: &str, name: &str) -> Option<&MidiMapping> {
        self.presets
            .iter()
            .find(|p| p.device == device && p.name == name)
            .map(|p| &p.mapping)
    }

    /// Remove the preset `name` for `device`. Returns `true` when removed.
    pub fn delete(&mut self, device: &str, name: &str) -> bool {
        let before = self.presets.len();
        self.presets
            .retain(|p| !(p.device == device && p.name == name));
        self.presets.len() != before
    }

    /// The preset names for `device`, in insertion order (stable for the GUI
    /// dropdown, unlike a HashMap iteration order).
    pub fn names_for(&self, device: &str) -> Vec<&str> {
        self.presets
            .iter()
            .filter(|p| p.device == device)
            .map(|p| p.name.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn parses_note_on_cc_and_note_off() {
        let on = parse_midi(&[0x90, 60, 100]).unwrap();
        assert_eq!(
            on,
            MidiMessage {
                channel: 0,
                kind: MidiKind::NoteOn,
                number: 60,
                value: 100,
            }
        );
        let cc = parse_midi(&[0xB5, 7, 64]).unwrap();
        assert_eq!(
            cc,
            MidiMessage {
                channel: 5,
                kind: MidiKind::ControlChange,
                number: 7,
                value: 64,
            }
        );
        let off = parse_midi(&[0x81, 60, 0]).unwrap();
        assert_eq!(off.kind, MidiKind::NoteOff);
        assert_eq!(off.channel, 1);
    }

    #[test]
    fn note_on_with_velocity_zero_is_normalized_to_note_off() {
        let msg = parse_midi(&[0x90, 36, 0]).unwrap();
        assert_eq!(msg.kind, MidiKind::NoteOff);
        assert_eq!(msg.value, 127);
    }

    #[test]
    fn rejects_unknown_and_malformed_frames() {
        assert!(parse_midi(&[0xE0, 0, 0]).is_none()); // pitch bend
        assert!(parse_midi(&[0xF0, 0, 0]).is_none()); // SysEx start
        assert!(parse_midi(&[0x90, 60]).is_none()); // truncated
        assert!(parse_midi(&[]).is_none());
    }

    #[test]
    fn dispatch_matches_exact_channel_kind_and_number() {
        let scene = Uuid::new_v4();
        let mapping = MidiMapping {
            bindings: vec![
                MidiBinding::new(0, MidiKind::ControlChange, 7, MidiAction::SetMasterVolume),
                MidiBinding::new(0, MidiKind::NoteOn, 60, MidiAction::SwitchScene(scene)),
                MidiBinding::new(1, MidiKind::ControlChange, 7, MidiAction::ToggleMute),
            ],
        };
        // CC 7 on channel 0 → master volume.
        let actions = mapping.dispatch(&parse_midi(&[0xB0, 7, 42]).unwrap());
        assert_eq!(actions, vec![&MidiAction::SetMasterVolume]);
        // Note 60 on channel 0 → scene switch.
        let actions = mapping.dispatch(&parse_midi(&[0x90, 60, 100]).unwrap());
        assert_eq!(actions, vec![&MidiAction::SwitchScene(scene)]);
        // Same CC number on a different channel must not match.
        let actions = mapping.dispatch(&parse_midi(&[0xB1, 7, 42]).unwrap());
        assert_eq!(actions, vec![&MidiAction::ToggleMute]);
        // Unbound message → nothing.
        assert!(mapping
            .dispatch(&parse_midi(&[0x90, 61, 100]).unwrap())
            .is_empty());
    }

    #[test]
    fn volume_ratio_scales_cc_value() {
        assert_eq!(MidiMapping::volume_ratio(0), 0.0);
        assert!((MidiMapping::volume_ratio(64) - 0.5039).abs() < 0.001);
        assert_eq!(MidiMapping::volume_ratio(127), 1.0);
    }

    #[test]
    fn mapping_survives_serde_round_trip() {
        let mapping = MidiMapping {
            bindings: vec![
                MidiBinding::new(0, MidiKind::ControlChange, 7, MidiAction::SetMasterVolume),
                MidiBinding::new(
                    2,
                    MidiKind::NoteOn,
                    60,
                    MidiAction::SwitchScene(Uuid::new_v4()),
                ),
            ],
        };
        let json = serde_json::to_string(&mapping).unwrap();
        let back: MidiMapping = serde_json::from_str(&json).unwrap();
        assert_eq!(mapping, back);
        // Action names are stable identifiers for the mapping table.
        assert_eq!(MidiAction::ToggleStream.name(), "ToggleStream");
        assert_eq!(MidiKind::ControlChange.label(), "CC");
    }

    #[test]
    fn preset_library_saves_loads_and_lists_per_device() {
        let mut lib = MidiPresetLibrary::default();
        let mapping = MidiMapping {
            bindings: vec![MidiBinding::new(
                0,
                MidiKind::ControlChange,
                7,
                MidiAction::SetMasterVolume,
            )],
        };
        assert!(!lib.save("nanoKONTROL2", "Streaming", mapping.clone()));
        assert!(!lib.save("nanoKONTROL2", "Recording", mapping.clone()));
        // Same preset name on another device is a separate entry.
        assert!(!lib.save("AKAI MIDImix", "Streaming", mapping.clone()));

        assert_eq!(
            lib.names_for("nanoKONTROL2"),
            vec!["Streaming", "Recording"]
        );
        assert_eq!(lib.names_for("AKAI MIDImix"), vec!["Streaming"]);
        assert!(lib.names_for("missing").is_empty());

        assert_eq!(lib.load("nanoKONTROL2", "Streaming"), Some(&mapping));
        assert_eq!(lib.load("nanoKONTROL2", "nope"), None);
    }

    #[test]
    fn preset_save_replaces_existing_entry_and_delete_removes_it() {
        let mut lib = MidiPresetLibrary::default();
        let first = MidiMapping {
            bindings: vec![MidiBinding::new(
                0,
                MidiKind::ControlChange,
                7,
                MidiAction::SetMasterVolume,
            )],
        };
        let second = MidiMapping {
            bindings: vec![MidiBinding::new(
                0,
                MidiKind::NoteOn,
                60,
                MidiAction::ToggleRecord,
            )],
        };
        assert!(!lib.save("dev", "Live", first.clone()));
        assert!(lib.save("dev", "Live", second.clone())); // replaced
        assert_eq!(lib.load("dev", "Live"), Some(&second));
        assert_eq!(lib.presets.len(), 1, "replacement must not duplicate");

        assert!(lib.delete("dev", "Live"));
        assert!(lib.names_for("dev").is_empty());
        assert!(!lib.delete("dev", "Live"), "second delete is a no-op");
    }

    #[test]
    fn preset_library_survives_serde_round_trip() {
        let mut lib = MidiPresetLibrary::default();
        lib.save(
            "nanoKONTROL2",
            "Streaming",
            MidiMapping {
                bindings: vec![MidiBinding::new(
                    0,
                    MidiKind::ControlChange,
                    7,
                    MidiAction::SetMasterVolume,
                )],
            },
        );
        lib.save(
            "AKAI MIDImix",
            "Live",
            MidiMapping {
                bindings: vec![MidiBinding::new(
                    2,
                    MidiKind::NoteOn,
                    60,
                    MidiAction::SwitchScene(Uuid::new_v4()),
                )],
            },
        );
        let json = serde_json::to_string(&lib).unwrap();
        let back: MidiPresetLibrary = serde_json::from_str(&json).unwrap();
        assert_eq!(lib, back);
        // Insertion order is preserved across serialization.
        assert_eq!(back.names_for("nanoKONTROL2"), vec!["Streaming"]);
    }
}
