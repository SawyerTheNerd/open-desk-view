//! spacedesk client networking: discovery, connection, the receive loop, and input writer.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;
use std::time::Duration;

use spacedesk_proto as proto;
use proto::{Encoding, Framebuffer, Header, Identification, MsgType, HEADER_LEN};

use crate::frame::SharedFrame;

/// Called by the receive loop whenever a new frame is ready, so the UI can repaint.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// Hard cap on any single message payload from the (untrusted) server. A 4K JPEG band is a few MB;
/// 64 MB is far more than legitimate traffic needs and bounds memory against a malicious length.
pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;

/// Upper bound passed to the JPEG decoder for its working buffer.
const MAX_JPEG_BUFFER: usize = 128 * 1024 * 1024;

/// A discovered server on the LAN.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub addr: SocketAddr,
    pub name: String,
}

/// Broadcast the discovery probe and collect replies for `wait`.
pub fn discover(wait: Duration) -> io::Result<Vec<Discovered>> {
    let sock = UdpSocket::bind(("0.0.0.0", 0))?;
    sock.set_broadcast(true)?;
    sock.set_read_timeout(Some(Duration::from_millis(250)))?;

    let target = ("255.255.255.255", proto::DEFAULT_PORT);
    for _ in 0..5 {
        let _ = sock.send_to(proto::DISCOVERY_MAGIC, target);
    }

    let mut found: Vec<Discovered> = Vec::new();
    let deadline = std::time::Instant::now() + wait;
    let mut buf = [0u8; 2048];
    while std::time::Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                let name = extract_utf16le_name(&buf[..n]);
                let addr = SocketAddr::new(from.ip(), proto::DEFAULT_PORT);
                if !found.iter().any(|d| d.addr == addr) {
                    found.push(Discovered { addr, name });
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
    }
    Ok(found)
}

/// Best-effort: find the longest run of printable UTF-16LE characters in a discovery reply.
fn extract_utf16le_name(buf: &[u8]) -> String {
    let mut best = String::new();
    let mut cur = String::new();
    let mut i = 0;
    while i + 1 < buf.len() {
        let lo = buf[i];
        let hi = buf[i + 1];
        if hi == 0 && lo.is_ascii_graphic() || (hi == 0 && lo == b' ') {
            cur.push(lo as char);
        } else {
            if cur.len() > best.len() {
                best = cur.clone();
            }
            cur.clear();
        }
        i += 2;
    }
    if cur.len() > best.len() {
        best = cur;
    }
    best
}

/// Headless protocol probe: connect, identify, and summarize what the server sends for `secs`.
/// Used to validate the handshake against a live server without opening a window.
pub fn probe(addr: SocketAddr, id: Identification, secs: u64) -> io::Result<()> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true).ok();
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    let mut wstream = stream.try_clone()?;

    wstream.write_all(&proto::build_identification(&id))?;
    eprintln!("[probe] sent identification ({} bytes)", HEADER_LEN + 334);

    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    let mut hbuf = [0u8; HEADER_LEN];
    let mut counts: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    let mut first_fb_logged = false;

    while std::time::Instant::now() < deadline {
        match stream.read_exact(&mut hbuf) {
            Ok(()) => {}
            Err(ref e)
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
        let header = Header::from_bytes(&hbuf).unwrap();
        let ty = header.get_u32(0);
        let plen = header.payload_len();
        if plen > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("server declared payload {plen} > cap {MAX_PAYLOAD}"),
            ));
        }
        *counts.entry(ty).or_insert(0) += 1;

        let payload = if plen > 0 {
            // drain payload even on timeout by looping
            let mut v = vec![0u8; plen];
            let mut got = 0;
            while got < plen {
                match stream.read(&mut v[got..]) {
                    Ok(0) => break,
                    Ok(n) => got += n,
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut => {}
                    Err(e) => return Err(e),
                }
            }
            v
        } else {
            Vec::new()
        };

        match MsgType::from_u32(ty) {
            MsgType::Framebuffer if !first_fb_logged => {
                let fb = Framebuffer::parse(&header);
                eprintln!(
                    "[probe] FIRST FRAMEBUFFER: {}x{} pitch={} pxfmt={} rect=({},{})-({},{}) enc={} fragInfo={} payload={}",
                    fb.width, fb.height, fb.pitch, fb.pixel_format,
                    fb.rect_left, fb.rect_top, fb.rect_right, fb.rect_bottom,
                    fb.encoding, fb.fragment_info, fb.payload_len
                );
                let n = payload.len().min(16);
                let head: Vec<String> = payload[..n].iter().map(|b| format!("{b:02X}")).collect();
                let kind = if payload.first() == Some(&0xFF) && payload.get(1) == Some(&0xD8) {
                    "JPEG SOI"
                } else if payload.starts_with(&[0, 0, 0, 1]) || payload.starts_with(&[0, 0, 1]) {
                    "H.264 Annex-B start code"
                } else {
                    "unknown"
                };
                eprintln!("[probe] payload[0..{n}] = {}  ({kind})", head.join(" "));
                first_fb_logged = true;
                // keep the stream flowing
                wstream.write_all(&proto::build_flow_ack()).ok();
            }
            MsgType::Framebuffer => {
                wstream.write_all(&proto::build_flow_ack()).ok();
            }
            MsgType::Ping => {
                wstream.write_all(&proto::build_ping()).ok();
            }
            _ => {}
        }
    }

    eprintln!("[probe] message type counts over {secs}s:");
    for (ty, n) in &counts {
        eprintln!("  type {:>2} ({:?}): {}", ty, MsgType::from_u32(*ty), n);
    }
    Ok(())
}

