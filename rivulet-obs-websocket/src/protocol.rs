//! OBS WebSocket v5 wire protocol (JSON) — constants, authentication, and
//! message helpers.
//!
//! Protocol reference: <https://github.com/obsproject/obs-websocket/blob/master/docs/generated/protocol.md>
//! This module implements the parts of v5 used by the Rivulet remote-control
//! server: op-codes 0..9, SHA-256 challenge/response authentication, request
//! status codes, and event subscriptions.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha2::{Digest, Sha256};

/// WebSocketOpCode values (v5).
pub mod op {
    pub const HELLO: u8 = 0;
    pub const IDENTIFY: u8 = 1;
    pub const IDENTIFIED: u8 = 2;
    pub const REIDENTIFY: u8 = 3;
    // NOTE: v4 used opcode 4 for `Heartbeat`; v5 deliberately has no opcode 4.
    pub const EVENT: u8 = 5;
    pub const REQUEST: u8 = 6;
    pub const REQUEST_RESPONSE: u8 = 7;
    pub const REQUEST_BATCH: u8 = 8;
    pub const REQUEST_BATCH_RESPONSE: u8 = 9;
}

/// EventSubscription bit flags (v5). Values verified against the generated
/// protocol reference.
pub mod intent {
    pub const GENERAL: u32 = 1 << 0;
    pub const CONFIG: u32 = 1 << 1;
    pub const SCENES: u32 = 1 << 2;
    pub const INPUTS: u32 = 1 << 3;
    pub const TRANSITIONS: u32 = 1 << 4;
    pub const FILTERS: u32 = 1 << 5;
    pub const OUTPUTS: u32 = 1 << 6;
    pub const SCENE_ITEMS: u32 = 1 << 7;
    pub const MEDIA_INPUTS: u32 = 1 << 8;
    pub const VENDORS: u32 = 1 << 9;
    pub const UI: u32 = 1 << 10;
    pub const CANVASES: u32 = 1 << 11;
    /// All non-high-volume event categories (matches the spec's default when
    /// `eventSubscriptions` is omitted).
    pub const ALL: u32 = GENERAL
        | CONFIG
        | SCENES
        | INPUTS
        | TRANSITIONS
        | FILTERS
        | OUTPUTS
        | SCENE_ITEMS
        | MEDIA_INPUTS
        | VENDORS
        | UI
        | CANVASES;
}

/// RequestStatus codes (v5). Only the values the server emits are declared;
/// the integer values are verified against the generated protocol reference.
pub mod status {
    /// The request has succeeded.
    pub const SUCCESS: u16 = 100;
    /// The `requestType` field is missing from the request data.
    pub const MISSING_REQUEST_TYPE: u16 = 203;
    /// The request type is invalid or does not exist.
    pub const UNKNOWN_REQUEST_TYPE: u16 = 204;
    /// Generic error; a comment is required.
    pub const GENERIC_ERROR: u16 = 205;
    /// A required request field is missing.
    pub const MISSING_REQUEST_FIELD: u16 = 300;
    /// The request does not have a valid requestData object.
    pub const MISSING_REQUEST_DATA: u16 = 301;
    /// A request field has the wrong data type.
    pub const INVALID_REQUEST_FIELD: u16 = 400;
    /// An output is running and cannot be in order to perform the request.
    pub const OUTPUT_RUNNING: u16 = 500;
    /// An output is not running and should be.
    pub const OUTPUT_NOT_RUNNING: u16 = 501;
    /// An output is paused and cannot be in order to perform the request.
    pub const OUTPUT_PAUSED: u16 = 502;
    /// An output is not paused and should be.
    pub const OUTPUT_NOT_PAUSED: u16 = 503;
    /// An output is disabled and cannot be used.
    pub const OUTPUT_DISABLED: u16 = 504;
    /// The resource was not found.
    pub const RESOURCE_NOT_FOUND: u16 = 600;
    /// The resource already exists.
    pub const RESOURCE_ALREADY_EXISTS: u16 = 601;
    /// The request could not be processed.
    pub const REQUEST_PROCESSING_FAILED: u16 = 702;
}

