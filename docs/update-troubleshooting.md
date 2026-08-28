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
The downloaded installer is cleaned up after a successful launch where the
platform permits it; Windows cleanup is intentionally deferred because
`msiexec.exe` may still hold the file after Rivulet exits.