/// An active connection. Send input messages (from `spacedesk_proto::build_*`) through `input`.
pub struct Connection {
    pub input: Sender<Vec<u8>>,
}

/// Connect, identify, and spawn the reader + writer threads. Returns immediately.
pub fn start(
    addr: SocketAddr,
    id: Identification,
    frame: SharedFrame,
    wake: Wake,
) -> io::Result<Connection> {
    let stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true).ok();
    let mut wstream = stream.try_clone()?;

    let (tx, rx) = channel::<Vec<u8>>();

    // Writer thread: drains the queue (identification, acks, ping replies, input) to the socket.
    std::thread::spawn(move || {
        while let Ok(buf) = rx.recv() {
            if wstream.write_all(&buf).is_err() {
                break;
            }
        }
    });

    // Kick off the handshake.
    tx.send(proto::build_identification(&id)).ok();

    // Reader thread.
    let tx_reader = tx.clone();
    std::thread::spawn(move || {
        if let Err(e) = read_loop(stream, tx_reader, frame, wake) {
            eprintln!("[net] receive loop ended: {e}");
        }
    });

    Ok(Connection { input: tx })
}

fn read_exact_vec(stream: &mut TcpStream, len: usize) -> io::Result<Vec<u8>> {
    let mut v = vec![0u8; len];
    stream.read_exact(&mut v)?;
    Ok(v)
}

fn read_loop(
    mut stream: TcpStream,
    tx: Sender<Vec<u8>>,
    frame: SharedFrame,
    wake: Wake,
) -> io::Result<()> {
    let mut hbuf = [0u8; HEADER_LEN];
    let mut frag_counter: u32 = 0;
    let mut warned_encoding = false;
    let mut h264_dec: Option<openh264::decoder::Decoder> = None;
    let mut h264_tried = false;
    let mut h264_frames: u64 = 0;

    loop {
        stream.read_exact(&mut hbuf)?;
        let header = Header::from_bytes(&hbuf).expect("128 bytes");
        let plen = header.payload_len();
        if plen > MAX_PAYLOAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("server declared payload {plen} > cap {MAX_PAYLOAD}"),
            ));
        }
        let payload = if plen > 0 { read_exact_vec(&mut stream, plen)? } else { Vec::new() };

        match header.msg_type() {
            MsgType::Framebuffer => {
                let fb = Framebuffer::parse(&header);
                let mut committed = false;
                if fb.encoding == Encoding::Jpeg as u32 {
                    if let Err(e) = decode_jpeg_tile(&fb, &payload, &frame) {
                        eprintln!("[net] jpeg decode failed: {e}");
                    }
                } else if fb.encoding == Encoding::H264 as u32 {
                    if h264_dec.is_none() && !h264_tried {
                        h264_tried = true;
                        match create_h264_decoder() {
                            Ok(d) => {
                                eprintln!("[net] OpenH264 decoder loaded");
                                h264_dec = Some(d);
                            }
                            Err(e) => eprintln!("[net] H.264 unavailable: {e}"),
                        }
                    }
                    if let Some(d) = h264_dec.as_mut() {
                        if decode_h264_frame(d, &payload, &frame) {
                            h264_frames += 1;
                            if h264_frames % 30 == 0 {
                                eprintln!("[net] H.264 rendered {h264_frames} frames");
                            }
                        }
                    }
                } else if !warned_encoding {
                    eprintln!("[net] server sent unsupported encoding {}", fb.encoding);
                    warned_encoding = true;
                }

                frag_counter = frag_counter.wrapping_add(1);
                // Ack + repaint on each complete frame: non-fragmented tile, or every 4th band.
                if fb.fragment_info == 0 || frag_counter % 4 == 0 {
                    tx.send(proto::build_flow_ack()).ok();
                    committed = true;
                }
                if committed {
                    {
                        let mut f = frame.lock().unwrap();
                        f.generation = f.generation.wrapping_add(1);
                    }
                    wake();
                }
            }
            MsgType::Ping => {
                tx.send(proto::build_ping()).ok();
            }
            MsgType::Disconnect => {
                eprintln!("[net] server requested disconnect");
                return Ok(());
            }
            _ => { /* Visibility / Rotation / Settings / cursor: ignored for now */ }
        }
    }
}

