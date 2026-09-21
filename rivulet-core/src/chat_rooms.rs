//! Twitch Shared Chat room-name resolution for the combined chat dock.
//!
//! Shared Chat messages duplicated into the joined room carry a
//! `source-room-id` tag (see [`crate::twitch_chat::ChatMessage`]). Twitch
//! exposes only the numeric room id over IRC, so this module resolves ids to
//! channel login names via the Helix `users` endpoint and caches the results
//! so the dock badge shows `via <login>` instead of a raw number.
//!
//! Design (matches the module's privacy and determinism posture):
//! - **In-flight collapse** — a pure [`SharedRoomNameService`] decides what
//!   to do per message: unknown id + no in-flight request => start a
//!   resolution; unknown id + in-flight request => leave the raw id
//!   (rendered identically once the result lands); known id => the cached
//!   login. Resolution never blocks rendering and never runs per frame.
//! - **Credentials at call time** — the Helix client id and the Twitch chat
//!   account token are supplied per [`HelixRoomResolver::resolve`] call and
//!   travel into the HTTPS request only; they are never stored on the
//!   service, never logged and never embedded in errors.
//! - **Bounded cache** — the id→login map is capped; the oldest entry is
//!   dropped when full (same policy as the chat/alert queues).
//! - **No idle I/O** — the resolution runs only when a shared-chat message
//!   with an unknown id is on screen; outside shared sessions nothing dials
//!   out.

use std::collections::HashMap;

/// Maximum cached id→login entries (oldest dropped when full).
pub const MAX_ROOM_NAME_CACHE: usize = 128;

/// The minimal Helix surface the resolver needs (implemented with `ureq` in
/// production and with the real `ureq` against a local TCP puppet in tests,
/// mirroring [`crate::stream_info::Http`]).
pub trait RoomNameResolver {
    /// Resolve room ids to login names. Returns the raw JSON body of a
    /// successful Helix `GET /helix/users?id=…&id=…` call; transport failures
    /// return `Err` with a user-safe message (no credentials, no URLs).
    fn resolve(&self, ids: &[String], client_id: &str, token: &str) -> Result<String, String>;
}

/// Real HTTP backend (ureq, rustls) for the room-name lookup.
pub struct HelixRoomResolver {
    /// Helix API base (production default; tests point at a local puppet).
    pub api_base: String,
}

impl Default for HelixRoomResolver {
    fn default() -> Self {
        Self {
            api_base: "https://api.twitch.tv/helix".to_owned(),
        }
    }
}

impl RoomNameResolver for HelixRoomResolver {
    fn resolve(&self, ids: &[String], client_id: &str, token: &str) -> Result<String, String> {
        if ids.is_empty() || client_id.trim().is_empty() || token.trim().is_empty() {
            return Err("no credentials for room-name lookup".to_owned());
        }
        let mut url = format!("{}/users?", self.api_base.trim_end_matches('/'));
        for (index, id) in ids.iter().enumerate() {
            if index > 0 {
                url.push('&');
            }
            url.push_str("id=");
            url.push_str(id.trim());
        }
        let outcome = ureq::get(&url)
            .header("Client-Id", client_id.trim())
            .header("Authorization", format!("Bearer {}", token.trim()))
            .call();
        let response = outcome.map_err(|e| format!("room-name lookup failed: {e}"))?;
        let status = response.status().as_u16();
        if status != 200 {
            return Err(format!("room-name lookup returned HTTP {status}"));
        }
        response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("room-name lookup body unreadable: {e}"))
    }
}

/// Outcome of feeding one pending id through [`SharedRoomNameService::step`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomNameStep {
    /// The id is known; render this login in the badge.
    Known(String),
    /// A resolution was already in flight for this id; keep the raw id for
    /// now (the badge self-improves once the result lands).
    Pending,
    /// A new resolution was started for this id; the caller must pass the
    /// listed ids to the resolver on a background thread.
    Started(Vec<String>),
}

/// Pure shared-chat room-name cache. No I/O, no threads: the GUI drives
/// [`SharedRoomNameService::step`] per badge render and drains
/// [`SharedRoomNameService::take_pending`] once per frame.
#[derive(Debug, Default)]
pub struct SharedRoomNameService {
    cache: HashMap<String, String>,
    in_flight: Vec<String>,
}

impl SharedRoomNameService {
    /// What to render for `raw_id`: the cached login if known, otherwise the
    /// raw id. Never blocks; never starts I/O by itself.
    pub fn label_for(&self, raw_id: &str) -> String {
        self.cache
            .get(raw_id)
            .cloned()
            .unwrap_or_else(|| raw_id.to_owned())
    }

    /// Decide the next step for a shared-chat message's source room. Pure
    /// apart from the in-flight bookkeeping: starts one resolution per
    /// unknown id and collapses repeats while it is in flight.
    pub fn step(&mut self, raw_id: &str) -> RoomNameStep {
        if let Some(login) = self.cache.get(raw_id) {
            return RoomNameStep::Known(login.clone());
        }
        if self.in_flight.iter().any(|id| id == raw_id) {
            return RoomNameStep::Pending;
        }
        self.in_flight.push(raw_id.to_owned());
        RoomNameStep::Started(vec![raw_id.to_owned()])
    }

    /// Drop all in-flight markers without results. Used when a dispatch was
    /// impossible (e.g. no Twitch credentials configured yet) so a later
    /// frame can retry instead of staying stuck on the raw id.
    pub fn release_in_flight(&mut self) {
        self.in_flight.clear();
    }

