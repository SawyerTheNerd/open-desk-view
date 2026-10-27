//! Server side of the spacedesk protocol: discovery, TCP sessions, framebuffer streaming,
//! flow control, and input injection. Speaks the same wire format documented in docs/PROTOCOL.md,
//! so the OpenDeskView viewer connects to it with no changes.
#![cfg(windows)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use jpeg_encoder::{ColorType, Encoder};
use proto::{Encoding, Header, MsgType, HEADER_LEN};
use spacedesk_proto as proto;

use crate::settings::Settings;
use crate::{capture, display, h264, inject};

const PORT: u16 = proto::DEFAULT_PORT;
/// Unacknowledged H.264 frames allowed in flight (covers the viewer decoder's buffering).
const H264_WINDOW: u32 = 3;
/// After this long without a screen change, poll at the slower `Settings::idle_poll` rate.
const IDLE_AFTER: Duration = Duration::from_secs(1);
const STATS_EVERY: Duration = Duration::from_secs(2);

/// Which monitor a session streams.
#[derive(Clone, Copy)]
pub enum Target {
    /// The primary display (mirror mode).
    Primary,
    /// A monitor by `--list-displays` index.
    Index(usize),
    /// The Virtual Display Driver monitor, resized to the viewer's screen (extended mode).
    Virtual,
}

