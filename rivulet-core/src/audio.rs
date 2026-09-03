use crate::Locale;

/// Identifies which input channel an [`AudioFrame`] belongs to.
///
/// Used when recording separate audio tracks so that the system audio and the
/// microphone are stored in distinct tracks of the output file instead of
/// being mixed together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTrack {
    /// The system/desktop audio ("what you hear").
    System,
    /// The microphone input.
    Microphone,
}

impl AudioTrack {
    /// Human-readable, language-neutral label for this track.
    ///
    /// Used for displaying which output track an input is routed to when
    /// recording separate tracks. Localized labels live in the GUI/i18n layer.
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System audio",
            Self::Microphone => "Microphone",
        }
    }

    /// Stable i18n key used by the GUI to render a localized track name.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::System => "system_audio",
            Self::Microphone => "microphone",
        }
    }
}

/// Interleaved PCM audio data produced by an audio capture source.
///
/// Samples are `f32` in the range `[-1.0, 1.0]` and interleaved by channel
/// (`[L, R, L, R, ...]`).
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl AudioFrame {
    pub fn new(data: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        Self {
            data,
            sample_rate,
            channels,
        }
    }

    /// Number of samples per channel in this frame.
    pub fn frame_len(&self) -> usize {
        self.data.len() / self.channels.max(1) as usize
    }
}

/// An audio filter that was requested but could not be built because its
/// GStreamer element is not installed (e.g. `webrtcdsp` on distros that do
/// not ship it).
///
/// The capture pipeline degrades gracefully by skipping such filters;
/// consumers (such as the GUI) use this to surface the omission to the user
/// instead of only logging a warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedFilter {
    /// GStreamer element factory name (e.g. `webrtcdsp`).
    pub element: &'static str,
    /// Human-readable feature name (e.g. `noise suppression`).
    pub feature: &'static str,
}

