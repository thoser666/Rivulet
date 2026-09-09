# Native Alert Ingestion (follows / subs / donations / raids)

Native, provider-neutral alert event ingestion surfaced in the chat dock.
Implements the M5 roadmap row **Alerts (follows/subs/donations)** — event
ingestion (Twitch EventSub or provider webhooks) mapped to localized alert
entries, with redaction of tokens and privacy tests.

## What ships

- **`rivulet-core::alerts_ingest`** — the deterministic ingestion contract:
  - `AlertEvent` / `AlertKind`: flat, privacy-safe event model (user names,
    optional recipient, counts, tier label, donation amount/currency, optional
    viewer message). `AlertEvent` deliberately has **no `Serialize` impl** so
    an event can never be serialized out of the app.
  - **Parsers**: `parse_streamlabs_webhook` (Streamlabs donation webhooks) and
    `parse_twitch_eventsub_notification` (`channel.follow`, `channel.subscribe`,
    `channel.subscription.gift`, `channel.raid`), both network-free and
    unit-tested with realistic payloads.
  - `verify_twitch_eventsub_signature`: **HMAC-SHA-256** verification per the
    Twitch spec (`sha256=`-prefixed hex over
    `message-id || message-timestamp || raw-body`), compared in constant time,
    with a pinned test vector.
  - `AlertIngest`: **bounded** local queue (default capacity 64, oldest
    dropped), enabled by default, `Debug` renders settings/counters **only** —
    never entry contents.
- **GUI** — Settings → **Alerts** panel (persisted toggle, local-only note)
  and chat-dock surfacing: ingested entries become chat entries with a
  distinct accent color; a **Preview** button queues one deterministic sample
  per kind for layout checks and GUI tests.
- **i18n** — `alert_kind_follow/subscribe/giftsub/donation/raid` plus panel
  keys, EN + DE (parity-enforced; 471 keys total).
- **Privacy tests** — no `Serialize` on events, `Debug` never leaks entry
  contents, `AlertEvent::sanitize_message` strips control characters, tokens
  and secrets are never stored or logged.

## Honest scope

This ships **no network receiver**. The HTTPS endpoint those webhook payloads
would arrive on, or an EventSub WebSocket connection, is a **documented
follow-up** — exactly like the telemetry transport. Nothing in the shipped
build listens on any socket for alerts; the parsers, signature verification and
queue are testable deterministically precisely because they are network-free.
Wiring a transport later is a drop-in: parse → verify → `AlertIngest::push`.

## Files

- `rivulet-core/src/alerts_ingest.rs` — event model, parsers, signature
  verification, bounded queue (+17 deterministic unit tests)
- `rivulet-gui/src/app.rs` — `alert_ingest`/`alert_ingest_enabled` fields,
  `apply_alerts_policy`, per-frame drain into the bounded chat list,
  `queue_alert_preview` / `alert_event_to_chat_message`, Settings panel,
  chat-dock preview button (+4 GUI tests)
- `rivulet-core/tests/ci_pinning.rs` — guard
  `m5_alerts_ingest_is_native_localized_and_pinned` (the ci_pinning guard
  pins the README/roadmap markers, the doc contents and the GUI wiring)
- `docs/obs-vision-roadmap.md` — M5 row marked **Done**