pub fn run(
    settings: Settings,
    bind: &str,
    allow_input: bool,
    target: Target,
    allow_h264: bool,
) -> std::io::Result<()> {
    // Discovery listens on the same interface as the TCP server, so --bind 127.0.0.1
    // keeps discovery local too instead of reflecting replies to the whole LAN.
    let disco_bind = bind.to_string();
    std::thread::spawn(move || {
        if let Err(e) = discovery_responder(&disco_bind) {
            eprintln!("[server] discovery responder stopped: {e}");
        }
    });

    let listener = TcpListener::bind((bind, PORT))?;
    println!("[server] listening on {bind}:{PORT}");
    println!(
        "[server] limits: up to {} fps, JPEG q{}, H.264 {}, idle poll {} ms",
        settings.max_fps,
        settings.jpeg_quality,
        match settings.bitrate_kbps {
            Some(k) => format!("{k} kbps"),
            None => format!("{} bits/pixel/s", settings.h264_bits_per_pixel),
        },
        settings.idle_poll.as_millis()
    );
    eprintln!("[server] SECURITY: connections are UNAUTHENTICATED and UNENCRYPTED.");
    if bind == "0.0.0.0" {
        eprintln!("[server]   Anyone who can reach this PC on the LAN can VIEW this screen.");
        eprintln!("[server]   Use --bind 127.0.0.1 to restrict to this machine only.");
    }
    if allow_input {
        eprintln!("[server]   INPUT INJECTION IS ENABLED — a connected viewer can control mouse/keyboard.");
    } else {
        eprintln!("[server]   Input injection is OFF (view-only). Pass --allow-input to let viewers control this PC.");
    }
    if allow_h264 {
        println!("[server] H.264 available for viewers that ask for it (--no-h264 to disable).");
        println!("[server]   OpenH264 Video Codec provided by Cisco Systems, Inc.");
    } else {
        println!("[server] H.264 disabled (--no-h264): streaming JPEG only.");
    }
    match target {
        Target::Primary => println!("[server] serving the primary display (mirror)"),
        Target::Index(i) => println!("[server] serving display index {i} (use --list-displays to see them)"),
        Target::Virtual => match display::find_virtual_display() {
            Some(n) => println!("[server] serving the virtual display {n} (extended)"),
            None => eprintln!("[server] WARNING: no Virtual Display Driver monitor found; clients will be refused"),
        },
    }

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(s, settings, allow_input, target, allow_h264) {
                        eprintln!("[server] client ended: {e}");
                    }
                });
            }
            Err(e) => eprintln!("[server] accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_client(
    stream: TcpStream,
    settings: Settings,
    allow_input: bool,
    target: Target,
    allow_h264: bool,
) -> std::io::Result<()> {
    let peer = stream.peer_addr().ok();
    println!("[server] client connected: {peer:?} (input {})", if allow_input { "ENABLED" } else { "off" });
    stream.set_nodelay(true).ok();
    let mut reader = stream.try_clone()?;
    let mut writer = stream;

    // Read the client's identification, which carries the resolution it wants.
    let mut hbuf = [0u8; HEADER_LEN];
    reader.read_exact(&mut hbuf)?;
    let h = Header::from_bytes(&hbuf).unwrap();
    let plen = h.payload_len();
    if plen > 0 && plen < (1 << 20) {
        let mut p = vec![0u8; plen];
        reader.read_exact(&mut p)?;
    }
    let (req_w, req_h) = if h.get_u32(48) >= 1 { (h.get_u32(52), h.get_u32(88)) } else { (0, 0) };
    // Requested encoding lives at offset 36 when the extended-identification flag (byte 38) is set.
    let req_enc = if h.bytes[38] & 0x01 != 0 { u16::from_le_bytes([h.bytes[36], h.bytes[37]]) as u32 } else { 0 };
    let mut use_h264 =
        allow_h264 && (req_enc == Encoding::H264 as u32 || req_enc == Encoding::ServerChoice as u32);
    // The viewer's preferred quality (1-100) and frame rate; our settings are the ceiling.
    let req_quality = h.get_u32(32);
    let req_fps = u16::from_le_bytes([h.bytes[44], h.bytes[45]]) as u32;
    let fps = settings.fps_for(req_fps);
    let jpeg_quality = settings.jpeg_quality_for(req_quality);
    println!(
        "[server] identification ok (client type {}, wants {req_w}x{req_h} {} q{req_quality} {req_fps} fps) -> up to {fps} fps",
        h.get_u32(16),
        if use_h264 { "H.264" } else { "JPEG" }
    );

    // Resolve the requested monitor to its STABLE device name. We identify by name (not
    // enumeration index) from here on, and — crucially — if a specific display was requested
    // we NEVER fall back to the primary: doing so would leak the main screen to the viewer.
    // If the requested display isn't present, refuse the connection.
    let target_name: Option<String> = match target {
        Target::Primary => None,
        Target::Index(i) => match capture::list_displays().into_iter().find(|d| d.index == i) {
            Some(d) => Some(d.name),
            None => {
                eprintln!("[server] display index {i} not found; refusing (will NOT fall back to primary)");
                return Ok(());
            }
        },
        Target::Virtual => match display::find_virtual_display() {
            Some(name) => {
                // Extended mode: size the virtual monitor to the viewer's screen.
                match display::match_mode(&name, req_w, req_h) {
                    Some((w, h)) => println!("[server] virtual display {name} set to {w}x{h}"),
                    None => eprintln!("[server] no mode fits {req_w}x{req_h}; keeping {name} as is"),
                }
                Some(name)
            }
            None => {
                eprintln!("[server] no Virtual Display Driver monitor; refusing (will NOT fall back to primary)");
                return Ok(());
            }
        },
    };

    let (ack_tx, ack_rx) = channel::<()>();
    let input_target = target_name.clone();
    // Input messages received from the viewer since the last stats line (diagnostics).
    let input_events = Arc::new(AtomicU32::new(0));
    let input_events_rx = input_events.clone();

    // Reader thread: flow-control acks + input + client pings.
    std::thread::spawn(move || {
        let mut hb = [0u8; HEADER_LEN];
        loop {
            if reader.read_exact(&mut hb).is_err() {
                break;
            }
            let hh = Header::from_bytes(&hb).unwrap();
            let plen = hh.payload_len();
            if plen > 0 {
                let mut p = vec![0u8; plen.min(1 << 20)];
                if reader.read_exact(&mut p).is_err() {
                    break;
                }
            }
            let ty = hh.msg_type();
            if matches!(ty, MsgType::Mouse | MsgType::Keyboard | MsgType::Touch) {
                input_events_rx.fetch_add(1, Ordering::Relaxed);
            }
            match ty {
                MsgType::FlowControlAck => {
                    let _ = ack_tx.send(());
                }
                // Remote control is gated behind --allow-input to avoid unauthenticated control.
                MsgType::Mouse if allow_input => inject_mouse(&hh, input_target.as_deref()),
                MsgType::Keyboard if allow_input => inject_key(&hh),
                MsgType::Touch if allow_input => inject_touch(&hh, input_target.as_deref()),
                _ => {}
            }
        }
    });

    // Sender loop: capture -> (skip if unchanged) -> encode -> framebuffer, paced by the frame
    // rate and flow-control acks. All buffers are reused, so steady state allocates nothing.
    let source_name = target_name.clone().unwrap_or_else(|| capture::primary_display().name);
    let mut source = capture::Source::new(&source_name);
    let frame_interval = Duration::from_secs_f64(1.0 / fps as f64);
    let mut encoder: Option<h264::H264Encoder> = None;
    let mut cur: Vec<u8> = Vec::new(); // this capture (BGRA)
    let mut prev: Vec<u8> = Vec::new(); // last capture we sent
    let mut scratch: Vec<u8> = Vec::new(); // odd-width crop for H.264
    let mut payload: Vec<u8> = Vec::new(); // encoded frame
    let mut msg: Vec<u8> = Vec::new(); // header + payload
    let mut last_ping = Instant::now();
    let mut last_frame = Instant::now() - frame_interval;
    let mut last_change = Instant::now();
    let mut in_flight: u32 = 0;
    let mut stats = Stats::default();
    let mut stats_t = Instant::now();
    loop {
        if stats_t.elapsed() >= STATS_EVERY {
            stats.print(stats_t.elapsed(), use_h264, input_events.swap(0, Ordering::Relaxed), allow_input);
            stats = Stats { dims: stats.dims, ..Stats::default() };
            stats_t = Instant::now();
        }
        if last_ping.elapsed() > Duration::from_secs(2) {
            if writer.write_all(&proto::build_ping()).is_err() {
                break;
            }
            last_ping = Instant::now();
        }

        while ack_rx.try_recv().is_ok() {
            in_flight = in_flight.saturating_sub(1);
        }
        // H.264 decoders hold a frame or two, so keep a few in flight; JPEG viewers ack each
        // frame as soon as it's painted.
        let window = if use_h264 { H264_WINDOW } else { 1 };
        if in_flight >= window {
            match ack_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(()) => in_flight -= 1,
                Err(RecvTimeoutError::Disconnected) => break, // reader thread ended: client gone
                Err(RecvTimeoutError::Timeout) => {
                    // No ack for 2s: check liveness, then let one frame through so a stalled
                    // viewer pipeline can drain instead of deadlocking.
                    if writer.write_all(&proto::build_ping()).is_err() {
                        break;
                    }
                    in_flight -= 1;
                }
            }
        }

        // Pace to the frame rate. Once the screen has been still for a moment, wait for the next
        // change inside the capture call instead (DXGI wakes us the moment one happens).
        let idle = last_change.elapsed() > IDLE_AFTER;
        if !idle {
            let since = last_frame.elapsed();
            if since < frame_interval {
                std::thread::sleep(frame_interval - since);
            }
        }
        last_frame = Instant::now();
        let wait = if idle { settings.idle_poll } else { Duration::ZERO };

        // Re-resolve the target by name each frame. If a requested display has gone away,
        // skip the frame and wait for it to return — never substitute the primary.
        let d = match &target_name {
            Some(name) => match capture::find_display_by_name(name) {
                Some(d) => d,
                None => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            },
            None => capture::primary_display(),
        };
        match source.grab(&d, wait, &mut cur) {
            capture::Grab::New => {
                stats.captures += 1;
                stats.capture += source.copy_time;
            }
            capture::Grab::Unchanged => {
                stats.unchanged += 1;
                continue;
            }
            capture::Grab::Failed => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        }

        // Nothing visibly changed (cursor included): don't encode or send anything.
        if cur == prev {
            stats.unchanged += 1;
            continue;
        }
        last_change = Instant::now();

        let (w, h) = (d.width as usize, d.height as usize);
        let t = Instant::now();
        let (encoding, w, h) = if use_h264 {
            let (view, w, h) = h264::even_view(&cur, w, h, &mut scratch);
            if encoder.as_ref().map_or(true, |e| e.dimensions() != (w, h)) {
                let bitrate = settings.h264_bitrate_for(w, h, req_quality);
                match h264::H264Encoder::new(w, h, bitrate) {
                    Ok(e) => {
                        println!("[server] H.264 encoder {w}x{h} @ {} kbps", bitrate / 1000);
                        encoder = Some(e);
                    }
                    Err(e) => {
                        eprintln!("[server] H.264 unavailable ({e}); falling back to JPEG");
                        use_h264 = false;
                        continue;
                    }
                }
            }
            if !encoder.as_mut().is_some_and(|e| e.encode(view, &mut payload)) {
                continue;
            }
            (Encoding::H264, w, h)
        } else {
            payload.clear();
            if Encoder::new(&mut payload, jpeg_quality)
                .encode(&cur, w as u16, h as u16, ColorType::Bgra)
                .is_err()
            {
                continue;
            }
            (Encoding::Jpeg, w, h)
        };
        stats.encode += t.elapsed();

        write_framebuffer(&mut msg, w as u32, h as u32, &payload, encoding);
        if writer.write_all(&msg).is_err() {
            break;
        }
        std::mem::swap(&mut prev, &mut cur);
        in_flight += 1;
        stats.frames += 1;
        stats.bytes += payload.len() as u64;
        stats.dims = (w, h);
    }
    println!("[server] client disconnected: {peer:?}");
    Ok(())
}

