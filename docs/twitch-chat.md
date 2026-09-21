# Chat Dock (Twitch / Kick / YouTube)

Rivulet can display the live chat of a streaming platform directly in the
app — no browser source or overlay needed for monitoring chat while
streaming. The dock supports **Twitch** (IRC), **Kick** (Pusher WebSocket)
and **YouTube** (Innertube polling) from one unified UI.

## Features

- **Combined multi-platform dock**: the dock holds a list of **accounts**
  (one per platform — Twitch, Kick, YouTube) instead of a single selector.
  Connecting spawns one worker per account (`rivulet_core::MultiChat`), and
  **every platform's messages appear in the same combined list**, each line
  badged with its platform (`[Twitch]`, `[Kick]`, `[YouTube]`). Alerts from
  the ingestion pipeline carry their source platform too (EventSub events
  are Twitch; Streamlabs aggregates several providers and stays unbadged).
  A broadcast send goes to **every capable account at once**, with
  per-platform delivery feedback (rejected legs are named — read-only
  YouTube or a rate-limited account never fail silently). Threaded replies
  are routed back to the platform the parent message came from.
- **Account management**: the "Add" row (platform, channel, optional token)
  appends an account; duplicates per platform are refused with a hint, and
  removing an account re-arms the worker rebuild and deletes its vault
  entry. Accounts persist **only platform and channel**: the account struct
  carries no token field at all — the entered token goes straight into the
  OS credential vault (Windows Credential Manager / kernel keyutils / macOS
  Keychain) and is resolved per-connect via a callback, never stored on the
  roster or in the config file. A failed vault write is reported via a
  status hint. Configs saved before the combined dock migrate their single
  platform/channel/token triple into the list on restore and move the
  legacy token into the vault.
- **Native Twitch IRC client** (`rivulet-core::twitch_chat`): connects to
  `irc.chat.twitch.tv`, handles the CAP/PASS/NICK/JOIN handshake, answers
  PING/PONG keepalives and parses IRCv3 tags (display name, color, badges,
  broadcaster marker) plus `/me` actions.
- **Kick WebSocket client** (`rivulet-core::kick_chat`): resolves the
  channel slug to its chatroom id via the Kick API, connects to the Pusher
  WebSocket, subscribes to the chatroom channel and parses
  `App\Events\ChatMessageEvent` payloads (username, color, badges).
- **YouTube polling client** (`rivulet-core::youtube_chat`): fetches the
  live-chat page to extract the continuation token, then polls the Innertube
  `get_live_chat` endpoint for new messages (author, color, badges). Best
  effort: YouTube's endpoints are not a stable public API, so a failure
  surfaces as a connection error instead of pretending chat works.
- **Chat dock on the Stream page**: the chat is embedded in the **Stream**
  workspace (Meld-style single broadcast page) — left column, with the
  combined alerts dock beside it and the stream status/health to the right.
  Channel input, connect/disconnect, optional token, and a bounded,
  auto-scrolling message list render in the dock; the chat no longer has its
  own sidebar entry.
- **Anonymous by default**: read-only chat works without any token on
  Twitch and Kick. With a token you get colors and badges for your own
  messages.
- **Reply in chat**: Twitch and Kick show a message input under the chat
  list when connected and a token is configured (Twitch OAuth `chat:send`,
  Kick session token) — type and press Enter (or click **Send**). Sending
  is **non-blocking** (enqueued to the worker thread). **YouTube is
  read-only** (anonymous clients cannot send); a hint replaces the input.
- **Send-budget indicator**: directly above the input field the dock shows
  how many platform messages are still allowed right now (e.g. “Send
  budget: 17/20 messages”). The line turns warning-colored at ≤ ¼ capacity
  and is replaced by a pause notice while the bucket is empty, so the
  platform rate limit is visible *before* a send is silently dropped.
  Hovering the line shows which platform limit applies and over what
  window (e.g. “Rate limit for Twitch: 20 messages per 30 s”), so the
  numbers stay compact without hiding the window.
