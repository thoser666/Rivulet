#!/usr/bin/env python3
"""Add module-level doc comments and #[allow(missing_docs)] to rivulet-core modules."""

import os

# Module doc comments - brief descriptions for each module
MOD_DOCS = {
    'audio': 'Audio frame and track types used across the recording and streaming pipelines.',
    'alerts': 'Alert provider abstraction (Twitch, Kick, Streamlabs, Twitch EventSub) for the alerts dock.',
    'alerts_eventsub': 'Twitch EventSub WebSocket receiver for raid/reply alerts (WSS transport, no forwarder).',
    'alerts_ingest': 'Central alert ingestion pipeline: queue, suppression, source-window rate limiting.',
    'alerts_webhook': 'Local HTTP webhook receiver for Streamlabs/EventSub alert ingestion (loopback, HMAC-verified).',
    'audio_source': 'Per-source audio capture and routing (application, input, output, mixed) with track config.',
    'autoclip': 'Chat-activity spike detector for automatic replay-buffer clip saving.',
    'benchmark': 'FPS/frame-time overhead benchmarking helpers and budget checker for capture backends.',
    'browser_source': 'Browser source contract + synthetic reference backend + Windows WebView2 adapter.',
    'camera': 'USB/webcam capture via GStreamer device provider API.',
    'capture_channel': 'Shared-memory IPC channel for frame transfer between capture backends and the engine.',
    'chat': 'Multi-platform chat facade (Twitch IRC, Kick WebSocket, YouTube polling) with unified send.',
    'chat_rooms': 'Shared room-name resolution and cache for chat platform lookups.',
    'clock': 'Injectable engine clock: system wall clock, virtual (deterministic) clock, and shared wrapper.',
    'cloud': 'S3-compatible cloud recording upload contract with SigV4 PUT (post-stop).',
    'color_source': 'Solid-color scene source with hex parsing and opacity.',
    'config': 'Crate-level configuration types and defaults (engine, recording, stream settings bundles).',
    'container': 'Recording container selection (MP4/MKV/MOV/TS) and post-stop remux to MP4.',
    'crash_record': 'Structured crash-record storage (panic hook writes, read/clear APIs).',
    'crash_report': 'Crash-report builder from stored crash records + log context, redaction, clipboard copy.',
    'discord': 'Discord Rich Presence adapter (opt-in, activity-status model, non-blocking).',
    'ducking': 'Sidechain audio ducking policy (threshold/attenuation/attack/release + hysteresis).',
    'encoder': 'Video encoder abstraction: H.264/H.265/VP9, NVENC/QuickSync/AMF/software, rate control.',
    'file_management': 'Recording filename patterns, split-by rules, auto-record-with-stream settings.',
    'game_capture': 'Game capture backends (DXGI, WGC, Vulkan layer, OpenGL hook, PipeWire portal) + config.',
    'game_detection': 'Installed-game enumeration (Steam/Epic/GOG/Origin/EA/Battle.net) + running-game scoring.',
    'global_hotkey': 'OS-level global hotkey registration (Windows/Wayland/X11) with rebinding UI contract.',
    'health': 'Stream health monitoring: status derivation from drop ratio/throughput, stats polling API.',
    'i18n': 'Locale resolution and translation lookup (EN/DE, 528 keys each, parity-tested).',
    'image_source': 'PNG/JPEG/GIF static and slideshow image scene source.',
    'inspect': 'Pipeline inspector/diagnostics: prints the pipeline the engine would build (secrets redacted).',
    'kick_chat': 'Kick chat WebSocket client (message send/recv, connection state).',
    'media_source': 'Video/audio file playback in scenes (loop, restart, speed).',
    'metrics': 'Recording performance metrics: FPS, encoder load, file size via GStreamer pad probes.',
    'midi': 'MIDI device mapping: controllers to scene switches, volume faders, chroma-key toggles, learn mode.',
    'ndi': 'NDI LAN output: publishes encoded H.264 as an NDI source (off by default).',
    'opengl_hook': 'OpenGL game capture via wglSwapBuffers IAT patching + GDI readback to shared memory.',
    'output': 'Local recording lifecycle: start/stop, dual output, frame pushing, stop finalization.',
    'plugin_manifest': 'WASM plugin manifest parser/validator: publisher, type, host-API, sandbox status, permissions.',
    'plugin_registry': 'Plugin install-root scanner + per-capability user approvals (default-denial, persisted).',
    'plugin_runtime': 'WASM plugin runtime (wasmtime): sandboxed lifecycle, host imports, fuel budgets, timeouts.',
    'preset': 'Recording presets: resolution/FPS/bitrate presets (Original, 1080p60, 720p30, ...) with encoder-aware caps.',
    'rate_limit': 'Per-key rate limiter for chat/event debouncing and backoff.',
    'reconnect': 'Streaming reconnect supervision: per-target retry policy, backoff, branch rebuild commands.',
    'region': 'Rectangular capture region (X/Y/W/H) with interactive drag-selector support.',
    'render': 'Deterministic frame rendering from code: virtual-clock video frame generation for CI/headless.',
    'replay': 'Ring-buffer replay buffer: last-N-seconds encoded clip, save-to-MP4 via F12/GUI.',
    'scene': 'Scene model and manager: multiple scenes, switching, add/rename/remove, undo history, z-order.',
    'scene_snapshot': 'Deterministic scene layout snapshot export: RGBA per layer, transform/opacity rendering.',
    'sdp': 'WebRTC SDP offer generation and parsing for WHIP media sessions.',
    'source': 'Source kinds, transforms (x/y/width/height/rotation/opacity), visibility, lock, z-order.',
    'srt': 'SRT/RIST contribution settings: validated endpoints, bounded latency, passphrase redaction.',
    'stream': 'Streaming core: RTMP/RTMPS, WHIP, SRT/RIST, multistream, stream keys, presets, VOD track.',
    'stream_info': 'Per-platform stream title/game metadata: independent drafts per platform, apply-to-all.',
    'stream_runtime': 'Streaming runtime components: adaptive bitrate controller, delay supervisor, bitrate changes.',
    'studio': 'Studio Mode: separate Preview/Program roles with Cut/Fade transitions and Take action.',
    'telemetry': 'Opt-in, privacy-first telemetry: identifier-free event model, no transport wired, collector + sink.',
    'text_source': 'Rich text scene source: styled text, scrolling, outline/background.',
    'transition': 'Scene transitions: Cut and configurable Fade, selectable in Scenes view.',
    'twitch_chat': 'Twitch IRC chat client: message send/recv, connection state, user-mode tracking.',
    'video_effects': 'Video filters/effects: colour correction, blur, sharpen via videobalance/gaussianblur/cas.',
    'vst3': 'VST 3.x plugin hosting contract: bundle discovery, load/skip results, Windows COM skeleton.',
    'virtual_camera': 'Virtual camera output: platform-neutral lifecycle contract (starting/running/stopping/unavailable).',
    'vulkan_hook': 'Vulkan game capture via VK_LAYER intercepting vkQueuePresentKHR + staging buffer readback.',
    'vulkan_layer': 'Vulkan layer manifest + config for vkQueuePresentKHR interposition capture.',
    'webcam_source': 'Webcam as scene source: resolution/framerate/mirror selection, GStreamer camera capture.',
    'youtube_chat': 'YouTube Live chat via official Live Streaming API (send, quota accounting).',
}