/// Per-interval streaming statistics for the log.
#[derive(Default)]
struct Stats {
    frames: u64,
    captures: u64,
    unchanged: u64,
    bytes: u64,
    capture: Duration,
    encode: Duration,
    dims: (usize, usize),
}

impl Stats {
    fn print(&self, elapsed: Duration, h264: bool, inputs: u32, allow_input: bool) {
        let secs = elapsed.as_secs_f64();
        println!(
            "[server] {:.1} fps ({}x{} {}, {} KB/frame, {:.1} Mbps) capture {:.1} ms, encode {:.1} ms, {} unchanged skipped{}",
            self.frames as f64 / secs,
            self.dims.0,
            self.dims.1,
            if h264 { "H.264" } else { "JPEG" },
            self.bytes / self.frames.max(1) / 1024,
            self.bytes as f64 * 8.0 / secs / 1e6,
            self.capture.as_secs_f64() * 1000.0 / self.captures.max(1) as f64,
            self.encode.as_secs_f64() * 1000.0 / self.frames.max(1) as f64,
            self.unchanged,
            match (inputs, allow_input) {
                (0, _) => String::new(),
                (n, true) => format!(", {n} input events"),
                (n, false) => format!(", {n} input events IGNORED (no --allow-input)"),
            }
        );
    }
}

