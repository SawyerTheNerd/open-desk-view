# RE journal

Reverse-engineering log for the open-source spacedesk viewer. Append confirmed facts as we learn them.

## 2026-10-21 — recon

- Target: spacedesk Server, `C:\Program Files\datronicsoft\spacedesk\spacedeskService.exe` (4.97 MB,
  build Sep 2025). Native C++ (MSVC, SegmentHeap manifest).
- Live server (`spacedeskService`, PID 34572) listens: **TCP 0.0.0.0:28252**, **UDP :28252** per interface.
- Strings dump → protocol is self-documenting:
  - Framing enum `PROTOCOL_SPCDSK_SMPL_HEADER_TYPE_{IDENTIFICATION, DISPLAY_SETTINGS, DISPLAY_FRAMEBUFFER,
    DISPLAY_VISIBILITY, AUDIO_SPEAKER, ROTATION, PING, ERROR, FRAMEBUFFER_FLOW_CONTROL_ACK,
    EVT_INPUT_{MOUSE,KEYBOARD,TOUCH,PEN}}`.
  - Codec tasks: `SETaskEncodeJpegCpu`, `SETaskEncodeMftH264`, `SETaskEncodeYuvCpu/Gpu`, RGB buffers.
  - `L4ThreadNetDiscovery` (UDP discovery), `L4SocketTcpCommBase::ReceiveIdentification` detects HTTP →
    `WebSocketPacketHandshakeResponseCreate` (so the same port speaks raw-TCP **and** WebSocket).
  - `L5Protocol::PacketIdentificationValidate` validates: OS type, client type, compression type, device id,
    protocol version `(major,minor) expected (major,minor)`.
- Probe (`decomp/probe.py`): WebSocket upgrade on 28252 returns `101 Switching Protocols`; plain `GET /`
  returns nothing (no page served); raw connect sends nothing until the client identifies first. → client
  must speak first with an Identification message.
- Cross-checked against two prior clean-room repos (MarcozVD/SpaceDeskLinux, jeromeollivon-star/
  spacedesk-linux). All agree: 128-byte LE header, type at off0, payloadLen at off4, protocol v4.8.
- Full synthesized spec written to `docs/PROTOCOL.md`.

### Decisions
- Native viewer uses **raw TCP** (not WebSocket) — simplest cross-platform, matches existing clients.
- First milestone: **JPEG (encoding 3)** display session — pure-Rust decode, no FFmpeg dependency.
- Render with `winit` + `softbuffer` (CPU blit). No OS webview. H.264 is the follow-up.

## 2026-10-21 — Android viewer (Chromebook target)

- Goal: Chromebook as a touchscreen second monitor. Chose a native **Kotlin Android app** (Chromebooks run
  Android apps with real touch) over Crostini/HTML5.
- Built `android/` (Gradle 8.11.1 cached, AGP 8.7.3, Kotlin 1.9.24, JDK 21 from Android Studio JBR,
  compileSdk 35 / minSdk 24). Protocol re-implemented in Kotlin (`Protocol.kt`), same offsets as the Rust
  crate. JPEG bands decoded with `BitmapFactory`, composited on an ARGB_8888 bitmap, painted straight to a
  `SurfaceView` from the net thread. Touch → absolute spacedesk Touch (type 12).
- Identification requests an **extended display at the device's own screen resolution** (not a mirror).
- **Verified on a real device** (Fire TV, Android 9, model AFTR, over network adb): connected
  `server 1920x1080 enc=3`, steady **~30 fps** (`rendered N frames` logs), TCP ESTABLISHED to the server.
  `adb screencap` shows black because it can't capture the SurfaceView HW layer — the render loop logs prove
  frames are flowing.
- Fire TV has no touchscreen, so **touch is not yet verified live** — that happens on the Chromebook.
- APK: `android/app/build/outputs/apk/debug/app-debug.apk` (~810 KB).

### Open questions
- Exact extended-identification flag byte (off38) vs the observed off36=0x10003 in one source — confirm by
  live capture against this server.
- Discovery reply record exact layout beyond name/port/version.
- H.264 NAL framing across type-2 messages.
- Pen (type 13) and multi-touch layouts.
