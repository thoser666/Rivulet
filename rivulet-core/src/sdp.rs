//! Deterministic Session Description Protocol (SDP) helpers for WHIP/WebRTC.
//!
//! The offer is generated from fixed, well-known codec parameters so that the
//! signaling path is fully unit-testable offline. A WHIP endpoint treats the
//! SDP as an opaque media description; the important property here is a
//! *stable, valid* description that an H.264/Opus WebRTC pipeline can
//! negotiate.

/// A deterministic, H.264 + Opus SDP offer.
pub struct SdpOffer {
    sdp: String,
}

impl SdpOffer {
    /// Build an offer for the given WHIP endpoint.
    ///
    /// The endpoint is used only for the session-ID seed and is never echoed in
    /// a redacted log; it is not parsed here.
    pub fn h264_opus(endpoint: &str) -> Self {
        let seed = seed_for(endpoint);
        Self {
            sdp: format!(
                "v=0\r\n\
                 o=- {seed} {seed} IN IP4 0.0.0.0\r\n\
                 s=Rivulet WHIP\r\n\
                 t=0 0\r\n\
                 a=group:BUNDLE 0 1\r\n\
                 a=ice-options:trickle\r\n\
                 m=video 9 UDP/TLS/RTP/SAVPF 96 97\r\n\
                 c=IN IP4 0.0.0.0\r\n\
                 a=mid:0\r\n\
                 a=sendonly\r\n\
                 a=rtcp-mux\r\n\
                 a=rtpmap:96 H264/90000\r\n\
                 a=fmtp:96 packetization-mode=1;profile-level-id=42e01f\r\n\
                 a=rtcp-fb:96 nack\r\n\
                 a=rtcp-fb:96 nack pli\r\n\
                 m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
                 c=IN IP4 0.0.0.0\r\n\
                 a=mid:1\r\n\
                 a=sendonly\r\n\
                 a=rtcp-mux\r\n\
                 a=rtpmap:111 opus/48000/2\r\n"
            ),
        }
    }

    /// The generated SDP text.
    pub fn to_sdp(&self) -> anyhow::Result<String> {
        if self.sdp.is_empty() {
            anyhow::bail!("SDP session string is empty");
        }
        Ok(self.sdp.clone())
    }
}

/// A tiny, stable numeric seed derived from the endpoint URL. This is not a
/// security boundary — it only keeps the session-id deterministic for tests.
fn seed_for(endpoint: &str) -> u64 {
    endpoint.bytes().fold(0u64, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u64)
    }) % 100_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offer_contains_h264_and_opus_sections() {
        let offer = SdpOffer::h264_opus("https://sfu.example/whip")
            .to_sdp()
            .unwrap();
        assert!(offer.contains("v=0"));
        assert!(offer.contains("m=video"));
        assert!(offer.contains("H264/90000"));
        assert!(offer.contains("m=audio"));
        assert!(offer.contains("opus/48000/2"));
        assert!(offer.contains("a=group:BUNDLE 0 1"));
    }

    #[test]
    fn offer_is_deterministic_for_same_endpoint() {
        let a = SdpOffer::h264_opus("https://sfu.example/whip")
            .to_sdp()
            .unwrap();
        let b = SdpOffer::h264_opus("https://sfu.example/whip")
            .to_sdp()
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn offer_never_contains_the_bearer_token_or_password() {
        let offer = SdpOffer::h264_opus("https://sfu.example/whip")
            .to_sdp()
            .unwrap();
        assert!(!offer.contains("secret"));
        assert!(!offer.contains("password"));
        assert!(!offer.contains("token"));
    }
}
