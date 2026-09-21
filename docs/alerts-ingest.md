# Native Alert Ingestion (follows / subs / donations / raids)

Native, provider-neutral alert event ingestion surfaced in the chat dock and
in the dedicated combined **alerts dock**.
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
- **`rivulet-core::alerts_webhook`** — the optional **loopback receiver**:
  - A tiny, dependency-free HTTP/1.1 listener bound to **`127.0.0.1`** (never
    a non-loopback address). Routes: `POST /webhook/streamlabs` (Streamlabs
    donation, no signature) and `POST /eventsub/twitch` (Twitch EventSub,
    HMAC verification against the configured secret; empty secret disables the
    route with `403`). Bodies bounded to 64 KiB; malformed/oversized/forged
    requests are rejected with honest status codes (Twitch retries non-`2xx`).
    Parsed events flow over a bounded channel into the same `AlertIngest`.
  - `AlertsReceiver::start(config)` bind error is surfaced as a Settings
    warning, never a crash. The pure `handle_webhook` handler is testable
    without sockets; two socket-level tests post real HTTP requests to the
    loopback listener (valid EventSub delivery + forged signature).
- **`rivulet-core::alerts_eventsub`** — the native **outbound EventSub
  WebSocket** transport (the live-webhook forwarder from the honest-scope note
  is not needed for Twitch): dials `wss://eventsub.wss.twitch.tv/ws` via
  `tungstenite` + rustls (webpki-roots, the same lockstep TLS stack `ureq`
  already uses), walks the session lifecycle (`session_welcome`/`keepalive`/
  `reconnect`/`revocation`) and pushes `notification` frames through
  `parse_twitch_eventsub_notification` into the same `AlertIngest`. It creates
  the four alert subscriptions (`channel.follow` v2,
  `channel.subscribe`/`channel.subscription.gift`/`channel.raid` v1) against
  the Helix API over TLS using the masked client ID + access token (scopes
  `moderator:read:followers`, `channel:read:subscriptions`; token acquisition/
  refresh is a documented follow-up). Automatic reconnect with backoff; a
  500 ms socket read timeout keeps worker shutdown prompt. The token and
  client ID are never logged, never `Debug`-printed and never embedded in
  events; unknown/rejected frames are only counted.One end-to-end GUI test
  feeds a real follow frame from a local WebSocket puppet into the chat dock.
  The `channel.raid` subscription is created with the **directional**
  condition (Twitch rejects a plain `broadcaster_user_id` for raids). The
  direction is **user-configurable** in the Alerts settings
  (`RaidAlertDirection`): **raid out** (`from_broadcaster_user_id`, the
  default and historical behavior), **raided** (`to_broadcaster_user_id`),
  or **both** (two subscriptions, one per condition field — Twitch's
  condition object cannot hold both at once). The raid parser reads the
  `from_broadcaster_*` payload fields, so raid-*out* alerts name the raiding
  channel; for the *raided* direction the same parser names the incoming
  raider (also carried in `from_broadcaster_user_name` of the delivered
  event, because every raid event is delivered from the raider's
  perspective). Changing the direction updates the worker config, whose
  comparison in `apply_alerts_eventsub` restarts the EventSub WebSocket
  worker so the subscriptions are re-created with the new condition.
- **Kick engagement events** — the Kick chat worker listens to the same
  Pusher chatroom channel for engagement payloads and routes them to a
  dedicated alert channel alongside the chat messages:
  `SubscriptionEvent` becomes `AlertKind::Subscribe` (tier label from
  `subscription_plan_name`, falling back to `subscription_plan`, then the
  default `"Tier 1"`), `GiftedSubscriptionsEvent` becomes
  `AlertKind::GiftSub` (count from the `gifted_usernames` list length,
  falling back to a `quantity` field, minimum 1; the gifter is named from
  `gifter_username` or `username`). The pure `parse_kick_alert_event` parser
  is unit-tested, a local-WebSocket worker test proves both channels deliver
  in parallel, the `MultiChat` facade exposes the receivers via
  `Chat::alerts()`/`MultiChat::alert_receivers()`, and the GUI reconcile
  drains them into the same `AlertIngest` — so Kick subs/gifts appear in
  both docks with the `[Kick]` platform badge, localized by the existing
  alert-kind keys. Unknown/malformed payloads yield no events.