    /// Record a successful (or partially successful) resolution. Ids that
    /// Twitch could not resolve simply stay out of the cache, so the badge
    /// keeps showing the raw id instead of a wrong name.
    pub fn apply_results(&mut self, resolved: Vec<(String, String)>) {
        for (id, login) in resolved {
            if id.trim().is_empty() || login.trim().is_empty() {
                continue;
            }
            self.in_flight.retain(|f| f != &id);
            self.cache.insert(id, login);
        }
        // Nothing resolved: drop the in-flight markers so a later attempt is
        // possible instead of stuck pending forever.
        self.in_flight.clear();
        while self.cache.len() > MAX_ROOM_NAME_CACHE {
            let oldest = self.cache.keys().next().cloned();
            match oldest {
                Some(key) => {
                    self.cache.remove(&key);
                }
                None => break,
            }
        }
    }

    /// Cached id→login entries (test/diagnostic access).
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

/// Parse a Helix `users` response body into `(id, login)` pairs, preserving
/// the API's data order. Unknown shapes yield an empty list (the badge stays
/// on the raw id).
pub fn helix_users_by_id(body: &str) -> Vec<(String, String)> {
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    parsed
        .get("data")
        .and_then(|d| d.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let id = entry.get("id")?.as_str()?.to_owned();
                    let login = entry.get("login")?.as_str()?.to_owned();
                    Some((id, login))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn labels_prefer_the_cached_login_over_the_raw_id() {
        let mut service = SharedRoomNameService::default();
        assert_eq!(service.label_for("12826"), "12826");
        service.apply_results(vec![("12826".to_owned(), "twitch".to_owned())]);
        assert_eq!(service.label_for("12826"), "twitch");
        assert_eq!(
            service.step("12826"),
            RoomNameStep::Known("twitch".to_owned())
        );
    }

    #[test]
    fn steps_start_one_batch_and_collapse_in_flight() {
        let mut service = SharedRoomNameService::default();
        match service.step("111") {
            RoomNameStep::Started(batch) => {
                assert_eq!(batch, vec!["111".to_owned()]);
            }
            other => panic!("first unknown id must start a resolution, got {other:?}"),
        }
        // Same id again while the request is in flight: pending, no second
        // dispatch. A second id starts its own lookup.
        assert_eq!(service.step("111"), RoomNameStep::Pending);
        assert!(matches!(service.step("222"), RoomNameStep::Started(_)));
        // A dispatch that was impossible releases the markers so later
        // frames can retry.
        service.release_in_flight();
        assert!(matches!(service.step("111"), RoomNameStep::Started(_)));
    }

    #[test]
    fn failed_lookups_fall_back_to_the_raw_id_and_retry_later() {
        let mut service = SharedRoomNameService::default();
        let _ = service.step("333");
        service.apply_results(Vec::new());
        assert_eq!(service.label_for("333"), "333");
        // The in-flight marker is cleared so a later frame may retry.
        assert!(matches!(service.step("333"), RoomNameStep::Started(_)));
    }

    #[test]
    fn cache_stays_bounded() {
        let mut service = SharedRoomNameService::default();
        for i in 0..(MAX_ROOM_NAME_CACHE + 16) {
            service.apply_results(vec![(format!("id{i}"), format!("name{i}"))]);
        }
        assert!(service.cache_len() <= MAX_ROOM_NAME_CACHE);
    }

    #[test]
    fn helix_users_body_is_parsed_in_order() {
        let body = r#"{"data":[{"id":"12826","login":"twitch","display_name":"Twitch"},{"id":"197886470","login":"twitchrivals","display_name":"TwitchRivals"}]}"#;
        let resolved = helix_users_by_id(body);
        assert_eq!(
            resolved,
            vec![
                ("12826".to_owned(), "twitch".to_owned()),
                ("197886470".to_owned(), "twitchrivals".to_owned()),
            ]
        );
        assert!(helix_users_by_id("not json").is_empty());
        assert!(helix_users_by_id(r#"{"data":[]}"#).is_empty());
    }

    #[test]
    fn helix_resolver_hits_the_users_endpoint_with_headers() {
        // Real ureq against a local HTTP puppet: the request must be a GET
        // with one `id=` per room plus Client-Id/Authorization headers, and
        // the parsed body must round-trip into the cache.
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind puppet");
        let port = listener.local_addr().unwrap().port();
        let requests: std::sync::Arc<std::sync::Mutex<Vec<String>>> =
            std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests_thread = requests.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 4096];
            let read = stream.read(&mut buffer).unwrap_or(0);
            requests_thread
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buffer[..read]).to_string());
            let body = br#"{"data":[{"id":"12826","login":"twitch"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(body);
        });
        let resolver = HelixRoomResolver {
            api_base: format!("http://127.0.0.1:{port}/helix"),
        };
        let body = resolver
            .resolve(&["12826".to_owned()], "cid", "oauth:abc")
            .expect("resolve");
        server.join().expect("puppet thread");
        let logged = requests.lock().unwrap();
        let request = logged.first().expect("one request");
        assert!(
            request.starts_with("GET /helix/users?id=12826"),
            "{request}"
        );
        // Header names arrive lower-cased by some HTTP stacks; compare
        // case-insensitively and print the raw request on mismatch.
        let lowered = request.to_ascii_lowercase();
        assert!(
            lowered.contains("client-id: cid"),
            "client-id header missing: {request}"
        );
        assert!(
            lowered.contains("authorization: bearer oauth:abc"),
            "authorization header missing: {request}"
        );
        let resolved = helix_users_by_id(&body);
        assert_eq!(resolved, vec![("12826".to_owned(), "twitch".to_owned())]);
    }

    #[test]
    fn helix_resolver_refuses_to_dial_without_credentials() {
        let resolver = HelixRoomResolver::default();
        assert!(resolver
            .resolve(&["12826".to_owned()], "", "oauth:abc")
            .is_err());
        assert!(resolver.resolve(&[], "cid", "oauth:abc").is_err());
    }
}
