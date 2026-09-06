## Offen (Follow-up)

Die M4-Bullet „VST 3.x-Support” ist auf **Konfigurations-/Entdeckungsebene**
erledigt: `rivulet-core::VstPlugin`/`VstChain` bieten einen validierten
`.vst3`-Bundle-Verweis, deterministische Entdeckung aus den
plattformüblichen Suchverzeichnissen und einen Verfügbarkeits-Probe — alles
ohne Plugin-Binary testbar (siehe `rivulet-core/src/vst3.rs`,
`docs/vst3.md`). Fehlt ist das eigentliche **Hosting**: ein VST3-Modul in den
Audio-Graphen laden.

## Subtasks

### Z96-1 — [M5] VST3 hosting: define a host runtime boundary type (config + discovery + “skip on error”)

Bevor eine echte Host-Runtime (COM auf Windows, dlopen/dylib auf macOS/Linux)
eingebunden wird, muss die **Hostgrenze** als Vertrag modelliert sein — so
kann der Rest der Pipeline und der GUI bereits auf den Host zielen, und die
Host-Runtime kann später unabhängig implementiert/getestet werden.

Acceptance criteria:
- [ ] Es gibt einen expliziten Host-Kontrakt (z. B. ein
  `VstHost`/`VstHostHandle`-Typ oder eine trait-grenzende Representierung),
  der sagt, welcher Plugin-Verweis (`VstPlugin`) geladen werden soll, wie ein
  Load/Availability-Ergebnis aussieht, und wie Fehler/Fehlbundles aussieht.
- [ ] Der Vertrag legt fest, dass ein fehlendes/broken Bundle **übersprungen**
  wird mit einem warnenden Log/Skipped-Zustand (derselbe Muster wie
  `SkippedFilter`), nie fatal.
- [ ] Der Vertrag ist ohne Plugin-Binary testbar (Mock/Stub-äquivalente
  representable results, deterministic failure cases).
- [ ] `VstChain` bleibt validierbar und verweisst auf das Host-Result, ohne
  selbst die Host-Runtime zu treffen.

### Z96-2 — [M5] VST3 hosting: implement Windows/Com host skeleton for factory/processor handshake (host-side, testable without plugin binary)

Erstes echtes Hosting-Ziel: Windows + COM-basiertes Laden des VST3-Moduls,
Factory/Processor-Handshake — als **Host-Skelett**, nicht als vollständiger
Plugin-Stack.

Acceptance criteria:
- [ ] Auf Windows wird das `.vst3`-Bundle über die COM/VST3-Host-Schnittstelle
  angesprochen (InProcess/Component/Factory/Processor-Pfade), so weit das
  Host-Skelett ohne echtes Plugin testbar ist.
- [ ] Die Implementierung trennt klar: Bundle laden / Factory besorgen / Processor
  besorgen / Lifecycle — damit später macOS/Linux (dlopen/dylib) den selben
  Vertrag nutzen können.
- [ ] Das Skelett kommt mit einem deterministischen „kein gültiges Bundle / keine
  gültige Factory”-Pfad, der nicht fatal ist.
- [ ] Windows-first, macOS/Linux ausdrücklich als Follow-up dokumentiert (siehe
  Issue-Körper und Doku).

### Z96-3 — [M5] VST3 hosting: add missing/broken-bundle skip path + host-boundary tests (no plugin binary)

Tests für die Host-Grenze, **ohne ein echtes Plugin-Binary** zu benötigen.

Acceptance criteria:
- [ ] Tests decken die Host-Vertrags-Fälle ab: bundle nicht vorhanden, bundle
  nicht lesbar / kein gültiges VST3, kein Factory/Processor — alles skippable,
  nichts fatal.
- [ ] Tests decken die Skip-Logik ab (gleiches Muster wie `SkippedFilter`):
  skipped plugins erscheinen nicht im aktiven Prozess, die Kette bleibt
  validierbar.
- [ ] Deterministic Discovery + Config-Tests bleiben grün (kein Rückbau des
  bisherigen `vst3`-Moduls).
- [ ] Der Host-Vertrag wird so getestet, dass später echte Plugin-Binaries
  ergänzt werden können, ohne den Test-Only-Grenztest wegwerfen zu müssen.

### Z96-4 — [M5] VST3 hosting: document hosting contract, platform matrix, and gating

Doku, die ehrlich zeigt, was Hosting jetzt bedeutet und was nicht.

Acceptance criteria:
- [ ] `docs/vst3.md` (oder äquivalenter Platz) beschreibt: was das
  `vst3`-Modul heute tut (Config/Entdeckung/Probe), was das Hosting-Subtask
  liefert (Host-Vertrag + Windows-Skelett + Skip-Pfade), was **nicht** dabei ist
  (vollständiger Plugin-Stack, GUI-Panel pro Spur, macOS/Linux-Host).
- [ ] Die README-M5-Bullet zu VST 3.x wird auf den aktuellen Stand gebracht
  (Config + Discovery done; Hosting als offener Follow-up mit eigenen Subtasks).
- [ ] Platform-Matrix / Gating ist konsistent mit dem Rest des M5-Dokuments
  (Windows first, macOS/Linux follow-up).
