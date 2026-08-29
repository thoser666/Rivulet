# Changelog

## [0.65.0-alpha.52] - 2026-08-29
- feat(presence): document optional Discord adapter roadmap
- fix(windows): make MSI upgrades replace existing install
- fix(m4): derive virtual camera defaults
- fix(gui): show implemented streaming view
- feat(m4): add virtual camera lifecycle contract
- fix(updater): avoid GUI deadlock during installation
- fix(updater): prevent update crash by adding file existence check and deferring installer cleanup
- fix(ci): isolate actionlint archive extraction
- debug(ci): print README head before parity check
- debug(ci): print heading sample when parity section is missing
- test(streaming): de-flake reconnect worker timing test
- docs(security): add responsible-disclosure security policy
- fix(ci): pin docker base and actionlint downloads
- fix(ci): deduplicate test attribute in M3 report test
- docs(m3): close streaming milestone with completion report
- feat(streaming): add NDI output contract; refresh action pins
- feat(streaming): add VOD track configuration model
- fix(ci): pin checkout action in wiki workflow
- feat(streaming): add WHIP session lifecycle and teardown
- fix(ci): validate and publish wiki checkout correctly
- fix(ci): accept wiki translation page pairs
- fix(ci): pass wiki checkout to translation checks
- fix(ci): detect wiki language links across lines
- docs(ci): automate bilingual wiki checks
- docs: define bilingual wiki navigation
- docs: define GitHub Wiki content policy
- docs: add end-user guide
- fix(ci): match RIST identity dump format
- fix(ci): make RIST buffer probe portable
- fix(ci): capture RIST receiver buffer diagnostics
- feat(streaming): complete M3 transport verification
- test(streaming): verify RIST delivery and retry windows
- fix(updater): accept MSI reboot-required exit code
- feat(streaming): expose live bitrate change diagnostics
- fix(updater): launch Windows installer asynchronously
- fix(ci): report RIST sender timeout
- fix(ci): use installed GStreamer binary in RIST smoke
- fix(ci): harden action pin and RIST diagnostics
- fix(gui): persist theme on application exit
- fix(streaming): restore queue telemetry API
- feat(streaming): show per-target queue telemetry
- fix(ci): stabilize RIST receiver readiness check
- fix(ci): bound RIST smoke test runtime
- ci(deps): split Cargo and action update ownership
- test(gui): add accessibility regression contract
- fix(ci): validate RIST image diagnostics
- fix(security): update yanked chacha20 dependency
- fix(ci): build RIST image before inspection
- fix(rist): payload MPEG-TS into RTP via rtpmp2tpay for ristsink
- Inspect Rist plugin (#94)
- fix(ci): declare RIST MPEG-TS caps
- fix(ci): use native RIST MPEG-TS caps
- fix(ci): stop Dependabot bot approval failures
- feat(streaming): add network telemetry to adaptive bitrate
- fix(ci): make RIST smoke caps explicit
- fix(ci): provide RTP caps for RIST smoke
- fix(ci): correct RIST MPEG-TS smoke pipeline
- fix(ci): stabilize release retries and RIST pipeline
- feat(perf): validate runtime resource telemetry
- test(ci): include RIST in required checks
- fix(ci): use valid RIST sink properties
- test(ci): add RIST interoperability smoke test
- feat(streaming): wire health-driven bitrate and WHIP contract
- feat(ci): activate resource efficiency gate
- feat(streaming): expose bounded delay overflow
- fix(release): reset generated files before branch reuse
- fix(ci): run SRT receiver in listener mode
- fix(release): make version branch retries idempotent
- fix(ci): stabilize SRT smoke and pinning checks
- fix(ci): use valid SRT smoke image reference
- feat(streaming): rebuild complete target branches
- test(ci): add reproducible SRT receiver smoke test
- feat(streaming): harden transport interoperability and reconnects
- fix(release): always publish GitHub releases after tagging
- feat(streaming): automate reconnect and transport fanout
- feat(streaming): add cancellable reconnect worker
- feat(streaming): add reconnect runtime contracts
- feat(streaming): harden reconnect and transport supervision
- feat(streaming): integrate multistream fanout
- feat(streaming): add live policy and target fanout contracts
- feat(streaming): add SRT and RIST contribution contract
- feat(streaming): add WHIP signaling client
- feat(streaming): add multistream target model
- fix(ci): unify upload artifact action pin
- feat(streaming): document WHIP strategy spike
- ci: classify OBS features against product vision
- ci: monitor OBS upstream feature releases
- test(gui): add egui regression snapshot job
- test(gui): add cross-platform UI smoke contract
- feat(streaming): add delay and multitrack video support
- feat(streaming): add stream presets and adaptive bitrate policy
- docs(m2): reflect closed milestone status
- docs(m2): close milestone with quality gate
- docs(m2): clarify cross-platform UX review procedure
- docs(m2): record cross-platform UI UX gate
- feat(filters): add per-source chroma key controls
- feat(m2): complete scene workflow controls and quality docs
- fix(ci): satisfy redundant closure lint in scene export
- feat(scenes): add profile workflows and scene overlays
- feat(scenes): add scene hotkeys and auto-switch rules
- feat(scenes): add collection import export and duplication workflows
- fix(ci): prepare releases without protected branch push
- fix(gui): satisfy live preview clippy
- fix(core): satisfy snapshot clippy lint
- feat(gui): add recording live preview
- feat(gui): add deterministic scene snapshots
- fix(ci): resolve current RustSec advisories
- fix(ci): remove yanked image codec dependency
- fix(ci): resolve cargo audit dependency failures
- ci: add dependency gates and distribution readiness
- ci: require security and scorecard checks
- docs(ci): refresh generated action pin table
- docs(security): document develop ruleset bypass
- ci: add required develop branch checks

## [0.64.0-alpha.1] - 2026-08-25
- feat(security): add OpenSSF Scorecard analysis

## [0.63.0-alpha.1] - 2026-08-25
- feat(security): enable CodeQL and dependency review

## [0.62.0-alpha.1] - 2026-08-25
- feat(gui): add milestone quality gates and studio mode

## [0.61.1-alpha.1] - 2026-08-25
- fix(gui): avoid nested egui context lock

## [0.61.0-alpha.1] - 2026-08-25
- feat(diagnostics): capture pre-Rust startup failures

## [0.60.1-alpha.1] - 2026-08-25
- fix(logging): record startup diagnostics before GUI launch
- refactor(logging): route diagnostics through tracing

## [0.60.0-alpha.1] - 2026-08-25
- feat(logging): add rotating diagnostic logs

## [0.59.1-alpha.1] - 2026-08-25
- fix(gui): keep transition updates non-blocking

## [0.59.0-alpha.1] - 2026-08-25
- feat(scenes): add cut and fade transitions

## [0.58.0-alpha.1] - 2026-08-25
- feat(composition): add per-scene source editor

## [0.57.0-alpha.1] - 2026-08-25
- feat(scenes): complete remaining M2 organisation work

## [0.56.0-alpha.1] - 2026-08-25
- feat(scenes): add collections and duplication

## [0.55.0-alpha.1] - 2026-08-25
- feat(scenes): add undo and redo history

## [0.54.0-alpha.1] - 2026-08-25
- feat(gui): refine responsive and accessible styling
- refactor(gui): align styling with desktop UI guidance

## [0.53.0-alpha.1] - 2026-08-25
- feat(gui): standardize palette and interaction feedback

## [0.52.0-alpha.1] - 2026-08-25
- feat(sources): add browser source contract and configuration

## [0.51.1-alpha.1] - 2026-08-24
- fix(tests): improve ci_signing skip-worktree error hint, update docs

## [0.51.0-alpha.1] - 2026-08-24
- feat(gui): add hover/active accent strokes and preview fade-in animation

## [0.50.0-alpha.1] - 2026-08-24
- feat: glassmorphism effect — semi-transparent panels

## [0.49.0-alpha.1] - 2026-08-24
- feat: wire theme::init into the GUI and drive colors from the palette

## [0.48.0-alpha.1] - 2026-08-24
- feat: add manual refresh + live window-list update to game-capture preview

## [0.47.1-alpha.1] - 2026-08-24
- fix: move game-window preview methods to a shared linux+windows impl

## [0.47.0-alpha.1] - 2026-08-24
- feat: game capture live preview + window picker on Linux

## [0.46.0-alpha.1] - 2026-08-24
- feat: game capture live preview + fix window title leaking into Source dropdown
- docs: add game capture live preview to M2 roadmap
- test: add theme persistence verification tests

## [0.45.0-alpha.1] - 2026-08-24
- feat: S5a + S6 + S7 + S8 — Browser spike, Media, Color, Audio sources

## [0.44.0-alpha.1] - 2026-08-24
- feat: S3 Text source + S4 Webcam source with i18n and tests

## [0.43.5-alpha.1] - 2026-08-23
- fix(updater): wait for installer on all platforms + fix macOS test

## [0.43.4-alpha.1] - 2026-08-23
- fix(updater): wait for installer process before deleting downloaded file

## [0.43.3-alpha.2] - 2026-08-23
- Initial release or no new commits.

## [0.43.3-alpha.1] - 2026-08-23
- fix: add 3-strategy GStreamer download to build-package.yml

## [0.43.2-alpha.1] - 2026-08-23
- fix: resolve clippy::field_reassign_with_default in source.rs tests

## [0.43.1-alpha.1] - 2026-08-23
- fix: gate source_label() for Linux/Windows only (macOS compilation)

## [0.43.0-alpha.1] - 2026-08-23
- feat: S2 — Image source with single-file and folder slideshow modes

## [0.42.1-alpha.1] - 2026-08-23
- fix: source selection persists when switching to window picker

## [0.42.0-alpha.1] - 2026-08-23
- feat: S1 — Source abstraction layer with transforms
- revert: remove unsupported continue-on-error from release workflow
- ci: make Windows build optional in release workflow (GStreamer 503)

## [0.41.7-alpha.1] - 2026-08-23
- fix: G6 PipeWire — use fully-qualified path for VideoInfoRaw in struct

## [0.41.6-alpha.1] - 2026-08-23
- fix: G6 PipeWire — apply rustfmt to match CI formatting

## [0.41.5-alpha.1] - 2026-08-23
- fix: G6 PipeWire — move all shared state into PipeWireUserData

## [0.41.4-alpha.1] - 2026-08-23
- fix: G6 PipeWire — wrap closures in Arc for Send safety on Linux

## [0.41.3-alpha.1] - 2026-08-23
- fix: G6 PipeWire — fix i32/u32 cast and MainLoopWeak Send issue

## [0.41.2-alpha.1] - 2026-08-23
- fix: G6 PipeWire — fix Linux CI API mismatches (source_type, parse, stream.size)

## [0.41.1-alpha.1] - 2026-08-23
- fix: G6 PipeWire portal — use Rc types for cross-platform compat, add GUI dep

## [0.41.0-alpha.1] - 2026-08-23
- feat: G6 – Linux PipeWire portal fullscreen capture

## [0.40.1-alpha.1] - 2026-08-23
- fix: robust GStreamer download with mirrored release fallback

## [0.40.0-alpha.1] - 2026-08-23
- feat: G5 – Performance verification benchmark framework + CI gate
- ci: add retry logic for GStreamer MSI download
- ci: cache GStreamer MSIs to avoid transient 503 download failures
- ci: cache GStreamer MSIs to avoid transient 503 download failures

## [0.39.3-alpha.1] - 2026-08-22
- fix: gate FrameHeader/DEFAULT_SHM_SIZE imports behind cfg(windows)

## [0.39.2-alpha.1] - 2026-08-22
- fix: gate OpenGL hook DLL behind #[cfg(target_os = "windows")]

## [0.39.1-alpha.1] - 2026-08-22
- fix: remove cfg gate from PathBuf import in opengl_hook.rs

## [0.39.0-alpha.1] - 2026-08-22
- feat: G4 – OpenGL wglSwapBuffers hook for fullscreen game capture

## [0.38.2-alpha.1] - 2026-08-22
- fix(vulkan-layer): fix CI clippy errors for Clippy 1.98

## [0.38.1-alpha.1] - 2026-08-22
- fix(vulkan-layer): fix loader ABI, add live smoke test, close Issue #55
- docs: mark G3 done with the G5 budget caveat, close Issue #56
- chore: fix full-workspace macOS build and clippy warnings

## [0.38.0-alpha.1] - 2026-08-22
- feat: wire Vulkan layer into Windows GUI as preferred game capture backend

## [0.37.1-alpha.1] - 2026-08-21
- fix: cleanup downloaded installer after successful update

## [0.37.0-alpha.1] - 2026-08-21
- feat: G3 build integration — build.rs copies layer manifest to target dir

## [0.36.0-alpha.1] - 2026-08-21
- feat: G3 start_vulkan_layer_capture() — channel-based frame reading from layer

## [0.35.0-alpha.1] - 2026-08-21
- feat: G3 capture channel reader — ShmReader for reading layer frames
- feat: G3 build integration — build.rs copies VkLayer_rivulet_capture.json to target dir, layer DLL + manifest colocated

## [0.34.0-alpha.1] - 2026-08-21
- feat: G3 capture channel — shared memory IPC for frame transfer

## [0.33.0-alpha.1] - 2026-08-21
- feat: G3 layer wiring — VulkanHook::with_backend() + multi-path layer discovery

## [0.35.0-alpha.1] - 2026-08-21
- feat: G3 build integration — build.rs copies VkLayer_rivulet_capture.json to target dir, layer DLL + manifest colocated
- feat: G3 layer activation — VulkanLayerConfig for VK_LAYER_PATH env setup
- feat: G3 layer wiring — VulkanHook::with_backend() enables layer in InstanceCreateInfo
- feat: G3 capture channel — shared memory IPC (FrameHeader protocol, CreateFileMapping/shm_open), 18 layer tests
- feat: G3 capture channel reader — `ShmReader` in rivulet-core for reading frames from layer, 5 tests

## [0.31.0-alpha.1] - 2026-08-21
- feat: G3 layer tests — 14 tests pass, clippy clean

## [0.30.0-alpha.1] - 2026-08-21
- feat: G3 Vulkan capture layer — ash 0.38, vkQueuePresentKHR interception, staging buffer readback

## [0.29.0-alpha.1] - 2026-08-21
- feat: G3 Vulkan capture pipeline — staging buffer readback, 12 tests pass

## [0.28.0-alpha.1] - 2026-08-21
- feat: G3 Vulkan layer (cdylib) — VK_LAYER_RIVULET_capture with vkQueuePresentKHR interception + staging buffer readback
- feat: G3 layer negotiation — vkNegotiateLoaderLayerInterfaceVersion, instance/device/swapchain/present hooks
- feat: G3 capture pipeline — image transition PRESENT_SRC → TRANSFER_SRC, cmd_copy_image_to_buffer, HOST_VISIBLE staging → RGBA pixels
- chore: replace non-compiling `vulkan` crate with `ash` 0.38 (full features)
- docs: link the new roadmap items to their GitHub issues
- docs: update G3 roadmap — layer done, next: recording pipeline integration
- feat: G3 layer activation — `VulkanLayerConfig` for `VK_LAYER_PATH` + `VK_INSTANCE_LAYERS` env var setup, 7 tests
- feat: G3 `VulkanHook::with_backend()` — enables layer in InstanceCreateInfo when backend is Layer

## [0.27.1-alpha.1] - 2026-08-21
- fix: decouple the non-compiling G3 Vulkan draft from the build
- chore: apply rustfmt to the G3 vulkan_hook module
- ci: verify the OBS feature-parity checklist against a machine-readable catalog
- docs: add OBS features without a Rivulet counterpart to the roadmap

## [0.27.0-alpha.1] - 2026-08-21
- feat: G3 Vulkan hook infrastructure for zero-overhead game capture

## [0.26.0-alpha.1] - 2026-08-21
- docs: note Windows GUI backend indicator in G2 roadmap entry
- feat(gui): show active capture backend during Windows recordings (G2)
- ci: make Linux game-window verification robust against window-map timing

## [0.25.0-alpha.1] - 2026-08-21
- fix(release): stop changelog size doubling and repair CHANGELOG.md
- docs: mark G2 DXGI backend done in M2 roadmap
- feat: G2 DXGI Desktop Duplication capture backend
- test: use child ids in scene ordering assertions
- fix: use as_chunks in record_screen example for clippy 1.98
- docs: mark scene management done in M2 roadmap
- feat: scene management - multiple scenes, switching, add/rename/remove
- docs: mark M1 milestone as closed in roadmap table
- docs: fix remaining roadmap status inconsistencies (M4/M5 + features)
- docs: correct M2 status and scene organisation checkbox
- docs: add M1 target date to milestone overview table
- docs: live milestone progress badges in roadmap overview
- chore: ignore .freebuff/ tool artifacts
- docs: replay buffer roadmap status + game capture strategy (G1)
- feat: replay buffer engine module (instant replay)
- feat: game capture implementation & scene organisation
- feat: game capture priority & replay buffer i18n updates

## [0.24.0-alpha.1] - 2026-08-20
- feat: scene organisation - folders, color coding, search/filter
- ci: avoid stale Cargo build artifacts
- test(ci): run Linux game-window test by name
- ci: install xdpyinfo for Linux game-window test
- test(ci): verify Linux game-window enumeration with xdotool
- Merge pull request #52 from thoser666/alert-autofix-47
- Merge pull request #53 from thoser666/alert-autofix-48
- docs: document camera and game capture features
- Potential fix for code scanning alert no. 48: Clear-text logging of sensitive information
- Potential fix for code scanning alert no. 47: Clear-text logging of sensitive information

## [0.23.2-alpha.1] - 2026-08-19

- fix(core): adapt Linux game capture to the xcap 0.9.8 API

## [0.23.1-alpha.1] - 2026-08-19

- fix(gui,core): repair macOS and Linux builds after camera/game capture feature

## [0.23.0-alpha.1] - 2026-08-19

- feat: camera and game capture sources with GUI integration

## [0.22.2-alpha.1] - 2026-08-19

- fix(gui): gate platform-specific recording functions with #[cfg] to fix macOS build

## [0.22.1-alpha.1] - 2026-08-19

- fix(gui): gate timer overlay behind recording platforms to fix macOS build

- docs(readme): clarify per-scene source positioning in M2

- docs(readme): add community-requested features — VST3, Master Mix, Scene Orga, MIDI, Undo/Redo

- docs(readme): prioritize gaming features — Game Capture (M2), Replay Buffer (M4)

- docs(readme): update roadmap — M1 complete, M2/M3/M4 aligned with OBS parity

- docs(readme): update features section (remove outdated version refs, update settings)

- docs(readme): remove redundant 'In Development' section

## [0.22.0-alpha.1] - 2026-08-19

- feat(overlay): add recording timer overlay and FPS counter

- docs(release): add release strategy and enable fix:-commit alpha releases

## [0.21.0-alpha.1] - 2026-08-18

- ci: check status color contrast (WCAG AA) in the lints job

- feat(gui): add theme infrastructure with scheme-aware status colors

## [0.20.0-alpha.1] - 2026-08-18

- feat(gui): add sidebar navigation skeleton with placeholder views

- docs(ui): add UI/design guide for navigation and egui conventions

- docs(release): add backfill tooling and docs for orphaned alpha tags

- fix(gui): show the target version throughout the update flow

## [0.19.1-alpha.1] - 2026-08-18

- fix(signing): verify the nested Windows signature instead of stripping

- fix(signing): resolve self-signed identity and strip pre-existing signature

- fix(signing): verify the Windows signature without root-store trust

- fix(signing): make the macOS smoke-test cert a leaf (CA:FALSE)

- fix(signing): export the macOS smoke-test p12 with -legacy

- fix(signing): use a committed self-signed cert for the Windows E2E test

- fix(signing): give the macOS smoke-test cert the Code Signing EKU

- fix(signing): sign macOS smoke test by identity hash, not name

- fix(signing): generate the Pester cert via .NET to avoid pwsh 7.5 hang

- fix(signing): scope macOS codesign to the throwaway keychain

- ci(signing-e2e): fix macOS p12 import and Windows Pester hang

- fix(signing): use base64 -D for macOS decode compatibility

- chore(deps): upgrade ureq from 2.12 to 3.4 (fix API migration)

- refactor(core): derive Default for VideoCodec (pre-existing clippy violation)

- fix(gui): keep hotkey handling compiling on macOS (no recording engine yet)

## [Unreleased]

- ci(signing-e2e): fix macOS smoke test — a self-signed test cert is never *trusted*, so `security find-identity -v` reported "0 valid identities" and signing aborted; the signer now lists every codesigning identity (no `-v`), imports via the canonical `-A -t cert -f pkcs12` recipe, and skips the RFC3161 timestamp so the test stays offline (p12 is generated with legacy algorithms and a CA:FALSE leaf cert because macOS 26 rejects OpenSSL 3's modern defaults)

- ci(signing-e2e): fix Windows Pester test — cert generation and root-store trust both hang on the runner, so a throwaway cert is committed as a fixture; cmd.exe's Microsoft signature can't be stripped (`signtool remove` fails with 0x57), so the applied signature is nested and is verified with `signtool verify /all /v`; make the RFC3161 timestamp skippable; fix Pester 5 scoping; add job timeouts; test renamed to `sign-pester.tests.ps1`

- ci(security): redact signing-secret details from the beta-gate JSON/comment output (CodeQL alert #45)

## [0.19.0-alpha.1] - 2026-08-18

- feat(region): add region capture with interactive drag selection and multi-monitor selection (Linux & Windows)

## [0.18.0-alpha.1] - 2026-08-18

- feat(preset): add recording preset management (1080p60, 720p30, ...)

## [0.17.0-alpha.1] - 2026-08-18

- feat(codec): add codec selection UI (H.264/H.265/VP9)

- docs(readme): mark hotkeys as done in M1, add to feature list

## [0.16.0-alpha.1] - 2026-08-17

- feat(hotkeys): add record/pause/mute hotkeys with pause & mute support

- refactor(audio): centralize pipeline warn/error messages

- test(audio): capture tracing output to verify record_skipped logs

- refactor(audio): emit the skipped-filter log via a testable helper

- refactor(core): centralize localized filter names in SkippedFilter

- test(gui): assert skipped-filter warnings match the capture log

- refactor(core): make the skipped-filter warning platform-neutral

- fix(audio): re-export SkippedFilter from the crate root

- refactor(audio): return skipped filters from build_source_branch

- docs(changelog): move skipped-filters GUI entry to the current release section

- feat(gui): warn in the Linux audio mixer when filters were skipped

## [0.15.0-alpha.1] - 2026-08-17

- refactor(audio): centralize audio pipeline warn/error messages as testable message functions

- test(audio): capture tracing output to verify record_skipped emits the skipped-filter warning

- refactor(audio): emit the skipped-filter log line via SkippedFilter::log_message and test its exact text

- refactor(core): centralize localized filter names in SkippedFilter::feature_name_in

- test(gui): verify skipped-filter warnings use the same feature names as the capture log

- refactor(core): move SkippedFilter into rivulet-core and make the skipped-filter warning formatting platform-neutral

- feat(gui): warn in the Linux audio mixer when filters were skipped (missing GStreamer elements)

- feat(audio): expose skipped audio filters via AudioCapture::skipped_filters

- docs(readme): document step-by-step signing secret setup

- fix(audio): skip audio filter elements that are not installed

- ci(beta-gate): evaluate beta-readiness on every push

- docs(readme): define the beta-gate criteria in the roadmap

## [0.14.0-alpha.1] - 2026-08-17

- feat(gui): extend the no-frame recording timeout to Linux

- ci(beta-gate): evaluate beta-readiness on every push (scripts/check-beta-gate.py)

- fix(audio): skip audio filter elements that are not installed (e.g. webrtcdsp on Ubuntu) instead of failing the capture

- docs(readme): document step-by-step signing secret setup (certificates + all 7 secrets)

- feat(audio): expose skipped audio filters via AudioCapture::skipped_filters()

## [0.13.0-alpha.1] - 2026-08-17

- feat(gui): make the no-frame recording timeout configurable

- docs(readme): define the beta-gate criteria in the roadmap

## [0.12.0-alpha.1] - 2026-08-17

- feat(gui): abort Windows recording when no frame arrives within 5 s

- feat(gui): make the no-frame timeout configurable via `--no-frame-timeout <seconds>`

- feat(gui): extend the no-frame timeout to Linux recordings (fires mid-recording too)

## [0.11.0-alpha.1] - 2026-08-17

- feat(gui): surface engine and capture errors in the UI

- feat(gui): abort a Windows recording with an error when no frame arrives within 5 s (pipeline never started)

- fix(gui): stop the disconnect probe from swallowing capture frames

- fix(gui): move AtomicBool/Ordering/Arc imports to unconditional scope

- fix(ci): add --entrypoint bash to ImageMagick docker run

- test(gui): add recording state and format_bytes tests

- docs: update README, CHANGELOG, and LINUX_BUILD

## [0.10.0-alpha.1] - 2026-08-17

- feat(updater): add download progress bar

- fix(gui): only stop recording when capture thread actually disconnects

- fix(gui): stop the disconnect probe from swallowing capture frames (no MP4 was written)

- feat(gui): surface engine and capture errors in the UI instead of only on the console

- fix(updater): show MSI installer UI instead of silent install

- fix(gui): enable eframe links feature for hyperlink_to

## [0.9.0-alpha.1] - 2026-08-17

- feat(gui): embed real Rivulet icon in the Windows executable

## [0.8.0-alpha.1] - 2026-08-17

- feat(audio): complete M1 audio module (filters + monitoring)

- ci(actions): add --json and --comment output to the pin checker

- ci(actions): bump pinned actions to current major versions

- ci(actions): separate same-major staleness from newer majors

- ci(actions): detect stale action pins outside Dependabot's schedule

- ci(actions): generate the action-pin table from the workflows

- ci(dependabot): auto-approve and auto-merge Dependabot PRs

- docs(ci): add central reference mapping action SHAs to upstream versions

- ci(dependabot): keep SHA-pinned GitHub Actions current

- ci(workflows): pin third-party actions to full commit SHAs

- ci(assets): pin the ImageMagick image by content digest

- chore(assets): add docker wrapper for regenerating assets

- ci(assets): pin asset generation to a fixed ImageMagick image

- ci(assets): verify generated assets match committed files

- ci(release): attach social preview images as release assets

- docs(assets): add 1200x630 OpenGraph fallback image

- docs(assets): add GitHub social preview (OpenGraph) image

- docs(assets): refine thumbnail contrast and colors

- chore(assets): add macOS app icon and wire it into the bundle

- chore(assets): generate thumbnail and icon reproducibly

- docs(assets): add app thumbnail and Linux AppImage icon

- test(ci): add shellcheck for macOS scripts and Pester tests for sign.ps1

- refactor(ci): extract build/package/sign steps into a reusable workflow

- ci: lint GitHub Actions workflows with actionlint

- ci(packaging): smoke-test code signing with self-signed certificates

- ci(packaging): complete code signing for Windows and macOS

- chore(core): translate remaining German log and error messages to English

## [0.7.0-alpha.1] - 2026-08-16

- feat(packaging): let users choose the Rivulet install directory in the MSI

- fix(updater): enable the ureq tls backend (rustls) for HTTPS requests

- fix(core): make recording metric tests deterministic with injectable clock

- fix(packaging): add Start Menu and Desktop shortcuts to the Windows MSI

## [0.6.0-alpha.1] - 2026-08-16

- feat(updater): add auto-update via GitHub Releases (check, download, install)

- chore: update copyright year in LICENSE

## [0.5.0-alpha.1] - 2026-08-16

- feat(core): track recording performance metrics (FPS, encoder load, file size)

- docs(readme): document M9 bot coexistence with the Vivid bot

## [0.4.0-alpha.1] - 2026-08-16

- feat(core): add i18n layer and switch project to English

- docs(readme): expand roadmap with OBS differentiation (M6-M8)

## [0.3.0-alpha.1] - 2026-08-16

- feat(core): hardware video encoding (NVENC/QuickSync/AMF) with auto-detection

- chore(deps): update eframe requirement from 0.35.0 to 0.36.1

## [0.2.4-alpha.1] - 2026-08-15

- fix(ci): declare MSI package as x64 platform (ICE80)

## [0.2.3-alpha.1] - 2026-08-15

- fix(ci): Win64 mark on harvested MSI components

## [0.2.2-alpha.1] - 2026-08-14

- fix(ci): 64-bit mark in MSI harvest (ICE80)

- chore(ci): fix GitHub Actions warnings

## [0.2.1-alpha.1] - 2026-08-14

- fix(ci): convert MSI version to numeric format

## [0.2.0-alpha.1] - 2026-08-14

- feat(ci): manual alpha release trigger via workflow_dispatch

- fix(ci): fix AppImage and MSI packaging

## [0.1.0-alpha.1] - 2026-08-14

- feat(ci): fix release versioning (first release + manifest update)

## [0.0.0-alpha.1] - 2026-08-14

- Initial release
