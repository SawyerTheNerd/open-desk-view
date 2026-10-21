# spacedesk wire protocol (v4.8)

Clean-room reconstruction of the spacedesk display-streaming protocol, assembled from:

- Binary strings of `spacedeskService.exe` (datronicsoft spacedesk Server, build 2.x, Sep 2025) — the
  protocol enums are self-documenting (`PROTOCOL_SPCDSK_SMPL_HEADER_TYPE_*`).
- Live probing of a running server (WebSocket upgrade accepted on 28252; nothing sent until the client
  identifies first).
- Two prior independent clean-room efforts, cross-checked against the above:
  - MarcozVD/SpaceDeskLinux (`docs/protocol.md`)
  - jeromeollivon-star/spacedesk-linux (`docs/protocol.md`)

All three sources agree on header layout, type codes, and the identification handshake. This document is
our own synthesis; the Rust implementation in `crates/` is written from this document, not copied from any
existing client.

> **Legend.** `off` = byte offset into the message. All integers are **little-endian**. `u32` unless noted.

---

## 1. Transport

The server (the Windows machine running the spacedesk driver) listens on **port 28252** for:

- **TCP** — native viewers. Stream of `[128-byte header][payload]` messages back-to-back.
- **WebSocket** — the HTML5 viewer. Same messages, one per binary WS frame. Server auto-detects an HTTP
  `Upgrade: websocket` request on the same port and replies `101 Switching Protocols`.
- **UDP** — LAN discovery (see §6).

This viewer uses **raw TCP** (simplest, lowest-overhead, works identically on desktop and mobile). TLS is
off by default on current builds and is not implemented here.

Protocol version is **4.8**. The server accepts minor versions **7 and 8**.

---

## 2. Message header (128 bytes)

Every message begins with a fixed 128-byte header. Only the first few `u32` fields are common to all types;
the rest of the header is type-specific (fields below are given per type).

| off | type | field        | notes                                          |
|-----|------|--------------|------------------------------------------------|
| 0   | u32  | `type`       | message type (see §3)                          |
| 4   | u32  | `payloadLen` | number of payload bytes following the header   |
| 8…  | …    | type-specific| see each message                              |

Payload (if any) immediately follows the 128 bytes.

---

## 3. Message types

| code | name             | direction | payload                     |
|------|------------------|-----------|-----------------------------|
| 0    | Identification   | C → S     | 334 bytes (device id + name)|
| 1    | Ping             | S ↔ C     | none (keepalive)            |
| 2    | Framebuffer      | S → C     | encoded image / tile        |
| 3    | Visibility       | S → C     | none / small                |
| 4    | Settings/CursorPos | S → C   | —                           |
| 5    | CursorBmp        | S → C     | cursor bitmap               |
| 7    | FlowControlAck   | C → S     | none                        |
| 8    | Disconnect/Error | S ↔ C     | reason                      |
| 9    | Rotation         | S → C     | —                           |
| 10   | Mouse            | C → S     | none (fields in header)     |
| 11   | Keyboard         | C → S     | none                        |
| 12   | Touch            | C → S     | none                        |
| 13   | Pen              | C → S     | none (layout TBD)           |
| 14   | Speaker (audio)  | S → C     | PCM (separate session)      |

(Names match the `PROTOCOL_SPCDSK_SMPL_HEADER_TYPE_*` strings in the service binary.)

---

## 4. Identification (type 0) — client → server, sent first

128-byte header + 334-byte payload = **462 bytes total**.

Header:

| off | type | field           | value / meaning                                              |
|-----|------|-----------------|--------------------------------------------------------------|
| 0   | u32  | type            | `0`                                                          |
| 4   | u32  | payloadLen      | `334`                                                        |
| 8   | u32  | versionMajor    | `4`                                                          |
| 12  | u32  | versionMinor    | `8` (7 also accepted)                                        |
| 16  | u32  | clientType      | `0` display (WindowsRemoteMonitor); `3` = speaker/audio      |
| 20  | u32  | —               | `3` (observed)                                               |
| 24  | u32  | —               | `3` (observed; legacy compression slot)                      |
| 28  | u32  | —               | `2` (observed)                                               |
| 32  | u32  | quality         | JPEG quality 1–100 (default 50)                              |
| 36  | u16  | encoding        | see §5; used when extended-id flag set                       |
| 38  | u8   | flags           | bit0 = extended identification (enables `encoding` at 36)    |
| 44  | u16  | fps             | target frames per second                                     |
| 46  | u16  | —               | `4` (observed)                                               |
| 48  | u32  | numResolutions  | count of (w,h) pairs that follow                             |
| 52  | u32  | width[0]        | preferred width                                              |
| 88  | u32  | height[0]       | preferred height                                             |

Payload (334 bytes): `u32 = 1`, then the device GUID as `{XXXXXXXX-....}` in **UTF-16LE**, then `00 00`,
then the device/computer name in **UTF-16LE**, zero-padded to fill 334 bytes.

