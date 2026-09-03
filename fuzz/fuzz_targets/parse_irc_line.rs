//! Fuzz the Twitch IRC line parser.
//!
//! `parse_irc_line` consumes untrusted lines straight from the IRC socket.
//! The invariant is simple: any input may return `Some(message)` or `None`,
//! but must never panic, overflow, or loop forever.

#![no_main]

use libfuzzer_sys::fuzz_target;
use rivulet_core::twitch_chat::parse_irc_line;

fuzz_target!(|data: &str| {
    if let Some(message) = parse_irc_line(data) {
        // Parsed messages must stay well-formed: no empty users, no
        // interior NULs leaking through (IRC lines are UTF-8 text).
        assert!(!message.user.is_empty(), "parsed message with empty user");
        assert!(
            !message.user.contains('\u{0}') && !message.text.contains('\u{0}'),
            "NUL byte leaked into ChatMessage"
        );
    }
});
