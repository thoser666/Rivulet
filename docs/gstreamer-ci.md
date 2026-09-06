# GStreamer CI provisioning (Windows)

How Windows CI installs the GStreamer MSVC SDK, which installer
generations exist, and how to move the version pin — including to the
1.28 series, whose installer format changed completely.

## The two installer generations

| | ≤ 1.26.x | ≥ 1.28.x |
|---|---|---|
| Upstream artifacts | two MSIs: `gstreamer-1.0-msvc-x86_64-<v>.msi` (runtime) + `gstreamer-1.0-devel-msvc-x86_64-<v>.msi` (devel) | **one** unified Inno Setup `.exe`: `gstreamer-1.0-msvc-x86_64-<v>.exe` |
| Silent install | `msiexec /qn /norestart INSTALLLEVEL=100` per MSI | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TYPE=devel /DIR=<root>` |
| Install layout | machine root registered via `GSTREAMER_1_0_ROOT_MSVC_X86_64` | same env var name (cerbero: `root_env_var = 'GSTREAMER_1_0_ROOT_%(arch)s'`), but registered **only** when installed elevated; `/PORTABLE=1` installs without admin and without machine ENV/registry |
| Default install dir (system-wide) | `C:\gstreamer\1.0\msvc_x86_64` | `%ProgramFiles%\gstreamer\1.0\msvc_x86_64`; `/DIR=` overrides; user-only installs default under `%LOCALAPPDATA%\Programs` |
| Registry | MSI product metadata | `Software\GStreamer1.0\<arch>\InstallDir` (+ `Version`, `SdkVersion`) |
| Authenticode | signed | **not signed** — the freedesktop `.sha256sum` is the only integrity anchor |

Facts verified against cerbero's installer generator
(`cerbero/packages/windows/inno_setup.py`, `data/inno/base.iss`,
`packages/gstreamer-1.0/gstreamer-1.0.package`) and the
[official download page](https://gstreamer.freedesktop.org/download/)
parameters list (`/DIR`, `/TYPE=devel|runtime|debug`, `/ALLUSERS`,
`/CURRENTUSER`, `/VERYSILENT`, `/PORTABLE`).

## The shared CI helper

`packaging/windows/install-gstreamer.ps1` is the single installation
path used by `ci.yml`, `build-package.yml` and `nightly.yml`. It:

1. detects the generation by probing what upstream ships for the pinned
   version (HEAD on the `.exe`; fallback = MSI pair);
2. downloads via **cache → mirror release → freedesktop.org** (with
   retries), verifying every artifact against the official
   `<name>.sha256sum` **before** it is trusted;
3. installs silently — MSI pair via `msiexec`, or the unified installer
   with `/TYPE=devel` (runtime + headers, the equivalent of the old two
   MSIs) into the fixed `-InstallRoot` (`C:\gstreamer\1.0\msvc_x86_64`),
   retrying once with `/PORTABLE=1` on non-elevated machines;
4. exports the same environment contract as before:
   `GSTREAMER_1_0_ROOT_MSVC_X86_64`, `PKG_CONFIG_PATH`,
   `GST_PLUGIN_PATH`, `GST_PLUGIN_SYSTEM_PATH`, `GST_PLUGIN_SCANNER`,
   `GST_REGISTRY`, and prepends `<root>\bin` to `PATH`.

The deterministic `/DIR=` (instead of the version-dependent
`%LOCALAPPDATA%` defaults) is what keeps the environment resolution
identical for both generations.

## Local verification of the 1.28 path (2026-09-06)

* `gstreamer-1.0-msvc-x86_64-1.28.6.exe` downloaded, SHA256 matched the
  official sidecar (`059251444d1267b4…`), Authenticode = NotSigned.
* Silent install with `/VERYSILENT /TYPE=devel /DIR=C:\gstreamer\1.0\msvc_x86_64`
  completed in ~3.5 min (528 MB solid LZMA2), exit 0, no reboot required.
* Layout verified: 542 plugin DLLs under `lib\gstreamer-1.0` (x264, libav,
  coreelements, flv present), `lib\pkgconfig` (155 `.pc` files),
  `libexec\gstreamer-1.0\gst-plugin-scanner.exe`.
* `pkg-config --modversion gstreamer-1.0` → `1.28.6` (app, video, pbutils
  likewise); `cargo check -p rivulet-core` green against the 1.28.6
  headers (gstreamer-rs 0.25.3 unchanged).

## Moving the pin (e.g. to 1.28.x)

1. Mirror the installers (the script auto-detects the `.exe` generation):
   `bash scripts/mirror-gstreamer-msi.sh 1.28.6`
2. Bump the version in **three workflows** (`ci.yml`, `build-package.yml`,
   `nightly.yml`: `-Version "x.y.z"` + the two `gstreamer-msvc-x.y.z`
   cache keys in `ci.yml`/`build-package.yml`).
3. Bump the default in `scripts/mirror-gstreamer-msi.sh` and the pin in
   the ci_pinning guard `windows_ci_installs_one_consistent_gstreamer_version`.
4. Run CI: the 28 pipeline-string tests and the full 3-platform matrix
   validate the engine against the new runtime (pad-linking behavior
   differs between GStreamer versions — see the flvmux hardening that
   was needed for ≤ 1.24 in PR #105).

The helper needs **no changes** for the move: both generations are
already implemented and were verified locally.
