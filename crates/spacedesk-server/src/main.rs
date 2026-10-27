//! Open-source spacedesk-compatible server (Windows): capture the screen and stream it to
//! OpenDeskView viewers over the documented protocol, injecting their input back.
//!
//! Usage:
//!   spacedesk-server                 # serve the primary display, VIEW-ONLY
//!   spacedesk-server --list-displays # list monitors (index, size, primary) and exit
//!   spacedesk-server --display 1     # serve a specific monitor
//!   spacedesk-server --virtual       # EXTENDED mode: serve the Virtual Display Driver monitor,
//!                                    #   resized to each viewer's screen
//!   spacedesk-server --allow-input   # also let the viewer control mouse/keyboard/touch
//!   spacedesk-server --bind 127.0.0.1  # restrict to this machine only (default 0.0.0.0 = LAN)
//!   spacedesk-server --no-h264       # never use OpenH264; always stream JPEG
//!   spacedesk-server --capture-test  # write one frame to capture_test.jpg and exit
//!
//! Resource usage (a profile sets everything; the other flags override it):
//!   --profile eco|balanced|performance   # default balanced (eco: 24 fps, performance: 60 fps)
//!   --max-fps 30                     # frame-rate ceiling
//!   --bitrate-kbps 4000              # fixed H.264 bitrate
//!   --quality 60                     # JPEG quality ceiling (1-100)
//! Viewers can ask for less than these, never more. An unchanged screen is not re-sent.
//!
//! H.264 needs Cisco's openh264-2.6.0-win64.dll next to the exe, downloaded separately from
//! Cisco. OpenH264 Video Codec provided by Cisco Systems, Inc.

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod cursor;
#[cfg(windows)]
mod display;
#[cfg(windows)]
mod dxgi;
#[cfg(windows)]
mod h264;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod server;
mod settings;

#[cfg(windows)]
fn main() {
    // Work in physical pixels on every monitor, so capture rects, cursor positions and input
    // coordinates all agree even when displays use different scaling.
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let value = |flag: &str| -> Option<&str> {
        let i = args.iter().position(|a| a == flag)?;
        match args.get(i + 1) {
            Some(v) => Some(v.as_str()),
            None => usage_error(&format!("{flag} needs a value")),
        }
    };
    let number = |flag: &str, range: std::ops::RangeInclusive<u32>| -> Option<u32> {
        let v = value(flag)?;
        match v.parse::<u32>() {
            Ok(n) if range.contains(&n) => Some(n),
            _ => usage_error(&format!("{flag} must be a number from {} to {}", range.start(), range.end())),
        }
    };

    if has("--list-displays") {
        for d in capture::list_displays() {
            println!(
                "display {}: {}x{} at ({},{}){}  [{}]",
                d.index,
                d.width,
                d.height,
                d.left,
                d.top,
                if d.primary { " PRIMARY" } else { "" },
                d.name
            );
        }
        match display::find_virtual_display() {
            Some(n) => println!("virtual display (--virtual): {n}"),
            None => println!("virtual display (--virtual): not found"),
        }
        return;
    }

    let display = number("--display", 0..=63).map(|i| i as usize);

    if has("--capture-test") {
        capture_test(display);
        return;
    }

    let profile = value("--profile").unwrap_or("balanced");
    let mut settings = settings::Settings::profile(profile).unwrap_or_else(|| {
        usage_error(&format!("unknown --profile {profile:?} (choose: {})", settings::PROFILE_NAMES))
    });
    if let Some(fps) = number("--max-fps", 1..=240) {
        settings.max_fps = fps;
    }
    if let Some(kbps) = number("--bitrate-kbps", 100..=100_000) {
        settings.bitrate_kbps = Some(kbps);
    }
    if let Some(q) = number("--quality", 1..=100) {
        settings.jpeg_quality = q as u8;
    }

    let allow_input = has("--allow-input");
    let allow_h264 = !has("--no-h264");
    let bind = value("--bind").unwrap_or("0.0.0.0").to_string();

    let target = if has("--virtual") {
        server::Target::Virtual
    } else if let Some(i) = display {
        server::Target::Index(i)
    } else {
        server::Target::Primary
    };

    println!("[server] profile: {profile}");
    if let Err(e) = server::run(settings, &bind, allow_input, target, allow_h264) {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn usage_error(msg: &str) -> ! {
    eprintln!("spacedesk-server: {msg} (see the usage notes in the README)");
    std::process::exit(2);
}

#[cfg(windows)]
fn capture_test(display: Option<usize>) {
    use jpeg_encoder::{ColorType, Encoder};
    let d = match display {
        Some(i) => match capture::list_displays().into_iter().find(|d| d.index == i) {
            Some(d) => d,
            None => return eprintln!("capture failed: no display {i}"),
        },
        None => capture::primary_display(),
    };
    let mut bgra = Vec::new();
    if !capture::Capturer::new().is_some_and(|mut c| c.capture(&d, &mut bgra, true)) {
        return eprintln!("capture failed");
    }
    let mut out = Vec::new();
    Encoder::new(&mut out, 80)
        .encode(&bgra, d.width as u16, d.height as u16, ColorType::Bgra)
        .expect("jpeg encode");
    std::fs::write("capture_test.jpg", &out).expect("write");
    println!("captured {}x{} -> capture_test.jpg ({} bytes)", d.width, d.height, out.len());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("spacedesk-server currently supports Windows only");
}