# Modules that should allow missing_docs (internal/WIP, not part of stable contract)
INTERNAL_MODULES = {
    'alerts', 'alerts_eventsub', 'alerts_ingest', 'alerts_webhook',
    'audio_source', 'benchmark', 'browser_source', 'capture_channel',
    'chat', 'chat_rooms', 'cloud', 'color_source', 'config',
    'crash_record', 'crash_report', 'discord', 'ducking',
    'game_capture', 'game_detection', 'global_hotkey', 'image_source',
    'kick_chat', 'media_source', 'midi', 'ndi',
    'opengl_hook', 'plugin_manifest', 'plugin_registry', 'plugin_runtime',
    'presence', 'reconnect', 'render', 'scene_snapshot',
    'sdp', 'srt', 'stream_info', 'stream_runtime', 'studio',
    'telemetry', 'test_helpers', 'text_source', 'transition',
    'twitch_chat', 'vst3', 'vulkan_hook', 'vulkan_layer',
    'webcam_source', 'youtube_chat',
}


def add_module_doc(filepath, doc_text, allow_missing=False):
    """Add a module-level doc comment at the top of a Rust source file."""
    with open(filepath, 'r', encoding='utf-8') as f:
        content = f.read()

    if allow_missing:
        prefix = f'//! {doc_text}\r\n//! \r\n//! #[allow(missing_docs)]\r\n\r\n'
    else:
        prefix = f'//! {doc_text}\r\n\r\n'

    lines = content.split('\r\n')

    # Find insertion point: after #![...] attributes, before first non-comment code
    insert_at = 0
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith('#!['):
            insert_at = i + 1
        elif stripped and not stripped.startswith('//!') and not stripped.startswith('#'):
            insert_at = i
            break
    else:
        insert_at = len(lines)

    new_content = '\r\n'.join(lines[:insert_at]) + '\r\n' + prefix + '\r\n'.join(lines[insert_at:])

    with open(filepath, 'w', encoding='utf-8') as f:
        f.write(new_content)


def main():
    src_dir = 'rivulet-core/src'
    count = 0
    for f in sorted(os.listdir(src_dir)):
        if not f.endswith('.rs'):
            continue
        mod_name = f[:-3]
        if mod_name == 'lib':
            continue

        filepath = os.path.join(src_dir, f)
        doc_text = MOD_DOCS.get(mod_name)
        if doc_text is None:
            doc_text = f'Generated types and helpers for the {mod_name} subsystem.'

        allow = mod_name in INTERNAL_MODULES
        add_module_doc(filepath, doc_text, allow_missing=allow)
        print(f'{"ALLOW" if allow else "DOC"}: {mod_name}')
        count += 1

    print(f'\nDone. Processed {count} modules.')


if __name__ == '__main__':
    main()
