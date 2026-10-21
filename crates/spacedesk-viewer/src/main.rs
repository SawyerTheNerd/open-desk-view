//! Open-source spacedesk viewer — native cross-platform (no OS webview).
//!
//! Usage:
//!   spacedesk-viewer                 # discover servers on the LAN, connect to the first
//!   spacedesk-viewer 192.168.1.103   # connect directly (default port 28252)
//!   spacedesk-viewer 192.168.1.103:28252

mod frame;
mod input;
mod net;

use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{Window, WindowId};

use frame::{Frame, SharedFrame};
use spacedesk_proto::{self as proto, Encoding, Identification};

#[derive(Debug, Clone, Copy)]
enum UserEvent {
    FrameReady,
}

struct App {
    frame: SharedFrame,
    input_tx: Option<std::sync::mpsc::Sender<Vec<u8>>>,
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    cursor: PhysicalPosition<f64>,
    last_gen: u64,
}

impl App {
    fn new(frame: SharedFrame, input_tx: std::sync::mpsc::Sender<Vec<u8>>) -> Self {
        App {
            frame,
            input_tx: Some(input_tx),
            window: None,
            context: None,
            surface: None,
            cursor: PhysicalPosition::new(0.0, 0.0),
            last_gen: 0,
        }
    }

    fn send(&self, msg: Vec<u8>) {
        if let Some(tx) = &self.input_tx {
            tx.send(msg).ok();
        }
    }

    /// Current (window size, framebuffer size).
    fn sizes(&self) -> ((u32, u32), (u32, u32)) {
        let win = self
            .window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width.max(1), s.height.max(1))
            })
            .unwrap_or((1, 1));
        let fb = {
            let f = self.frame.lock().unwrap();
            (f.width, f.height)
        };
        (win, fb)
    }

    fn redraw(&mut self) {
        let (Some(window), Some(surface)) = (self.window.as_ref(), self.surface.as_mut()) else {
            return;
        };
        let size = window.inner_size();
        let (ww, wh) = (size.width.max(1), size.height.max(1));
        surface
            .resize(NonZeroU32::new(ww).unwrap(), NonZeroU32::new(wh).unwrap())
            .unwrap();
        let mut buf = surface.buffer_mut().unwrap();

        let f = self.frame.lock().unwrap();
        if f.width == 0 || f.height == 0 {
            buf.fill(0x00101014);
        } else {
            let (fw, fh) = (f.width as usize, f.height as usize);
            for y in 0..wh as usize {
                let sy = (y * fh) / wh as usize;
                let src_row = sy * fw;
                let dst_row = y * ww as usize;
                for x in 0..ww as usize {
                    let sx = (x * fw) / ww as usize;
                    buf[dst_row + x] = f.pixels[src_row + sx];
                }
            }
        }
        drop(f);
        buf.present().unwrap();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("OpenDeskView — open-source spacedesk-compatible viewer")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
        let window = Rc::new(el.create_window(attrs).expect("create window"));
        let context = Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("softbuffer surface");
        self.window = Some(window);
        self.context = Some(context);
        self.surface = Some(surface);
    }

    fn user_event(&mut self, _el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::FrameReady => {
                let gen = self.frame.lock().unwrap().generation;
                if gen != self.last_gen {
                    self.last_gen = gen;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.send(proto::build_disconnect());
                el.exit();
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                let (win, fb) = self.sizes();
                if let Some(msg) = input::mouse_move_absolute(position, win, fb) {
                    self.send(msg);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                let btn = match button {
                    MouseButton::Left => input::Btn::Left,
                    MouseButton::Right => input::Btn::Right,
                    MouseButton::Middle => input::Btn::Middle,
                    _ => return,
                };
                let (win, fb) = self.sizes();
                self.send(input::mouse_button(self.cursor, win, fb, btn, down));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let ticks = match delta {
                    MouseScrollDelta::LineDelta(_, y) => (y * 120.0) as i32,
                    MouseScrollDelta::PixelDelta(p) => p.y as i32,
                };
                let (win, fb) = self.sizes();
                self.send(input::mouse_wheel(self.cursor, win, fb, ticks));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let down = event.state == ElementState::Pressed;
                    if let Some(msg) = input::keyboard(code, down) {
                        self.send(msg);
                    }
                }
            }
            _ => {}
        }
    }
}

