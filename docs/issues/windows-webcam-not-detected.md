# Bug: Webcam-Geräte werden unter Windows nicht erkannt / nicht in der GUI aufgelistet

## Zusammenfassung
Rivulet zeigt auf Windows keine verfügbaren Webcam-Geräte an. In der GUI (z. B. beim Hinzufügen einer Webcam-Quelle) bleibt die Geräteleiste leer, obwohl ein Bildschirm-/Webcam-Hardwareprüf-Arbeitsgang möglich wäre.

## Erwartetes Verhalten
- Bei einem Windows-System mit angeschlossener Webcam sollte die GUI verfügbare Webcam-Geräte auflisten.
- Die Geräteauswahl für Webcam-Quellen sollte funktionsfähig und auffindbar sein.
- Wenn keine Webcam erkannt wird, sollte das dem Nutzer klar kommuniziert werden (keine stumme leere Liste ohne Erklärung).

## Tatsächliches Verhalten
- Webcam-Geräte werden nicht erkannt / nicht aufgelistet.
- Die Webcam-Auswahl zeigt keine Einträge an.
- Der Nutzer kann keine Webcam-Quelle über die gewohnte GUI-Wahl konfigurieren.

## Beobachtungen aus dem Code
1. `rivulet-core/src/camera.rs`
   - `list_cameras()` verwendet GStreamer `DeviceProviderFactory` und prüft aktuell nur:
     - `video4linux2deviceprovider`
     - `pipewiredeviceprovider`
   - Im sichtbaren Pfad gibt es keinen Windows-DeviceProvider-Aufruf in dieser Funktion.

2. GUI-Rückgriff in `rivulet-gui/src/app.rs`
   - `refresh_camera_devices()` ruft `rivulet_core::camera::list_cameras()` auf.
   - `scene_device_entries()` für `SourceKind::Webcam` nutzt `self.camera_devices`.
   - Die Tests für den Szenen-Device-Picker prüfen die Webcams-Logik über eine manuell besetzte `camera_devices`-Liste.

3. i18n-Vorbereitung für andere Plattformen
   - In `rivulet-core/src/inspect.rs` existieren Bezeichnungen wie:
     - `webcam-windows-ksvideo` → `Webcam (Windows Media Foundation)`
     - `webcam-macos-avf` → `Webcam (AVFoundation)`
   - Das deutet darauf hin, dass die Bezeichnung für Windows/Mac bereits vorbereitet ist, aber die tatsächliche Windows-Enumeration in `camera.rs` fehlt oder nicht greift.

## Mögliche Ursachen
1. Fehlendes Windows-Backend in `list_cameras()`.
   - Die Core-Funktion enumeriert nur Linux-Pfade. Unter Windows gibt es dann keine Geräte für `list_cameras()`.
2. GUI zeigt nur das, was Core liefert.
   - Weil `camera_devices` leer bleibt, bleibt auch der Webcam-Picker in der GUI leer.
3. Backend-Abhängigkeit / Gerätekontext.
   - Möglicherweise wird unter Windows eine andere enumerative Ebene benötigt (Windows-Media-Foundation-ähnlich / ksvideosrc-fähiger Provider), die aktuell nicht angesprochen wird.

## Gewünschte Behandler-Richtung
1. Prüfen, ob `list_cameras()` unter Windows korrekt enumerieren kann.
2. Falls Windows-Unterstützung beabsichtigt ist: Windows-DeviceEnumeration ergänzen oder zumindest klar dokumentieren, wo die Grenze der Unterstützung liegt.
3. Falls keine Windows-Webcam-Erkennung intendiert ist: Oberfläche und Dokumentation sollten das transparent machen (z. B. Hinweis "Keine Webcam erkannt"), statt eine undurchsichtig leere Auswahl zu zeigen.
4. Optional: Gentle Nutzer-Feedback in der GUI, wenn die Liste leer bleibt.

## Umgebung
- Betriebssystem: Windows 10/11
- Plattformziel: Windows-Webcam-Quelle über GUI
- Begleits.Context: Nutzer meldet, dass Webcams nicht erkannt werden.

## Beobachtete Randpunkte (für Test/Analyse)
- Wenn der Bug nachweisbar sein soll, empfiehlt sich ein reproduzierbares Enummerations-Check, das den Rückgabewert von `list_cameras()` auf Windows prüft.
- Falls Windows-Unterstützung gar nicht intendiert war, ist das ein Dokumentations-/UX-Thema statt eines reine Implementierungsfehlers.

## Anmerkung
Dies ist ein Code-basiertes Beobachtungs- und Vorgehensdokument. Die genaue Windows-Implementierungsstrategie (DirectShow / Media Foundation / GStreamer-Windows-DeviceProvider / anderes) sollte im Kontext der bestehenden Webcam-Backend-Architektur entschieden werden.

EOF
echo "written"