# Feature-Erkennung & Laufzeit-Diagnose (Issue #295)

## Ziel
Die Feature-Detection-API ermöglicht es einer einbettenden Anwendung (`rivulet-core` als Bibliothek), die Verfügbarkeit von Encodern, Capture-Backends, Hardware-Decoding und Audio-Quellen zu ermitteln — **ohne Log-Ausgabe zu parsen und ohne GUI zu starten**.

## Design-Prinzipien

1. **Maschinenlesbare Gründe** (`why_not`) — nicht lokalisierte, übersetzte Sätze, sondern stabile Strings (z. B. `"nvenc element not found: nvh264enc/nvh265enc not in gst-plugins-bad (NVCODEC)"`), auf die ein Downstream-Consumer sicher matchen kann.
2. **Strukturierte Fallback-Entscheidungen** — `EncoderFallback` und `CaptureBackendFallback` zeigen nicht nur *welcher* Encoder gewählt wird, sondern *warum* (erste verfügbare Hardware, Software als letztes Mittel, etc.).
3. **Cacheable** — alle Detection-Ergebnisse werden in `FEATURE_DETECTION_CACHE` (Lazy, Thread-Safe) zwischengespeichert. Die erste Ermittlung prüft GStreamer-Element-Factories; folgende Aufrufe sind O(1).
4. **Ohne echte Hardware testbar** — die Tests prüfen die *Struktur* der Ergebnisse (nie leer, Software immer als Fallback vorhanden, Reihenfolge stabil, `why_not` konsistent mit `available`), nicht die konkrete Verfügbarkeit auf dem CI-Runner.
5. **Einheitliches Muster** — Encoder (`VideoEncoder`), Capture-Backends (`CaptureBackendInfo`), Hardware-Decoding (`FeatureInfo`) und Audio-Quellen (`FeatureInfo`) folgen demselben Schema: `Vec<T>` mit `available` + `why_not`.

## Öffentliche API (`rivulet-core`)

### Encoder
- `detect_available_encoders_with_info(codec) -> Vec<EncoderInfo>` — strukturierte Verfügbarkeit pro Backend + Codec, mit `why_not`.
- `detect_available_encoders_for_codec(codec) -> Vec<VideoEncoder>` — einfache Reihenfolge (Hardware → Software).
- `detect_available_encoders() -> Vec<VideoEncoder>` — H.264-Standard (rückwärtskompatibel).
- `detect_available_encoders_info() -> Vec<EncoderInfo>` — strukturierte H.264-Daten.
- `best_encoder_for_codec(codec) -> VideoEncoder` — erste verfügbare.
- `best_encoder() -> VideoEncoder` — H.264-Standard.
- `encoder_fallback_for_codec(codec) -> EncoderFallback` — strukturierte Fallback-Entscheidung (gewählt + Kandidatenliste + `why_chosen`).
- `encoder_fallback() -> EncoderFallback` — H.264-Standard.

Strukturtypen:
- `EncoderInfo { backend, codec, available, why_not }`
- `EncoderFallback { chosen, codec, candidates, why_chosen }`
- `RateControlMode` (Cbr, Vbr, Cq, CqVbr) — erweitert um `label()` und `hint()` für UI.
- `RateControl` (mode, max_bitrate_kbps, quality, custom_options) — öffentlich.

### Capture-Backends
- `detect_capture_backends() -> Vec<CaptureBackendInfo>` — Liste aller erkannten Capture-Backends (xcap, PipeWire, V4L2, WGC, AVFoundation, etc.) mit `platform` und `why_not`.
- `capture_backend_fallback() -> CaptureBackendFallback` — strukturierter Fallback (gewählt + Kandidaten + `why_chosen`).

Strukturtypen:
- `CaptureBackendInfo { backend, available, why_not, platform }`
- `CaptureBackendFallback { chosen, candidates, why_chosen }`

### Hardware-Decoding
- `detect_hardware_decoding_availability(codec) -> Vec<FeatureInfo>` — NVDEC, VideoToolbox, D3D11, VAAPI.
- `FeatureInfo { feature, available, why_not }`

### Audio-Quellen
- `detect_audio_source_availability() -> Vec<FeatureInfo>` — Mikrofon, PipeWire, PulseAudio, CoreAudio, WASAPI.
- `audio_source_fallback() -> AudioSourceFallback` — strukturierter Fallback.

## Cache-Mechanismus

Die globale `FEATURE_DETECTION_CACHE` (`Lazy<FeatureDetectionCache>`) speichert Ergebnisse für `(VideoCodec, DetectionType)` und ermöglicht `clear_for_test()` für deterministische Testläufe.

## Tests

Die `feature_detection_tests` (unter `#[cfg(test)]`) prüfen:
- `feature_detection_cache_is_clearable()` — Cache kann für deterministische Tests zurückgesetzt werden.
- `detect_capture_backends_always_has_test_source()` — Software-Testquelle immer als Fallback.
- `detect_audio_sources_never_empty()` — Audio-Quellenliste nie leer.
- `detect_hardware_decoding_never_empty()` — Hardware-Decoding nicht leer für alle Codecs.
- `encoder_info_is_available_matches_backend_availability()` — `EncoderInfo::is_available()` konsistent mit `VideoEncoder::is_available_for_codec()`.
- `encoder_info_why_not_is_empty_when_available()` und `encoder_info_why_not_is_non_empty_when_unavailable()` — `why_not`-String konsistent mit `available`.
- `detect_available_encoders_info_never_empty()` und `_order_matches_preference()` — Reihenfolge stabil (Nvenc > QuickSync > Amf > Software).
- `detect_available_encoders_info_hardware_before_software()` — Hardware vor Software in der Liste.
- `encoder_fallback_picks_first_available_backend()` — `EncoderFallback::chosen` = erste verfügbare.

## Roadmap-Bezug
- **M8** (`rivulet-core` als stabile, embeddable API) — Feature-Detection liefert die programmatische Schnittstelle, die M8 fordert (`detect_available_encoders_with_info`, `detect_encoder_availability`).
- **M9** (`Modern Architecture`, #319) — Die `FeatureInfo` und `CaptureBackendInfo`-Strukturen bereiten das erweiterte Scene-Management (Folder/Group, Component-Subscene-Isolation) vor.
- **M11** (`Extensible UI & Plugin Platform`) — Die non-localisierten `why_not`-Codes sind für Plugin-Autoren stabil, ohne dass sie GStreamer-Log-Ausgabe parsen müssen.
