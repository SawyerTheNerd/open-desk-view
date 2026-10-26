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

use crate::{capture, inject};

const PORT: u16 = proto::DEFAULT_PORT;

pub fn run(quality: u8, bind: &str, allow_input: bool) -> std::io::Result<()> {
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

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                std::thread::spawn(move || {
                    if let Err(e) = handle_client(s, quality, allow_input) {
                        eprintln!("[server] client ended: {e}");
                    }
                });
            }
            Err(e) => eprintln!("[server] accept error: {e}"),
        }
    }
    Ok(())
}

fn handle_client(stream: TcpStream, quality: u8, allow_input: bool) -> std::io::Result<()> {
    let peer = stream.peer_addr().ok();
    println!("[server] client connected: {peer:?} (input {})", if allow_input { "ENABLED" } else { "off" });
    stream.set_nodelay(true).ok();
    let mut reader = stream.try_clone()?;
    let mut writer = stream;

    // Read the client's identification (we stream the primary display regardless of its request).
    let mut hbuf = [0u8; HEADER_LEN];
    reader.read_exact(&mut hbuf)?;
    let h = Header::from_bytes(&hbuf).unwrap();
    let plen = h.payload_len();
    if plen > 0 && plen < (1 << 20) {
        let mut p = vec![0u8; plen];
        reader.read_exact(&mut p)?;
    }
    println!("[server] identification ok (client type {})", h.get_u32(16));

    let (ack_tx, ack_rx) = channel::<()>();

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
                MsgType::Mouse if allow_input => inject_mouse(&hh),
                MsgType::Keyboard if allow_input => inject_key(&hh),
                MsgType::Touch if allow_input => inject_touch(&hh),
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

        let frame = match capture::capture_primary() {
            Some(f) => f,
            None => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
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

fn inject_mouse(h: &Header) {
    inject::mouse(h.get_i32(8), h.get_i32(12), h.get_i32(16), h.get_u32(20));
}

fn inject_key(h: &Header) {
    inject::key(h.get_u32(8) as u16, h.get_u32(12) as u16, h.get_u32(16));
}

const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;

fn inject_touch(h: &Header) {
    let x = h.get_i32(8) as i64;
    let y = h.get_i32(12) as i64;
    let resx = h.get_u32(16).max(1) as i64;
    let resy = h.get_u32(20).max(1) as i64;
    let flags = h.get_u32(24);
    let ax = (x * 65535 / resx) as i32;
    let ay = (y * 65535 / resy) as i32;
    // Drive the cursor as absolute mouse (real multi-touch injection comes later).
    inject::mouse(ax, ay, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE);
    if flags & proto::TOUCH_DOWN != 0 {
        inject::mouse(ax, ay, 0, MOUSEEVENTF_LEFTDOWN | MOUSEEVENTF_ABSOLUTE);
    }
    if flags & proto::TOUCH_UP != 0 {
        inject::mouse(ax, ay, 0, MOUSEEVENTF_LEFTUP | MOUSEEVENTF_ABSOLUTE);
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
