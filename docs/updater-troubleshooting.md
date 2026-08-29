# Updater troubleshooting

The updater downloads the platform package in a worker thread and launches the
installer without waiting for `msiexec`. On Windows it suppresses inherited
stdio handles and uses `REBOOT=ReallySuppress`; exit code 3010 is therefore a
successful installation that may require a later reboot.

The GUI never calls egui/eframe from the updater worker. After installation the
worker publishes a terminal state, and the UI thread closes the viewport on its
next frame. This avoids an epaint `RwLock` deadlock that can otherwise make the
application appear frozen or crash with `Failed to acquire RwLock read after
10s`.

If an update still fails, retain the updater error shown in Settings, verify
that the downloaded asset exists and that Windows Installer is available, then
retry. A missing or incomplete asset is rejected before starting the installer.
