# Changelog

## [0.65.0-alpha.110] - 2026-09-02
- ci: serialize alpha releases and complete the fuzz job's build deps
- ci(fuzz): fix the fuzz smoke job's missing glib headers and pin comment
- fix(security): validate shared-memory frame headers against forged geometry
- test(security): fuzz the untrusted-input parsers with a CI smoke gate
- ci(release): write SHA256SUMS outside the scanned directory
- feat(security): verify release installers against SHA256SUMS before install
- ci(wiki): audit repo-doc wiki references backwards too
- ci(wiki): audit interwiki and external links too
- build(deps): bump async-trait from 0.1.89 to 0.1.92 (#86)
- ci(wiki): audit repo-doc links (files + heading anchors)
- build(deps): bump libc from 0.2.180 to 0.2.189 (#88)
- docs(presence): add the application-id rotation runbook
- feat(presence): guarded fallback chain makes the app id updateable
- ci(toolchain): pin rustfmt via rust-toolchain.toml for CI/local parity
- style: bound the release-gate needle so CI and local rustfmt agree
- feat(presence): zero-config Discord Rich Presence with official defaults
- ci(release): include ci: commits in the alpha release gate
- ci(release): ship alpha releases from build/chore commits too
- build(packaging): ship Discord setup assets inside all three installers
- feat(assets): add a dedicated Discord App Icon asset for the member list
- docs(presence): record the member-list rendering limit
- feat(presence): mirror the art asset to small_image for the member list
- docs(presence): fix the CDN verification path and keep the rivulet_logo asset
- fix(presence): update the Unix-socket IPC smoke to the swapped card lines
- feat(presence): swap Discord card lines and add OBS-WS handshake load test
- fix(obs-ws): restore blocking before the handshake to stop CI flake
- feat(ui): warn on Apply when the Discord presence payload violates rules
- ci(presence): enforce Discord SET_ACTIVITY validation rules in CI
- test(presence): omit empty details in the live handshake script + CDN check
- feat(m5): reply in Twitch chat from the Chat dock
- ci(assets): regenerate Discord artwork via the asset generator
- fix(presence): omit empty Discord details field (4000 rejected)
- docs(presence): ship Rivulet Discord artwork + upload guide
- docs(presence): document how to verify Rich Presence on the Discord app
- feat(m5): import stream alert overlays via the browser source
- feat(presence): OBS-style Discord activity card with artwork and clean layout
- feat(m5): Twitch chat dock + visible record-dialog cancellation logging
- feat(ui): validate the Discord client id on Apply
- feat(ui): add one-click Discord reconnect in the Stream view
- fix(presence): use Discord IPC v1 opcode framing
- fix(logging): default the daily log to info level
- fix(presence): drop useless PathBuf conversion flagged by CI clippy
- fix(presence): surface Discord connection diagnostics
- fix(gui): restore persisted app state on startup
- docs(presence): add state-reference table for all six presence states
- feat(ui): status legend with per-state tooltips in the Stream view
- feat(presence): wire up the Error activity state
- fix(presence): sync Discord status every frame, not only in the Stream view
- fix(ui): make tabs responsive so controls stay reachable on small windows
- feat(midi): learn mode and per-device mapping presets (M5)
- fix(ci): unblock MIDI release build and harden obs-websocket shutdown test
- feat(midi): MIDI controller mapping (scene/fader/filter) (M5)
- test(core): fix month-boundary flake in recording path test
- ci(obs-websocket): add loopback smoke job against a real client
- docs(obs-websocket): add Stream Deck / TouchPortal step-by-step guide
- feat(obs-websocket): OBS WebSocket v5 server (#72)
- feat(hotkeys): OBS-style rebinding + global hotkeys (M5 #80)
- feat(presence): configurable Discord client id, game name in status, IPC smoke test
- feat(presence): add opt-out, non-blocking Discord Rich Presence adapter
- ci(nightly): make newer-major action-pin gaps fatal (--fail-on-major)
- feat(m4): real S3 upload after stop_recording (SigV4 PUT) + VST3 hosting issue
- chore(ci): bump codeql-action to v4 and dependency-review to v5
- fix(ci): repair nightly workflow — compound-action pins, stale SHAs, flaky test
- docs(m4): mark M4 milestone complete in README status and parity checklist
- fix(ci): collapse nested filter-restart if and update tracing assertion
- fix(ci): complete AudioFilters literals in linux-only tests
- docs(m4): record the M4 output quality gate
- fix(ci): hoist tr() labels in mixer UI and fix filter-name test
- feat(m4): VST 3.x contract + cloud recordings contract
- feat(m4): audio + video filters/effects
- feat(m4): multi-track audio export with per-track toggles
- feat(m4): advanced rate control (VBR/CQ/CQ-VBR + custom options)
- feat(m4): master audio mix with output VU meter
- ci: verify the wiki remote hash after publish
- test: add local wiki-sync smoke test
- docs: add repository-hygiene policy for .gitattributes/.gitignore
- chore: ignore wiki working clone and Python cache, drop orphaned theme WIP
- docs: normalize README line endings to LF and split merged M4 bullets
- feat(m4): live recording split via splitmuxsink
- feat(m4): recording file management (patterns, split, auto-record)
- feat(m4): execute GStreamer remux after recording stop (#71)
- feat(m4): recording container formats and crash-safe remux
- feat(m4): add sidechain audio ducking filter
- fix(ui): resolve help docs from workspace root
- feat(streaming): complete keyring and transport safeguards
- feat(streaming): secure keys and bound private tests
- fix(gui): show complete help page links
- feat(gui): make help documentation links actionable
- fix(packaging): map msi product version into compared fields
- docs(quality): add M10 completion gate
- docs(roadmap): add extensible UI plugin milestone
- feat(gui): add help documentation menu
- feat(streaming): add RTMPS reachability preflight
- fix(ui): stop idle repaints so the gui rests in egui's reactive mode
- feat(streaming): add platform test ingest defaults
- test(ci): pin release-branch reconciliation to current-source contract
- fix(ci): rebuild release branch from current source
- chore(release): prepare v0.65.0-alpha.55 (#95)
- fix(updater): satisfy non-Windows release builds
- fix(updater): silence non-Windows watchdog parameters
- fix(ci): restore cross-platform updater builds
- fix(update): wait for app exit before replacing files on Windows
- fix(ui): redact secrets from smoke reports
- feat(streaming): add platform setup assistant
- test(streaming): add documented RTMPS smoke test
- docs: add first stream platform checklist
- feat(presence): localize Discord activity and include game name
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

## [Unreleased]
- ci(release): serialize alpha release runs with a `release-alpha` concurrency group — two pushes in quick succession raced for the same next version number, release branch and GitHub release object (observed as `v0.65.0-alpha.109` tag-push rejections and an asset-upload "Not Found" while the sibling run mutated the release; the release itself landed complete); `cancel-in-progress: false` queues the latest run instead of discarding it; ci_pinning guard added
- ci(fuzz): install the glib/gstreamer pkg-config files (libglib2.0-dev, libgstreamer1.0-dev, libgstreamer-plugins-base1.0-dev) in the fuzz smoke job — rivulet-core's gstreamer-rs -sys crates need them at build time even though the targets never touch GStreamer; nightly toolchain is now selected via the action's `toolchain:` input instead of a `#nightly` ref comment that generate-action-pins misread as a second branch pin; fuzz guard extended accordingly
- fix(security): validate shared-memory frame headers against forged geometry before reading — `FrameHeader::is_plausible` (core) rejects zero/absurd geometry, unknown pixel formats, `data_size` ≠ width×height×4 (checked u64 math), payloads above a 16 MiB cap and data beyond the mapping; readers now query the real mapping size (Windows `VirtualQuery`, Linux `fstat`-clamped mmap) instead of trusting DEFAULT_SHM_SIZE, copy pixels into a private snapshot immediately, and the Windows reader finally unmaps its view on drop; both writers (Vulkan layer, OpenGL hook DLL) compute `data_size` with checked multiplication and refuse geometry/data disagreement; 4 new capture-channel tests incl. overflow and cap cases, ci_pinning guard `shared_memory_frames_are_validated_before_read`, documented in docs/security.md
- test(security): cargo-fuzz targets for every untrusted-input parser with a CI smoke gate — new `fuzz/` crate (workspace-excluded) with libFuzzer targets `parse_irc_line` (Twitch IRC), `sdp_offer_endpoint` (WHIP endpoint → SDP generator, asserts structural invariants), `parse_latest_release` (GitHub release JSON, now `pub` for the target) and `parse_checksums` (SHA256SUMS manifest); CI job "Fuzz smoke (regression corpus)" builds with cargo-fuzz on the pinned nightly and runs 256 executions per target on Linux, failing the CI aggregate and uploading crash inputs on failure; `scripts/fuzz-smoke.sh` mirrors the job locally (sanitizer-aware: Windows without the VS ASan component skips with instructions, WSL/CI are the supported paths); ci_pinning guard pins targets, symbols, workspace exclusion and CI wiring; docs in fuzz/README.md, security.md and ci-action-pins.md
- feat(security): updater verifies release installers against a SHA256SUMS manifest before install — the release workflow generates `SHA256SUMS` over every attached asset (relative paths, sorted, `-r` for the empty edge case) and attaches it to the release; the updater downloads the manifest for the release tag (`releases/download/<tag>/SHA256SUMS`), looks up the asset name and compares digests fail-closed (missing entry, unreachable manifest, malformed digest and mismatch all refuse the install); the GUI download flow runs the verification between download and the Install button, a failure surfaces as an update error and the installer is never started; 6 updater unit tests incl. a local end-to-end manifest round-trip, 2 ci_pinning guards (workflow generation + GUI wiring, sha2 workspace dependency), docs in update-troubleshooting.md and security.md
- ci(wiki): backwards link audit — `audit-wiki-links.py --check-repo-docs` now also validates wiki references *from* the repo docs: deep `…/wiki/Page[#anchor]` links in docs/*.md, README and CONTRIBUTING must resolve to real wiki pages + heading anchors, and backticked page references must match canonical page names (separator-containing drift like "Getting Started" is reported; single-word identifiers such as the `streaming` commit scope are exempt); fixed the two drift findings in docs/wiki-content-policy.md (`Getting Started` → `Getting-Started`, `Fehlerbehebung und FAQ` → `Troubleshooting-und-FAQ`); wired into the wiki workflow and sync smoke, negative self-test + ci_pinning guard extended
- ci(wiki): full wiki link audit — `scripts/audit-wiki-links.py` now validates all three link kinds: interwiki pages + anchors (including the Languages.md template, code fences are ignored), repo-doc files + GitHub heading anchors, and external URL reachability (HEAD first, GET fallback, `--skip-external` for offline runs); wired into the scheduled wiki workflow and sync smoke check 4; negative self-tests + ci_pinning guard; live run: 86 interwiki + 17 repo-doc links + 2 external URLs across 21 pages, all clean
- docs(presence): add the id-rotation runbook to docs/activity-status.md — the exact steps for rotating the official Discord application id (portal prep, DEFAULT_CLIENT_ID bump, feat-commit + release-notes block, post-release verification via log/handshake/card/custom-id checks); ci_pinning guard keeps the runbook in the versioned docs
- feat(presence): guarded fallback chain for the Discord application id — `effective_client_id`/`effective_large_image_key` (rivulet-core::discord) resolve configured id → official default → adapter off, and the adapter reconcile uses them; this doubles as the retirement path for a deprecated app id: a release bumps `DEFAULT_CLIENT_ID`, documents the change in its release notes and reaches users via the regular updater (the release payload is the updater manifest); core chain tests + ci_pinning guard + docs updated
- ci(toolchain): pin the Rust toolchain via `rust-toolchain.toml` (channel 1.98.0, components rustfmt + clippy) — every local and CI `cargo` invocation now resolves to the exact same compiler/rustfmt, ending the rustfmt version flip-flop that reddened the Lints job in the alpha.100 window; pinned by a new ci_pinning guard and documented in docs/ci-action-pins.md
- feat(presence): zero-config Discord Rich Presence — Rivulet ships the official application id (`1544027006847680532`) and the `rivulet_logo` artwork as defaults, so the branded presence card (logo + status) works on first launch without creating a Discord application; both Settings fields remain as overrides for custom branding, an empty client id now restores the official default, and restores with an empty persisted id are migrated (custom ids stay untouched); core default test, GUI default/migration tests, ci_pinning guard, docs and wiki updated
- ci(release): trigger alpha releases from `build:`/`chore:`/`ci:` commits too — the release gate now treats `feat`/`fix`/`build`/`chore`/`ci` as releasable (build/chore/ci bump the patch version via `scripts/release-version.sh`), so packaging, housekeeping and CI changes ship automatically instead of needing a manual workflow dispatch (the alpha.98/alpha.99 gap); docs/test/style-only pushes still skip the release; ci_pinning guard + CONTRIBUTING.md updated
- build(packaging): ship the Discord Rich Presence setup files (app icon, Rich Presence artwork, activity-status docs) inside all three installers — Windows MSI/portable stage a `discord\` folder into the bundle (harvested into the MSI), the AppImage installs `usr/share/doc/rivulet/discord/`, the macOS bundle carries `Contents/Resources/discord/`, and the GitHub release additionally attaches the three files as standalone downloads; pinned by a new asset integration test
- feat(assets): add a dedicated Discord App Icon asset (`docs/assets/rivulet-app-icon-512.png`, 512×512) — a punchier small-avatar variant of the wave symbol on the dark brand gradient without padding, generated deterministically as step 8 in `scripts/generate-assets.sh`, registered in `scripts/check-assets.py`, covered by new asset contract tests and documented as the upload for the member-list icon swap
- docs(presence): document the Discord platform limit — the server member list renders only the application icon and the registered app name ("Rivulet"), never the payload `details`/`state` lines; the game-controller icon there is replaced by the portal **App Icon** (512×512), not by the Rich-Presence art asset
- feat(presence): mirror the configured art asset to `small_image` so the member list shows the logo instead of the game-controller placeholder — `large_image` renders on the profile card, Discord renders `small_image` in the server member list; the same uploaded asset key covers both (live-verified: Discord accepts and echoes large+small with the resolved asset id), wire-contract test + ci_pinning guard + docs updated
- feat(presence): swap the Discord status-message lines — `details` is Discord's first card line and now always carries the status label ("Recording"/"Aufnahme"), `state` is the second line and carries the selected game name; empty `state` is omitted (Discord rejects empty strings with 4000, same rule previously applied to `details`), the payload wire-contract test, ci_pinning guard and docs/activity-status.md were updated accordingly
- test(obs-ws): add a parallel handshake load test — `parallel_clients_all_complete_handshake_under_load` bursts 24 clients via a `Barrier` at one server, requires every client to complete Hello/Identify plus a request round-trip and a fresh client to still connect afterwards, permanently securing handshake stability under load; the duplicated retry loops in the smoke were unified onto the single `connect_with_retry` helper (`TestClient::connect` now delegates to it), ci_pinning guard + docs/obs-websocket.md updated
- fix(obs-ws): restore blocking on accepted sockets before the WebSocket handshake — the accept loop's non-blocking listener made Windows sockets inherit non-blocking mode, intermittently failing tungstenite's handshake read (`Protocol(HandshakeIncomplete)`) in the CI smoke under parallel load; the auth-rejection smoke now uses the same retry as TestClient::connect, ci_pinning guard + docs/obs-websocket.md updated
- feat(ui): warn immediately in Settings when the Discord presence payload would violate Discord's rules — Apply runs validate_set_activity_payload and shows a red warning for overlong status/game-name text (>128 chars) or an implausible art asset key (empty key stays valid, placeholder icon); DE/EN i18n, GUI tests, ci_pinning guard and docs/activity-status.md updated
- ci(presence): enforce Discord's SET_ACTIVITY validation rules in CI — a dedicated "Discord payload contract check" step serializes every payload variant (6 activities × 2 locales × game/no-game/long-game × asset/no-asset/bad-asset) and asserts the wire JSON satisfies the documented rules (empty `details` omitted, `state`/`details` ≤ 128 chars, plausible `large_image` key); `validate_set_activity_payload` exposes the same contract as a reusable pre-wire validator (`PayloadIssue::FieldTooLong`/`InvalidAssetKey`), the serializer now filters implausible asset keys instead of sending them verbatim (Discord silently drops the image), ci_pinning guard + docs/activity-status.md updated — so a 4000 rejection surfaces locally in CI instead of on a live Discord client
- feat(m5): reply in Twitch chat — the Chat view gains a message input (Enter/Send) that writes PRIVMSG via the worker; sending is non-blocking, gated on a connected chat with an OAuth token (`chat:send`), and never logs the token; core send smoke (local listener) + GUI gate tests + ci_pinning guard + docs updated
- fix(presence): omit the Discord activity `details` field when empty instead of sending an empty string — Discord rejects `SET_ACTIVITY` with `4000: "details" is not allowed to be empty` (verified live), which silently dropped every presence update whenever no game name was selected; regression test + ci_pinning guard + docs updated
- docs(presence): ship a ready-to-upload Discord Rich Presence artwork (`docs/assets/rivulet-rich-presence-1024.png`, 1024×1024 PNG with padding) and document the exact upload path (Developer Portal → Rich Presence → Art Assets, asset name `rivulet_logo`) plus size/format/padding requirements in docs/activity-status.md
- feat(m5): alert overlay import — Streamlabs/StreamElements widget URLs (and any custom https URL) can be imported into the browser source via a guided provider + token flow in the Scenes view; the token is cleared after import and never logged; core URL-shape/validation tests, GUI import tests, ci_pinning guard and docs/alerts.md added
- feat(presence): OBS-style Discord activity card — the payload now carries the art asset (`large_image`) uploaded in the Discord Developer Portal (configurable asset key in Settings, persisted), so the card shows Rivulet artwork instead of the generic placeholder icon; the layout splits state (plain status label) and details (selected game name) like OBS, and the app name is no longer duplicated into the payload (Discord renders it from the application registration); tests + ci_pinning guard + docs updated
- feat(m5): Twitch chat dock — native IRC client (`rivulet-core::twitch_chat`) with CAP/PASS/NICK/JOIN handshake, PING/PONG keepalive and IRCv3 tag parsing (colors, badges, broadcaster, `/me`), plus a Chat view in the sidebar (channel + optional OAuth token, bounded auto-scrolling message list, DE/EN i18n); anonymous read-only works without a token, tokens are never logged; documented in docs/twitch-chat.md (roadmap: docs/obs-vision-roadmap.md M5 community dock)
- fix(logging): log a cancelled recording save dialog at `info` level on all platforms (Windows, PipeWire portal, Linux fallback) instead of `debug`, so a Record press that never starts a capture is diagnosable from the daily log ("File selection cancelled" present = dialog cancelled; absent = no source selected); ci_pinning guard + docs updated

## [0.65.0-alpha.55] - 2026-08-30
- feat(ui): validate the Discord client id on Apply in Settings — a non-numeric value, wrong length, or pasted URL shows an immediate warning instead of silently keeping the adapter off; core validator + tests + ci_pinning guard added — shown while the adapter is not connected and a client id is configured; it rebuilds the worker + handshake without restarting the app (tests + ci_pinning guard added) `[opcode:u32][length:u32]` (op 0 = HANDSHAKE, op 1 = FRAME) — the old 4-byte-only framing was rejected by Discord with `{"code":1003,"message":"protocol error"}`, so the status never appeared even with a valid client ID and pipe; verified live against a running Discord client (READY reply), wire tests assert the opcodes end to end, and a ci_pinning guard locks the framing in — EnvFilter::from_default_env() filtered out everything when RUST_LOG was unset, so the daily log stayed empty (no startup, engine, or Discord diagnostics at all); an explicit RUST_LOG still overrides the default; regression tests + ci_pinning guard added — the worker now logs every IPC success/failure (visible in the crash logs) and exposes a shared connection state; the Stream view shows whether the handshake was accepted ("Connected") instead of silently showing only Discord's plain game-detection card; tests + ci_pinning guard + docs updated — save() wrote the eframe storage but nothing read it back, silently dropping every configured setting (theme, Discord client id, hotkeys, OBS WebSocket, MIDI presets, …); RivuletApp::new now restores via eframe::get_value under APP_KEY and re-attaches the live engine + CLI timeout; eframe-storage round-trip tests and a ci_pinning guard added
- docs(presence): add a state-reference table to docs/activity-status.md covering all six states — DE/EN labels, trigger conditions, and transitions; the ci_pinning guard now keeps the documented table in sync with the model
- feat(ui): add a status legend to the Stream view — one row per presence state with a hover tooltip explaining when it appears and what it means; the active state is highlighted, keys are localized DE/EN and pinned by ci_pinning
- feat(presence): wire the previously unused PresenceActivity::Error state — engine/capture failures surface as a localized "Error"/"Fehler" status with priority over activity labels, streaming starts clear stale errors, and the payload never leaks the raw error text (tests + docs updated)
- fix(presence): sync Discord Rich Presence from the global UI update every frame, not only while the Stream view is visible — starting/stopping a recording now updates the Discord status from any tab; regression + ci_pinning guards added, docs updated
- fix(ui): make tabs responsive — the central view content and the navigation sidebar now live in scroll areas (controls stay reachable when the window is shrunk) and main.rs sets a minimum window size; responsive layout is covered by ui_smoke/ui_accessibility/ui_regression source-contract tests and a ci_pinning guard
- feat(m5): MIDI learn mode (capture the next moved control into the add-binding row without dispatching it) and per-device mapping presets (save/apply/delete, keyed by stable port name) in Settings
- feat(m5): add MIDI controller mapping (Korg NanoKontrol etc.) — hardware-free parse/dispatch core in `rivulet-core::midi` (Note/CC → scene switch, master-volume fader, mute, chroma-key toggle), `midir` device bridge in the GUI, Settings section with device picker and binding table, persisted bindings, i18n DE/EN (see docs/midi.md)
- ci: install libasound2-dev on Linux for the midir ALSA backend; add CI wiring test for the MIDI mapping
- feat(m5): add OBS WebSocket v5 remote control server (`rivulet-obs-websocket`) for Stream Deck / TouchPortal — scenes, sources, recording/streaming control, optional SHA-256 auth, event subscriptions, request batches; verified end-to-end with a real WebSocket client (#72)
- feat(gui): wire OBS WebSocket server into Settings (enable/port/password), bridge it to the real scene manager/engine, and broadcast GUI-initiated changes to connected clients (see docs/obs-websocket.md)
- feat(presence): add opt-out, non-blocking Discord Rich Presence adapter (`rivulet-core::discord`) with IPC + graceful degradation
- feat(presence): wire opt-out toggle into the Stream view and persist it
- feat(presence): include the user-selected game/source name in the presence state; make the Discord application client ID configurable in Settings
- test(presence): add CI smoke test that runs the Discord adapter against a local IPC listener (Unix socket + named pipe) and verifies handshake + SET_ACTIVITY
- fix(update): wait for app exit before replacing files on Windows
- fix(ui): redact secrets from smoke reports
- feat(streaming): add platform setup assistant
- test(streaming): add documented RTMPS smoke test
- docs: add first stream platform checklist
- feat(presence): localize Discord activity and include game name
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