/// Build one full-frame framebuffer message (a whole JPEG image or one H.264 access unit)
/// into `msg`, reusing its allocation.
fn write_framebuffer(msg: &mut Vec<u8>, w: u32, h: u32, payload: &[u8], encoding: Encoding) {
    let mut hd = Header::new(MsgType::Framebuffer);
    hd.set_u32(4, payload.len() as u32); // payloadLen
    hd.set_u32(8, w); // width
    hd.set_u32(12, h); // height
    hd.set_u32(16, w * 4); // pitch
    hd.set_u32(20, 21); // A8R8G8B8
    hd.set_u32(24, 0); // rect left
    hd.set_u32(28, 0); // rect top
    hd.set_u32(32, w); // rect right
    hd.set_u32(36, h); // rect bottom
    hd.set_u32(40, encoding as u32); // encoding
    hd.set_u32(60, 1); // fragment count
    hd.set_u32(64, 0); // fragmentInfo: 0 = whole frame
    hd.set_u32(68, payload.len() as u32); // payloadSize
    msg.clear();
    msg.extend_from_slice(&hd.bytes);
    msg.extend_from_slice(payload);
}

fn inject_mouse(h: &Header, target: Option<&str>) {
    let (x, y, wheel, flags) = (h.get_i32(8), h.get_i32(12), h.get_i32(16), h.get_u32(20));
    if flags & MOUSEEVENTF_ABSOLUTE == 0 {
        inject::mouse(x, y, wheel, flags); // relative motion needs no mapping
        return;
    }
    // Absolute coords are 0..=65535 across the streamed frame.
    if let Some((ax, ay, extra)) = to_abs(target, x as i64, 65536, y as i64, 65536) {
        inject::mouse(ax, ay, wheel, flags | extra);
    }
}

