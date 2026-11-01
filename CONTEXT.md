# OpenDeskView — domain language

Terms used in code, docs and reviews. Use these words; don't invent synonyms.

- **Server** — the PC side (`crates/spacedesk-server`, or datronicsoft's spacedesk server): captures a
  monitor, encodes it, streams it, injects input.
- **Viewer** — the second-screen side (`crates/spacedesk-viewer` on desktop, `android/` on Android,
  ChromeOS, tablets).
- **Session** — one viewer connection, from identification to disconnect. The Android viewer
  reconnects into a new session after an established one drops.
- **Identification** — the viewer's first message: protocol version, encoding, quality, fps and the
  resolution it wants.
- **Target monitor** — the monitor a session streams, held by its stable device name
  (`\\.\DISPLAYn`). A session never falls back to the primary monitor if its target disappears.
- **Virtual display** — the monitor created by the Virtual Display Driver (IddCx) for extended mode,
  resized to each viewer's screen.
- **Frame** — one whole screen image as streamed: one framebuffer message (a JPEG image or one H.264
  access unit) from our server, or four JPEG bands from spacedesk's.
- **Ack** — the viewer's flow-control message (type 7) for a frame it accepted.
- **Flow control** — the server's rules for how many frames may be unacked (the *window*: 3 for
  H.264, 1 for JPEG), when the viewer counts as *stalled*, and when it gets a *refresh*
  (`crates/spacedesk-server/src/flow.rs`).
- **Stall** — the window stayed full with no ack for the stall timeout (1 s, backing off to 4 s).
- **Refresh** — after a stall, the current screen re-sent as a keyframe even if nothing changed, so
  the viewer can resume decoding.
- **Keyframe** — an H.264 IDR access unit (with SPS/PPS); a decoder can start from one and from
  nothing else.
- **Video sink** — the Android viewer's interface for showing one encoding on the current surface
  (`VideoSink`, adapters `H264Sink` and `JpegSink`). It decides which frames are acked.
