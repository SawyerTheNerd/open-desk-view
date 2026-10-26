//! Open-source spacedesk-compatible server (Windows): capture the screen and stream it to
//! OpenDeskView viewers over the documented protocol, injecting their input back.
//!
//! Usage:
//!   spacedesk-server                 # serve the primary display, VIEW-ONLY (JPEG q80)
//!   spacedesk-server --allow-input   # also let the viewer control mouse/keyboard/touch
//!   spacedesk-server --bind 127.0.0.1  # restrict to this machine only (default 0.0.0.0 = LAN)
//!   spacedesk-server --quality 60    # lower quality / less bandwidth
//!   spacedesk-server --capture-test  # write one frame to capture_test.jpg and exit

#[cfg(windows)]
mod capture;
#[cfg(windows)]
mod inject;
#[cfg(windows)]
mod server;

#[cfg(windows)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--capture-test") {
        capture_test();
        return;
    }

    let mut quality: u8 = 80;
    if let Some(i) = args.iter().position(|a| a == "--quality") {
        if let Some(v) = args.get(i + 1).and_then(|s| s.parse::<u8>().ok()) {
            quality = v.clamp(1, 100);
        }
    }

    let allow_input = args.iter().any(|a| a == "--allow-input");

    let mut bind = "0.0.0.0".to_string();
    if let Some(i) = args.iter().position(|a| a == "--bind") {
        if let Some(v) = args.get(i + 1) {
            bind = v.clone();
        }
    }

    if let Err(e) = server::run(quality, &bind, allow_input) {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn capture_test() {
    use jpeg_encoder::{ColorType, Encoder};
    match capture::capture_primary() {
        Some(f) => {
            let rgb = capture::bgra_to_rgb(&f.bgra, f.w, f.h);
            let mut out = Vec::new();
            Encoder::new(&mut out, 80)
                .encode(&rgb, f.w as u16, f.h as u16, ColorType::Rgb)
                .expect("jpeg encode");
            std::fs::write("capture_test.jpg", &out).expect("write");
            println!("captured {}x{} -> capture_test.jpg ({} bytes)", f.w, f.h, out.len());
        }
        None => eprintln!("capture failed"),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("spacedesk-server currently supports Windows only");
}
