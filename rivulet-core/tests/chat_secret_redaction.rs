//! Chat credentials must never render through `Debug`.
//!
//! CodeQL `rust/cleartext-logging` (alert #93) flags a value on its way to a
//! log sink, and the roster it named is token-free by construction. The types
//! that *do* carry a secret are the configs, and they used to derive `Debug`:
//! any `{:?}`, any panic message that formats the config and any error chain
//! built from it would have printed the OAuth token or the YouTube API key in
//! clear. Each of them now implements `Debug` by hand and reports the secret as
//! `<redacted>`; these tests pin that, so a later `#[derive(Debug)]` cannot
//! quietly undo it.
//!
//! The real invariant is narrower than "no secret in a log line": a redaction
//! that also hides the channel would be useless for debugging, so every test
//! checks both directions - the secret is gone *and* the rest is still there.

use rivulet_core::chat::{ChatConfig, ChatPlatform};
use rivulet_core::kick_chat::KickChatConfig;
use rivulet_core::twitch_chat::TwitchChatConfig;

const TOKEN: &str = "oauth:sentinel-token-value";
const API_KEY: &str = "AIza-sentinel-api-key";
const PLACEHOLDER: &str = "<redacted>";

#[test]
fn chat_config_debug_redacts_the_token_and_the_youtube_api_key() {
    let mut config = ChatConfig::new(ChatPlatform::Twitch, "rivulet".to_owned(), TOKEN.to_owned());
    config.youtube_api_key = API_KEY.to_owned();

    let rendered = format!("{config:?}");

    assert!(
        !rendered.contains(TOKEN),
        "the OAuth token reached a Debug line: {rendered}"
    );
    assert!(
        !rendered.contains(API_KEY),
        "the YouTube API key reached a Debug line: {rendered}"
    );
    assert_eq!(
        rendered.matches(PLACEHOLDER).count(),
        2,
        "both secrets must render as placeholders: {rendered}"
    );
    // A Debug impl that hides everything is not worth printing: the
    // non-secret fields have to stay readable.
    assert!(rendered.contains("channel: \"rivulet\""), "{rendered}");
    assert!(rendered.contains("twitch_endpoint"), "{rendered}");

    // `{:#?}` is the same `fmt` with an alternate flag; it must not become a
    // second, unredacted path.
    let pretty = format!("{config:#?}");
    assert!(
        !pretty.contains(TOKEN) && !pretty.contains(API_KEY),
        "pretty-printed Debug leaked a secret: {pretty}"
    );
}

#[test]
fn twitch_config_debug_redacts_the_oauth_token() {
    let config = TwitchChatConfig {
        oauth_token: TOKEN.to_owned(),
        channel: "rivulet".to_owned(),
        ..Default::default()
    };

    let rendered = format!("{config:?}");

    assert!(
        !rendered.contains(TOKEN),
        "the Twitch OAuth token reached a Debug line: {rendered}"
    );
    assert!(rendered.contains(PLACEHOLDER), "{rendered}");
    assert!(rendered.contains("channel: \"rivulet\""), "{rendered}");
    // The endpoint is the field an operator actually needs when a connection
    // fails, so it must survive the redaction.
    assert!(rendered.contains("irc.chat.twitch.tv:6697"), "{rendered}");
}

#[test]
fn kick_config_debug_redacts_the_session_token() {
    let config = KickChatConfig {
        token: TOKEN.to_owned(),
        channel: "rivulet".to_owned(),
        ..Default::default()
    };

    let rendered = format!("{config:?}");

    assert!(
        !rendered.contains(TOKEN),
        "the Kick session token reached a Debug line: {rendered}"
    );
    assert!(rendered.contains(PLACEHOLDER), "{rendered}");
    assert!(rendered.contains("channel: \"rivulet\""), "{rendered}");
    assert!(rendered.contains("ws_endpoint"), "{rendered}");
}
