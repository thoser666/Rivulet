//! End-to-end streaming run against a loopback RTMP server (M7 closeout,
//! issue #289).
//!
//! `rivulet stream` differs from `record` in one way only a real session can
//! prove: the frames have to leave the process over RTMP. This test runs a
//! short session against a loopback server that speaks the handshake, answers
//! `connect`/`createStream`/`publish`, and counts the audio and video messages
//! it receives — no network, no ingest account, no capture hardware.
//!
//! The fixture is deliberately smaller than `rivulet-core`'s
//! (`tests/recording_job.rs`), which additionally covers dual output and file
//! finalization. Here the server only has to be a valid RTMP peer for one
//! publishing session.
//!
//! Skipped when the local GStreamer installation lacks the streaming leg
//! (`flvmux` + `rtmp2sink`): the engine's parse failure names the missing
//! element, which is the skip condition below.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rivulet_cli::config::{AudioConfig, StreamConfig, StreamOutputConfig, VideoConfig};
use rivulet_cli::stream::{StreamJob, StreamStatusEvent};

// ---------------------------------------------------------------------------
// Minimal RTMP server: handshake + command replies, then message counting
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct RtmpStats {
    handshake_done: bool,
    published: bool,
    /// The stream name the client published (`publish(..., name, ...)`).
    publish_name: Option<String>,
    video_msgs: u32,
    audio_msgs: u32,
}

/// Per-chunk-stream reassembly state.
#[derive(Debug, Default, Clone)]
struct ChunkState {
    msg_len: u32,
    received: u32,
    msg_type: u8,
    timestamp: u64,
    ts_delta: u32,
    has_ext_ts: bool,
    buf: Vec<u8>,
}

