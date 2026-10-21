//! Clean-room implementation of the spacedesk display-streaming wire protocol (v4.8).
//!
//! See `docs/PROTOCOL.md` for the full specification this is written from. Every message is a
//! fixed 128-byte little-endian header optionally followed by a payload whose length lives at
//! offset 4. This crate is transport-agnostic: it only builds and parses bytes.

#![forbid(unsafe_code)]

pub const HEADER_LEN: usize = 128;
pub const DEFAULT_PORT: u16 = 28252;
pub const PROTO_MAJOR: u32 = 4;
pub const PROTO_MINOR: u32 = 8;

/// UDP discovery probe: ASCII, NUL-terminated, broadcast 5x (see PROTOCOL.md §10).
pub const DISCOVERY_MAGIC: &[u8] = b"SPACEDESK-NET-CLIENT\0";

/// Message type codes (header offset 0). Names match the service binary's
/// `PROTOCOL_SPCDSK_SMPL_HEADER_TYPE_*` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MsgType {
    Identification = 0,
    Ping = 1,
    Framebuffer = 2,
    Visibility = 3,
    Settings = 4,
    CursorBmp = 5,
    FlowControlAck = 7,
    Disconnect = 8,
    Rotation = 9,
    Mouse = 10,
    Keyboard = 11,
    Touch = 12,
    Pen = 13,
    Speaker = 14,
    Unknown = 0xFFFF_FFFF,
}

impl MsgType {
    pub fn from_u32(v: u32) -> MsgType {
        match v {
            0 => MsgType::Identification,
            1 => MsgType::Ping,
            2 => MsgType::Framebuffer,
            3 => MsgType::Visibility,
            4 => MsgType::Settings,
            5 => MsgType::CursorBmp,
            7 => MsgType::FlowControlAck,
            8 => MsgType::Disconnect,
            9 => MsgType::Rotation,
            10 => MsgType::Mouse,
            11 => MsgType::Keyboard,
            12 => MsgType::Touch,
            13 => MsgType::Pen,
            14 => MsgType::Speaker,
            _ => MsgType::Unknown,
        }
    }
}

/// Video encoding / compression (identification offset 36, framebuffer offset 40).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum Encoding {
    RawBgrx = 0,
    Yuv420 = 1,
    Jpeg = 3,
    H264 = 4,
    ServerChoice = 5,
}

impl Encoding {
    pub fn from_u32(v: u32) -> Option<Encoding> {
        match v {
            0 => Some(Encoding::RawBgrx),
            1 => Some(Encoding::Yuv420),
            3 => Some(Encoding::Jpeg),
            4 => Some(Encoding::H264),
            5 => Some(Encoding::ServerChoice),
            _ => None,
        }
    }
}

/// A raw 128-byte message header. Fields are read/written by offset.
#[derive(Clone)]
pub struct Header {
    pub bytes: [u8; HEADER_LEN],
}

impl Default for Header {
    fn default() -> Self {
        Header { bytes: [0u8; HEADER_LEN] }
    }
}

impl Header {
    pub fn new(ty: MsgType) -> Self {
        let mut h = Header::default();
        h.set_u32(0, ty as u32);
        h
    }

    pub fn from_bytes(buf: &[u8]) -> Option<Header> {
        if buf.len() < HEADER_LEN {
            return None;
        }
        let mut bytes = [0u8; HEADER_LEN];
        bytes.copy_from_slice(&buf[..HEADER_LEN]);
        Some(Header { bytes })
    }

