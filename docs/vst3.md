# VST 3.x-Support

Die meisten modernen Audio-Plugins sind **nur als VST3** erhältlich. Rivulet
legt dafür den Konfigurations- und Entdeckungs-Contract (`rivulet-core::vst3`):

- `VstPlugin` — validierter Verweis auf ein `.vst3`-Bundle (Anzeigename +
  Pfad), `validate()` prüft Name und `.vst3`-Endung, `bundle_available()`
  prüft die Existenz auf der Platte ohne VST3-Runtime.
- `VstChain` — geordnete Kette von Plugins pro Eingangsspur (leere Kette =
  keine VST-Verarbeitung).
- `vst3_search_dirs()` / `discover_vst3_plugins()` — deterministische
  Entdeckung aus den plattformüblichen Suchpfaden:
  - Windows: `C:\Program Files\Common Files\VST3`, `C:\Program Files\VST3`,
    `%LOCALAPPDATA%\Programs\Common\VST3`, `%APPDATA%\VST3`
  - macOS: `/Library/Audio/Plug-Ins/VST3`, `~/Library/Audio/Plug-Ins/VST3`
  - Linux: `/usr/lib/vst3`, `/usr/local/lib/vst3`, `~/.vst3`

Die Entdeckung ist rein dateisystembasiert und dadurch ohne Plugin-Binary
testbar.

## Hosting (Z96-1 + Z96-2)

Die Host-Runtime ist in zwei Subtasks aufgeteilt:

**Z96-1 — Host-Boundary-Type** (`VstHost` trait):
- `VstHost` trait mit `load_plugin()` → `HostLoadResult` (Loaded/Skipped)
- `SkipReason` enum: BundleNotFound, BundleInvalid, NoFactory, NoProcessor, HostError
- `HostHandle` opaque identity type (Send-safe für Audio-Thread)
- `ChainLoadResults` summary type mit `load_chain()` helper
- Skip-on-error pattern (nie fatal) — siehe `docs/vst3-host-boundary.md`

**Z96-2 — Windows COM Host Skeleton** (`WindowsVstHost`):
- `resolve_bundle_path()`: Absolute Pfade direkt, relative Pfade über
  Suchverzeichnisse (`C:\Program Files\Common Files\VST3` + User-Dirs)
- `load_library()`: LoadLibraryW via `windows-sys 0.59` mit Null-Handle-Guard
- Vier-Stufen-Dispatch: BundleResolve → FactoryObtain → ProcessorCreate → Loaded
- Non-Windows Stubs: Kein Laden möglich, alle Plugins werden übersprungen
- `discover_vst3_plugins()`: Findet `.vst3`-Bundles in Suchverzeichnisse

**Z96-3 — Host-Boundary Skip-Path Tests** (done):
- 8 Tests für alle `SkipReason`-Varianten (BundleNotFound/BundleInvalid/NoFactory/NoProcessor/HostError)
- Mock-Hosts testen Skip-Logik ohne echtes Plugin-Binary
- Chain bleibt immer validierbar, auch wenn alle Plugins übersprungen werden
- Deterministisch, erweiterbar für spätere echte Plugin-Binaries

## Z96-4 — Hosting-Contract, Plattform-Matrix und Gating (dieses Dokument)

Dieser Abschnitt ist die verbindliche Dokumentation des Hosting-Stands und
dokumentiert ehrlich, was Hosting **heute** bedeutet und was nicht.

### Was das `vst3`-Modul heute tut

- **Config/Entdeckung/Probe** (M4): `VstPlugin`/`VstChain`-Konfigurationsvertrag,
  deterministische Bundle-Entdeckung aus den plattformüblichen Suchpfaden,
  dateisystembasierter Verfügbarkeits-Probe — alles ohne Plugin-Binary testbar.
- **Host-Vertrag** (Z96-1): `VstHost`-Trait, `HostLoadResult` (Loaded/Skipped),
  `SkipReason`-Enum, `HostHandle`, `ChainLoadResults`/`load_chain()` — der
  Skip-on-Error-Vertrag (nie fatal, Muster wie `SkippedFilter`) ist normativ in
  [`docs/vst3-host-boundary.md`](vst3-host-boundary.md) beschrieben.
- **Windows-Host-Skelett** (Z96-2): `WindowsVstHost` mit
  Bundle-Resolution (`LoadLibraryW` via `windows-sys 0.59`, Null-Handle-Guard)
  und Vier-Stufen-Dispatch BundleResolve → FactoryObtain → ProcessorCreate →
  Loaded; Non-Windows-Stubs überspringen alle Plugins.
- **Skip-Path-Tests** (Z96-3): alle `SkipReason`-Varianten durch Mock-Hosts
  abgedeckt, deterministisch, ohne Plugin-Binary.

### Was ausdrücklich NICHT dabei ist

- **Vollständiger Plugin-Stack**: Es findet noch keine Audio-Verarbeitung in
  geladenen Plugins statt — kein Process-Call, kein Parameter-Routing, kein
  Bus-/Layout-Handling. Z96-2 ist ein Host-**Skelett** (Laden/Handshake-Stufen),
  kein funktionierender Effekt.
- **GUI-Panel pro Spur**: Die Plugin-Auswahl/Reihenfolge pro Eingangsspur
  (basiert auf der Discovery) ist ein offener Follow-up (siehe unten).
- **macOS/Linux-Host**: dlopen/dylib-Hosts sind Follow-up; Non-Windows wird
  derzeit durch Stubs abgedeckt, die sauber überspringen.
- **UI-Plugins (Qt)**: aus Scope, wie im M5-Roadmap-Bullet festgelegt.

### Plattform-Matrix und Gating

| Plattform | Host-Runtime | Stand |
|---|---|---|
| Windows | COM-Skelett (`WindowsVstHost`, Z96-2) | Skelett gelandet; echter Audio-Durchsatz offen |
| macOS | dlopen/dylib | Follow-up (Stubs überspringen sauber) |
| Linux | dlopen/dylib | Follow-up (Stubs überspringen sauber) |

Gating-Regeln, die so in CI abgesichert sind:

- Der Pinning-Guard `vst3_host_boundary_and_skeleton_are_wired`
  (`rivulet-core/tests/ci_pinning.rs`) pinnt Host-Vertrag, Windows-Skelett und
  Skip-Path-Tests an die Quell- und Doku-Dateien — ein stiller Rückbau schlägt
  CI fehl.
- Die Skip-on-Error-Semantik ist durch Z96-3-Tests abgesichert: ein fehlendes
  oder kaputtes Bundle kann die Kette niemals invalidieren oder die Pipeline
  zum Absturz bringen.
- Die Plattform-Feature-Matrix im README führt VST 3.x ehrlich als
  „Konfigurationsvertrag + Entdeckung; Host-Skelett gelandet, Audio-Routing
  offen“.

Referenzen: [issue #96](https://github.com/thoser666/Rivulet/issues/96) (M5),
Subtask-Doku [`docs/issues/96-vst3-hosting-subtasks.md`](issues/96-vst3-hosting-subtasks.md),
Host-Vertrag [`docs/vst3-host-boundary.md`](vst3-host-boundary.md),
Plugin-System-RFC [`docs/plugin-system-rfc.md`](plugin-system-rfc.md).

## Offen (Follow-up)

- GUI: Plugin-Auswahl/Reihenfolge pro Spur (basiert dann auf der Discovery).
- Echter Audio-Durchsatz durch geladene Plugins (Process-Call in die Engine-
  Kette) — baut auf dem Z96-2-Skelett auf.
