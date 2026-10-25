# Capture-Backends hinter stabile Traits (#296)

## Trait-Garantien (`CaptureBackend`)

```rust
pub trait CaptureBackend: Send + Sync + 'static {
    fn list_windows(&self) -> Vec<GameWindow>;
    fn start_capture(&self, window: &GameWindow, config: &GameCaptureConfig) -> Result<(Receiver<GameCaptureFrame>, GameCaptureHandle), CaptureError>;
    fn name(&self) -> &str;
}
```

**Nicht-verhandelbare Zusagen (M8-Gate):**
- `Send + Sync` — Thread-sicher, in Worker-Threads verwendbar.
- `start_capture` liefert einen `Receiver<GameCaptureFrame>` (Reihenfolge garantiert, Timestamp im Frame) und einen `GameCaptureHandle` (Abbruch durch Drop oder explizit).
- `CaptureError` ist typisiert (`WindowNotFound`, `BackendUnavailable`, `CaptureFailed`) — keine plattformspezifischen Sonderfälle beim Aufrufer.
- `list_windows` liefert stabile IDs (UUID) für Wiedererkennung über Sitzungen hinweg.

## Stabilitätsmatrix

| Backend | Status | Plattform | Hinweis |
|---|---|---|---|
| `XCapBackend` | stabil | Linux | `xcap`-Crash; 9 Regressionstests in `tests/capture_backend.rs` |
| `PipeWireBackend` | experimentell | Linux/Wayland | Stub; `BackendUnavailable`; echte PipeWire-Integration folgt |
| `MetalWgcBackend` | experimentell | Windows/macOS | Stub; `BackendUnavailable` außerhalb Windows; WGC/Metal-Integration folgt |

New backends must implement `CaptureBackend`; the existing enumeration test (`all_backends_implement_trait`) breaks if a backend is added without it.

## Auswahl über API

`CaptureBackend` wird über `rivulet-core` als `pub trait` exportiert; ein einbettendes Produkt kann einen Quelltyp wählen (z. B. `XCapBackend::new()`) ohne Neu-Kompilierung von Rivulet.
