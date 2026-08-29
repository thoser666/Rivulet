# Updater troubleshooting

## Windows

The updater downloads the matching `.msi` asset to the user temp directory and
starts `msiexec.exe` detached with `/passive`, `/norestart`, and
`REBOOT=ReallySuppress`. Rivulet then closes so Windows Installer can replace the
running application. A successful download alone is not an installation
success; the UI must reach the installing state and Windows Installer must be
started.

If clicking **Install** appears to do nothing:

1. Check the Settings update status for the exact error.
2. Verify that the release contains a `rivulet-windows-x86_64.msi` asset.
3. Check `%TEMP%` and Windows Event Viewer → Windows Logs → Application for
   `msiexec` entries.
4. Retry as a normal user first; elevation is requested by Windows Installer if
   required.
5. If policy blocks MSI installation, download the same signed MSI from the
   GitHub Release and run it manually.

The updater never logs or displays stream keys, tokens, or download credentials.
The installer path is validated for existence prior to launching process commands across all platforms.
The downloaded installer is cleaned up after a successful launch where the
platform permits it (e.g., when the application remains open after opening a DMG on macOS).
On Windows, immediate file removal upon process launch is explicitly deferred to prevent deleting the `.msi` file while `msiexec.exe` is opening it.

The MSI uses one stable `UpgradeCode`, installs into the fixed `Rivulet` folder,
and performs a major upgrade before registering the new product. This prevents
old and new versions from remaining as separate Control Panel entries. The
Start Menu and desktop shortcuts target `rivulet-launcher.exe`, which resolves
the GUI executable beside itself; they therefore continue to start the version
that was actually installed by the upgrade. If an old shortcut still points to
an older installation directory, delete and recreate that shortcut or launch
Rivulet from the current Start Menu entry.

### Exit code 3010

`msiexec` exit code **3010** means *success, reboot required*. It is not an
update failure. Rivulet treats both `0` and `3010` as successful installer
outcomes; the update is applied, while Windows may complete replacement after a
restart. Codes such as `1603` remain errors and are shown in the update status.
