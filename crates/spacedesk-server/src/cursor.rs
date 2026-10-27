//! Software cursor compositing for captures that don't include the pointer (DXGI Desktop
//! Duplication). The current cursor is decoded once and cached until its shape changes.
#![cfg(windows)]

use std::ffi::c_void;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, HICON, ICONINFO,
};

enum Kind {
    /// 32-bit colour with per-pixel alpha.
    Alpha,
    /// Colour image applied through an AND mask: dst = (dst & mask) ^ color.
    MaskedColor,
    /// Monochrome: `mask` holds the AND plane then the XOR plane (e.g. the inverting I-beam).
    Mono,
}

struct Shape {
    w: usize,
    h: usize,
    hot_x: i32,
    hot_y: i32,
    kind: Kind,
    color: Vec<u32>,
    mask: Vec<u32>,
}

#[derive(Default)]
pub struct CursorPainter {
    cached: Option<(isize, Shape)>,
}

impl CursorPainter {
    /// Draw the current cursor into a top-down BGRA `frame` of `fw`x`fh` whose top-left is at
    /// (`left`, `top`) on the virtual desktop. Hidden or off-frame cursors draw nothing.
    pub fn paint(&mut self, frame: &mut [u8], fw: usize, fh: usize, left: i32, top: i32) {
        let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        unsafe {
            if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
                return;
            }
        }
        let handle = ci.hCursor.0 as isize;
        if self.cached.as_ref().map(|(h, _)| *h) != Some(handle) {
            self.cached = unsafe { decode(HICON(ci.hCursor.0)) }.map(|s| (handle, s));
        }
        let Some((_, s)) = &self.cached else { return };

        let ox = ci.ptScreenPos.x - left - s.hot_x;
        let oy = ci.ptScreenPos.y - top - s.hot_y;
        for cy in 0..s.h {
            let fy = oy + cy as i32;
            if fy < 0 || fy as usize >= fh {
                continue;
            }
            for cx in 0..s.w {
                let fx = ox + cx as i32;
                if fx < 0 || fx as usize >= fw {
                    continue;
                }
                let di = (fy as usize * fw + fx as usize) * 4;
                let dst = u32::from_le_bytes([frame[di], frame[di + 1], frame[di + 2], frame[di + 3]]);
                let i = cy * s.w + cx;
                let out = match s.kind {
                    Kind::Alpha => blend(dst, s.color[i]),
                    Kind::MaskedColor => (dst & s.mask[i]) ^ (s.color[i] & 0x00FF_FFFF),
                    Kind::Mono => (dst & s.mask[i]) ^ s.mask[i + s.w * s.h],
                };
                frame[di..di + 4].copy_from_slice(&out.to_le_bytes());
            }
        }
    }
}

/// Straight-alpha blend of `src` (BGRA, alpha in the top byte) over `dst`.
fn blend(dst: u32, src: u32) -> u32 {
    let a = src >> 24;
    if a == 0 {
        return dst;
    }
    if a == 255 {
        return src;
    }
    let mix = |shift: u32| {
        let s = (src >> shift) & 0xFF;
        let d = (dst >> shift) & 0xFF;
        ((s * a + d * (255 - a)) / 255) << shift
    };
    mix(0) | mix(8) | mix(16) | (dst & 0xFF00_0000)
}

unsafe fn decode(icon: HICON) -> Option<Shape> {
    let mut ii = ICONINFO::default();
    GetIconInfo(icon, &mut ii).ok()?;
    let shape = decode_bitmaps(&ii);
    // GetIconInfo hands us copies of the cursor bitmaps; free them.
    if !ii.hbmMask.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
    }
    if !ii.hbmColor.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
    }
    shape
}

unsafe fn decode_bitmaps(ii: &ICONINFO) -> Option<Shape> {
    let mut bm = BITMAP::default();
    if GetObjectW(HGDIOBJ(ii.hbmMask.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut c_void)) == 0 {
        return None;
    }
    let w = bm.bmWidth as usize;
    let mask_h = bm.bmHeight as usize;
    let mask = bits32(ii.hbmMask, w, mask_h)?;
    let (kind, h, color) = if ii.hbmColor.is_invalid() {
        (Kind::Mono, mask_h / 2, Vec::new())
    } else {
        let color = bits32(ii.hbmColor, w, mask_h)?;
        let kind = if color.iter().any(|p| p >> 24 != 0) { Kind::Alpha } else { Kind::MaskedColor };
        (kind, mask_h, color)
    };
    Some(Shape { w, h, hot_x: ii.xHotspot as i32, hot_y: ii.yHotspot as i32, kind, color, mask })
}

/// Read a bitmap as top-down 32-bit pixels (monochrome bitmaps expand to 0x000000 / 0xFFFFFF).
unsafe fn bits32(bmp: HBITMAP, w: usize, h: usize) -> Option<Vec<u32>> {
    let mut bi = BITMAPINFO::default();
    bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w as i32;
    bi.bmiHeader.biHeight = -(h as i32);
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB.0 as u32;
    let mut px = vec![0u32; w * h];
    let dc = GetDC(None);
    let got = GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    (got != 0).then_some(px)
}

#[cfg(test)]
mod tests {
    use super::blend;

    #[test]
    fn blend_extremes_and_half() {
        assert_eq!(blend(0x00112233, 0x00FFFFFF), 0x00112233); // transparent
        assert_eq!(blend(0x00112233, 0xFFABCDEF), 0xFFABCDEF); // opaque
        assert_eq!(blend(0x00000000, 0x80FFFFFF) & 0x00FF_FFFF, 0x00808080); // ~50%
    }
}
