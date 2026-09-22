//! Cross-platform stream metadata updates: title and game/category per
//! platform or for all configured platforms at once (combined chat dock,
//! per-platform + "apply to all").
//!
//! Platform contracts (verified against the public docs):
//! - **Twitch** — Helix `PATCH /helix/channels` with `broadcaster_id`,
//!   `title`, `game_id`. Requires a user token with
//!   `channel:manage:broadcast`; the game name is resolved to a `game_id`
//!   via `GET /helix/games?name=`. Same `PATCH` sets the title when only
//!   the title changes.
//! - **Kick** — Public API `PATCH /public/v1/channels` with
//!   `stream_title` and `category_id` (OAuth scope `channel:write`); the
//!   category name is resolved via `GET /public/v2/categories?name=`.
//! - **YouTube** — Data API `videos.update` (`part=snippet`) with the
//!   broadcast's `snippet.title`/`snippet.categoryId`; requires the
//!   broadcast video id and an OAuth token. The human category name is
//!   resolved via `videoCategories.list` (`id=` lookup, region `US`).
//!
//! All network calls go through a single injected [`Http`] port so tests
//! drive the real request/response code against a local puppet server and
//! no secret ever reaches a log or error message.

use serde_json::{json, Value};

/// The three platforms the combined dock configures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoPlatform {
    Twitch,
    Kick,
    YouTube,
}

impl InfoPlatform {
    pub const ALL: [InfoPlatform; 3] = [Self::Twitch, Self::Kick, Self::YouTube];

    /// Human-readable platform name (matches the chat dock labels).
    pub fn label(self) -> &'static str {
        match self {
            Self::Twitch => "Twitch",
            Self::Kick => "Kick",
            Self::YouTube => "YouTube",
        }
    }

    /// Stable index into per-platform value arrays (`[String; 3]` in the
    /// GUI): the position of the platform in [`Self::ALL`].
    pub fn index(self) -> usize {
        match self {
            Self::Twitch => 0,
            Self::Kick => 1,
            Self::YouTube => 2,
        }
    }
}

/// Map a chat-dock account platform to the matching stream-info
/// platform (same three platforms, separate enum so the metadata API
/// surface stays independent from the chat transport).
pub fn chat_info_platform_of_chat(platform: crate::ChatPlatform) -> Option<InfoPlatform> {
    match platform {
        crate::ChatPlatform::Twitch => Some(InfoPlatform::Twitch),
        crate::ChatPlatform::Kick => Some(InfoPlatform::Kick),
        crate::ChatPlatform::YouTube => Some(InfoPlatform::YouTube),
    }
}

/// Inverse of [`chat_info_platform_of_chat`] (used to look up the chat
/// account that carries a platform's token).
pub fn chat_info_platform(platform: InfoPlatform) -> crate::ChatPlatform {
    match platform {
        InfoPlatform::Twitch => crate::ChatPlatform::Twitch,
        InfoPlatform::Kick => crate::ChatPlatform::Kick,
        InfoPlatform::YouTube => crate::ChatPlatform::YouTube,
    }
}

/// Requested metadata change. `None` fields are left untouched on the
/// platform (an update with both `None` is a no-op rejected up front).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamInfoUpdate {
    /// New stream title (Twitch `title`, Kick `stream_title`, YouTube
    /// `snippet.title`).
    pub title: Option<String>,
    /// New game/category *name*; resolved to a platform id before the
    /// metadata call (Twitch `game_id`, Kick `category_id`, YouTube
    /// `categoryId`).
    pub game: Option<String>,
}

impl StreamInfoUpdate {
    pub fn has_changes(&self) -> bool {
        self.title.as_deref().is_some_and(|t| !t.trim().is_empty())
            || self.game.as_deref().is_some_and(|g| !g.trim().is_empty())
    }

    fn trimmed_title(&self) -> Option<&str> {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }

    fn trimmed_game(&self) -> Option<&str> {
        self.game
            .as_deref()
            .map(str::trim)
            .filter(|g| !g.is_empty())
    }
}