fn parse_target(arg: &str) -> Option<SocketAddr> {
    if let Ok(a) = arg.parse::<SocketAddr>() {
        return Some(a);
    }
    if let Ok(ip) = arg.parse::<std::net::IpAddr>() {
        return Some(SocketAddr::new(ip, proto::DEFAULT_PORT));
    }
    None
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    // Headless protocol probe: `spacedesk-viewer --probe <ip>`
    let probe_mode = args.iter().any(|a| a == "--probe");
    // Opt-in convenience: connect to the first discovered server without an explicit IP.
    let connect_first = args.iter().any(|a| a == "--connect-first");
    // Request H.264 (encoding 4) instead of the default JPEG.
    let use_h264 = args.iter().any(|a| a == "--h264");
    args.retain(|a| a != "--probe" && a != "--connect-first" && a != "--h264");

    let mut id = Identification::default();
    if use_h264 {
        id.encoding = Encoding::H264;
    }

    let target: SocketAddr = if let Some(first) = args.first() {
        match parse_target(first) {
            Some(a) => a,
            None => {
                eprintln!("could not parse '{first}' as IP or IP:port");
                std::process::exit(2);
            }
        }
    } else {
        eprintln!("[discovery] broadcasting on UDP {} ...", proto::DEFAULT_PORT);
        match net::discover(Duration::from_millis(1200)) {
            Ok(list) if !list.is_empty() => {
                eprintln!("discovered {} server(s):", list.len());
                for (i, d) in list.iter().enumerate() {
                    eprintln!("  [{i}] {}  ({})", d.addr.ip(), d.name);
                }
                if connect_first {
                    eprintln!("[discovery] --connect-first: using [0] {}", list[0].addr.ip());
                    list[0].addr
                } else {
                    // Do NOT auto-connect: the stream is unencrypted, so a rogue host that answers
                    // discovery could receive your keystrokes. Require an explicit, trusted choice.
                    eprintln!();
                    eprintln!("Not auto-connecting: the connection is UNENCRYPTED, so only connect to a server");
                    eprintln!("you trust. Re-run with its IP, e.g.:");
                    eprintln!("    spacedesk-viewer {}", list[0].addr.ip());
                    return;
                }
            }
            Ok(_) => {
                eprintln!("no spacedesk servers found. Pass the server IP, e.g. `spacedesk-viewer 192.168.1.103`");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("discovery failed: {e}");
                std::process::exit(1);
            }
        }
    };

    eprintln!("[net] connecting to {target} ...");
    eprintln!("[warning] the spacedesk stream is UNENCRYPTED — screen and input travel in clear text on the LAN.");

    if probe_mode {
        match net::probe(target, id.clone(), 5) {
            Ok(()) => return,
            Err(e) => {
                eprintln!("probe failed: {e}");
                std::process::exit(1);
            }
        }
    }

    let frame: SharedFrame = Arc::new(Mutex::new(Frame::new()));

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();

    let wake: net::Wake = Arc::new(move || {
        let _ = proxy.send_event(UserEvent::FrameReady);
    });

    if use_h264 {
        eprintln!("[note] desktop GUI H.264 decode isn't built in this release (OpenH264 needs a C/C++ toolchain).");
        eprintln!("[note] Rendering the window with JPEG instead. The H.264 request itself works: try");
        eprintln!("[note] `--probe --h264` to inspect the stream, or use the Android app for H.264 playback.");
        id.encoding = Encoding::Jpeg;
    }

    let conn = match net::start(target, id, frame.clone(), wake) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connection failed: {e}");
            std::process::exit(1);
        }
    };

    let mut app = App::new(frame, conn.input);
    event_loop.run_app(&mut app).expect("run");
}