impl SkippedFilter {
    /// The English, human-readable feature name provided by a GStreamer
    /// element factory.
    ///
    /// This is the single source of truth for the feature name used both in
    /// the capture log message and in the GUI warning, so the two can never
    /// drift apart.
    pub fn feature_name(element: &str) -> &'static str {
        match element {
            "webrtcdsp" => "noise suppression",
            "audiodynamic" => "compressor/limiter/expander/gate",
            "audioamplify" => "gain",
            "equalizer-10bands" => "10-band equalizer",
            _ => "audio filter",
        }
    }

    /// The feature name for a GStreamer element factory, localized for the
    /// given [`Locale`].
    ///
    /// English ([`SkippedFilter::feature_name`]) is the fallback and the name
    /// used by the capture log; other locales translate the same element
    /// mapping, so the GUI warning and the log stay consistent for every
    /// locale. A locale without an explicit translation falls back to English.
    pub fn feature_name_in(element: &str, locale: Locale) -> &'static str {
        match (element, locale) {
            ("webrtcdsp", Locale::De) => "Rauschunterdrückung",
            ("audiodynamic", Locale::De) => "Kompressor/Limiter/Expander/Gate",
            ("audioamplify", Locale::De) => "Verstärkung",
            ("equalizer-10bands", Locale::De) => "10-Band-Equalizer",
            (_, Locale::De) => "Audiofilter",
            _ => Self::feature_name(element),
        }
    }

    /// The console log message emitted when this filter is skipped during
    /// pipeline construction.
    ///
    /// Kept as a pure function of the struct so the exact log text can be
    /// tested independently of the `tracing` backend.
    pub fn log_message(&self) -> String {
        format!(
            "{} skipped: GStreamer element `{}` is not installed",
            self.feature, self.element
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_creates_frame_with_given_data() {
        let data = vec![0.1, -0.2, 0.3, -0.4];
        let frame = AudioFrame::new(data.clone(), 48_000, 2);
        assert_eq!(frame.data, data);
        assert_eq!(frame.sample_rate, 48_000);
        assert_eq!(frame.channels, 2);
    }

    #[test]
    fn frame_len_computes_samples_per_channel() {
        let frame = AudioFrame::new(vec![0.0; 8], 48_000, 2);
        assert_eq!(frame.frame_len(), 4);
    }

    #[test]
    fn frame_len_never_divides_by_zero() {
        let frame = AudioFrame::new(vec![0.0; 4], 48_000, 0);
        assert_eq!(frame.frame_len(), 4);
    }

    #[test]
    fn empty_frame_has_zero_len() {
        let frame = AudioFrame::new(Vec::new(), 48_000, 2);
        assert_eq!(frame.frame_len(), 0);
    }

    #[test]
    fn audio_track_variants_are_distinct() {
        assert_ne!(AudioTrack::System, AudioTrack::Microphone);
        assert_eq!(AudioTrack::System, AudioTrack::System);
        assert_eq!(AudioTrack::Microphone, AudioTrack::Microphone);
    }

    #[test]
    fn skipped_filter_feature_name_is_the_shared_mapping() {
        assert_eq!(
            SkippedFilter::feature_name("webrtcdsp"),
            "noise suppression"
        );
        assert_eq!(
            SkippedFilter::feature_name("audiodynamic"),
            "compressor/limiter/expander/gate"
        );
        assert_eq!(SkippedFilter::feature_name("audioamplify"), "gain");
        assert_eq!(
            SkippedFilter::feature_name("equalizer-10bands"),
            "10-band equalizer"
        );
        assert_eq!(
            SkippedFilter::feature_name("future-element"),
            "audio filter"
        );
    }

    #[test]
    fn feature_name_in_localizes_from_the_shared_mapping() {
        // English must always match the log's canonical name.
        for element in [
            "webrtcdsp",
            "audiodynamic",
            "audioamplify",
            "equalizer-10bands",
            "future-element",
        ] {
            assert_eq!(
                SkippedFilter::feature_name_in(element, Locale::En),
                SkippedFilter::feature_name(element),
                "English GUI name for `{element}` must match the log name"
            );
        }
        // German translates the same mapping instead of defining its own.
        assert_eq!(
            SkippedFilter::feature_name_in("webrtcdsp", Locale::De),
            "Rauschunterdrückung"
        );
        assert_eq!(
            SkippedFilter::feature_name_in("audiodynamic", Locale::De),
            "Kompressor/Limiter/Expander/Gate"
        );
        assert_eq!(
            SkippedFilter::feature_name_in("audioamplify", Locale::De),
            "Verstärkung"
        );
        assert_eq!(
            SkippedFilter::feature_name_in("equalizer-10bands", Locale::De),
            "10-Band-Equalizer"
        );
        assert_eq!(
            SkippedFilter::feature_name_in("future-element", Locale::De),
            "Audiofilter"
        );
    }

    #[test]
    fn log_message_reports_the_exact_text() {
        let cases = [
            (
                "webrtcdsp",
                "noise suppression skipped: GStreamer element `webrtcdsp` is not installed",
            ),
            (
                "audiodynamic",
                "compressor/limiter/expander/gate skipped: GStreamer element `audiodynamic` is not installed",
            ),
            (
                "future-element",
                "audio filter skipped: GStreamer element `future-element` is not installed",
            ),
        ];
        for (element, expected) in cases {
            let filter = SkippedFilter {
                element,
                feature: SkippedFilter::feature_name(element),
            };
            assert_eq!(filter.log_message(), expected);
        }
    }

    #[test]
    fn audio_track_labels_and_keys_are_stable() {
        assert_eq!(AudioTrack::System.label(), "System audio");
        assert_eq!(AudioTrack::Microphone.label(), "Microphone");
        assert_eq!(AudioTrack::System.i18n_key(), "system_audio");
        assert_eq!(AudioTrack::Microphone.i18n_key(), "microphone");
        // The i18n key resolves to a localized name in the GUI layer.
        assert_eq!(Locale::De.tr(AudioTrack::Microphone.i18n_key()), "Mikrofon");
        assert_eq!(Locale::En.tr(AudioTrack::System.i18n_key()), "System audio");
    }
}