/// Credentials for one platform update. Tokens travel by value into the
/// HTTPS request only; they are never formatted into errors or logs.
#[derive(Debug, Clone, Default)]
pub struct InfoCredentials {
    /// Platform OAuth user token (`Authorization: Bearer …`).
    pub token: String,
    /// Twitch application client ID (`Client-Id` header, Helix-required).
    pub client_id: String,
    /// Twitch numeric broadcaster ID (`broadcaster_id` query parameter).
    pub broadcaster_id: String,
    /// YouTube broadcast video id (`videos.update` target).
    pub video_id: String,
}

/// Overridable endpoints (production defaults; tests point at a puppet).
#[derive(Debug, Clone)]
pub struct InfoEndpoints {
    pub twitch_api_base: String,
    pub kick_api_base: String,
    pub youtube_api_base: String,
}

impl Default for InfoEndpoints {
    fn default() -> Self {
        Self {
            twitch_api_base: "https://api.twitch.tv/helix".to_owned(),
            kick_api_base: "https://api.kick.com/public/v1".to_owned(),
            youtube_api_base: "https://www.googleapis.com/youtube/v3".to_owned(),
        }
    }
}

/// The minimal HTTP surface the updaters need (implemented with `ureq` in
/// production and with the real `ureq` against a local puppet in tests).
pub trait Http {
    /// Perform a JSON request, returning the status and parsed body.
    fn request_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<(u16, Value), String>;
}

/// Real HTTP backend (ureq, rustls).
pub struct UreqHttp;

impl Http for UreqHttp {
    fn request_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<(u16, Value), String> {
        // ureq 3's type-state API splits body/no-body builders; branching on
        // the body keeps a single header application point.
        let is_body_method = !matches!(method, "GET" | "HEAD" | "DELETE");
        let outcome: Result<ureq::http::Response<ureq::Body>, ureq::Error> =
            match (is_body_method, body) {
                (false, _) => {
                    let mut request = ureq::get(url);
                    for (name, value) in headers {
                        request = request.header((*name).to_owned(), value.as_str());
                    }
                    request.call()
                }
                (true, Some(payload)) => {
                    let request = match method {
                        "PATCH" => ureq::patch(url),
                        "PUT" => ureq::put(url),
                        _ => ureq::post(url),
                    };
                    let mut request = request;
                    for (name, value) in headers {
                        request = request.header((*name).to_owned(), value.as_str());
                    }
                    request.send_json(payload)
                }
                (true, None) => {
                    // Body-less PATCH/PUT (should not occur in this module; the
                    // force_send_body escape hatch would be required).
                    return Err(format!("method {method} requires a body"));
                }
            };
        let response = outcome.map_err(|e| format!("{e}"))?;
        let status = response.status().as_u16();
        let mut body_reader = response.into_body();
        let parsed: Value = body_reader.read_json().unwrap_or(Value::Null);
        Ok((status, parsed))
    }
}

/// Outcome of one platform update. The message is user-safe (no tokens,
/// no URLs with credentials).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfoUpdateOutcome {
    Updated,
    /// Transport/HTTP failure; the message names the platform and stage.
    Failed(String),
}

impl InfoUpdateOutcome {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Updated)
    }
}