- **Threaded replies (Twitch)**: every Twitch message keeps its IRCv3
  `id=` tag; clicking the **↩** affordance on a message arms a reply
  target (banner: “Replying to <user>”, cancel with ✕) and the next Send
  goes out as `@reply-parent-msg-id=<id> PRIVMSG`, so the bot answers the
  exact line a viewer asked about. Kick/YouTube messages carry no IRC id
  and stay plain sends.
- **Phone-verification notice (Twitch)**: when the server rejects sending
  with `msg_id=msg_requires_verified_phone_number`, the worker sets a flag
  and the dock shows a warning that the bot account must be phone-verified
  before it can chat (cleared on reconnect).
- **Privacy-safe**: tokens are never written to logs or the message model
  (dedicated test) and never persisted to the config file (serde skip +
  JSON-absence test) — they live in the OS credential vault.

## Setup

1. Open the **Stream** tab — the chat dock sits in the left column.
2. Pick the platform (default: Twitch).
3. Enter the target:
   - **Twitch**: channel name without `#`, e.g. `yourtwitchname`.
   - **Kick**: channel slug, e.g. `forsen`.
   - **YouTube**: live video id, e.g. the `v=` value of the live stream URL.
4. Click **Connect**. The status line shows `Connected` / `Disconnected —
   retrying` / `Off`.
5. Optional: paste a token into the token field **before** connecting —
   Twitch OAuth (`oauth:...`, `chat:read` to read with colors, `chat:send`
   to reply) or a Kick session token (`x-sess-token`, required to send).
   YouTube needs no token.

## Configuration notes

- The Twitch channel is lowercased on connect (case-insensitive).
- Kick chatroom ids are resolved once per connect and re-resolved on
  reconnect; the WebSocket URL and API base are overridable (used by the
  tests against local listeners).
- If the worker cannot reach the platform (offline, no network, stream
  ended), it retries with backoff and the view keeps showing the last
  messages.
- The message list is bounded to a fixed maximum (`MAX_CHAT_MESSAGES`) so a
  long stream does not grow memory unboundedly.

### Stream info editor

The combined dock also carries a **stream info editor**: one shared title
and game/category draft with an *apply* button per platform plus an
**Apply to all** button. Each apply runs on a background thread and
reports an honest per-platform outcome:

| Platform | API | Notes |
| --- | --- | --- |
| Twitch | Helix `PATCH /channels` | game name resolved via `GET /games`; needs the EventSub client ID, broadcaster ID and the account token |
| Kick | `PATCH /public/v1/channels` | category name resolved via `/public/v2/categories`; needs the account token |
| YouTube | Data API `videos.update` | title only in practice; the chat account's channel field must hold the live video ID |

A failure on one platform never blocks the others; the failed platform
shows its error inline until the next apply. Tokens are read from the OS
credential vault at apply time and never enter persisted state or logs.

### Twitch Shared Chat attribution

When the joined Twitch channel participates in a **Shared Chat** session
("Stream Together"), messages duplicated from other participating channels
arrive on the normal IRC connection with a `source-room-id` tag naming the
room they were *sent from* (the per-delivery `id` stays unique, the original
message id is preserved in `source-id`). Rivulet parses that tag into
`ChatMessage::source_room_id` and shows a small `↦ <name>` badge before the
sender name, with a translated hover text explaining the origin.

- **Room-name resolution**: the numeric room id is resolved to the channel
  login via a cached Helix `users` lookup
  (`rivulet_core::SharedRoomNameService`). Unknown ids render as the raw id
  and start one batched background lookup (collapsed while in flight, retry
  released when no Twitch credentials are configured); results land in a
  bounded cache (128 entries) so the lookup runs at most once per channel,
  never per frame, and never blocks rendering. Credentials (client ID and
  the Twitch chat account token) are read from the vault at dispatch time
  and never logged or persisted.
- Messages sent in the joined room itself carry no `source-room-id` and
  render exactly as before — outside shared sessions nothing changes.