fn inject_key(h: &Header) {
    inject::key(h.get_u32(8) as u16, h.get_u32(12) as u16, h.get_u32(16));
}

const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_VIRTUALDESK: u32 = 0x4000;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;

/// Map a point given as `x/xden`, `y/yden` of the streamed display to SendInput absolute
/// coordinates. With a target monitor the point is placed on that monitor and expressed
/// across the whole virtual desktop. Returns None (drop the event) if the target monitor
/// is gone, so input is never redirected to another screen.
fn to_abs(target: Option<&str>, x: i64, xden: i64, y: i64, yden: i64) -> Option<(i32, i32, u32)> {
    let (xden, yden) = (xden.max(1), yden.max(1));
    let Some(name) = target else {
        return Some(((x * 65535 / xden) as i32, (y * 65535 / yden) as i32, 0));
    };
    let d = capture::find_display_by_name(name)?;
    let (vx, vy, vw, vh) = capture::virtual_screen();
    let px = d.left as i64 + x.clamp(0, xden - 1) * d.width as i64 / xden;
    let py = d.top as i64 + y.clamp(0, yden - 1) * d.height as i64 / yden;
    let ax = (px - vx as i64) * 65535 / (vw as i64 - 1).max(1);
    let ay = (py - vy as i64) * 65535 / (vh as i64 - 1).max(1);
    Some((ax as i32, ay as i32, MOUSEEVENTF_VIRTUALDESK))
}

fn inject_touch(h: &Header, target: Option<&str>) {
    let x = h.get_i32(8) as i64;
    let y = h.get_i32(12) as i64;
    let resx = h.get_u32(16) as i64;
    let resy = h.get_u32(20) as i64;
    let flags = h.get_u32(24);
    let Some((ax, ay, extra)) = to_abs(target, x, resx, y, resy) else {
        return;
    };
    let abs = MOUSEEVENTF_ABSOLUTE | extra;
    // Drive the cursor as absolute mouse (real multi-touch injection comes later).
    inject::mouse(ax, ay, 0, MOUSEEVENTF_MOVE | abs);
    if flags & proto::TOUCH_DOWN != 0 {
        inject::mouse(ax, ay, 0, MOUSEEVENTF_LEFTDOWN | abs);
    }
    if flags & proto::TOUCH_UP != 0 {
        inject::mouse(ax, ay, 0, MOUSEEVENTF_LEFTUP | abs);
    }
}

