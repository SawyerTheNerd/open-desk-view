//! Screen capture via GDI BitBlt. Simple and dependency-free; good enough for a first server.
//! Can capture the primary display or any specific monitor (e.g. a virtual extended display
//! created by an IddCx driver), so the server can drive a true second monitor rather than mirror.
#![cfg(windows)]

use std::ffi::c_void;
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>, // top-down BGRA
}

/// One monitor in the desktop, in virtual-screen coordinates (left/top may be negative).
#[derive(Clone)]
pub struct Display {
    pub index: usize,
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
    pub name: String,
}

unsafe extern "system" fn enum_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rc: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = &mut *(lparam.0 as *mut Vec<Display>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut mi.monitorInfo as *mut _ as *mut MONITORINFO).as_bool() {
        let r = mi.monitorInfo.rcMonitor;
        let name = String::from_utf16_lossy(&mi.szDevice)
            .trim_end_matches('\0')
            .to_string();
        list.push(Display {
            index: list.len(),
            left: r.left,
            top: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
            primary: mi.monitorInfo.dwFlags & 0x1 != 0, // MONITORINFOF_PRIMARY
            name,
        });
    }
    BOOL(1)
}

/// Enumerate all monitors (primary first is not guaranteed; check the `primary` flag).
pub fn list_displays() -> Vec<Display> {
    let mut list: Vec<Display> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_proc),
            LPARAM(&mut list as *mut _ as isize),
        );
    }
    list
}

/// Capture a rectangle of the virtual desktop (coords are virtual-screen, may be negative).
fn capture_rect(left: i32, top: i32, w: i32, h: i32) -> Option<Frame> {
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));

        // Source origin is in virtual-screen space; GetDC(None) maps (0,0) to the primary's
        // top-left, so other monitors sit at their signed offsets.
        let _ = BitBlt(mem, 0, 0, w, h, screen, left, top, SRCCOPY);

        let mut bi = BITMAPINFO::default();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = w;
        bi.bmiHeader.biHeight = -h; // negative = top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB.0 as u32;

        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        let got = GetDIBits(
            mem,
            bmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut c_void),
            &mut bi,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        if got == 0 {
            return None;
        }
        Some(Frame { w: w as u32, h: h as u32, bgra: buf })
    }
}

/// Capture a specific monitor.
pub fn capture_display(d: &Display) -> Option<Frame> {
    capture_rect(d.left, d.top, d.width, d.height)
}

/// Bounding box of the whole virtual desktop: (left, top, width, height).
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Find a monitor by its stable device name (e.g. "\\.\DISPLAY2"). Identity is re-checked
/// this way each frame so a requested display is never silently confused with another.
pub fn find_display_by_name(name: &str) -> Option<Display> {
    list_displays().into_iter().find(|d| d.name == name)
}

/// Capture the primary display. Returns None on failure.
pub fn capture_primary() -> Option<Frame> {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        // Primary's top-left is the origin of GetDC(None), so source (0,0).
        capture_rect(0, 0, w, h)
    }
}

/// Convert top-down BGRA to tightly packed RGB8.
pub fn bgra_to_rgb(bgra: &[u8], w: u32, h: u32) -> Vec<u8> {
    let n = (w as usize) * (h as usize);
    let mut rgb = vec![0u8; n * 3];
    for i in 0..n {
        let s = i * 4;
        rgb[i * 3] = bgra[s + 2];
        rgb[i * 3 + 1] = bgra[s + 1];
        rgb[i * 3 + 2] = bgra[s];
    }
    rgb
}