- Kick and YouTube have no shared-chat equivalent; their parsers always
  leave `source_room_id` empty, and alert previews never grow the badge.

### Outbound rate limiting

Every outbound bot message passes through a shared token-bucket limiter
(`rivulet-core::rate_limit`) before it reaches a worker channel, so the bot
can never burst against a platform limit:

| Platform | Default | Rationale |
| --- | --- | --- |
| Twitch | 20 msgs / 30 s | documented global ceiling for non-broadcaster/mod/VIP accounts |
| Kick | 10 msgs / 30 s | undocumented API — deliberately more conservative than Twitch |
| YouTube | 1 msg / day (burst 1) | official `insert` costs ~200 quota units; serialized sends never overdraw |

- The default applies when `ChatConfig::rate_limit` is `None`; a custom
  `RateLimitConfig { capacity, window_secs }` overrides it per platform.
- `Chat::send_message` drops (and logs) a send when the bucket is empty;
  `Chat::rate_limit_remaining()` exposes the current budget for a status
  line, and `Chat::rate_limit_config()` for the settings UI.
- The bucket is clock-injectable (`RateLimiter::with_clock`) so the unit
  tests are deterministic; the production clock is monotonic.
- The dock reads the live budget through `Chat::rate_limit_remaining()`
  and `Chat::rate_limit_config()` (helpers `chat_rate_limit_detail()` /
  `chat_rate_budget()` in the app) and renders it above the input —
  warning at ≤ ¼ capacity, pause notice while empty. The tooltip
  (`chat_rate_window`) names the platform and the window the limit
  applies over.

### Threaded replies and phone verification (Twitch)

- `ChatMessage.id` carries the Twitch `id=` tag (`None` on Kick/YouTube).
  `TwitchChat::send_reply(text, reply_to_id)` enqueues a
  `Msg::SendReply`; the worker writes
  `@reply-parent-msg-id=<id> PRIVMSG #channel :text` (the tags capability
  is always requested on connect). `Chat::send_reply` forwards it through
  the same shared rate limiter as plain sends and rejects non-Twitch
  platforms.
- `parse_notice(line) -> Option<TwitchNotice>` classifies IRC NOTICE lines
  by their `msg-id` tag. `msg_requires_verified_phone_number` flips
  `TwitchChat::phone_verification_required()` (also forwarded on the
  facade); the dock renders a translated warning while the flag is set.
  Other notices are deliberately ignored.

## Architecture

```
rivulet-core::chat                         # unified facade
  ├─ ChatPlatform (Twitch / Kick / YouTube) / ChatConfig / Chat
  ├─ rate_limit  RateLimiter / RateLimitConfig             # token bucket,
  │                                                         #   per-platform defaults
  ├─ twitch_chat  parse_irc_line -> Option<ChatMessage>   # pure, deterministic
  ├─ kick_chat    parse_kick_event / kick_chatroom_id     # pure + API resolution
  └─ youtube_chat parse_youtube_payload / youtube_initial_continuation
  └─ worker_loop(rx, cfg, stop)            # dedicated thread per platform, I/O here

rivulet-gui::app
  ├─ chat_action_pending / ChatAction          # Connect/Disconnect/Send/SendReply
  ├─ reconcile_chat()                          # one action per frame
  ├─ send_chat_message(text)                   # token + platform gate
  ├─ send_chat_reply(text, parent_id)          # threaded reply (Twitch)
  ├─ chat_rate_budget()                        # live (remaining, capacity)
  ├─ draw_chat_dock(ui, max_list_height)       # platform selector + inputs + reply
  │                                            #   affordance + phone warning +
  │                                            #   send-budget line, embedded in
  │                                            #   the Stream workspace
  └─ i18n keys: chat_* (DE/EN)
```

## Tests

