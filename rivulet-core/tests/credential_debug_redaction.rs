//! Credential-bearing config types must not render their secrets through
//! `Debug`.
//!
//! `chat_secret_redaction.rs` covers the chat configs. The same hole existed in
//! the stream and webhook types, and there it guards a *live* credential: a
//! `tracing::debug!("{:?}", settings)`, an `expect` message or an error chain
//! formatting any of these structs used to print a stream key, a WHIP bearer
//! token, a platform OAuth token or a Twitch EventSub secret in clear - the
//! sink CodeQL `rust/cleartext-logging` (alert #93) describes.
//!
//! `StreamSettings` carries the secret twice: `stream_key`, and an ingest URL
//! that can embed the key in its path (Kick documents such URLs), so both are
//! redacted and the rest of the struct stays readable for debugging.

use rivulet_core::alerts_webhook::AlertsReceiverConfig;
use rivulet_core::stream::{StreamSettings, WhipSettings};
use rivulet_core::stream_info::InfoCredentials;

const STREAM_KEY: &str = "live_482913_sentinel-stream-key";
const BEARER: &str = "sentinel-whip-bearer-token";
const EVENTSUB_SECRET: &str = "sentinel-eventsub-secret";
const PLACEHOLDER: &str = "<redacted>";

#[test]
fn stream_settings_debug_hides_the_key_and_the_ingest_url() {
    let settings = StreamSettings::twitch(STREAM_KEY);

    let rendered = format!("{settings:?}");

    assert!(
        !rendered.contains(STREAM_KEY),
        "the stream key reached a Debug line: {rendered}"
    );
    // The ingest URL is a second path to the same secret (a custom or Kick URL
    // can carry the key in its path), so it is redacted as a whole.
    assert!(
        !rendered.contains("live.twitch.tv"),
        "the ingest URL reached a Debug line: {rendered}"
    );
    assert!(rendered.contains("<redacted stream URL>"), "{rendered}");
    assert!(rendered.contains(PLACEHOLDER), "{rendered}");
    // The platform and the selected presets stay readable - that is what makes
    // the impl worth printing when a session fails.
    assert!(rendered.contains("Twitch"), "{rendered}");
    assert!(rendered.contains("preset"), "{rendered}");
    assert!(rendered.contains("adaptive_bitrate"), "{rendered}");

    let pretty = format!("{settings:#?}");
    assert!(
        !pretty.contains(STREAM_KEY) && !pretty.contains("live.twitch.tv"),
        "pretty-printed Debug leaked the key: {pretty}"
    );
    // `masked_key` is the deliberate, short prefix shown in the UI; it must not
    // become a way to reach the full key through Debug either.
    assert!(
        !settings.masked_key().contains(STREAM_KEY),
        "{}",
        settings.masked_key()
    );
}

#[test]
fn whip_settings_debug_redacts_the_bearer_token() {
    let settings =
        WhipSettings::new("https://whip.example.test/endpoint").with_bearer_token(BEARER);

    let rendered = format!("{settings:?}");

    assert!(
        !rendered.contains(BEARER),
        "the WHIP bearer token reached a Debug line: {rendered}"
    );
    assert!(rendered.contains(PLACEHOLDER), "{rendered}");
    assert!(
        rendered.contains("https://whip.example.test/endpoint"),
        "the endpoint must stay readable: {rendered}"
    );
    // A missing token is not a secret and is reported as such, so an operator
    // can tell "not configured" from "configured but redacted".
    let without = format!(
        "{:?}",
        WhipSettings::new("https://whip.example.test/endpoint")
    );
    assert!(without.contains("None"), "{without}");
    assert!(!without.contains(PLACEHOLDER), "{without}");
}

#[test]
fn info_credentials_debug_redacts_the_oauth_token() {
    let credentials = InfoCredentials {
        token: STREAM_KEY.to_owned(),
        client_id: "sentinel-client-id".to_owned(),
        broadcaster_id: "123456".to_owned(),
        video_id: "abcdef".to_owned(),
    };

    let rendered = format!("{credentials:?}");

    assert!(
        !rendered.contains(STREAM_KEY),
        "the OAuth token reached a Debug line: {rendered}"
    );
    assert!(rendered.contains(PLACEHOLDER), "{rendered}");
    // The identifiers are what a failed update report needs; they are not
    // credentials and must survive the redaction.
    assert!(rendered.contains("sentinel-client-id"), "{rendered}");
    assert!(rendered.contains("123456"), "{rendered}");
    assert!(rendered.contains("abcdef"), "{rendered}");
}

#[test]
fn alerts_receiver_config_debug_redacts_the_eventsub_secret() {
    let config = AlertsReceiverConfig {
        port: 4711,
        twitch_secret: EVENTSUB_SECRET.to_owned(),
    };

    let rendered = format!("{config:?}");

    assert!(
        !rendered.contains(EVENTSUB_SECRET),
        "the EventSub secret reached a Debug line: {rendered}"
    );
    assert!(rendered.contains(PLACEHOLDER), "{rendered}");
    assert!(
        rendered.contains("4711"),
        "the port must stay readable: {rendered}"
    );

    // An unset secret stays visible as an empty value instead of a placeholder:
    // the receiver disables its EventSub route in that case, which is exactly
    // what a debug line should say.
    let disabled = format!("{:?}", AlertsReceiverConfig::default());
    assert!(disabled.contains("twitch_secret"), "{disabled}");
    assert!(!disabled.contains(PLACEHOLDER), "{disabled}");
}
