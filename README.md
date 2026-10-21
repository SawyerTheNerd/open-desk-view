# OpenDeskView — open-source spacedesk-compatible viewer

> **Independent project — not affiliated with, endorsed by, or connected to datronicsoft or spacedesk.**
> "spacedesk" is used only to describe protocol compatibility. See [`NOTICE.md`](NOTICE.md).

A clean-room, cross-platform **viewer** (client) that is compatible with the
[spacedesk](https://www.spacedesk.net/) display-streaming protocol, the Windows screen-extension software. spacedesk's own Windows/Android/iOS viewers are closed source and there
is **no Linux client at all** — this project reimplements the viewer side of the protocol in Rust so the
same core runs on Windows, Linux, macOS, and (later) mobile.

**Native rendering, no OS webview.** Pixels are decoded and drawn directly (`winit` + `softbuffer`); nothing
is wrapped in a system WebView.

> Status: **milestone 1 working** — LAN discovery, connection, identification handshake, JPEG framebuffer
> decode + composite, flow control, ping keepalive, and mouse/keyboard input. Verified live against
> spacedesk Server 2.x on port 28252. H.264 decode is the next milestone.

## Layout

```
crates/
  spacedesk-proto/   # transport-agnostic wire protocol (header, messages, discovery) + tests
  spacedesk-viewer/  # the app: TCP client, JPEG decode, winit/softbuffer window, input
docs/PROTOCOL.md     # the reverse-engineered wire protocol (v4.8), written from scratch
MODLOG.md            # reverse-engineering journal
```

## Build & run

Requires a recent stable Rust toolchain.

```bash
cargo build --release
```

Run it (the machine running the spacedesk **Server** must be reachable):

```bash
# discover servers on the LAN and list them (does NOT auto-connect — the stream is unencrypted)
./target/release/spacedesk-viewer

# or connect to a known server
./target/release/spacedesk-viewer 192.168.1.103
./target/release/spacedesk-viewer 192.168.1.103:28252

# headless protocol check (no window): connect, identify, print what the server streams
./target/release/spacedesk-viewer --probe 192.168.1.103
```

The window shows the extended display. Move the mouse / type to control the server (input is EXPERIMENTAL —
Win32 `SendInput` field semantics are still being verified; see `MODLOG.md`).

## How it works

spacedesk's server listens on **TCP/UDP 28252**. The viewer:

1. (optional) broadcasts `SPACEDESK-NET-CLIENT\0` over UDP to discover servers.
2. opens a TCP connection and sends an **Identification** message (protocol v4.8, encoding = JPEG).
3. receives **Framebuffer** messages — the screen split into 4 horizontal JPEG bands — decodes each and
   composites it into a full-resolution buffer.
4. sends a **FlowControlAck** after each complete frame (the server won't send the next frame otherwise) and
   replies to the server's **Ping** keepalives.
5. forwards mouse/keyboard/touch back to the server.

Full details in [`docs/PROTOCOL.md`](docs/PROTOCOL.md).

## Android viewer (Chromebook / tablet / phone / Fire TV)

A native Kotlin app lives in [`android/`](android/) — this is the path for using a **Chromebook as a
touchscreen second monitor**. It reuses the same protocol, decodes the JPEG bands with the platform's
`BitmapFactory`, renders to a `SurfaceView`, and forwards touch as absolute spacedesk touch events. It
requests an **extended display at the device's own resolution** (a real second screen, not a mirror).

Build & install:

```bash
cd android
# uses the Gradle in your cache; or just open android/ in Android Studio and press Run
gradle assembleDebug          # -> app/build/outputs/apk/debug/app-debug.apk

# install on a device reachable over adb (e.g. your Chromebook with ADB debugging on)
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

On the Chromebook: enable **Linux/ADB debugging** (Settings → "Develop Android apps") or sideload the APK,
open **OpenDeskView**, enter the PC's IP (or tap *Discover on LAN*), and Connect. The spacedesk Server
must be running on the PC. Status: display + flow control + ping verified live at ~30 fps on a real device;
touch plumbing is in place and gets its first live test on a touchscreen.

## Roadmap

- [x] Milestone 1: JPEG display + input (this release)
- [ ] H.264 (encoding 4) decode via openh264/FFmpeg for low-bandwidth, high-res streams
- [ ] Verify + finalize input field semantics against the live server
- [ ] Audio (type 14 speaker session)
- [ ] Mobile (Android/iOS) front-ends over the same `spacedesk-proto` core
- [ ] TLS/WSS transport; HTML5-viewer-compatible WebSocket mode

## Credit & scope

Protocol understanding was cross-checked against two prior independent clean-room efforts
([MarcozVD/SpaceDeskLinux](https://github.com/MarcozVD/SpaceDeskLinux),
[jeromeollivon-star/spacedesk-linux](https://github.com/jeromeollivon-star/spacedesk-linux)) and our own
analysis of the server binary's self-documenting protocol strings. All code here is written independently
from the spec in `docs/PROTOCOL.md`. This is an interoperability project for use with a spacedesk Server you
run yourself; it ships none of datronicsoft's code or assets. "spacedesk" is a trademark of datronicsoft,
used here only nominatively. See [`NOTICE.md`](NOTICE.md) for the full disclaimer.

## License

MIT — see [`LICENSE`](LICENSE).
