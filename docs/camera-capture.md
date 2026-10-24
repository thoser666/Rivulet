# Camera capture

How Rivulet finds and opens cameras, and what is supported on each platform.
Tracked by issue [#318](https://github.com/thoser666/Rivulet/issues/318).

## Entry points

- `rivulet_core::camera::list_cameras()` — enumerates the cameras of the
  current platform. The GUI fills its camera picker from it **at startup**
  (`refresh_camera_devices()` in `rivulet-gui/src/app.rs`) and again on every
  🔄 refresh click; a 🔄 is never required to see devices that were already
  plugged in when the app started.
- `rivulet_core::camera::start_camera_capture()` — opens one device and
  yields `CameraFrame`s over a channel (RGBA), with `CameraConfig` width /
  height / fps `0` meaning "keep the source's values".

## Platform support status

Enumeration runs through GStreamer **device providers**; each platform has its
own chain, probed in order:

| Platform | Providers (probe order) | Consuming element | Device property | Enumeration |
| --- | --- | --- | --- | --- |
| Linux | `video4linux2deviceprovider` → `pipewiredeviceprovider` | `v4l2src` / `pipewiresrc` | `device` / `path` | Supported |
| Windows | `mfdeviceprovider` → `ksdeviceprovider` → `dshowdeviceprovider` | `mfvideosrc` / `ksvideosrc` / `dshowvideosrc` | `device-path` / `device-path` / `device` | Supported |
| macOS | `avfdeviceprovider` (gst-plugins-bad `applemedia`, since 1.20) | `avfvideosrc` | `unique-id` | Supported |

Notes per platform:

- **Linux** — v4l2 first because it hands out the `/dev/videoN` path
  `v4l2src` wants; PipeWire is only consulted when v4l2 found nothing.
- **Windows** — before #318 neither of the two Linux providers exists there,
  so `list_cameras()` always returned an empty list. Media Foundation is
  probed first and lists physical *and* virtual cameras (verified live on
  2026-10-24: an NVIDIA Broadcast camera and an OBS Virtual Camera both
  arrive via `device.api = mediafoundation`). Kernel Streaming is the
  fallback for cameras only it can see; DirectShow (`dshowvideosrc`, rank
  `none`) is the last resort.
- **macOS** — `avfdeviceprovider` enumerates AVFoundation capture devices;
  devices are addressed by their `AVCaptureDevice` unique id
  (`device-index` is deprecated and non-functional since GStreamer 1.30).

Enumeration and capture are separate claims: the table documents what
`list_cameras()` returns and which element/property pair `start_camera_capture()`
builds the pipeline from. If a listed camera fails to open, run
`gst-device-monitor-1.0 Video/Source` and compare — see Troubleshooting.

## Rules the code guarantees

- **Fallback, never a merge.** The first provider that returns devices owns
  the whole list; two providers are never mixed (v4l2 and PipeWire describe
  the same physical cameras in different path formats and would double them
  up). Pinned by `tests/camera.rs::the_fallback_chain_never_mixes_two_providers_and_dedups_paths`.
- **Device paths are unique** over the whole result (deduplicated across the
  entire list, not just adjacent pairs).
- **Empty lists explain themselves.** The picker renders the localized
  `camera_none` string ("No camera found." / "Keine Kamera gefunden.") for
  cameras instead of a silent empty combo, so "no camera attached" is
  distinguishable from "platform not supported".

## Tests

`rivulet-core/tests/camera.rs` runs headless — no camera is opened:

- `platform_provider_chains_are_pinned_per_platform` pins all three chains
  (compiled on every platform, so Linux CI covers the Windows/macOS lists).
- `a_stub_provider_feeds_the_camera_list_without_hardware` feeds a fake
  provider through the `list_cameras_with()` injection seam — the #318
  acceptance test that fails without the fix.
- `device_property_mapping_is_pinned` pins the element/property table above.
- `list_cameras_stays_inside_the_platform_consumer_set` runs the real
  `list_cameras()` and keeps its result inside the platform's element set.
- Guard: `ci_pinning::camera_provider_chains_cover_every_platform_and_are_documented`
  keeps code and this page in sync — extending a provider chain without
  documenting it here fails CI.

## Troubleshooting

If `gst-device-monitor-1.0 Video/Source` shows a camera but Rivulet does not,
the app is probably not loading the GStreamer build that contains the
platform's provider (mixed installations: an official installer *and* a
package-manager build both on the plugin path). Symptoms of that mismatch are
GStreamer warnings at startup such as
`Failed to load plugin '.../libgstmediafoundation.dll'` — the provider factory
then never registers and the chain falls through to an empty list. Point
`GST_PLUGIN_SYSTEM_PATH_1_0` at exactly one installation and re-run the
monitor; the app must resolve the same one.