- `rivulet-core`: parser unit tests for all three platforms (Twitch IRC
  tags/colors/badges/`/me`, Kick chat events, YouTube Innertube payloads +
  continuation extraction), privacy-safe serialization, disabled-state
  behavior and end-to-end smoke tests that run the **real workers** against
  local listeners (deterministic in CI, no real network); the token-bucket
  rate limiter is covered by deterministic clock-injected unit tests (burst
  capacity, refill over the window, capacity cap, reset, fractional
  accumulation) and facade tests (platform defaults, drop on exhausted
  limit, read-only platforms do not consume tokens):
  - Twitch: local TCP listener asserts CAP/NICK/JOIN, PING→PONG,
    `PRIVMSG #channel :text` after `send_message`, the threaded
    `@reply-parent-msg-id=<id> PRIVMSG` after `send_reply`, and that a
    `msg_requires_verified_phone_number` NOTICE flips the
    `phone_verification_required()` flag. `parse_notice` is covered by
    unit tests (recognized notice vs. ignored notices/PRIVMSG/PING).
    IRC is line-based and every worker reply is read before the fixture
    closes, so no HTTP-style request draining is needed (documented in
    the fixture).
  - Kick: local HTTP listener serves the chatroom resolution and a local
    WebSocket server (`tungstenite::accept`) delivers a parsed chat event.
  - YouTube: a local HTTP listener serves the initial page (continuation)
    and one `get_live_chat` poll response.
- **Windows RST hygiene**: every local HTTP fixture (YouTube page +
  `get_live_chat` poll, Kick chatroom-resolution endpoints) fully drains
  the incoming request — headers plus any `Content-Length` body
  (`drain_request` / `drain_http_request`) — before answering. Answering
  while request bytes are still unread makes Windows close the socket
  with `WSAECONNRESET` (os error 10054) instead of a clean FIN, which
  intermittently failed the YouTube worker smoke on windows-latest until
  fixed; the Kick WebSocket fixture is unaffected (tungstenite performs
  the full HTTP upgrade read itself).
- `rivulet-gui`: navigation contract (no standalone chat sidebar entry; chat
  is part of the Stream workspace), view coverage and i18n parity, plus
  behavior tests for the send and threaded-reply paths (no token / no
  worker / whitespace / missing parent id rejected, YouTube read-only;
  `submit_chat_input` arms `Send` vs. `SendReply`, clears the input and
  consumes the armed reply target, an empty submit keeps the target, and
  `cancel_chat_reply` (banner ✕) disarms without touching the draft) and
  source-contract tests that the reply input only renders when connected,
  a token is configured and the platform can send, and that the
  “Replying to <user>” banner (translated, above the input) keeps its ✕
  cancel affordance wired to the testable cancel helper.
- `ci_pinning.rs`: guards that the chat dock stays embedded in the Stream
  workspace, each platform worker keeps its local-listener smoke test, the
  send path (`send_message` / `ChatAction::Send` / `PRIVMSG`) stays covered,
  Twitch replies stay threaded (`send_reply` / `@reply-parent-msg-id` /
  `chat_reply_target` / `submit_chat_input` / `ChatAction::SendReply`) with
  the phone-verification flag surfaced in the dock and i18n,  and Kick/YouTube wiring (platform selector, read-only gate, i18n keys,
  docs) cannot silently regress. The send-budget indicator is pinned too
  (`chat_rate_budget` helper + rendered keys + translated strings).

## Roadmap

Chat and alerts are tracked in `docs/obs-vision-roadmap.md` (M5
"Community-Dock": chat for Twitch/Kick/YouTube + alert import via the
existing browser source). Alert overlays (Streamlabs/StreamElements or own
EventSub → overlay) are a follow-up item. Native alert ingestion
(follows/subs/donations/raids parsed from provider webhooks and surfaced in
the chat dock) ships in `rivulet-core::alerts_ingest`, with an opt-in
loopback webhook receiver (`alerts_webhook`) and a native outbound
**EventSub WebSocket** transport (`alerts_eventsub`, `wss://`, no forwarder)
— see [`docs/alerts-ingest.md`](alerts-ingest.md).