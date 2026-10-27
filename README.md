# OpenDeskView — open-source spacedesk-compatible viewer and server

> **Independent project — not affiliated with, endorsed by, or connected to datronicsoft or spacedesk.**
> "spacedesk" is used only to describe protocol compatibility. See [`NOTICE.md`](NOTICE.md).

A clean-room, cross-platform **viewer** (client) that is compatible with the
[spacedesk](https://www.spacedesk.net/) display-streaming protocol, the Windows screen-extension software. spacedesk's own Windows/Android/iOS viewers are closed source and there
is **no Linux client at all** — this project reimplements the viewer side of the protocol in Rust so the
same core runs on Windows, Linux, macOS, and (later) mobile.

**Native rendering, no OS webview.** Pixels are decoded and drawn directly (`winit` + `softbuffer`); nothing
is wrapped in a system WebView.

It also includes an open-source **Windows server**, so a Chromebook, tablet or phone can be a true
extended touchscreen monitor with no spacedesk software at all (see [Server](#server-windows--no-spacedesk-needed)).

> Status: viewers (desktop + Android) and server working end to end — extended display via a virtual
> monitor, H.264, cursor, touch/touchpad/keyboard input. Verified live on a Chromebook at 30+ fps.

## Layout

```
crates/
  spacedesk-proto/   # transport-agnostic wire protocol (header, messages, discovery) + tests
  spacedesk-viewer/  # the app: TCP client, JPEG decode, winit/softbuffer window, input
  spacedesk-server/  # open-source server (Windows): capture, stream, inject input — replaces spacedesk
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

# H.264 instead of JPEG (lower bandwidth). Needs Cisco's prebuilt OpenH264 library next to the exe
# (see "OpenH264" below; it is not bundled).
./target/release/spacedesk-viewer --h264 192.168.1.103

# headless protocol check (no window): connect, identify, print what the server streams
./target/release/spacedesk-viewer --probe 192.168.1.103
./target/release/spacedesk-viewer --probe --h264 192.168.1.103
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

It defaults to **H.264** (hardware `MediaCodec`, decoded straight to the display Surface — low bandwidth);
untick the box on the start screen to fall back to JPEG if a device's decoder misbehaves.

On the Chromebook: enable **Linux/ADB debugging** (Settings → "Develop Android apps") or sideload the APK,
open **OpenDeskView**, enter the PC's IP (or tap *Discover on LAN*), and Connect. A server must be
running on the PC — OpenDeskView's own (below) or spacedesk's. Touchscreen, touchpad (move, click,
right-click, scroll) and hardware keyboard are forwarded; verified live on a Chromebook.

The start screen's **Performance** choice trades smoothness for battery and bandwidth: *Battery saver*
(24 fps, lower quality), *Balanced* (30 fps) or *Smoothest* (60 fps, best quality). The app remembers
it, the PC's IP and the H.264 setting.

## Server (Windows) — no spacedesk needed

`spacedesk-server` speaks the same protocol, so the viewers above connect to it unchanged and spacedesk
can be uninstalled. For a **true extended display** (a real second monitor, not a mirror) it streams a
virtual monitor created by the open-source, code-signed
[Virtual Display Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver) (MIT, IddCx).

One-time driver install (admin; no test-signing or reboot needed). Download `VDD.Control.<version>.zip`
from that project's releases, extract it, and from an elevated PowerShell in the extracted folder:

```powershell
New-Item -ItemType Directory -Force C:\VirtualDisplayDriver | Out-Null
Copy-Item .\SignedDrivers\x86\VDD\* C:\VirtualDisplayDriver   # "x86" folder holds the x64 build
pnputil /add-driver C:\VirtualDisplayDriver\MttVDD.inf /install
.\Dependencies\devcon.exe install C:\VirtualDisplayDriver\MttVDD.inf Root\MttVDD
```

(or run the bundled `VDD Control.exe` and click Install). Then:

```bash
spacedesk-server --virtual --allow-input   # extended mode; the virtual monitor is resized to each viewer
spacedesk-server --list-displays           # show monitors and which one --virtual uses
spacedesk-server                           # mirror the primary display instead
```

**Resource usage.** Pick a profile, then fine-tune with flags if you like:

| `--profile` | max fps | H.264 budget | JPEG quality | still-screen check |
|---|---|---|---|---|
| `eco` | 24 | 4 bits/pixel/s | 60 | every 250 ms |
| `balanced` (default) | 30 | 6 bits/pixel/s | 75 | every 100 ms |
| `performance` | 60 | 10 bits/pixel/s | 85 | every 16 ms |

`--max-fps N`, `--bitrate-kbps N` (fixed H.264 bitrate) and `--quality N` (JPEG) override the profile.
These are ceilings: each viewer can ask for less (the Android app's *Performance* setting), never more.

The server captures with **DXGI Desktop Duplication**, so the GPU says when something changed: an
unchanged screen is never re-encoded or re-sent, and capturing a 1080p frame takes ~2.5 ms (GDI, the
automatic fallback, takes ~35 ms). Measured streaming a 1080p screen with live animation, on one CPU
core: `eco` ~20%, `balanced` ~31%, `performance` ~54% at ~40 fps. The stats line it prints every two
seconds shows frame rate, bitrate, capture/encode time and skipped frames.

Viewers that ask for H.264 get it when Cisco's OpenH264 library sits next to the server (see below);
otherwise, or with `--no-h264`, the server streams JPEG. The Android app decodes H.264 in hardware, so
it is much smoother than JPEG (about 30+ fps vs 4 on a Chromebook).

Stop spacedesk's own service first if it's installed — both use port 28252. Connections are
**unauthenticated and unencrypted**: anyone on your LAN can view the streamed display, and with
`--allow-input` control this PC. Use it only on a network you trust.

## OpenH264 (Windows H.264)

The Windows server and desktop viewer use Cisco's prebuilt OpenH264 library for H.264. Cisco's
patent license only covers the library when you download it **directly from Cisco**, so it is not
bundled here. To enable H.264:

1. Download <http://ciscobinary.openh264.org/openh264-2.6.0-win64.dll.bz2>.
2. Decompress it (e.g. with 7-Zip) to `openh264-2.6.0-win64.dll`.
3. Put it next to `spacedesk-server.exe` / `spacedesk-viewer.exe`, or set `OPENH264_DLL` to its
   absolute path. Only Cisco's genuine build is accepted (its hash is checked).

OpenH264 Video Codec provided by Cisco Systems, Inc. — license text in [`NOTICE.md`](NOTICE.md).

## Roadmap

- [x] Milestone 1: JPEG display + input
- [x] H.264 (encoding 4) on Android via hardware `MediaCodec` (toggle in the app; default on)
- [x] H.264 on desktop (Rust) via OpenH264 (`--h264`), loading Cisco's prebuilt library at runtime
- [x] Open-source Windows server with true extended display (Virtual Display Driver)
- [x] Server H.264 encode; cursor in the stream; touch/touchpad/keyboard input verified on a Chromebook
- [x] DXGI Desktop Duplication capture, change-only streaming, resource profiles
- [ ] Server: PIN/TLS authentication, real multi-touch injection
- [ ] Audio (type 14 speaker session)
- [ ] iOS front-end over the same `spacedesk-proto` core
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