/// Locate Cisco's prebuilt OpenH264 library: `OPENH264_DLL` env, next to the exe, or CWD.
fn find_openh264_lib() -> Option<std::path::PathBuf> {
    const NAME: &str = "openh264-2.6.0-win64.dll";
    if let Ok(p) = std::env::var("OPENH264_DLL") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(NAME);
            if p.exists() {
                return Some(p);
            }
        }
    }
    let p = std::path::PathBuf::from(NAME);
    if p.exists() {
        return Some(p);
    }
    None
}

fn create_h264_decoder() -> Result<openh264::decoder::Decoder, String> {
    let lib = find_openh264_lib().ok_or_else(|| {
        "openh264-2.6.0-win64.dll not found — put it next to the viewer, set OPENH264_DLL, or \
         download it from http://ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2"
            .to_string()
    })?;
    let api = openh264::OpenH264API::from_blob_path(&lib)
        .map_err(|e| format!("loading {}: {e}", lib.display()))?;
    openh264::decoder::Decoder::with_api_config(api, openh264::decoder::DecoderConfig::new())
        .map_err(|e| format!("decoder init failed: {e}"))
}

/// Decode one full Annex-B access unit to RGB and composite it as a full frame.
/// Returns true when a picture was produced.
fn decode_h264_frame(dec: &mut openh264::decoder::Decoder, payload: &[u8], frame: &SharedFrame) -> bool {
    match dec.decode(payload) {
        Ok(Some(yuv)) => {
            use openh264::formats::YUVSource;
            let (w, h) = yuv.dimensions();
            if w == 0 || h == 0 {
                return false;
            }
            let mut rgb = vec![0u8; w * h * 3];
            yuv.write_rgb8(&mut rgb);
            let mut f = frame.lock().unwrap();
            f.blit_rgb_tile(w as u32, h as u32, 0, 0, w as u32, h as u32, &rgb);
            true
        }
        Ok(None) => false, // decoder needs more NAL units before the first picture
        Err(_e) => false,  // corrupt/early frame: skip, keep going (openh264 recommends this)
    }
}

fn decode_jpeg_tile(fb: &Framebuffer, payload: &[u8], frame: &SharedFrame) -> io::Result<()> {
    use jpeg_decoder::{Decoder, PixelFormat};
    let mut dec = Decoder::new(std::io::Cursor::new(payload));
    dec.set_max_decoding_buffer_size(MAX_JPEG_BUFFER);
    let pixels = dec
        .decode()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    let info = dec
        .info()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no jpeg info"))?;

    // Normalize to RGB24.
    let rgb: Vec<u8> = match info.pixel_format {
        PixelFormat::RGB24 => pixels,
        PixelFormat::L8 => {
            let mut out = Vec::with_capacity(pixels.len() * 3);
            for g in pixels {
                out.push(g);
                out.push(g);
                out.push(g);
            }
            out
        }
        other => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported jpeg pixel format {other:?}"),
            ));
        }
    };

    let tile_w = info.width as u32;
    let tile_h = info.height as u32;
    let mut f = frame.lock().unwrap();
    f.blit_rgb_tile(fb.width, fb.height, fb.rect_left, fb.rect_top, tile_w, tile_h, &rgb);
    Ok(())
}
