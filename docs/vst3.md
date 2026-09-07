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

**Offen (Z96-4):**
- Z96-4: Doku Hosting-Contract, Plattform-Matrix, Gating

追踪在 [issue #96](https://github.com/thoser666/Rivulet/issues/96) (M5).

## Offen (Follow-up)

- GUI: Plugin-Auswahl/Reihenfolge pro Spur (basiert dann auf der Discovery).