/// WebSocketCloseCode values (v5) used by the server.
pub mod close {
    pub const DONT_CLOSE: u16 = 0;
    pub const MESSAGE_DECODE_ERROR: u16 = 4002;
    pub const MISSING_DATA_FIELD: u16 = 4003;
    pub const INVALID_DATA_FIELD_TYPE: u16 = 4004;
    pub const INVALID_DATA_FIELD_VALUE: u16 = 4005;
    pub const UNKNOWN_OP_CODE: u16 = 4006;
    pub const NOT_IDENTIFIED: u16 = 4007;
    pub const ALREADY_IDENTIFIED: u16 = 4008;
    pub const AUTHENTICATION_FAILED: u16 = 4009;
    pub const UNSUPPORTED_RPC_VERSION: u16 = 4010;
}

/// The protocol RPC version this server implements.
pub const RPC_VERSION: u32 = 1;

/// Subprotocol we advertise/accept (JSON over text frames).
pub const JSON_SUBPROTOCOL: &str = "obswebsocket.json";

/// An obs-websocket v5 request name. Currently supported subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestType {
    GetVersion,
    GetAuthRequired,
    GetSceneList,
    GetCurrentProgramScene,
    SetCurrentProgramScene,
    GetInputList,
    GetRecordStatus,
    StartRecording,
    StopRecording,
    ToggleRecording,
    GetStreamStatus,
    StartStreaming,
    StopStreaming,
    ToggleStreaming,
}

impl RequestType {
    pub fn as_str(self) -> &'static str {
        match self {
            RequestType::GetVersion => "GetVersion",
            RequestType::GetAuthRequired => "GetAuthRequired",
            RequestType::GetSceneList => "GetSceneList",
            RequestType::GetCurrentProgramScene => "GetCurrentProgramScene",
            RequestType::SetCurrentProgramScene => "SetCurrentProgramScene",
            RequestType::GetInputList => "GetInputList",
            RequestType::GetRecordStatus => "GetRecordStatus",
            RequestType::StartRecording => "StartRecording",
            RequestType::StopRecording => "StopRecording",
            RequestType::ToggleRecording => "ToggleRecording",
            RequestType::GetStreamStatus => "GetStreamStatus",
            RequestType::StartStreaming => "StartStreaming",
            RequestType::StopStreaming => "StopStreaming",
            RequestType::ToggleStreaming => "ToggleStreaming",
        }
    }
}

impl std::str::FromStr for RequestType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "GetVersion" => Ok(RequestType::GetVersion),
            "GetAuthRequired" => Ok(RequestType::GetAuthRequired),
            "GetSceneList" => Ok(RequestType::GetSceneList),
            "GetCurrentProgramScene" => Ok(RequestType::GetCurrentProgramScene),
            "SetCurrentProgramScene" => Ok(RequestType::SetCurrentProgramScene),
            "GetInputList" => Ok(RequestType::GetInputList),
            "GetRecordStatus" => Ok(RequestType::GetRecordStatus),
            "StartRecording" => Ok(RequestType::StartRecording),
            "StopRecording" => Ok(RequestType::StopRecording),
            "ToggleRecording" => Ok(RequestType::ToggleRecording),
            "GetStreamStatus" => Ok(RequestType::GetStreamStatus),
            "StartStreaming" => Ok(RequestType::StartStreaming),
            "StopStreaming" => Ok(RequestType::StopStreaming),
            "ToggleStreaming" => Ok(RequestType::ToggleStreaming),
            _ => Err(()),
        }
    }
}

/// Compute the base64-encoded SHA-256 secret from the password and salt.
///
/// v5 authentication: `secret = base64(SHA256(password + salt))`.
pub fn compute_secret(password: &str, salt: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    hasher.update(salt.as_bytes());
    BASE64.encode(hasher.finalize())
}

/// Compute the authentication response sent in `Identify`.
///
/// v5 authentication: `auth_response = base64(SHA256(secret + challenge))`
/// where `secret` is [`compute_secret`].
pub fn compute_auth_response(secret: &str, challenge: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(challenge.as_bytes());
    BASE64.encode(hasher.finalize())
}

