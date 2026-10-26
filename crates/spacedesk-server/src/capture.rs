//! Screen capture via GDI BitBlt. Simple and dependency-free; good enough for a first server.
//! (DXGI Desktop Duplication / an IddCx virtual display come later for performance + extended mode.)
#![cfg(windows)]

use std::ffi::c_void;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

pub struct Frame {
    pub w: u32,
    pub h: u32,
    pub bgra: Vec<u8>, // top-down BGRA
}

/// Capture the primary display. Returns None on failure.
pub fn capture_primary() -> Option<Frame> {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        if w <= 0 || h <= 0 {
            return None;
        }

        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));

        let _ = BitBlt(mem, 0, 0, w, h, screen, 0, 0, SRCCOPY);

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