    #[inline]
    pub fn set_u32(&mut self, off: usize, v: u32) {
        self.bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn set_u16(&mut self, off: usize, v: u16) {
        self.bytes[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn set_i32(&mut self, off: usize, v: i32) {
        self.bytes[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    #[inline]
    pub fn get_u32(&self, off: usize) -> u32 {
        u32::from_le_bytes(self.bytes[off..off + 4].try_into().unwrap())
    }
    #[inline]
    pub fn get_i32(&self, off: usize) -> i32 {
        i32::from_le_bytes(self.bytes[off..off + 4].try_into().unwrap())
    }

    pub fn msg_type(&self) -> MsgType {
        MsgType::from_u32(self.get_u32(0))
    }
    pub fn payload_len(&self) -> usize {
        self.get_u32(4) as usize
    }
}

/// Parameters for the identification handshake.
#[derive(Debug, Clone)]
pub struct Identification {
    pub client_type: u32,
    pub quality: u32,
    pub encoding: Encoding,
    pub fps: u16,
    pub width: u32,
    pub height: u32,
    pub device_guid: String, // e.g. "{A1B2C3D4-....}"
    pub device_name: String,
}

impl Default for Identification {
    fn default() -> Self {
        Identification {
            client_type: 0, // display / WindowsRemoteMonitor
            quality: 80,
            encoding: Encoding::Jpeg,
            fps: 60,
            width: 1920,
            height: 1080,
            device_guid: "{00000000-0000-0000-0000-000000000001}".to_string(),
            device_name: "spacedesk-rs".to_string(),
        }
    }
}

const IDENT_PAYLOAD_LEN: usize = 334;

fn utf16le(s: &str, out: &mut Vec<u8>) {
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
}

/// Build the identification message (header + 334-byte payload) = 462 bytes.
pub fn build_identification(id: &Identification) -> Vec<u8> {
    let mut h = Header::new(MsgType::Identification);
    h.set_u32(4, IDENT_PAYLOAD_LEN as u32);
    h.set_u32(8, PROTO_MAJOR);
    h.set_u32(12, PROTO_MINOR);
    h.set_u32(16, id.client_type);
    h.set_u32(20, 3);
    h.set_u32(24, 3);
    h.set_u32(28, 2);
    h.set_u32(32, id.quality);
    h.set_u16(36, id.encoding as u16);
    h.bytes[38] = 0x01; // extended identification flag (enables encoding@36)
    h.set_u16(44, id.fps);
    h.set_u16(46, 4);
    h.set_u32(48, 1); // one resolution follows
    h.set_u32(52, id.width);
    h.set_u32(88, id.height);

    // Payload: u32=1, "{GUID}" UTF-16LE, 0x0000, device name UTF-16LE, zero-padded to 334.
    let mut payload = Vec::with_capacity(IDENT_PAYLOAD_LEN);
    payload.extend_from_slice(&1u32.to_le_bytes());
    utf16le(&id.device_guid, &mut payload);
    payload.extend_from_slice(&[0, 0]);
    utf16le(&id.device_name, &mut payload);
    payload.resize(IDENT_PAYLOAD_LEN, 0);

    let mut msg = Vec::with_capacity(HEADER_LEN + IDENT_PAYLOAD_LEN);
    msg.extend_from_slice(&h.bytes);
    msg.extend_from_slice(&payload);
    msg
}

/// A header-only message of the given type.
fn header_only(ty: MsgType) -> Vec<u8> {
    Header::new(ty).bytes.to_vec()
}

pub fn build_ping() -> Vec<u8> {
    header_only(MsgType::Ping)
}
pub fn build_flow_ack() -> Vec<u8> {
    header_only(MsgType::FlowControlAck)
}
pub fn build_disconnect() -> Vec<u8> {
    header_only(MsgType::Disconnect)
}

/// Mouse event (header-only). Deltas are relative unless the caller sets an absolute flag.
pub fn build_mouse(x: i32, y: i32, wheel: i32, buttons: u32, flags: u32) -> Vec<u8> {
    let mut h = Header::new(MsgType::Mouse);
    h.set_i32(8, x);
    h.set_i32(12, y);
    h.set_i32(16, wheel);
    h.set_u32(20, buttons);
    h.set_u32(24, flags);
    h.bytes.to_vec()
}

/// Keyboard event (header-only): virtual key, scan code, flags.
pub fn build_keyboard(vkey: u32, scancode: u32, flags: u32) -> Vec<u8> {
    let mut h = Header::new(MsgType::Keyboard);
    h.set_u32(8, vkey);
    h.set_u32(12, scancode);
    h.set_u32(16, flags);
    h.bytes.to_vec()
}

pub const TOUCH_ABSOLUTE: u32 = 0x01;
pub const TOUCH_DOWN: u32 = 0x02;
pub const TOUCH_UP: u32 = 0x04;

/// Touch event (header-only).
pub fn build_touch(x: i32, y: i32, res_x: u32, res_y: u32, flags: u32, ts: u32) -> Vec<u8> {
    let mut h = Header::new(MsgType::Touch);
    h.set_i32(8, x);
    h.set_i32(12, y);
    h.set_u32(16, res_x);
    h.set_u32(20, res_y);
    h.set_u32(24, flags);
    h.set_u32(28, ts);
    h.bytes.to_vec()
}

/// Parsed framebuffer header (type 2).
#[derive(Debug, Clone, Copy)]
pub struct Framebuffer {
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub pixel_format: u32,
    pub rect_left: u32,
    pub rect_top: u32,
    pub rect_right: u32,
    pub rect_bottom: u32,
    pub encoding: u32,
    pub fragment_count: u32,
    pub fragment_info: u32,
    pub payload_len: usize,
}

impl Framebuffer {
    pub fn parse(h: &Header) -> Framebuffer {
        Framebuffer {
            width: h.get_u32(8),
            height: h.get_u32(12),
            pitch: h.get_u32(16),
            pixel_format: h.get_u32(20),
            rect_left: h.get_u32(24),
            rect_top: h.get_u32(28),
            rect_right: h.get_u32(32),
            rect_bottom: h.get_u32(36),
            encoding: h.get_u32(40),
            fragment_count: h.get_u32(60),
            fragment_info: h.get_u32(64),
            payload_len: h.payload_len(),
        }
    }
    pub fn rect_w(&self) -> u32 {
        self.rect_right.saturating_sub(self.rect_left)
    }
    pub fn rect_h(&self) -> u32 {
        self.rect_bottom.saturating_sub(self.rect_top)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ident_is_462_bytes_and_well_formed() {
        let msg = build_identification(&Identification::default());
        assert_eq!(msg.len(), HEADER_LEN + IDENT_PAYLOAD_LEN);
        let h = Header::from_bytes(&msg).unwrap();
        assert_eq!(h.msg_type(), MsgType::Identification);
        assert_eq!(h.payload_len(), IDENT_PAYLOAD_LEN);
        assert_eq!(h.get_u32(8), PROTO_MAJOR);
        assert_eq!(h.get_u32(12), PROTO_MINOR);
        assert_eq!(h.get_u32(52), 1920);
        assert_eq!(h.get_u32(88), 1080);
        assert_eq!(u16::from_le_bytes([h.bytes[36], h.bytes[37]]), Encoding::Jpeg as u16);
    }

    #[test]
    fn header_only_messages_are_128_bytes() {
        assert_eq!(build_ping().len(), HEADER_LEN);
        assert_eq!(build_flow_ack().len(), HEADER_LEN);
        assert_eq!(MsgType::from_u32(7), MsgType::FlowControlAck);
    }

    #[test]
    fn framebuffer_roundtrip() {
        let mut h = Header::new(MsgType::Framebuffer);
        h.set_u32(4, 1234);
        h.set_u32(8, 1920);
        h.set_u32(12, 1080);
        h.set_u32(24, 0);
        h.set_u32(28, 0);
        h.set_u32(32, 1920);
        h.set_u32(36, 270);
        h.set_u32(40, Encoding::Jpeg as u32);
        let fb = Framebuffer::parse(&h);
        assert_eq!(fb.width, 1920);
        assert_eq!(fb.rect_w(), 1920);
        assert_eq!(fb.rect_h(), 270);
        assert_eq!(fb.encoding, Encoding::Jpeg as u32);
        assert_eq!(fb.payload_len, 1234);
    }
}