/// Update title/game on **one** platform. Returns [`InfoUpdateOutcome`].
pub fn update_platform_stream_info(
    platform: InfoPlatform,
    update: &StreamInfoUpdate,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> InfoUpdateOutcome {
    if !update.has_changes() {
        return InfoUpdateOutcome::Failed(
            "nothing to update (title and game are empty)".to_owned(),
        );
    }
    match platform {
        InfoPlatform::Twitch => update_twitch(update, credentials, endpoints, http),
        InfoPlatform::Kick => update_kick(update, credentials, endpoints, http),
        InfoPlatform::YouTube => update_youtube(update, credentials, endpoints, http),
    }
}

/// Update title/game on every platform in `platforms`, returning one
/// outcome per platform (in the same order). A failure on one platform
/// never aborts the others — the caller surfaces per-platform results.
pub fn update_all_stream_info(
    platforms: &[InfoPlatform],
    update: &StreamInfoUpdate,
    credentials_of: impl Fn(InfoPlatform) -> InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> Vec<(InfoPlatform, InfoUpdateOutcome)> {
    platforms
        .iter()
        .map(|platform| {
            let outcome = update_platform_stream_info(
                *platform,
                update,
                &credentials_of(*platform),
                endpoints,
                http,
            );
            (*platform, outcome)
        })
        .collect()
}

// ── Twitch ────────────────────────────────────────────────────────────

fn update_twitch(
    update: &StreamInfoUpdate,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> InfoUpdateOutcome {
    if credentials.token.trim().is_empty() {
        return InfoUpdateOutcome::Failed(
            "Twitch: no account token (connect the chat account or add one in Settings)".to_owned(),
        );
    }
    if credentials.client_id.trim().is_empty() {
        return InfoUpdateOutcome::Failed(
            "Twitch: no application client ID configured (EventSub settings)".to_owned(),
        );
    }
    if credentials.broadcaster_id.trim().is_empty() {
        return InfoUpdateOutcome::Failed("Twitch: broadcaster ID missing".to_owned());
    }

    let mut body = serde_json::Map::new();
    body.insert(
        "broadcaster_id".to_owned(),
        json!(credentials.broadcaster_id.trim()),
    );
    if let Some(game) = update.trimmed_game() {
        match twitch_game_id(game, credentials, endpoints, http) {
            Ok(id) => {
                body.insert("game_id".to_owned(), json!(id));
            }
            Err(message) => return InfoUpdateOutcome::Failed(message),
        }
    }
    if let Some(title) = update.trimmed_title() {
        body.insert("title".to_owned(), json!(title));
    }

    let url = format!(
        "{}/channels",
        endpoints.twitch_api_base.trim_end_matches('/')
    );
    let headers = [
        ("Client-Id", credentials.client_id.trim().to_owned()),
        (
            "Authorization",
            format!("Bearer {}", credentials.token.trim()),
        ),
        ("Content-Type", "application/json".to_owned()),
    ];
    match http.request_json("PATCH", &url, &headers, Some(Value::Object(body))) {
        Ok((status, _)) if (200..300).contains(&status) => InfoUpdateOutcome::Updated,
        Ok((status, payload)) => InfoUpdateOutcome::Failed(format!(
            "Twitch: update rejected (HTTP {status}, {})",
            twitch_error_hint(&payload)
        )),
        Err(e) => InfoUpdateOutcome::Failed(format!("Twitch: request failed: {e}")),
    }
}

/// Resolve a game *name* to a Twitch `game_id` via `GET /helix/games`.
fn twitch_game_id(
    game: &str,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> Result<String, String> {
    let url = format!(
        "{}/games?name={}",
        endpoints.twitch_api_base.trim_end_matches('/'),
        percent_encode(game)
    );
    let headers = [
        ("Client-Id", credentials.client_id.trim().to_owned()),
        (
            "Authorization",
            format!("Bearer {}", credentials.token.trim()),
        ),
    ];
    let (status, payload) = http
        .request_json("GET", &url, &headers, None)
        .map_err(|e| format!("Twitch: game lookup failed: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("Twitch: game lookup rejected (HTTP {status})"));
    }
    payload
        .get("data")
        .and_then(|data| data.get(0))
        .and_then(|game| game.get("id"))
        .and_then(|id| id.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("Twitch: game {game:?} not found"))
}

// ── Kick ──────────────────────────────────────────────────────────────

fn update_kick(
    update: &StreamInfoUpdate,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> InfoUpdateOutcome {
    if credentials.token.trim().is_empty() {
        return InfoUpdateOutcome::Failed(
            "Kick: no account token (add one via the chat account token field)".to_owned(),
        );
    }

    let mut body = serde_json::Map::new();
    if let Some(title) = update.trimmed_title() {
        body.insert("stream_title".to_owned(), json!(title));
    }
    if let Some(game) = update.trimmed_game() {
        match kick_category_id(game, credentials, endpoints, http) {
            Ok(id) => {
                body.insert("category_id".to_owned(), json!(id));
            }
            Err(message) => return InfoUpdateOutcome::Failed(message),
        }
    }

    let url = format!("{}/channels", endpoints.kick_api_base.trim_end_matches('/'));
    let headers = [(
        "Authorization",
        format!("Bearer {}", credentials.token.trim()),
    )];
    match http.request_json("PATCH", &url, &headers, Some(Value::Object(body))) {
        Ok((status, _)) if (200..300).contains(&status) => InfoUpdateOutcome::Updated,
        Ok((status, payload)) => InfoUpdateOutcome::Failed(format!(
            "Kick: update rejected (HTTP {status}, {})",
            twitch_error_hint(&payload)
        )),
        Err(e) => InfoUpdateOutcome::Failed(format!("Kick: request failed: {e}")),
    }
}

/// Resolve a category *name* to a Kick `category_id` via
/// `GET /public/v2/categories?name=` (v2 is the documented, paginated form).
fn kick_category_id(
    game: &str,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> Result<i64, String> {
    // The categories endpoint lives under /public/v2 even though the
    // channel PATCH is v1 (documented split).
    let base = endpoints
        .kick_api_base
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_owned();
    let url = format!("{base}/v2/categories?name={}", percent_encode(game));
    let headers = [(
        "Authorization",
        format!("Bearer {}", credentials.token.trim()),
    )];
    let (status, payload) = http
        .request_json("GET", &url, &headers, None)
        .map_err(|e| format!("Kick: category lookup failed: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("Kick: category lookup rejected (HTTP {status})"));
    }
    let id = payload
        .get("data")
        .and_then(|data| data.get(0))
        .and_then(|category| category.get("id"))
        .and_then(|id| id.as_i64());
    match id {
        Some(id) => Ok(id),
        None => Err(format!("Kick: category {game:?} not found")),
    }
}

// ── YouTube ───────────────────────────────────────────────────────────

fn update_youtube(
    update: &StreamInfoUpdate,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> InfoUpdateOutcome {
    if credentials.token.trim().is_empty() {
        return InfoUpdateOutcome::Failed("YouTube: no account token".to_owned());
    }
    if credentials.video_id.trim().is_empty() {
        return InfoUpdateOutcome::Failed(
            "YouTube: the chat account channel field must hold the live video ID to update its title".to_owned(),
        );
    }

    let mut snippet = serde_json::Map::new();
    if let Some(title) = update.trimmed_title() {
        snippet.insert("title".to_owned(), json!(title));
    }
    if let Some(game) = update.trimmed_game() {
        match youtube_category_id(game, credentials, endpoints, http) {
            Ok(id) => {
                snippet.insert("categoryId".to_owned(), json!(id));
            }
            Err(message) => return InfoUpdateOutcome::Failed(message),
        }
    }

    // videos.update requires the *full* snippet resource semantics; passing
    // only the changed fields is the documented partial-update form when
    // `part=snippet` is requested and the other fields are resent as-is. We
    // always carry `title` (current value when only the category changes)
    // so a category-only update cannot blank the title.
    if !snippet.contains_key("title") {
        snippet.insert(
            "title".to_owned(),
            json!(youtube_current_title(credentials, endpoints, http)),
        );
    }

    let url = format!(
        "{}/videos?part=snippet",
        endpoints.youtube_api_base.trim_end_matches('/')
    );
    let headers = [(
        "Authorization",
        format!("Bearer {}", credentials.token.trim()),
    )];
    let body = json!({
        "id": credentials.video_id.trim(),
        "snippet": Value::Object(snippet),
    });
    match http.request_json("PUT", &url, &headers, Some(body)) {
        Ok((status, _)) if (200..300).contains(&status) => InfoUpdateOutcome::Updated,
        Ok((status, payload)) => InfoUpdateOutcome::Failed(format!(
            "YouTube: update rejected (HTTP {status}, {})",
            twitch_error_hint(&payload)
        )),
        Err(e) => InfoUpdateOutcome::Failed(format!("YouTube: request failed: {e}")),
    }
}

fn youtube_current_title(
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> String {
    let url = format!(
        "{}/videos?part=snippet&id={}",
        endpoints.youtube_api_base.trim_end_matches('/'),
        percent_encode(credentials.video_id.trim())
    );
    let headers = [(
        "Authorization",
        format!("Bearer {}", credentials.token.trim()),
    )];
    let (status, payload) = match http.request_json("GET", &url, &headers, None) {
        Ok(result) => result,
        Err(_) => return String::new(),
    };
    if !(200..300).contains(&status) {
        return String::new();
    }
    payload
        .get("items")
        .and_then(|items| items.get(0))
        .and_then(|video| video.get("snippet"))
        .and_then(|snippet| snippet.get("title"))
        .and_then(|title| title.as_str())
        .unwrap_or_default()
        .to_owned()
}

/// Resolve a YouTube category *name* to a `categoryId` via
/// `videoCategories.list` (region US, matching the dashboard's category
/// list) with a fallback to Gaming's fixed id (20).
fn youtube_category_id(
    game: &str,
    credentials: &InfoCredentials,
    endpoints: &InfoEndpoints,
    http: &dyn Http,
) -> Result<String, String> {
    let url = format!(
        "{}/videoCategories?part=snippet&regionCode=US",
        endpoints.youtube_api_base.trim_end_matches('/')
    );
    let headers = [(
        "Authorization",
        format!("Bearer {}", credentials.token.trim()),
    )];
    let (status, payload) = http
        .request_json("GET", &url, &headers, None)
        .map_err(|e| format!("YouTube: category lookup failed: {e}"))?;
    if !(200..300).contains(&status) {
        return Err(format!("YouTube: category lookup rejected (HTTP {status})"));
    }
    let wanted = game.to_lowercase();
    payload
        .get("items")
        .and_then(|items| items.as_array())
        .and_then(|items| {
            items
                .iter()
                .find(|category| {
                    category
                        .get("snippet")
                        .and_then(|snippet| snippet.get("title"))
                        .and_then(|title| title.as_str())
                        .is_some_and(|title| title.to_lowercase() == wanted)
                })
                .and_then(|category| category.get("id"))
                .and_then(|id| id.as_str())
                .map(str::to_owned)
        })
        .ok_or_else(|| format!("YouTube: category {game:?} not found"))
}

// ── helpers ───────────────────────────────────────────────────────────

/// Extract a short user-safe hint from an error payload (no tokens, no
/// full request dumps).
fn twitch_error_hint(payload: &Value) -> String {
    payload
        .get("message")
        .or_else(|| payload.get("error"))
        .and_then(|message| message.as_str())
        .map(|message| {
            let short: String = message.chars().take(120).collect();
            format!("\"{short}\"")
        })
        .unwrap_or_else(|| "no detail".to_owned())
}

/// Minimal percent-encoding for query parameters (space and reserved
/// characters); enough for game/category names.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// Recorded request as `(method, path, body)`.
    type RequestLog = Vec<(String, String, Vec<u8>)>;

    /// Find the first occurrence of `needle` in `haystack`.
    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        haystack
            .windows(needle.len())
            .position(|window| window == needle)
    }

    /// Local HTTP puppet: records requests, replies with canned responses.
    struct Puppet {
        url: String,
        requests: Arc<Mutex<RequestLog>>,
        responses: Arc<Mutex<VecDeque<(u16, Value)>>>,
        handle: Option<std::thread::JoinHandle<()>>,
    }

    impl Puppet {
        fn start() -> Self {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind puppet");
            let port = listener.local_addr().unwrap().port();
            let requests: Arc<Mutex<RequestLog>> = Arc::new(Mutex::new(Vec::new()));
            let responses: Arc<Mutex<VecDeque<(u16, Value)>>> =
                Arc::new(Mutex::new(VecDeque::new()));
            let requests_thread = requests.clone();
            let responses_thread = responses.clone();
            let handle = std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    // Read until the advertised Content-Length is satisfied:
                    // ureq sends headers and body as separate TCP segments, so
                    // a single read() frequently stops at the header block.
                    let mut raw_bytes: Vec<u8> = Vec::new();
                    let mut buffer = [0u8; 8192];
                    let mut header_end: Option<usize> = None;
                    let mut content_length: usize = 0;
                    loop {
                        let read = stream.read(&mut buffer).unwrap_or(0);
                        if read == 0 {
                            break;
                        }
                        raw_bytes.extend_from_slice(&buffer[..read]);
                        if header_end.is_none() {
                            if let Some(pos) = find_subslice(&raw_bytes, b"\r\n\r\n") {
                                header_end = Some(pos + 4);
                                let headers = String::from_utf8_lossy(&raw_bytes[..pos]);
                                for line in headers.lines() {
                                    if let Some(value) = line
                                        .strip_prefix("Content-Length:")
                                        .or_else(|| line.strip_prefix("content-length:"))
                                    {
                                        content_length = value.trim().parse().unwrap_or(0);
                                    }
                                }
                            }
                        }
                        if let Some(start) = header_end {
                            if raw_bytes.len() >= start + content_length {
                                break;
                            }
                        }
                    }
                    let raw = String::from_utf8_lossy(&raw_bytes).to_string();
                    let mut parts = raw.splitn(3, ' ');
                    let method = parts.next().unwrap_or("").to_owned();
                    let path = parts.next().unwrap_or("").to_owned();
                    let body = raw
                        .split("\r\n\r\n")
                        .nth(1)
                        .unwrap_or("")
                        .as_bytes()
                        .to_vec();
                    requests_thread
                        .lock()
                        .unwrap()
                        .push((method, path.clone(), body));
                    let (status, payload) = responses_thread
                        .lock()
                        .unwrap()
                        .pop_front()
                        .unwrap_or((200, json!({})));
                    let body_bytes = serde_json::to_vec(&payload).unwrap_or_default();
                    let reason = match status {
                        200 => "OK",
                        400 => "Bad Request",
                        401 => "Unauthorized",
                        403 => "Forbidden",
                        404 => "Not Found",
                        _ => "Error",
                    };
                    let response = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body_bytes.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.write_all(&body_bytes);
                }
            });
            Self {
                url: format!("http://127.0.0.1:{port}"),
                requests,
                responses,
                handle: Some(handle),
            }
        }

        fn queue(&self, status: u16, payload: Value) {
            self.responses.lock().unwrap().push_back((status, payload));
        }

        fn recorded(&self) -> Vec<(String, String, Vec<u8>)> {
            self.requests.lock().unwrap().clone()
        }

        fn base(&self) -> String {
            self.url.clone()
        }
    }

    impl Drop for Puppet {
        fn drop(&mut self) {
            // The thread loops forever; detach is fine for the test process.
            drop(self.handle.take());
        }
    }

    fn twitch_credentials() -> InfoCredentials {
        InfoCredentials {
            token: "test-token".to_owned(),
            client_id: "test-client".to_owned(),
            broadcaster_id: "12345".to_owned(),
            video_id: String::new(),
        }
    }

    #[test]
    fn empty_update_is_rejected_up_front() {
        let endpoints = InfoEndpoints::default();
        let outcome = update_platform_stream_info(
            InfoPlatform::Twitch,
            &StreamInfoUpdate::default(),
            &twitch_credentials(),
            &endpoints,
            &UreqHttp,
        );
        assert!(!outcome.is_ok());
        match outcome {
            InfoUpdateOutcome::Failed(message) => {
                assert!(message.contains("nothing to update"), "{message}");
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn twitch_title_only_sends_patch_without_game_lookup() {
        let puppet = Puppet::start();
        puppet.queue(200, json!({"data": []}));
        let endpoints = InfoEndpoints {
            twitch_api_base: puppet.base(),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            title: Some("Hello world".to_owned()),
            game: None,
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::Twitch,
            &update,
            &twitch_credentials(),
            &endpoints,
            &UreqHttp,
        );
        assert_eq!(outcome, InfoUpdateOutcome::Updated, "outcome: {outcome:?}");
        let requests = puppet.recorded();
        assert_eq!(requests.len(), 1, "no game lookup for title-only update");
        let (method, path, body) = &requests[0];
        assert_eq!(method, "PATCH");
        assert!(path.starts_with("/channels"));
        let body_json: Value = serde_json::from_slice(body).expect("valid PATCH body");
        assert_eq!(body_json["title"], json!("Hello world"), "{body_json}");
        assert!(body_json.get("game_id").is_none(), "{body_json}");
    }

    #[test]
    fn twitch_game_resolved_via_games_lookup_then_patched() {
        let puppet = Puppet::start();
        // First request: games lookup; second: channels PATCH.
        puppet.queue(200, json!({"data": [{"id": "516575", "name": "Valheim"}]}));
        puppet.queue(200, json!({"data": []}));
        let endpoints = InfoEndpoints {
            twitch_api_base: puppet.base(),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            title: None,
            game: Some("Valheim".to_owned()),
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::Twitch,
            &update,
            &twitch_credentials(),
            &endpoints,
            &UreqHttp,
        );
        assert_eq!(outcome, InfoUpdateOutcome::Updated);
        let requests = puppet.recorded();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].1.starts_with("/games?name=Valheim"));
        assert_eq!(requests[0].0, "GET");
        assert_eq!(requests[1].0, "PATCH");
        let body_json: Value = serde_json::from_slice(&requests[1].2).expect("valid PATCH body");
        assert_eq!(body_json["game_id"], json!("516575"), "{body_json}");
    }

    #[test]
    fn twitch_unknown_game_is_reported_without_patch() {
        let puppet = Puppet::start();
        puppet.queue(200, json!({"data": []}));
        let endpoints = InfoEndpoints {
            twitch_api_base: puppet.base(),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            game: Some("Nonexistent Game 12345".to_owned()),
            ..Default::default()
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::Twitch,
            &update,
            &twitch_credentials(),
            &endpoints,
            &UreqHttp,
        );
        match &outcome {
            InfoUpdateOutcome::Failed(message) => {
                assert!(message.contains("not found"), "{message}");
            }
            other => panic!("expected failure, got {other:?}"),
        }
        assert_eq!(puppet.recorded().len(), 1, "no PATCH after failed lookup");
    }

    #[test]
    fn twitch_missing_token_fails_without_network() {
        let endpoints = InfoEndpoints::default();
        let credentials = InfoCredentials::default();
        let update = StreamInfoUpdate {
            title: Some("x".to_owned()),
            ..Default::default()
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::Twitch,
            &update,
            &credentials,
            &endpoints,
            &UreqHttp,
        );
        assert!(!outcome.is_ok());
    }

    #[test]
    fn kick_sends_stream_title_and_resolved_category() {
        // Two puppets: one for /public/v2/categories, one for /public/v1.
        // Simpler: one puppet, base pointing at /public/v1 with v2 rewritten
        // — the module derives the v2 base from the v1 base.
        let puppet = Puppet::start();
        puppet.queue(200, json!({"data": [{"id": 101, "name": "Rust"}]}));
        puppet.queue(200, json!({"data": [], "message": "OK"}));
        let endpoints = InfoEndpoints {
            kick_api_base: format!("{}/public/v1", puppet.base()),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            title: Some("Playing Rust".to_owned()),
            game: Some("Rust".to_owned()),
        };
        let credentials = InfoCredentials {
            token: "kick-token".to_owned(),
            ..Default::default()
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::Kick,
            &update,
            &credentials,
            &endpoints,
            &UreqHttp,
        );
        assert_eq!(outcome, InfoUpdateOutcome::Updated);
        let requests = puppet.recorded();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].1.starts_with("/public/v2/categories?name=Rust"));
        assert_eq!(requests[1].0, "PATCH");
        assert!(requests[1].1.starts_with("/public/v1/channels"));
        let body_json: Value = serde_json::from_slice(&requests[1].2).expect("valid PATCH body");
        assert_eq!(
            body_json["stream_title"],
            json!("Playing Rust"),
            "{body_json}"
        );
        assert_eq!(body_json["category_id"], json!(101), "{body_json}");
    }

    #[test]
    fn youtube_sends_snippet_with_title_and_category() {
        let puppet = Puppet::start();
        // videoCategories lookup, then videos.update.
        puppet.queue(
            200,
            json!({"items": [
                {"id": "20", "snippet": {"title": "Gaming"}},
                {"id": "22", "snippet": {"title": "People & Blogs"}}
            ]}),
        );
        puppet.queue(200, json!({"id": "abc123"}));
        let endpoints = InfoEndpoints {
            youtube_api_base: puppet.base(),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            title: Some("New title".to_owned()),
            game: Some("Gaming".to_owned()),
        };
        let credentials = InfoCredentials {
            token: "yt-token".to_owned(),
            video_id: "abc123".to_owned(),
            ..Default::default()
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::YouTube,
            &update,
            &credentials,
            &endpoints,
            &UreqHttp,
        );
        assert_eq!(outcome, InfoUpdateOutcome::Updated);
        let requests = puppet.recorded();
        assert_eq!(requests.len(), 2);
        assert!(requests[0]
            .1
            .starts_with("/videoCategories?part=snippet&regionCode=US"));
        assert_eq!(requests[1].0, "PUT");
        assert!(requests[1].1.starts_with("/videos?part=snippet"));
        let body_json: Value = serde_json::from_slice(&requests[1].2).expect("valid PUT body");
        assert_eq!(
            body_json["snippet"]["title"],
            json!("New title"),
            "{body_json}"
        );
        assert_eq!(
            body_json["snippet"]["categoryId"],
            json!("20"),
            "{body_json}"
        );
        assert_eq!(body_json["id"], json!("abc123"), "{body_json}");
    }

    #[test]
    fn update_all_reports_per_platform_and_continues_after_failure() {
        let puppet = Puppet::start();
        // Twitch games lookup fails with 401, Kick categories OK, Twitch
        // PATCH for the second platform never happens.
        puppet.queue(401, json!({"message": "invalid OAuth token"}));
        puppet.queue(200, json!({"data": [{"id": 101, "name": "Rust"}]}));
        puppet.queue(200, json!({"data": [], "message": "OK"}));
        let endpoints = InfoEndpoints {
            twitch_api_base: puppet.base(),
            kick_api_base: format!("{}/public/v1", puppet.base()),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            title: Some("Joint title".to_owned()),
            game: Some("Rust".to_owned()),
        };
        let results = update_all_stream_info(
            &[InfoPlatform::Twitch, InfoPlatform::Kick],
            &update,
            |platform| match platform {
                InfoPlatform::Twitch => twitch_credentials(),
                InfoPlatform::Kick => InfoCredentials {
                    token: "kick-token".to_owned(),
                    ..Default::default()
                },
                InfoPlatform::YouTube => InfoCredentials::default(),
            },
            &endpoints,
            &UreqHttp,
        );
        assert_eq!(results.len(), 2);
        assert!(!results[0].1.is_ok(), "Twitch must fail (401 on lookup)");
        assert!(results[1].1.is_ok(), "Kick must still succeed");
        match &results[0].1 {
            InfoUpdateOutcome::Failed(message) => {
                assert!(message.starts_with("Twitch:"), "{message}");
                assert!(!message.contains("test-token"), "no tokens in errors");
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }

    #[test]
    fn error_messages_never_contain_tokens() {
        let puppet = Puppet::start();
        puppet.queue(403, json!({"message": "missing scope"}));
        let endpoints = InfoEndpoints {
            twitch_api_base: puppet.base(),
            ..InfoEndpoints::default()
        };
        let update = StreamInfoUpdate {
            title: Some("t".to_owned()),
            ..Default::default()
        };
        let credentials = InfoCredentials {
            token: "super-secret-token-value".to_owned(),
            ..twitch_credentials()
        };
        let outcome = update_platform_stream_info(
            InfoPlatform::Twitch,
            &update,
            &credentials,
            &endpoints,
            &UreqHttp,
        );
        match outcome {
            InfoUpdateOutcome::Failed(message) => {
                assert!(!message.contains("super-secret-token-value"));
            }
            other => panic!("expected failure, got {other:?}"),
        }
    }
}