fn read_exact_or(stream: &mut std::net::TcpStream, buf: &mut [u8]) -> bool {
    let mut i = 0;
    while i < buf.len() {
        match stream.read(&mut buf[i..]) {
            Ok(0) => return false,
            Ok(n) => i += n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
    true
}

// -- AMF0 encoding (server replies only need strings, numbers, null, objects)

fn amf0_str(s: &str) -> Vec<u8> {
    let mut v = vec![0x02u8];
    v.extend_from_slice(&(s.len() as u16).to_be_bytes());
    v.extend_from_slice(s.as_bytes());
    v
}

fn amf0_num(n: f64) -> Vec<u8> {
    let mut v = vec![0x00u8];
    v.extend_from_slice(&n.to_be_bytes());
    v
}

fn amf0_null() -> Vec<u8> {
    vec![0x05u8]
}

fn amf0_obj(props: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut v = vec![0x03u8];
    for (key, value) in props {
        v.extend_from_slice(&(key.len() as u16).to_be_bytes());
        v.extend_from_slice(key.as_bytes());
        v.extend_from_slice(value);
    }
    v.extend_from_slice(&[0x00, 0x00, 0x05]); // object end
    v
}

fn amf0_read_str(bytes: &[u8], i: &mut usize) -> Option<String> {
    if *i >= bytes.len() || bytes[*i] != 0x02 {
        return None;
    }
    *i += 1;
    if *i + 2 > bytes.len() {
        return None;
    }
    let len = u16::from_be_bytes([bytes[*i], bytes[*i + 1]]) as usize;
    *i += 2;
    if *i + len > bytes.len() {
        return None;
    }
    let out = String::from_utf8_lossy(&bytes[*i..*i + len]).into_owned();
    *i += len;
    Some(out)
}

fn amf0_read_num(bytes: &[u8], i: &mut usize) -> Option<f64> {
    if *i + 9 > bytes.len() || bytes[*i] != 0x00 {
        return None;
    }
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[*i + 1..*i + 9]);
    *i += 9;
    Some(f64::from_be_bytes(raw))
}

fn len24(len: usize) -> [u8; 3] {
    [(len >> 16) as u8, (len >> 8) as u8, len as u8]
}

/// One message from the server: fmt-0 header on chunk-stream id 3. Every reply
/// here is far below the announced 4096-byte chunk size, so one chunk always
/// carries it.
fn server_msg(msg_type: u8, msg_stream_id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + payload.len());
    out.push(0x03); // fmt 0, csid 3
    out.extend_from_slice(&[0x00, 0x00, 0x00]); // timestamp
    out.extend_from_slice(&len24(payload.len()));
    out.push(msg_type);
    out.extend_from_slice(&msg_stream_id.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Read a fixed-size chunk header: fmt 0 (11 bytes), 1 (7) or 2 (3).
fn read_msg_header(stream: &mut std::net::TcpStream, fmt: u8, st: &mut ChunkState) -> bool {
    match fmt {
        0 => {
            let mut h = [0u8; 11];
            if !read_exact_or(stream, &mut h) {
                return false;
            }
            let ts = ((h[0] as u64) << 16) | ((h[1] as u64) << 8) | h[2] as u64;
            st.msg_len = ((h[3] as u32) << 16) | ((h[4] as u32) << 8) | h[5] as u32;
            st.msg_type = h[6];
            st.has_ext_ts = ts == 0xFFFF_FFFF;
            st.timestamp = if st.has_ext_ts {
                let mut ext = [0u8; 4];
                if !read_exact_or(stream, &mut ext) {
                    return false;
                }
                u32::from_be_bytes(ext) as u64
            } else {
                ts
            };
            st.buf.clear();
            st.received = 0;
            true
        }
        1 => {
            let mut h = [0u8; 7];
            if !read_exact_or(stream, &mut h) {
                return false;
            }
            let delta = ((h[0] as u32) << 16) | ((h[1] as u32) << 8) | h[2] as u32;
            st.ts_delta = delta;
            st.msg_len = ((h[3] as u32) << 16) | ((h[4] as u32) << 8) | h[5] as u32;
            st.msg_type = h[6];
            st.has_ext_ts = delta == 0xFFFF_FFFF;
            if st.has_ext_ts {
                let mut ext = [0u8; 4];
                if !read_exact_or(stream, &mut ext) {
                    return false;
                }
                st.ts_delta = u32::from_be_bytes(ext);
            }
            st.timestamp = st.timestamp.wrapping_add(st.ts_delta as u64);
            st.buf.clear();
            st.received = 0;
            true
        }
        2 => {
            let mut h = [0u8; 3];
            if !read_exact_or(stream, &mut h) {
                return false;
            }
            let delta = ((h[0] as u32) << 16) | ((h[1] as u32) << 8) | h[2] as u32;
            st.ts_delta = delta;
            st.has_ext_ts = delta == 0xFFFF_FFFF;
            if st.has_ext_ts {
                let mut ext = [0u8; 4];
                if !read_exact_or(stream, &mut ext) {
                    return false;
                }
                st.ts_delta = u32::from_be_bytes(ext);
            }
            st.timestamp = st.timestamp.wrapping_add(st.ts_delta as u64);
            st.buf.clear();
            st.received = 0;
            true
        }
        3 => {
            // Continuation of the running message, or a brand-new message that
            // reuses the previous headers.
            if st.received >= st.msg_len {
                st.buf.clear();
                st.received = 0;
                st.timestamp = st.timestamp.wrapping_add(st.ts_delta as u64);
            }
            if st.has_ext_ts {
                let mut ext = [0u8; 4];
                if !read_exact_or(stream, &mut ext) {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}

/// Handle one fully reassembled message from the client.
fn handle_message(
    stream: &mut std::net::TcpStream,
    msg_type: u8,
    payload: Vec<u8>,
    stats: &mut RtmpStats,
    chunk_size: &mut usize,
) {
    match msg_type {
        1 => {
            // Set Chunk Size from the client.
            if payload.len() >= 4 {
                *chunk_size = u32::from_be_bytes(payload[0..4].try_into().unwrap()) as usize;
            }
        }
        8 => stats.audio_msgs += 1,
        9 => stats.video_msgs += 1,
        20 => {
            let mut i = 0;
            let name = amf0_read_str(&payload, &mut i);
            let txn = amf0_read_num(&payload, &mut i).unwrap_or(0.0);
            match name.as_deref() {
                Some("connect") => {
                    let mut reply = Vec::new();
                    reply.extend_from_slice(&amf0_str("_result"));
                    reply.extend_from_slice(&amf0_num(txn));
                    reply.extend_from_slice(&amf0_obj(&[
                        ("fmsVer", amf0_str("FMS/3,5,6,7009")),
                        ("capabilities", amf0_num(31.0)),
                    ]));
                    reply.extend_from_slice(&amf0_obj(&[
                        ("level", amf0_str("status")),
                        ("code", amf0_str("NetConnection.Connect.Success")),
                        ("description", amf0_str("Connection succeeded.")),
                        ("objectEncoding", amf0_num(0.0)),
                    ]));
                    let _ = stream.write_all(&server_msg(20, 0, &reply));
                    stats.handshake_done = true;
                }
                Some("createStream") => {
                    let mut reply = Vec::new();
                    reply.extend_from_slice(&amf0_str("_result"));
                    reply.extend_from_slice(&amf0_num(txn));
                    reply.extend_from_slice(&amf0_null());
                    reply.extend_from_slice(&amf0_num(1.0));
                    let _ = stream.write_all(&server_msg(20, 0, &reply));
                }
                Some("publish") => {
                    stats.published = true;
                    // `publish(transaction, null, name, type)` — record the
                    // name (the stream key) so the test can prove the target
                    // from the config actually reached the ingest.
                    let mut j = i;
                    if j < payload.len() && payload[j] == 0x05 {
                        j += 1; // null
                    }
                    stats.publish_name = amf0_read_str(&payload, &mut j);
                    let mut reply = Vec::new();
                    reply.extend_from_slice(&amf0_str("onStatus"));
                    reply.extend_from_slice(&amf0_num(0.0));
                    reply.extend_from_slice(&amf0_null());
                    reply.extend_from_slice(&amf0_obj(&[
                        ("level", amf0_str("status")),
                        ("code", amf0_str("NetStream.Publish.Start")),
                        ("description", amf0_str("Publishing stream.")),
                    ]));
                    let _ = stream.write_all(&server_msg(20, 1, &reply));
                }
                Some(_) if txn != 0.0 => {
                    // releaseStream/FCPublish/… — answer `_result` so a client
                    // waiting for it cannot stall the session.
                    let mut reply = Vec::new();
                    reply.extend_from_slice(&amf0_str("_result"));
                    reply.extend_from_slice(&amf0_num(txn));
                    reply.extend_from_slice(&amf0_null());
                    reply.extend_from_slice(&amf0_null());
                    let _ = stream.write_all(&server_msg(20, 0, &reply));
                }
                _ => {}
            }
        }
        _ => {} // ack, peer bandwidth, data (onMetaData), user control
    }
}

/// Run one RTMP client session: handshake, command replies, then count the
/// audio/video messages until the client disconnects.
fn serve_rtmp(mut stream: std::net::TcpStream, c0: u8) -> RtmpStats {
    let mut stats = RtmpStats::default();
    if c0 != 0x03 {
        return stats;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));

    // Handshake: C0 + C1, then S0 + S1 + S2 (S2 echoes C1), then C2.
    let mut c1 = [0u8; 1536];
    if !read_exact_or(&mut stream, &mut c1) {
        return stats;
    }
    let mut hello = Vec::with_capacity(1 + 1536 + 1536);
    hello.push(0x03);
    hello.extend_from_slice(&[0u8; 1536]); // S1: time 0 + zeros
    hello.extend_from_slice(&c1); // S2: echo the client's C1
    if stream.write_all(&hello).is_err() {
        return stats;
    }
    let mut c2 = [0u8; 1536];
    if !read_exact_or(&mut stream, &mut c2) {
        return stats;
    }

    // Server preamble: our chunk size plus the flow-control messages every
    // RTMP server sends before `_result`.
    if stream
        .write_all(&server_msg(1, 0, &4096u32.to_be_bytes()))
        .is_err()
    {
        return stats;
    }
    let _ = stream.write_all(&server_msg(5, 0, &2_147_483_647u32.to_be_bytes()));
    let _ = stream.write_all(&server_msg(6, 0, &[0xFF, 0xFF, 0xFF, 0xFF, 0x02]));

    let mut chunk_size: usize = 128;
    let mut states: HashMap<u32, ChunkState> = HashMap::new();

    loop {
        let mut head = [0u8; 1];
        if !read_exact_or(&mut stream, &mut head) {
            break; // client closed — the session is over
        }
        let fmt = head[0] >> 6;
        let csid_raw = (head[0] & 0x3F) as u32;
        let csid = if csid_raw == 0 {
            let mut ext = [0u8; 1];
            if !read_exact_or(&mut stream, &mut ext) {
                break;
            }
            64 + ext[0] as u32
        } else if csid_raw == 1 {
            let mut ext = [0u8; 2];
            if !read_exact_or(&mut stream, &mut ext) {
                break;
            }
            64 + ext[0] as u32 + (ext[1] as u32) * 256
        } else {
            csid_raw
        };

        let st = states.entry(csid).or_default();
        if !read_msg_header(&mut stream, fmt, st) {
            break;
        }
        // Payload chunk(s) for this header.
        let want = std::cmp::min(chunk_size as u32, st.msg_len - st.received) as usize;
        let mut chunk = vec![0u8; want];
        if !read_exact_or(&mut stream, &mut chunk) {
            break;
        }
        st.buf.extend_from_slice(&chunk);
        st.received += chunk.len() as u32;
        if st.received < st.msg_len {
            continue;
        }
        let msg_type = st.msg_type;
        let payload = std::mem::take(&mut st.buf);
        st.received = 0;
        handle_message(&mut stream, msg_type, payload, &mut stats, &mut chunk_size);
    }
    stats
}

/// What the fixture should expect: one publishing session, or just the
/// pre-flight probe (the dry run).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureMode {
    OneSession,
    ProbesOnly,
}

/// A loopback RTMP server on an ephemeral port.
///
/// The job probes the ingest with a bare TCP connect before publishing, so the
/// accept loop distinguishes the probe (no byte, or an immediate EOF) from a
/// real client (C0 = 0x03) instead of committing to the first connection.
struct RtmpFixture {
    port: u16,
    done: std::sync::mpsc::Receiver<RtmpStats>,
}

impl RtmpFixture {
    /// Wait for one publishing session (the real run).
    fn start() -> Self {
        Self::spawn(FixtureMode::OneSession)
    }

    /// Return as soon as the pre-flight probe was seen, without waiting for a
    /// session (the dry run must never open one).
    fn start_probes_only() -> Self {
        Self::spawn(FixtureMode::ProbesOnly)
    }

    fn spawn(mode: FixtureMode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback RTMP");
        listener
            .set_nonblocking(true)
            .expect("non-blocking accept loop");
        let port = listener.local_addr().expect("local addr").port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut probes = 0u32;
            loop {
                if Instant::now() > deadline {
                    break;
                }
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut c0 = [0u8; 1];
                        if read_exact_or(&mut stream, &mut c0) && c0[0] == 0x03 {
                            let stats = serve_rtmp(stream, c0[0]);
                            let _ = tx.send(stats);
                            return;
                        }
                        // The pre-flight probe: connect, read nothing, close.
                        probes += 1;
                        if mode == FixtureMode::ProbesOnly && probes >= 1 {
                            break;
                        }
                    }
                    Err(ref err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(RtmpStats::default());
        });
        RtmpFixture { port, done: rx }
    }

    /// Wait for the server thread to finish (the client disconnects after the
    /// run returns), up to `budget`.
    fn finish(self, budget: Duration) -> RtmpStats {
        let deadline = Instant::now() + budget;
        loop {
            match self.done.recv_timeout(Duration::from_millis(100)) {
                Ok(stats) => return stats,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("the RTMP fixture thread died without reporting")
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    assert!(
                        Instant::now() < deadline,
                        "the RTMP fixture never saw a complete client session \
                         (handshake + publish + disconnect) within {budget:?}"
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// #289 acceptance: a headless `rivulet stream` session really publishes
// ---------------------------------------------------------------------------

fn stream_config(port: u16) -> StreamConfig {
    StreamConfig {
        stream: StreamOutputConfig {
            platform: "custom".to_string(),
            ingest_url: Some(format!("rtmp://127.0.0.1:{port}/live")),
            stream_key: Some("fixture_key".to_string()),
            preset: Some("standard".to_string()),
        },
        video: VideoConfig {
            source: "test".to_string(),
            test_pattern: None,
            width: 320,
            height: 240,
            fps: 30,
        },
        audio: AudioConfig { enabled: true },
        duration_secs: Some(2),
    }
}

#[test]
fn a_stream_run_publishes_video_and_audio_to_a_local_ingest() {
    let fixture = RtmpFixture::start();
    let events: Arc<Mutex<Vec<StreamStatusEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let diagnostics: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let job_events = Arc::clone(&events);
    let job_diagnostics = Arc::clone(&diagnostics);
    let mut job = StreamJob::new(stream_config(fixture.port))
        .with_event_sink(Box::new(move |event| {
            job_events
                .lock()
                .expect("event sink lock")
                .push(event.clone());
        }))
        .with_diagnostic_sink(Box::new(move |line| {
            job_diagnostics
                .lock()
                .expect("diagnostic sink lock")
                .push(line.to_string());
        }));

    let report = match job.run(|| false) {
        Ok(report) => report,
        Err(failure) => {
            let message = failure.to_string();
            // No streaming leg in this GStreamer installation: the pipeline
            // parse failure names the missing element. Nothing to prove then.
            if message.contains("flvmux") || message.contains("rtmp2sink") {
                eprintln!("skipping: the streaming leg is unavailable ({message})");
                return;
            }
            panic!("the stream run must succeed: {message}");
        }
    };

    let stats = fixture.finish(Duration::from_secs(30));
    assert!(stats.handshake_done, "the client completed the handshake");
    assert!(stats.published, "the client sent `publish`");
    assert_eq!(
        stats.publish_name.as_deref(),
        Some("fixture_key"),
        "the configured stream key reaches the ingest (stats = {stats:?})"
    );
    assert!(
        stats.video_msgs > 0,
        "the server received video messages (stats = {stats:?})"
    );
    assert!(
        stats.audio_msgs > 0,
        "the server received audio messages (stats = {stats:?})"
    );

    // The report is the library-facing outcome: a two-second session at 30 fps
    // pushes ~60 frames, and the ingest saw at least one.
    assert!(report.frames > 0, "report = {report:?}");
    assert!(report.seconds >= 1, "report = {report:?}");
    assert!(
        stats.video_msgs as u64 <= report.frames + 2,
        "the sink must not report more frames than were pushed: \
         server={} report={report:?}",
        stats.video_msgs
    );

    // The status stream is the contract: started → progress* → stopped.
    let events = events.lock().expect("events lock");
    let started = events
        .iter()
        .find(|event| matches!(event, StreamStatusEvent::Started { .. }))
        .unwrap_or_else(|| panic!("no `started` event in {events:?}"));
    match started {
        StreamStatusEvent::Started {
            width,
            height,
            audio_enabled,
            ingest,
            key,
            ..
        } => {
            assert_eq!((*width, *height), (320, 240));
            assert!(*audio_enabled, "the session was configured with audio");
            assert!(
                ingest.starts_with("rtmp://127.0.0.1:"),
                "the ingest is reduced to scheme + authority: {ingest}"
            );
            assert!(!ingest.contains("live"), "no path in the event: {ingest}");
            assert_eq!(key, "fi••••", "the key is masked");
        }
        other => panic!("unexpected first event: {other:?}"),
    }
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamStatusEvent::Stopped { .. })),
        "the stream must end with a `stopped` event: {events:?}"
    );
    let json = serde_json::to_string(&*events).expect("events serialize");
    assert!(
        !json.contains("fixture_key"),
        "no event may leak the key: {json}"
    );

    // Diagnostics stay human-readable and never carry the key either.
    let diagnostics = diagnostics.lock().expect("diagnostics lock");
    assert!(
        diagnostics
            .iter()
            .any(|line| line.contains("ingest reachable")),
        "the pre-flight probe is reported: {diagnostics:?}"
    );
    assert!(
        !diagnostics.iter().any(|line| line.contains("fixture_key")),
        "no diagnostic may leak the key: {diagnostics:?}"
    );
}

#[test]
fn a_dry_run_against_a_local_ingest_reports_the_plan_without_publishing() {
    // The dry run probes the same endpoint but must never open a publish
    // session: no handshake, no video.
    let fixture = RtmpFixture::start_probes_only();
    let report = StreamJob::new(stream_config(fixture.port))
        .dry_run()
        .unwrap_or_else(|failure| {
            panic!("the dry run must reach the fixture: {failure}");
        });

    assert!(report.pipeline.contains("flvmux"), "{}", report.pipeline);
    assert!(report.pipeline.contains("rtmp2sink"), "{}", report.pipeline);
    assert_eq!(report.key, "fi••••");
    assert_eq!(
        report.ingest,
        format!("rtmp://127.0.0.1:{}", fixture.port),
        "the plan reports scheme + authority only"
    );
    let json = serde_json::to_string(&report).expect("plan serializes");
    assert!(!json.contains("fixture_key"), "{json}");

    // The fixture only sees the probe connection, which is not a session.
    let stats = fixture.finish(Duration::from_secs(10));
    assert!(
        !stats.handshake_done,
        "the dry run must not connect as a client"
    );
    assert_eq!(stats.video_msgs, 0);
}
