//! Input injection via Win32 SendInput. The viewer already sends Win32 MOUSEEVENTF/KEYEVENTF
//! flag bits in the message, so we pass them through almost directly.
#![cfg(windows)]

use windows::Win32::UI::Input::KeyboardAndMouse::*;

fn send(input: INPUT) {
    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

/// `flags` are Win32 MOUSEEVENTF_* bits; `dx`/`dy` are absolute 0..=65535 when ABSOLUTE is set.
pub fn mouse(dx: i32, dy: i32, wheel: i32, flags: u32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: wheel as u32,
                dwFlags: MOUSE_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(input);
}

/// `flags` are Win32 KEYEVENTF_* bits (e.g. KEYEVENTF_KEYUP = 0x0002).
pub fn key(vk: u16, scan: u16, flags: u32) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: KEYBD_EVENT_FLAGS(flags),
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(input);
}
