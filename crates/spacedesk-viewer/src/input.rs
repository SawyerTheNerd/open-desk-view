//! Maps winit input events to spacedesk input messages.
//!
//! EXPERIMENTAL: the mouse/keyboard field semantics follow Win32 `SendInput` conventions
//! (`MOUSEEVENTF_*` / `KEYEVENTF_*`), which the server almost certainly feeds into `SendInput`.
//! Exact bit placement is pending live verification against the running server (see MODLOG).

use spacedesk_proto as proto;
use winit::dpi::PhysicalPosition;
use winit::keyboard::KeyCode;

// Win32 MOUSEEVENTF_*
const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;

// Win32 KEYEVENTF_*
const KEYEVENTF_KEYUP: u32 = 0x0002;

const MOUSE_FLAG_MOVE: u32 = 0x01;

#[derive(Clone, Copy)]
pub enum Btn {
    Left,
    Right,
    Middle,
}

/// Normalize a window-space cursor position to 0..=65535 over the server framebuffer.
fn absolute(pos: PhysicalPosition<f64>, win: (u32, u32), fb: (u32, u32)) -> Option<(i32, i32)> {
    let (ww, wh) = win;
    let (fw, fh) = fb;
    if fw == 0 || fh == 0 || ww == 0 || wh == 0 {
        return None;
    }
    let nx = (pos.x / ww as f64).clamp(0.0, 1.0);
    let ny = (pos.y / wh as f64).clamp(0.0, 1.0);
    Some(((nx * 65535.0) as i32, (ny * 65535.0) as i32))
}

pub fn mouse_move_absolute(
    pos: PhysicalPosition<f64>,
    win: (u32, u32),
    fb: (u32, u32),
) -> Option<Vec<u8>> {
    let (x, y) = absolute(pos, win, fb)?;
    Some(proto::build_mouse(
        x,
        y,
        0,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
        MOUSE_FLAG_MOVE,
    ))
}

pub fn mouse_button(
    pos: PhysicalPosition<f64>,
    win: (u32, u32),
    fb: (u32, u32),
    btn: Btn,
    down: bool,
) -> Vec<u8> {
    let (x, y) = absolute(pos, win, fb).unwrap_or((0, 0));
    let evt = match (btn, down) {
        (Btn::Left, true) => MOUSEEVENTF_LEFTDOWN,
        (Btn::Left, false) => MOUSEEVENTF_LEFTUP,
        (Btn::Right, true) => MOUSEEVENTF_RIGHTDOWN,
        (Btn::Right, false) => MOUSEEVENTF_RIGHTUP,
        (Btn::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
        (Btn::Middle, false) => MOUSEEVENTF_MIDDLEUP,
    };
    proto::build_mouse(x, y, 0, evt | MOUSEEVENTF_ABSOLUTE, 0)
}

pub fn mouse_wheel(
    pos: PhysicalPosition<f64>,
    win: (u32, u32),
    fb: (u32, u32),
    ticks: i32,
) -> Vec<u8> {
    let (x, y) = absolute(pos, win, fb).unwrap_or((0, 0));
    proto::build_mouse(x, y, ticks, MOUSEEVENTF_WHEEL | MOUSEEVENTF_ABSOLUTE, 0)
}

/// Map a physical key to a Win32 virtual-key code. Returns `None` for keys we don't map yet.
fn vk(code: KeyCode) -> Option<u32> {
    use KeyCode::*;
    let v: u32 = match code {
        KeyA => 0x41, KeyB => 0x42, KeyC => 0x43, KeyD => 0x44, KeyE => 0x45,
        KeyF => 0x46, KeyG => 0x47, KeyH => 0x48, KeyI => 0x49, KeyJ => 0x4A,
        KeyK => 0x4B, KeyL => 0x4C, KeyM => 0x4D, KeyN => 0x4E, KeyO => 0x4F,
        KeyP => 0x50, KeyQ => 0x51, KeyR => 0x52, KeyS => 0x53, KeyT => 0x54,
        KeyU => 0x55, KeyV => 0x56, KeyW => 0x57, KeyX => 0x58, KeyY => 0x59, KeyZ => 0x5A,
        Digit0 => 0x30, Digit1 => 0x31, Digit2 => 0x32, Digit3 => 0x33, Digit4 => 0x34,
        Digit5 => 0x35, Digit6 => 0x36, Digit7 => 0x37, Digit8 => 0x38, Digit9 => 0x39,
        Enter => 0x0D, Escape => 0x1B, Backspace => 0x08, Tab => 0x09, Space => 0x20,
        Minus => 0xBD, Equal => 0xBB, BracketLeft => 0xDB, BracketRight => 0xDD,
        Backslash => 0xDC, Semicolon => 0xBA, Quote => 0xDE, Comma => 0xBC,
        Period => 0xBE, Slash => 0xBF, Backquote => 0xC0,
        ArrowLeft => 0x25, ArrowUp => 0x26, ArrowRight => 0x27, ArrowDown => 0x28,
        Home => 0x24, End => 0x23, PageUp => 0x21, PageDown => 0x22,
        Insert => 0x2D, Delete => 0x2E,
        ShiftLeft => 0xA0, ShiftRight => 0xA1, ControlLeft => 0xA2, ControlRight => 0xA3,
        AltLeft => 0xA4, AltRight => 0xA5, SuperLeft => 0x5B, SuperRight => 0x5C,
        CapsLock => 0x14,
        F1 => 0x70, F2 => 0x71, F3 => 0x72, F4 => 0x73, F5 => 0x74, F6 => 0x75,
        F7 => 0x76, F8 => 0x77, F9 => 0x78, F10 => 0x79, F11 => 0x7A, F12 => 0x7B,
        _ => return None,
    };
    Some(v)
}

pub fn keyboard(code: KeyCode, down: bool) -> Option<Vec<u8>> {
    let vkey = vk(code)?;
    let flags = if down { 0 } else { KEYEVENTF_KEYUP };
    // scancode left 0 for now; the server injects by VK when scancode is absent (needs verification).
    Some(proto::build_keyboard(vkey, 0, flags))
}