The server validates OS type, client type, encoding, device id, and `versionMajor,versionMinor`. On failure
it logs `L5Protocol::PacketIdentificationValidate failed!` and drops the connection after ~2 s.

---

## 5. Encoding (compression) values

| value | meaning                                   | framing                         |
|-------|-------------------------------------------|---------------------------------|
| 0     | raw BGRX (A8R8G8B8, uncompressed)         | 4 horizontal fragments per frame|
| 1     | raw YUV 4:2:0                             | —                               |
| 3     | **JPEG**                                  | 4 horizontal band tiles / frame |
| 4     | **H.264** (Annex-B)                       | one full frame per message      |
| 5     | server chooses (HW H.264 or SW JPEG)      | —                               |

This viewer implements **JPEG (3)** first; **H.264 (4)** is the follow-up (openh264 / FFmpeg decode).

---

## 6. Framebuffer (type 2) — server → client

Header:

| off | type | field        | meaning                                                   |
|-----|------|--------------|-----------------------------------------------------------|
| 0   | u32  | type         | `2`                                                       |
| 4   | u32  | payloadLen   | encoded bytes following header                            |
| 8   | u32  | width        | full screen width                                         |
| 12  | u32  | height       | full screen height                                        |
| 16  | u32  | pitch        | source stride in bytes                                    |
| 20  | u32  | pixelFormat  | `21` = A8R8G8B8                                           |
| 24  | u32  | rectLeft     | dirty/tile rectangle                                      |
| 28  | u32  | rectTop      |                                                           |
| 32  | u32  | rectRight    |                                                           |
| 36  | u32  | rectBottom   |                                                           |
| 40  | u32  | encoding     | see §5                                                     |
| 60  | u32  | fragmentCount| fragments making up this frame                            |
| 64  | u32  | fragmentInfo | `1` when this message is one fragment of a larger frame   |
| 68  | u32  | payloadSize  | redundant with payloadLen                                 |

For JPEG, the screen is split into **4 horizontal bands**; each band arrives as its own type-2 message whose
rect gives where to composite the decoded tile into the full framebuffer.

---

## 7. Flow control (type 7) — client → server

Header-only, `type=7`, `payloadLen=0`. The server will **not send the next frame until it receives an ack**.
Each ack grants one frame of credit, so acks must track real frame consumption, not be sent as heartbeats.
Official clients ack after each complete frame (every 4th JPEG fragment, or immediately for a whole tile).

---

## 8. Keepalive (type 1, Ping)

The server sends a Ping roughly every 10 s; the client must reply with a Ping (type 1). A client that stops
replying is dropped after ~45 s.

---

## 9. Input — client → server (all header-only)

### Mouse (type 10)
| off | field       | meaning                                                   |
|-----|-------------|-----------------------------------------------------------|
| 8   | x           | relative delta X (or absolute, per flags)                 |
| 12  | y           | relative delta Y                                          |
| 16  | wheelDelta  | scroll                                                     |
| 20  | buttonData  | Win32 `MOUSEEVENTF_*` button bits                         |
| 24  | flags       | `0x01` = move                                             |

### Keyboard (type 11)
Virtual key, scan code, and flags (extended, key-up, Unicode). Server injects by **scan code**.

### Touch (type 12)
| off | field  | meaning                                                   |
|-----|--------|-----------------------------------------------------------|
| 8   | x      | X                                                         |
| 12  | y      | Y                                                         |
| 16  | resX   | reported resolution width                                 |
| 20  | resY   | reported resolution height                                |
| 24  | flags  | `0x01` absolute, `0x02` down, `0x04` up                   |
| 28  | ts     | timestamp                                                 |

### Pen (type 13)
Layout not yet reversed.

---

## 10. Discovery (UDP 28252)

1. Client **broadcasts** the ASCII string `SPACEDESK-NET-CLIENT` followed by one `NUL` byte, repeated **5
   times** (to `255.255.255.255:28252` or the subnet broadcast).
2. Each server **replies** with a **308-byte** record containing the computer name (UTF-16LE), the TCP port,
   and version fields (version 4.8 appears at off 272 and off 276).

A viewer can also skip discovery and connect to a known server IP directly.

---

## 11. Connection sequence (JPEG display session)

```
client ── UDP broadcast SPACEDESK-NET-CLIENT\0 (x5) ──▶ (optional discovery)
server ── 308-byte reply (name, port, version) ──────▶
client ── TCP connect :28252 ───────────────────────▶
client ── Identification (type 0, encoding=3 JPEG) ──▶
server ── Visibility / Ping ─────────────────────────▶
server ── Framebuffer (type 2) band 0..3 ────────────▶  decode JPEG, composite
client ── FlowControlAck (type 7) ───────────────────▶  after each complete frame
server ── Ping (type 1) every ~10s ──────────────────▶
client ── Ping (type 1) reply ───────────────────────▶
client ── Mouse/Keyboard/Touch (10/11/12) ───────────▶  on user input
...
client ── Disconnect (type 8) ───────────────────────▶  on quit
```
