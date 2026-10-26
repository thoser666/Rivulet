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
- `list_windows` liefert stabile Fenster-IDs (`u64`: X11-Fenster-ID bzw. HWND) für die Wiedererkennung innerhalb einer Sitzung — und antwortet **nie mit einem Panic**: eine leere Liste ist eine gültige Antwort (siehe Abschnitt unten).

## Stabilitätsmatrix

| Backend | Status | Plattform | Hinweis |
|---|---|---|---|
| `XCapBackend` | stabil | Linux | `xcap`-Crash; 9 Regressionstests in `tests/capture_backend.rs` |
| `PipeWireBackend` | experimentell | Linux/Wayland | Stub; `BackendUnavailable`; echte PipeWire-Integration folgt |
| `MetalWgcBackend` | experimentell | Windows/macOS | Stub; `BackendUnavailable` außerhalb Windows; WGC/Metal-Integration folgt |

New backends must implement `CaptureBackend`; the existing enumeration test (`all_registered_backends_implement_trait`) breaks if a backend is added without it.

## Fensterliste: leer ist gültig

`list_windows()` darf auf jedem Host leer zurückkommen und muss dabei nie panicken — genau das sichern die Enumeration-Tests in `rivulet-core/src/capture_backend.rs` ab:

- **Headless-Hosts (CI):** Auf den GitHub-Runnern läuft kein Display-Server; `xdotool search` findet dort keine Fenster. Eine leere `Vec` ist die korrekte Antwort, kein Fehler — ein `assert!(!windows.is_empty())` macht genau diesen Test auf Ubuntu rot.
- **Stubs enumerieren nichts:** `PipeWireBackend` und `MetalWgcBackend` liefern immer eine leere Liste und dürfen dort niemals Fenster erfinden.
- **Berichtete Fenster sind vollständig:** Titel nicht leer, Breite/Höhe > 0 (die Heuristik von `list_game_windows()` filtert zusätzlich auf > 640×480 und leeren Titel).
- **Kein `unwrap()` auf `new()`:** Die Konstruktoren sind plattformgeated — `XCapBackend::new()` verweigert außerhalb Linux, `MetalWgcBackend::new()` außerhalb Windows mit dem typisierten `BackendUnavailable`. Diese Ablehnung ist Teil des Vertrags und wird als `Err`-Fall getestet; ein `unwrap()` macht die Enumeration auf Windows und macOS rot.
- **Thread-Sicherheit:** `Send + Sync + 'static` wird über `assert_send_sync_static::<…>()` geprüft und kompilierungsseitig durch `Box<dyn CaptureBackend>` erzwungen.

## Auswahl über API

`CaptureBackend` wird über `rivulet-core` als `pub trait` exportiert; ein einbettendes Produkt kann einen Quelltyp wählen (z. B. `XCapBackend::new()`) ohne Neu-Kompilierung von Rivulet.

`CaptureBackendChoice::default_backend()` liefert Linux → `xcap`, Windows → `metal-wgc`, sonst den typisierten Fehler `No capture backend available for this platform`. `list_windows_with_default_backend()` fällt in diesem Fall auf eine leere Liste zurück, statt zu fehlen oder zu panicken.
