# Mobile & HTTP remote companion (phone / browser)

Rivulet ships a tiny HTTP page server — the *remote companion* — so a phone or a
browser on the same network can switch scenes, start/stop recording, and
(gated) start/stop streaming. The page is fully self-contained and talks to the
already-shipped OBS WebSocket v5 server (`docs/obs-websocket.md`), so the phone
drives Rivulet through the **same authenticated surface** Stream Deck and
TouchPortal already use — no second protocol.

- Issue: [#99](https://github.com/thoser666/Rivulet/issues/99) (M6 roadmap)
- Page: plain HTML/CSS/JS served by `rivulet-obs-websocket::companion`
- Wire format reference: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>

## What is implemented

| Area | Behaviour |
|---|---|
| HTTP server | Serves the mobile page at `GET /` and a `{"wsPort","authRequired"}` JSON at `GET /config` |
| Bind | Loopback (`127.0.0.1`) by default; explicit **Allow access from the network (LAN)** widens the bind for the page **and** the OBS WebSocket server |
| Login | v5 SHA-256 challenge/response over WebSocket, identical to any obs-websocket client; without a password the page connects anonymously |
| Scenes | Loads and switches scenes (works without permission) |
| Recording | Start/stop recording (works without permission) |
| Streaming | Start/stop/toggle streaming — **only with the explicit permission flag** (see below) |
| Security headers | `Content-Security-Policy` (`default-src 'none'`, inline script/style, same-origin + `ws:`/`wss:` only), `X-Content-Type-Options: nosniff`, `Cache-Control: no-store` |
| No access logging | Passwords, request bodies, and identifiers are never written to logs |

The page is dependency-free: no CDN, no external fetch. Because a LAN page is
usually served over plain `http://` (not a secure context), the Web Crypto API
is used when available and a compact **pure-JS SHA-256** acts as fallback — it
self-tests against the FIPS 180-4 `"abc"` vector on load.

## How to enable

1. Open **Settings → Remote companion (phone / browser)** and tick **Enable
   phone / browser remote**.
2. The page runs while the **OBS WebSocket server above is enabled** — it is
   the transport the phone connects through.
3. Choose a **page port** (default `4456`).
4. Keep **Allow access from the network (LAN)** off for local-only use. To
   reach the page from a phone:
   - Tick **Allow access from the network (LAN)** (both listeners bind to
     `0.0.0.0`), and
   - set a **password** on the OBS WebSocket server above — a LAN bind without
     a password is refused, in the GUI and by the server crate.
5. To let the phone start/stop the stream, also tick **Allow remote stream
   start / stop**. Without it the page can still switch scenes and record.
6. On the phone, open `http://<PC-LAN-IP>:4456` (the Settings status line shows
   the address; the **Open page** button opens loopback here on the PC).

## Security model

- **Loopback first.** Nothing listens on the network unless "Allow access from
  the network (LAN)" is explicitly on.
- **LAN requires a password.** The obs-websocket server refuses to bind to
  `0.0.0.0` without a non-empty password (`io::ErrorKind::InvalidInput`), so a
  LAN page always faces the authenticated challenge/response handshake.
- **Stream control is an explicit permission.** `StartStreaming`,
  `StopStreaming`, and `ToggleStreaming` are denied with status `205`
  (`GenericError`, comment `Remote stream start/stop requires explicit
  permission`) whenever the obs server is bound beyond loopback and the
  permission flag is off. The denial is
  enforced by the **server crate**, not by the page — a hand-rolled client gets
  the same answer. Loopback retains the M4/M5 behaviour.
- The page connects over WebSocket (`ws:`/`wss:`), which the CSP allows; the
  password lives in `sessionStorage` and is only ever used for the challenge
  and never persisted or logged.

> **Compatibility / risk boundary:** this is a *remote control* surface. It does
> not stream video to the phone, does not expose settings, and never registers
> webhooks or external accounts. It is a plain HTTP + obs-websocket v5 surface
> on your LAN.

## Verification

- Unit tests in `src/companion.rs` (GET-only path parsing, loopback/4456
  default config).
- **End-to-end smoke** in `tests/companion_smoke.rs`: a real TCP + tungstenite
  client serves the page and checks the security headers over loopback, reads
  `/config`, verifies the shutdown releases the port, then binds a second
  server **on the LAN** (`BindAddress::All`) and proves the permission gate
  from a real client: stream control denied without the flag, allowed with it,
  denied for a missing/wrong password (close code 4009), and a passwordless
  LAN bind refused at startup.
- **CI loopback smoke** — the `Remote Companion Smoke` job in
  `.github/workflows/ci.yml` runs
  `cargo test -p rivulet-obs-websocket --test companion_smoke` on every push.
  The job is wired into the required `CI` aggregate, and
  `rivulet-core/tests/ci_pinning.rs` (`m6_remote_companion_is_wired_up_and_pinned`)
  fails if the wiring, the permission gate, the GUI surface, the i18n keys, or
  the roadmap marker drift.
- **GUI integration**: toggle/port/bind/permission persistence is covered by
  the GUI test harness, and i18n keys exist in both locales.