# Twitch Chat Dock

Rivulet can display the Twitch chat of a channel directly in the app — no
browser source or overlay needed for monitoring chat while streaming.

## Features

- **Native IRC client** (`rivulet-core::twitch_chat`): connects to
  `irc.chat.twitch.tv`, handles the CAP/PASS/NICK/JOIN handshake, answers
  PING/PONG keepalives and parses IRCv3 tags (display name, color, badges,
  broadcaster marker) plus `/me` actions.
- **Chat view** (`Chat` tab in the sidebar): channel input, connect/
  disconnect, optional OAuth token, and a bounded, auto-scrolling message
  list that renders each user in their Twitch color.
- **Anonymous by default**: read-only chat works without any token (Twitch
  allows anonymous IRC reads). With an OAuth token (`chat:read`) you get
  colors and badges for your own messages.
- **Reply in chat**: with an OAuth token that has the **`chat:send`** scope,
  a message input appears under the chat list — type a message and press
  Enter (or click **Send**) and it is written to the joined channel via
  `PRIVMSG`. Sending is **non-blocking** (enqueued to the worker thread) and
  the input is only enabled while connected **and** a token is configured,
  because Twitch rejects messages from the anonymous read-only nick.
- **Privacy-safe**: tokens are never written to logs or the message model;
  the message serialization is covered by a dedicated test.

## Setup

1. Open the **Chat** tab.
2. Enter the channel name (without `#`), e.g. `yourtwitchname`.
3. Click **Connect**. The status line shows `Connected` / `Disconnected —
   retrying` / `Off`.
4. Optional: paste an OAuth token (`oauth:...`) with the `chat:read` scope
   into the token field **before** connecting. To **reply** in the chat, the
   token needs the additional **`chat:send`** scope (generate one at
   https://twitchtokengenerator.com or in your Twitch developer console);
   without it the message input stays disabled with a hint.

## Configuration notes

- The channel is lowercased on connect (Twitch channel names are
  case-insensitive).
- If the worker cannot reach Twitch (offline, no network), it retries with
  backoff and the view keeps showing the last messages.
- The message list is bounded to a fixed maximum (`MAX_CHAT_MESSAGES`) so a
  long stream does not grow memory unboundedly.

## Architecture

```
rivulet-core::twitch_chat
  ├─ parse_irc_line(line) -> Option<ChatMessage>   # pure, deterministic
  ├─ TwitchChatConfig / ChatConnState / ChatMessage (serde)
  └─ worker_loop(rx, cfg, stop)                    # dedicated thread, I/O here

rivulet-gui::app
  ├─ chat_action_pending / ChatAction               # Connect/Disconnect/Send
  ├─ reconcile_twitch_chat()                        # one action per frame
  ├─ send_chat_message(text)                        # token + worker gate
  ├─ draw_chat_view()                               # inputs + message list + reply
  └─ i18n keys: chat_* (DE/EN)
```

## Tests

- `rivulet-core`: parser unit tests (tags, colors, badges, `/me` actions,
  missing-tag fallback), privacy-safe serialization, disabled-state behavior
  and an end-to-end smoke test that runs the worker against a **local TCP
  listener** (deterministic in CI, no real network) — including sending:
  the listener asserts the worker writes `PRIVMSG #channel :text` back on
  the same socket after `send_message`.
- `rivulet-gui`: navigation contract (`AppView::Chat`, `nav_chat`), view
  coverage and i18n parity, plus behavior tests for the send gate (no token
  / no worker / whitespace are rejected) and a source-contract test that the
  reply input only renders when connected and a token is configured.
- `ci_pinning.rs`: guard that the chat view stays wired into the sidebar,
  the worker keeps its local-listener smoke test, and the send path
  (`send_message` / `ChatAction::Send` / `PRIVMSG`) stays covered.

## Roadmap

Chat and alerts are tracked in `docs/obs-vision-roadmap.md` (M5
"Community-Dock": Twitch chat + alert import via the existing browser
source). Alert overlays (Streamlabs/StreamElements or own EventSub → overlay)
are a follow-up item.
