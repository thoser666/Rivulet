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

## Offen (Follow-up)

- **Hosting**: ein VST3-Modul tatsächlich laden (Plugin-Binary, Processor
  instanziieren, Audio durchschleifen) — erfordert eine VST3-Host-Runtime
  (z. B. `vst3-sys`/`baseplug`) und einen Host-Thread. Als eigenes
  Arbeitspaket getrackt in
  [issue #96](https://github.com/thoser666/Rivulet/issues/96) (M5).
- GUI: Plugin-Auswahl/Reihenfolge pro Spur (basiert dann auf der Discovery).