- **Shared Chat sessions** ("Stream Together"): alerts are **not merged**
  across participants. EventSub delivers engagement notifications per
  subscription condition, and `from_broadcaster_user_id` raid subscriptions
  only ever fire for the broadcaster in the condition — so a raid *into* a
  fellow session participant's channel appears only in *their* Rivulet.
  Follows/subs/gifts on other participants' channels are separate
  subscriptions Rivulet deliberately does not create. Alerts keep working
  unchanged inside a session (your own channel's events arrive as usual),
  and the combined dock's per-line platform badge stays accurate because
  alert events are tagged per delivery platform, not per session. Rivulet
  also does not subscribe to the session-lifecycle subscription types
  (`channel.shared_chat.begin`/`update`/`end`) — the dock treats shared
  sessions as a chat-level concern (see the Shared Chat attribution badge
  in `docs/twitch-chat.md`).
- **GUI** — Settings → **Alerts** panel: persisted **ingestion** toggle
  (on by default, purely local), an opt-in **webhook receiver** section
  (enabled toggle, port, masked Twitch EventSub secret) and an opt-in
  **EventSub (WebSocket)** section (masked client ID, masked access token,
  broadcaster user ID, live connection indicator), plus surfacing in two
  places: ingested entries become chat entries with a distinct accent color
  **and** land in the dedicated combined alerts dock (Stream page, middle
  column) — one live list for follows, subs, donations and raids from all
  connected platforms, each line carrying the localized text and the platform
  badge. A **Preview** button queues one deterministic sample per kind for
  layout checks and GUI tests. Missing EventSub credentials never dial out.
- **i18n** — `alert_kind_follow/subscribe/giftsub/donation/raid` plus panel,
  receiver and EventSub keys, EN + DE (parity-enforced; 512 keys total).
- **Privacy tests** — no `Serialize` on events, `Debug` never leaks entry
  contents, `AlertEvent::sanitize_message` strips control characters, tokens
  and secrets are never embedded in events or logged.

## Honest scope

The **webhook receiver listens on `127.0.0.1` only** and is off by default.
Real Twitch/Streamlabs deliveries arrive over **public HTTPS**, so a live
webhook setup needs a local HTTPS terminator, a tunnel, or a reverse proxy in
front that forwards provider POSTs to the loopback port — that forwarder is a
documented follow-up, exactly like the telemetry transport. (Twitch only: the
**EventSub WebSocket** transport in `alerts_eventsub` is the forwarder-free
path — it dials out to Twitch directly, needs no shared secret and needs only
a masked client ID + user token.) Because the receiver is loopback bounded and
the parsers/signature/queue are network-free, everything is still testable
deterministically; wiring a public transport later is a drop-in:
parse → verify → `AlertIngest::push`.

## Files

- `rivulet-core/src/alerts_ingest.rs` — event model, parsers, signature
  verification, bounded queue (+17 deterministic unit tests)
- `rivulet-core/src/alerts_webhook.rs` — loopback webhook receiver, pure
  `handle_webhook` handler, `AlertsReceiver` (+11 tests incl. two socket-level
  loopback deliveries)
- `rivulet-core/src/alerts_eventsub.rs` — outbound EventSub WebSocket
  transport, protocol parser, Helix subscription creation, worker with
  reconnect (12 unit/protocol tests + one WebSocket-puppet end-to-end)
- `rivulet-gui/src/app.rs` — `alert_ingest`/`alert_ingest_enabled` +
  `alerts_receiver_enabled`/`alerts_receiver_port`/`alerts_twitch_secret` +
  `alerts_eventsub_enabled`/`alerts_eventsub_client_id`/`alerts_eventsub_token`/
  `alerts_eventsub_broadcaster_id` fields, `apply_alerts_policy` /
  `apply_alerts_receiver` / `apply_alerts_eventsub`, per-frame drain into the
  bounded chat list **and** the bounded alerts dock (`alert_events`,
  `MAX_ALERT_EVENTS`), `queue_alert_preview` / `alert_event_to_chat_message`,
  `draw_alerts_dock` / `clear_alert_events`, Settings panels, chat-dock
  preview button (+16 GUI tests incl. loopback
  Receiver/Streamlabs-end-to-end and EventSub-puppet-end-to-end)
- `rivulet-core/src/kick_chat.rs` — engagement-event parsing
  (`parse_kick_alert_event`) and the worker alert channel wired from the
  Pusher chat stream (+5 parser tests, +1 worker e2e test)
- `rivulet-core/tests/ci_pinning.rs` — guards
  `m5_alerts_ingest_is_native_localized_and_pinned` (the ci_pinning guard
  pins the README/roadmap markers, the doc contents and the GUI wiring) and
  `kick_engagement_alerts_feed_the_alerts_dock`
- `docs/obs-vision-roadmap.md` — M5 row marked **Done**