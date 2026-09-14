# M10 — AI Creative Studio (Spark-like)

**Status:** Accepted into M10 roadmap (feasibility scratch)
**Date:** 2026-09-12
**Tracked in:** Milestone M10 — AI Chat Assistant (accepted feature candidate)
**Reference:** [Meld Spark docs](https://meldstudio.co/docs/spark/), [Meld Spark blog](https://meldstudio.co/blog/), [Meld transparency](https://meldstudio.co/transparency/)

## Problem

Streamers today buy overlay/emote packs or run cloud SaaS (Streamlabs, StreamElements, Meld Spark). Meld Spark — Meld Studio's chat-based creative partner — lets a streamer say *"add a countdown timer, top-right, cyberpunk style"* and get a working, animated, event-reactive overlay in their scene in seconds. But Spark is a **paid, cloud** product (Google Gemini on Meld servers, usage tiers, Meld/OW account required), and it **does not generate images/emotes** — its output is always code (HTML/CSS/JS). Rivulet is positioned as *local-first, privacy-first, subscription-free*; a Spark-like feature only fits if the whole pipeline (chat + code generation + rendering + image generation) runs locally and stays free.

## Goal

A **local-first "AI Creative Studio"** inside Rivulet, driven by the M10 Assistant chat panel:

1. **Code-based overlays & widgets (Spark parity)** — the Ollama LLM turns natural-language requests into HTML/CSS/JS, rendered via Rivulet's browser source and automatable as overlay streams: alerts, lower thirds, countdown timers, goal trackers, leaderboards, chat overlays, chat-controlled mini-games, starting-soon / BRB / ending screens, and **full scene packages**.
2. **Live-event reactivity** — generated overlays react to follows, subs, raids, chat messages, and custom `!commands` across Twitch, Kick, and YouTube, through the already-shipped chat and alert pipelines (M5/M6).
3. **Studio-grade UX** — conversational refinement, reference-image color pickups, per-message restore points (undo/redo), test-event firing, and a pre-made gallery, all local.
4. **Emote/asset generation (additional feature)** — an optional local T2I pipeline produces platform-ready channel emotes (transparent PNG, Twitch/Kick sizes), reusing the same chat brain for prompt crafting.
5. **Alert-sound & voice generation (additional feature)** — an optional local sound pipeline produces alert SFX (follow/sub/raid/donation stingers), short jingles and TTS voice read-outs ("Kira spent 20 euros") from natural-language prompts, wired into the alert settings per alert kind — the audio counterpart to the emote pipeline.
6. **First-class off-switches** — the AI chatbot, the creative studio, the emote/T2I generator, and the sound/voice generator are **off by default** and independently disableable, so a streamer pays no model/resource cost and gives up no trust unless they opt in.

Everything runs locally: **Ollama** for the chat brain and code generation, Rivulet's own browser source (on top of `wry`) for rendering, and a local T2I backend for images. Free open-weights models (Apache-2.0) throughout — no API keys, no accounts, no throttles.

## Research findings (2026-09-12)

### 1. What Meld Spark actually does (feature map)

| Spark capability | Local Rivulet equivalent |
| --- | --- |
| Chat-based creative partner (Gemini, cloud) | Ollama LLM via M10 assistant (local) |
| Writes code, **not** images (explicit FAQ) | Ollama code model + browser source (`wry`) |
| Overlays, widgets, alerts, timers, lower thirds, leaderboards | HTML/CSS/JS overlay hosting |
| Reacts to follows/subs/raids/chat/commands | Already-shipped alerts (M5) + chat adapters (M6/M10) |
| Multi-scene packages, consistent aesthetic | Scene manager + browser-source sources per scene |
| Layer reference by name, reposition, move between scenes | Scene API (already exposed via obs-websocket server) |
| Conversational refinement, "make it bigger / change color" | Same chat turn-based loop |
| Drag an image into chat → extract color palette | Ollama vision (e.g. `llava`, `qwen3-vl`) on dropped file |
| Restore points (undo/redo per message) | Snapshot of generated overlay assets per turn |
| Fire test events (chat/raid/follow/sub/random) | Reuse M5 alert-ingest preview path (`queue_alert_preview` exists) |
| Read console logs from browser layers | Browser-source bridge: JS console → Rust → chat |
| Gallery of ready-made elements | Local gallery folder of templates + snapshots |
| Usage tiers, plan gates, cloud account | N/A — no limits, no accounts; **opt-in and off by default** |

Spark also *manages scenes end-to-end* (build/reorganize full scene packages, apply effects to layers). For Rivulet this lands in the same scope: the engine already has a `SceneManager` and the obs-websocket server exposes scene/source control. V2 phase could act on them.

### 2. Ollama image generation is NOT a reliable foundation (yet)

- Ollama added experimental T2I on 2026-01-20 (`x/z-image-turbo`, `x/flux2-klein`), **macOS only**; Windows/Linux never shipped.
- It was **removed in v0.32.6** (≈Sept 2026, "temporary"); the imagegen tree and API routes are gone.
- Verdict for the **emote sub-feature**: do **not** pin image generation on Ollama today. Keep Ollama as the chat/prompt/code brain. For images use a dedicated local T2I backend (below).

### 3. Code-generation models (Ollama, all Apache-2.0)

Frontend/codegen is a chatbot strength — no need for a huge model for single-file HTML/CSS/JS:

| Size tier | Model (`ollama run …`) | Footprint | Use |
| --- | --- | --- | --- |
| 8 GB GPU | `qwen2.5-coder:7b` | ~4.7 GB | Fast inner loop; single-file overlays |
| 16–24 GB GPU | `devstral` (small), `qwen3-coder:30b-a3b` (MoE) | ~14–19 GB | Agentic multi-file/scene packages |
| Big rigs | `qwen3-coder` 480B-A35B | ~290 GB | Best quality; usually overkill for overlays |

Recommended default: **`qwen2.5-coder:7b`** (or `devstral` small on 16 GB) — single-file overlays are short, well-scoped code-gen tasks.

> Correction (spike finding): the originally listed `qwen3-coder:8b` does not
> exist in the Ollama library — the qwen3-coder family ships 30b/480b MoE
> variants only. The 8 GB-tier candidate is `qwen2.5-coder:7b`; verified
> against the registry tag list during the spike.

### 4. Emote/asset generation — free local T2I (additional feature)

| Model | License | Notes |
| --- | --- | --- |
| **FLUX.2 Klein 4B** | **Apache-2.0** | Fast, good sticker/emote vectors, readable text |
| **Z-Image Turbo 6B** | Apache-2.0 | Photorealism + bilingual text (via Ollama, macOS-only today) |
| **SDXL + transparent-BG LoRA** | SDXL license (commercial OK) | Fine style control; transparency via LoRA or post-process |

Runtime options (via local HTTP, like Ollama): **stable-diffusion.cpp** (single ggml binary, low memory, easiest), **ComfyUI** (best-quality control, heavier setup), or **`candle`** (pure Rust, in-process — matches Rivulet's engine philosophy, more integration work).

### 5. Emote platform delivery is NOT free/API-able

| Platform | Emote upload | Free API path |
| --- | --- | --- |
| Twitch | Creator Dashboard only (no public upload API) | **7TV** (public REST/GraphQL: emote upload + set management) — viewer-extension based |
| Kick | Streamer Dashboard → Community → Emotes (60 channel / 24 subscriber emotes as Affiliate) | none public (Kick Dev API has no emote-upload endpoint) |
| YouTube | No custom emote API at all | none |

Reachable feature: **generate + validate + export a platform kit** (Twitch/Kick PNGs incl. alpha/28/56/112, 7TV-ready) and, where a free API exists, publish to a **7TV emote set**. Native Twitch/Kick uploads stay a "open dashboard / copy files" manual step.

### 6. Sound generation — free local T2S/TTS (additional feature)

Researched 2026-09-14. The alert/overlay feature set needs three audio classes: short **SFX stingers** (follow/sub/donation, ≤ 3 s), **jingles/stingers** (raid intro, BRB loop, ≤ 47 s) and **spoken read-outs** (donation/subscription text-to-speech). All three have local, free options:

| Model | License | Class | Notes |
| --- | --- | --- | --- |
| **Stable Audio Open Small** (341M) | Stability AI Community License (free commercial use) | SFX/stingers ≤ 11 s | Co-developed with Arm for **on-device** inference; runs on phones, so an 8 GB desktop GPU is comfortable. The alert-stinger default. |
| **Stable Audio Open 1.0** (1.1B) | Stability AI Community License (free commercial use) | Samples/jingles ≤ 47 s | Higher quality, heavier; the quality pick for intros/jingles on 12+ GB cards. |
| **Kokoro-82M** | **Apache-2.0** | TTS read-outs | 82M params, runs fast on CPU, competitive voice quality; the default for donation/sub read-outs. English first; check voice coverage for DE before promising it. |
| AudioLDM 2 | research license (non-commercial) | SFX/music | **Excluded** — license does not fit Rivulet's free-for-everyone positioning. |

Runtime options mirror the T2I decision space: **stable-audio-tools / audio-diffusion-python via local HTTP** (separate process, like ComfyUI), or a **`candle`**-style in-process port (pure Rust — matches the engine philosophy, more integration work; no known Kokoro/stable-audio port today, so phase 1 uses the local-HTTP shape with a small worker). Prompt crafting for SFX is a chat-model strength: the Ollama code model turns "upbeat 1-second donation ding, glassy, not cheesy" into a structured generation prompt (style, duration, BPM, avoid-list), exactly like the emote T2I prompt path.

Delivery: generated audio is trimmed/normalized (44.1 kHz stereo WAV → OGG), stored under `~/.rivulet/creative/sounds/`, and **assigned per alert kind** in the alert settings (reusing the alert-settings surface that overlays already target). A generated sound is previewable in the studio and replaceable by the user's own file at any time — generation is an aid, not a lock-in.

## Kill-switch design (off by default)

All AI features are **off by default** and independently disableable through **Settings**. This is the resource-budget, trust, and failure-isolation boundary at once — a streamer with a mid-range GPU plays without any model resident, and a broken Ollama/T2I install cannot produce error states, only a disabled feature.

### Switch hierarchy

1. **Master switch — "Enable AI features"** (default **off**). Kills the chatbot, the creative studio, and the emote/T2I generator together: no model load, no worker threads, no queues. This is the single kill-path for users who want no AI at all (mirrors Meld Spark being optional).
2. **Per-feature toggles** (default **off**, independent): "AI Chat Assistant", "AI Creative Studio (overlays)", "Emote/T2I generator", "Sound/voice generator". Enabling a per-feature toggle while the master is off is rejected (or implies the master).
3. **Runtime override — "Pause AI while live"** (default **off**). When streaming (Go Live), models suspend; streaming and capture stay unblocked. Combines with the M10 gate's low-priority/cancellation behaviour.

### Persistence & localization

- Switches persist through the existing Settings serialization (serde round-trip, same path as `allow_remote_stream_control`/MIDI/theme settings), localized EN/DE.
- The default-off state and switch round-trip are covered by engine tests; the Settings UI is covered by the GUI contract suite (i18n parity + toggle behaviour).

### Precedent

The direct precedence pattern is the M6 remote-companion precedent: explicit permission gated behind a toggle (`allow_remote_stream_control`), persisted, localized, and pinned by a ci_pinning guard so the three sources (README / gate / spec) cannot drift.

## Settings placement: infrastructure in Settings, workflow on the Assistant tab

AI settings live on **two levels** so the global Settings page does not get
overloaded — the Assistant tab (an existing, currently placeholder M9
navigation view) is the natural home for feature-control that is only
relevant inside the assistant context.

**Settings page (infrastructure + trust boundary, like OBS-WebSocket /
remote-companion):**
- **Master switch "Enable AI features"** (the kill-path, default off).
- **Runtime override "Pause AI while live"** (suspend models on Go Live).
- **Ollama connection**: endpoint (default `http://127.0.0.1:11434`),
  model for orchestrator/code-gen, optional GPU/VRAM budget.
- **Emote/T2I backend**: generator choice (`stable-diffusion.cpp` /
  `ComfyUI` / `candle`), endpoint or model file, per-feature toggle
  "Emote/T2I generator".

**Assistant tab (feature workflow, only while the panel is open):**
- **Per-feature toggles** for "AI Chat Assistant" and "AI Creative Studio
  (overlays)" — context switches the streamer flips next to the chat,
  guarded by the master switch and the resource budget.
- **Studio-local controls**, where the generated overlay lives:
  overlay folder (`~/.rivulet/creative/`), model choice for the current
  session, test-event firing, restore points, and the gallery.

The rule: anything describing **how** the models connect to the machine
(endpoints, engines, VRAM, kill-path) goes to Settings; anything the user
touches **while creating** belongs on the Assistant tab. Both serialize
through the same `AiSwitches`-in-Settings round-trip (see persistence
above) — placement changes nothing about storage, only about which panel
renders the control.

These markers are pinned by `ci_pinning` so the two-level placement stays
true across README / gate / spec:
`infrastructure in Settings, workflow on the Assistant tab` +
`Assistant tab`.

## Proposed architecture (sketch)

### Pipeline (async, driven from the M10 Assistant chat panel)

1. **Understand** — user message routed to the Ollama **orchestrator** LLM (the code model), which classifies: *overlay/widget/scene* request, *modify*, *emote/asset* request, *debug/test*, or plain chat.
2. **Generate code** — for overlays: model emits a self-contained HTML file (embedded CSS/JS), written to `~/.rivulet/creative/<overlay-id>/`. For emotes: model emits a structured T2I prompt (`GenerateRequest`) instead.
3. **Render/preview** — browser source loads the overlay via `file://` URL; it appears in the current scene immediately (like Spark: no approval dialog for new content).
4. **Reactive wiring** — the overlay subscribes to a small host IPC bridge (`window.rivulet.on('follow'|'sub'|'raid'|'chat'|'command', …)`); events pushed from the M5 alert-ingest and chat pipelines.
5. **Refine** — every subsequent message is a new turn over the same overlay: "change the color to #0D1B2A", "make it pulse at 30s", "show subscriber names only". Restore point per turn if the change regresses.
6. **Emote export** — T2I render → alpha extraction (`rembg`/`u2net`, ONNX local) → platform kit (28/56/112 PNG + 7TV-ready) → optional 7TV push via its public API.
7. **Sound generation** — SFX/jingle/TTS request routed to the sound backend (chat model crafts the generation prompt), rendered by the local T2S worker, normalized to OGG, assigned per alert kind; TTS read-outs take the alert's message text directly.

### New engine pieces (all inside `rivulet-core`)

```rust
// Orchestrator: classify request, pick code-vs-image path, hold RAG context per scene.
pub struct CreativeStudio {
    llm: LlmProvider,             // M10: Ollama code model
    overlays: OverlayRegistry,    // id -> (html, source refs, restore points)
    events: AlertPush,            // M5 alert ingest + chat pipeline
    gallery: Vec<GallerySeed>,    // bundled templates + user snapshots
}

// One generated overlay, rendered as a browser source.
pub struct GeneratedOverlay {
    pub id: OverlayId,
    pub html: String,               // self-contained HTML/CSS/JS
    pub scene: SceneSlot,           // which scene + position
    pub restore_points: Vec<RestorePoint>, // per-message undo/redo
}

// T2I backend for the emote/asset sub-feature.
pub trait EmoteGeneratorProvider: Send + Sync {
    fn generate(&self, req: GenerateRequest) -> Result<GeneratedImage, GenerationError>;
}

// T2S/TTS backend for the sound/voice sub-feature (same shape as the T2I
// trait — a local HTTP worker first, in-process port later).
pub trait SoundGeneratorProvider: Send + Sync {
    fn generate(&self, req: SoundRequest) -> Result<GeneratedSound, GenerationError>;
    // `None` when the runtime cannot speak (SFX-only backend).
    fn speak(&self, text: &str, voice: &str) -> Result<GeneratedSound, GenerationError>;
}

// Kill-switch state, serialized into the Settings struct.
pub struct AiSwitches {
    pub master_enabled: bool,           // default false: turns all AI off
    pub assistant_enabled: bool,        // default false
    pub creative_studio_enabled: bool,  // default false
    pub emote_enabled: bool,            // default false
    pub sound_enabled: bool,            // default false: SFX/jingle/TTS generator
    pub pause_while_live: bool,         // default false
}
```

### What Rivulet already has (reuse, not rebuild)

- **Chat adapters** (Twitch IRC/WebSocket, Kick WebSocket, YouTube Live API) — M10 planned / partially shipped (M5 chat dock).
- **Alerts** (`alerts_ingest`, `alerts_webhook`, `alerts_eventsub`, `queue_alert_preview`) — M5 shipped. These are the natural event sources for reactive overlays.
- **Browser source** (`browser_source.rs`, validated contract + config UI, native `wry` adapter as follow-up) — M2 (S5b) shipped; the render surface for generated HTML.
- **Scene manager + obs-websocket server** (scene/source control, remote companion) — M2/M5/M6 shipped.
- **Assistant chat panel** (M10 GUI) — the UI shell where the creative conversation lives.

## Scope boundaries

### In scope (M10 extension)
- Overlay/widget/alert generation from natural language (HTML/CSS/JS), rendered via browser source.
- Live-event reactivity via shipped alert + chat pipelines (Twitch/Kick/YouTube).
- Conversational refinement + restore points per turn.
- Gallery/seed pack, test-event firing, reference-image color pickup.
- **Emote/asset generator** as an additional T2I pipeline with platform export kit (7TV push optional).
- **Sound/voice generator** as an additional T2S/TTS pipeline: alert SFX, jingles and spoken read-outs, assigned per alert kind (research §6).
- **Kill-switch design**: master + per-feature toggles + pause-while-live, default off, persisted, localized.

### Out of scope (phase 1)
- Managing scene/render pipelines end-to-end (source reposition/move between scenes kept for phase 2 mirror-Spark parity).
- Cloud AI providers — contradicts local-first M10 positioning.
- Animated emotes (GIF/webp loop) — later on top of the same pipeline.
- Native Twitch/Kick auto-upload — blocked by platform APIs (documented above).
- Full music generation (songs, long background loops) — Stable Audio 3.0 sizes out for the target hardware; revisit when open weights shrink.
- Live voice cloning of a specific person — consent/ethics minefield with no streaming upside; only the model's stock voices ship.

## Resource budget (M10 quality gate)

The M10 gate (`milestone-quality-gates.md` §M10) already requires local-model CPU/RAM/VRAM budgets and graceful degradation. The Creative Studio inherits the same gate:

- LLM generation runs in the M10 worker (one job at a time, low priority while streaming).
- Browser-source overlay rendering must never block the capture pipeline (already a core S5b requirement: bounded input queue, RGBA frame hand-off).
- T2I (if enabled) runs as a separate process/queue, cancelled when VRAM pressure is detected; studio degrades to "overlay-only" gracefully when the generator is unavailable.
- T2S/TTS (if enabled) shares the separate-process/queue shape; Kokoro-82M is small enough for CPU fallback when VRAM is tight, and the studio degrades to "no generated sound" (user-supplied files keep working) when the worker is unavailable. Audio is rendered **before** Go Live or during low-priority windows — sound generation never blocks the audio mux.
- **Off by default** is a hard resource rule: with the master switch off, no model is loaded, no worker spawned, no queue scheduled — measured by the M10 gate's resource report.

## Open questions

1. **LLM choice** — ✅ decided by the code-gen spike (2026-09-12, RTX 4060 Ti 8 GB,
   harness in [`scripts/codegen-spike/`](../scripts/codegen-spike/README.md)):
   **`qwen2.5-coder:7b` is the default.** 4/5 real overlay prompts produced valid
   single-file overlays (structure, no-CDN, JS-syntax, host-API all pass) at
   8–15 s per prompt after first load. Only miss: donation goal bar (missing CSS
   transition, progress not initialized). `devstral` (14 GB) CPU-offloads on this
   card: 2/5 prompts finished inside 300 s (183–291 s) and 3 timed out — unusable
   for interactive generation on 8 GB, but its two outputs were the most complete
   (goal bar correctly initialized and animated), so **devstral stays the quality
   pick for 16+ GB cards** and for non-interactive batch generation. `qwen3-coder`
   30b/480b remain out of reach for this tier.
2. **Scene integration depth**: phase 1 = overlay into current scene only; phase 2 = full scene packages + layer reference by name (mirror-Spark). Keep phase 2 out of the M10 gate.
3. **Vision**: reference-image color pickup needs an Ollama vision model (`llava` / `qwen3-vl`). Optional in phase 1 (hex palettes accepted as text); do not gate on it.
4. **WASM-plugin tie-in (M11)**: could ship the generator as a host-built plugin later; not required for phase 1.

## Status tracker (mirrors m6-* pattern)

| Phase | Content | Status |
| --- | --- | --- |
| **Scratch** | Feasibility + Spark parity research | ✅ This document |
| **Acceptance** | Accepted into M10 scope; README bullet + quality-gate bullet + ci_pinning guard | ✅ README M10 bullets (creative studio + off-switches), M10 gate bullets, `m10_creative_studio_is_specified_in_readme_gate_and_spec` guard, CHANGELOG |
| **Spike** | Code-gen quality spike (qwen2.5-coder:7b vs devstral) + overlay-in-scene prototype | ✅ Spike done (2026-09-12): `qwen2.5-coder:7b` wins on 8 GB (4/5 valid @ 8–15 s; devstral 2/5 + 3 timeouts). Artifacts + review index in `scripts/codegen-spike/`. Overlay-in-scene prototype still open. |
| **Feature** | Overlay pipeline + reactive wiring + GUI panel | |
| **Emote sub-feature** | T2I backend + platform export kit (7TV push optional) | |
| **Sound sub-feature** | T2S/TTS research (§6: Stable Audio Open Small default, Kokoro TTS) + alert-sound assignment | Research done (2026-09-14); pipeline implementation open |
| **Kill-switch** | `AiSwitches` persistence + Settings UI + default-off tests + pause-while-live hook | |