fn discovery_responder(bind: &str) -> std::io::Result<()> {
    let sock = UdpSocket::bind((bind, PORT))?;
    let mut buf = [0u8; 2048];

    // Reflection/amplification defenses. The reply (308 B) is larger than the probe
    // (~20 B), so an unbounded responder could be abused as a UDP amplifier.
    const MIN_INTERVAL: Duration = Duration::from_millis(500); // per-source min gap
    const MAX_TRACKED: usize = 4096; // hard cap on the per-IP map
    const PRUNE_EVERY: Duration = Duration::from_secs(1); // timer-based pruning
    const STALE_AFTER: Duration = Duration::from_secs(60);
    const GLOBAL_MAX_PER_SEC: u32 = 50; // total replies/sec across all sources

    let mut last_reply: HashMap<IpAddr, Instant> = HashMap::new();
    let mut last_prune = Instant::now();
    let mut window_start = Instant::now();
    let mut window_count: u32 = 0;

    loop {
        let (n, from) = sock.recv_from(&mut buf)?;
        // Require the full magic token, not just a prefix, to reduce spoofed triggers.
        if !buf[..n].starts_with(b"SPACEDESK-NET-CLIENT") {
            continue;
        }
        // This is a LAN discovery beacon: only local-network sources can be legitimate
        // clients. Ignoring everything else removes the internet reflection vector and
        // keeps a remote flood from draining the shared global budget below.
        if !is_lan_source(&from.ip()) {
            continue;
        }
        let now = Instant::now();

        // Prune stale entries on a timer, not on every packet.
        if now.duration_since(last_prune) >= PRUNE_EVERY {
            last_reply.retain(|_, t| now.duration_since(*t) < STALE_AFTER);
            last_prune = now;
        }

        // Global cap: never emit more than GLOBAL_MAX_PER_SEC replies in any 1s window.
        if now.duration_since(window_start) >= Duration::from_secs(1) {
            window_start = now;
            window_count = 0;
        }
        if window_count >= GLOBAL_MAX_PER_SEC {
            continue;
        }

        // Per-source cap.
        let ip = from.ip();
        if let Some(t) = last_reply.get(&ip) {
            if now.duration_since(*t) < MIN_INTERVAL {
                continue;
            }
        } else if last_reply.len() >= MAX_TRACKED {
            // Map is full of other recent sources; drop rather than grow unbounded.
            continue;
        }
        last_reply.insert(ip, now);

        window_count += 1;
        let reply = build_discovery_reply();
        let _ = sock.send_to(&reply, from);
    }
}

/// True for source addresses that could be a client on the local network:
/// loopback, RFC1918 private, link-local, CGNAT, plus IPv6 loopback / link-local (fe80::/10)
/// / unique-local (fc00::/7). Everything else (public internet) is ignored.
fn is_lan_source(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || is_cgnat(v4)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback() || is_v6_link_local(v6) || is_v6_unique_local(v6)
        }
    }
}

// 100.64.0.0/10 (RFC 6598 carrier-grade NAT), as used by e.g. Tailscale.
fn is_cgnat(v4: &Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 100 && (o[1] & 0xc0) == 64
}

fn is_v6_link_local(v6: &Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xffc0) == 0xfe80
}

fn is_v6_unique_local(v6: &Ipv6Addr) -> bool {
    (v6.segments()[0] & 0xfe00) == 0xfc00
}

fn build_discovery_reply() -> Vec<u8> {
    let mut reply = vec![0u8; 308];
    let name = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "OpenDeskView".to_string());
    let name_utf16: Vec<u8> = name.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let end = (8 + name_utf16.len()).min(reply.len());
    reply[8..end].copy_from_slice(&name_utf16[..end - 8]);
    // version 4.8 at offsets 272 / 276 (matches observed replies)
    reply[272..276].copy_from_slice(&4u32.to_le_bytes());
    reply[276..280].copy_from_slice(&8u32.to_le_bytes());
    reply
}
