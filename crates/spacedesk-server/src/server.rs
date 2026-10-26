//! Server side of the spacedesk protocol: discovery, TCP sessions, framebuffer streaming,
//! flow control, and input injection. Speaks the same wire format documented in docs/PROTOCOL.md,
//! so the OpenDeskView viewer connects to it with no changes.
#![cfg(windows)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, TcpListener, TcpStream, UdpSocket};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

use jpeg_encoder::{ColorType, Encoder};
use proto::{Encoding, Header, MsgType, HEADER_LEN};
use spacedesk_proto as proto;

use crate::{capture, display, inject};

const PORT: u16 = proto::DEFAULT_PORT;

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

pub fn run(quality: u8, bind: &str, allow_input: bool, target: Target) -> std::io::Result<()> {
    // Discovery listens on the same interface as the TCP server, so --bind 127.0.0.1
    // keeps discovery local too instead of reflecting replies to the whole LAN.
    let disco_bind = bind.to_string();
    std::thread::spawn(move || {
        if let Err(e) = discovery_responder(&disco_bind) {
            eprintln!("[server] discovery responder stopped: {e}");
        }
    });

    let listener = TcpListener::bind((bind, PORT))?;
    println!("[server] listening on {bind}:{PORT} (JPEG q{quality})");
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
                    if let Err(e) = handle_client(s, quality, allow_input, target) {
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
    quality: u8,
    allow_input: bool,
    target: Target,
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
    println!(
        "[server] identification ok (client type {}, wants {req_w}x{req_h})",
        h.get_u32(16)
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
            match hh.msg_type() {
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

    // Sender loop: capture -> JPEG -> framebuffer, paced by flow-control acks.
    let mut last_ping = Instant::now();
    let mut have_credit = true; // send the first frame immediately
    let mut frames: u64 = 0;
    let mut fps_t = Instant::now();
    loop {
        if !have_credit {
            // wait for the viewer to ack the previous frame before sending the next
            if ack_rx.recv_timeout(Duration::from_secs(2)).is_err() {
                // no ack in time; send a ping to check liveness and continue
                if writer.write_all(&proto::build_ping()).is_err() {
                    break;
                }
            }
        }

        // Re-resolve the target by name each frame. If a requested display has gone away,
        // skip the frame and wait for it to return — never substitute the primary.
        let frame = match &target_name {
            Some(name) => match capture::find_display_by_name(name).and_then(|d| capture::capture_display(&d)) {
                Some(f) => f,
                None => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            },
            None => match capture::capture_primary() {
                Some(f) => f,
                None => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
            },
        };
        let rgb = capture::bgra_to_rgb(&frame.bgra, frame.w, frame.h);
        let mut jpeg = Vec::new();
        if Encoder::new(&mut jpeg, quality)
            .encode(&rgb, frame.w as u16, frame.h as u16, ColorType::Rgb)
            .is_err()
        {
            continue;
        }

        let msg = build_framebuffer(frame.w, frame.h, &jpeg);
        if writer.write_all(&msg).is_err() {
            break;
        }
        have_credit = false;
        frames += 1;
        if fps_t.elapsed() >= Duration::from_secs(2) {
            let fps = frames as f64 / fps_t.elapsed().as_secs_f64();
            println!("[server] {fps:.1} fps ({}x{}, {} KB/frame)", frame.w, frame.h, jpeg.len() / 1024);
            frames = 0;
            fps_t = Instant::now();
        }

        if last_ping.elapsed() > Duration::from_secs(2) {
            let _ = writer.write_all(&proto::build_ping());
            last_ping = Instant::now();
        }

        // drain any extra acks so credit tracks reality
        while ack_rx.try_recv().is_ok() {
            have_credit = true;
        }
    }
    println!("[server] client disconnected: {peer:?}");
    Ok(())
}

fn build_framebuffer(w: u32, h: u32, jpeg: &[u8]) -> Vec<u8> {
    let mut hd = Header::new(MsgType::Framebuffer);
    hd.set_u32(4, jpeg.len() as u32); // payloadLen
    hd.set_u32(8, w); // width
    hd.set_u32(12, h); // height
    hd.set_u32(16, w * 4); // pitch
    hd.set_u32(20, 21); // A8R8G8B8
    hd.set_u32(24, 0); // rect left
    hd.set_u32(28, 0); // rect top
    hd.set_u32(32, w); // rect right
    hd.set_u32(36, h); // rect bottom
    hd.set_u32(40, Encoding::Jpeg as u32); // encoding
    hd.set_u32(60, 1); // fragment count
    hd.set_u32(64, 0); // fragmentInfo: 0 = whole frame
    hd.set_u32(68, jpeg.len() as u32); // payloadSize
    let mut msg = Vec::with_capacity(HEADER_LEN + jpeg.len());
    msg.extend_from_slice(&hd.bytes);
    msg.extend_from_slice(jpeg);
    msg
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