/// Verify a client-supplied authentication string against the expected
/// password/challenge/salt triple.
pub fn verify_authentication(password: &str, salt: &str, challenge: &str, provided: &str) -> bool {
    let secret = compute_secret(password, salt);
    let expected = compute_auth_response(&secret, challenge);
    // Compare in constant time-ish fashion: same length check first avoids
    // leaking length differences via timing (lengths are already public).
    provided.len() == expected.len() && {
        let mut diff = 0u8;
        for (a, b) in provided.bytes().zip(expected.bytes()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// Build the `{ "op": N, "d": ... }` envelope.
pub fn envelope(op: u8, d: serde_json::Value) -> serde_json::Value {
    serde_json::json!({ "op": op, "d": d })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn request_type_roundtrips_through_its_wire_name() {
        for rt in [
            RequestType::GetVersion,
            RequestType::GetAuthRequired,
            RequestType::GetSceneList,
            RequestType::GetCurrentProgramScene,
            RequestType::SetCurrentProgramScene,
            RequestType::GetInputList,
            RequestType::GetRecordStatus,
            RequestType::StartRecording,
            RequestType::StopRecording,
            RequestType::ToggleRecording,
            RequestType::GetStreamStatus,
            RequestType::StartStreaming,
            RequestType::StopStreaming,
            RequestType::ToggleStreaming,
        ] {
            assert_eq!(RequestType::from_str(rt.as_str()), Ok(rt));
        }
        assert!(RequestType::from_str("NotARequest").is_err());
    }

    #[test]
    fn auth_handshake_computes_expected_response() {
        // Scripted challenge/salt (any values work; the response must be
        // deterministic for a fixed input sequence).
        let password = "supersecretpassword";
        let salt = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
        let challenge = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";

        let secret = compute_secret(password, salt);
        let response = compute_auth_response(&secret, challenge);

        assert!(verify_authentication(password, salt, challenge, &response));
        // A wrong password must fail.
        assert!(!verify_authentication(
            "wrongpassword",
            salt,
            challenge,
            &response
        ));
        // A tampered response must fail.
        assert!(!verify_authentication(password, salt, challenge, "AAAA"));
    }

    #[test]
    fn auth_depends_on_both_salt_and_challenge() {
        let password = "pw1";
        let salt = "salt1";
        let challenge = "challenge1";
        let secret = compute_secret(password, salt);
        let response = compute_auth_response(&secret, challenge);
        // Different salt -> different secret -> different response.
        let other_secret = compute_secret(password, "salt2");
        let other_response = compute_auth_response(&other_secret, challenge);
        assert_ne!(response, other_response);
        // Different challenge -> different response with the same secret.
        let other_challenge_response = compute_auth_response(&secret, "challenge2");
        assert_ne!(response, other_challenge_response);
    }

    #[test]
    fn intent_all_covers_all_subscriptions() {
        for bit in [
            intent::GENERAL,
            intent::CONFIG,
            intent::SCENES,
            intent::INPUTS,
            intent::TRANSITIONS,
            intent::FILTERS,
            intent::OUTPUTS,
            intent::SCENE_ITEMS,
            intent::MEDIA_INPUTS,
            intent::VENDORS,
            intent::UI,
            intent::CANVASES,
        ] {
            assert_ne!(intent::ALL & bit, 0, "bit {bit} must be inside ALL");
        }
    }

    #[test]
    fn status_and_close_codes_match_spec() {
        assert_eq!(status::SUCCESS, 100);
        assert_eq!(status::MISSING_REQUEST_TYPE, 203);
        assert_eq!(status::UNKNOWN_REQUEST_TYPE, 204);
        assert_eq!(status::GENERIC_ERROR, 205);
        assert_eq!(status::MISSING_REQUEST_DATA, 301);
        assert_eq!(status::INVALID_REQUEST_FIELD, 400);
        assert_eq!(status::OUTPUT_RUNNING, 500);
        assert_eq!(status::OUTPUT_NOT_RUNNING, 501);
        assert_eq!(status::RESOURCE_NOT_FOUND, 600);
        assert_eq!(close::AUTHENTICATION_FAILED, 4009);
        assert_eq!(close::NOT_IDENTIFIED, 4007);
    }

    #[test]
    fn envelope_has_op_and_data() {
        let e = envelope(6, serde_json::json!({ "a": 1 }));
        assert_eq!(e["op"], 6);
        assert_eq!(e["d"]["a"], 1);
    }
}
