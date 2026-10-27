//! Screen capture of the primary display or any specific monitor (e.g. a virtual extended display
//! created by an IddCx driver), so the server can drive a true second monitor rather than mirror.
//! [`Source`] prefers DXGI Desktop Duplication (only changed frames, cheap GPU copies) and falls
//! back to GDI BitBlt ([`Capturer`]). Both reuse their buffers, so steady state allocates nothing.
#![cfg(windows)]

use std::ffi::c_void;
use std::time::{Duration, Instant};

use crate::cursor::CursorPainter;
use crate::dxgi::{self, Duplicator};
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, GetSystemMetrics, CURSORINFO, CURSOR_SHOWING, DI_NORMAL,
    HICON, ICONINFO, SM_CXSCREEN, SM_CXVIRTUALSCREEN, SM_CYSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

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

/// The primary display as a [`Display`] (its top-left is the virtual-desktop origin).
pub fn primary_display() -> Display {
    list_displays().into_iter().find(|d| d.primary).unwrap_or_else(|| {
        let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
        Display { index: 0, left: 0, top: 0, width: w, height: h, primary: true, name: String::new() }
    })
}

pub enum Grab {
    /// A new frame (cursor included) is in the output buffer.
    New,
    /// Nothing changed within the wait.
    Unchanged,
    Failed,
}

/// Frames of one monitor: DXGI Desktop Duplication when available, else GDI.
pub struct Source {
    name: String,
    dxgi: Option<Duplicator>,
    dxgi_retry_at: Instant,
    gdi: Option<Capturer>,
    /// Latest desktop image from DXGI, without the cursor.
    image: Vec<u8>,
    cursor: CursorPainter,
    /// Which backend we last announced (true = DXGI), to log only on changes.
    announced: Option<bool>,
    /// Time spent copying/compositing the last new frame (excludes waiting).
    pub copy_time: Duration,
}

impl Source {
    /// `name` is the monitor's GDI device name (e.g. `\\.\DISPLAY21`).
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            dxgi: None,
            dxgi_retry_at: Instant::now(),
            gdi: None,
            image: Vec::new(),
            cursor: CursorPainter::default(),
            announced: None,
            copy_time: Duration::ZERO,
        }
    }

    /// Capture monitor `d` into `out` (top-down BGRA). With DXGI, waits up to `wait` for a change
    /// and returns `Unchanged` if there was none; GDI sleeps `wait` and always captures.
    pub fn grab(&mut self, d: &Display, wait: Duration, out: &mut Vec<u8>) -> Grab {
        if self.dxgi.is_none() && Instant::now() >= self.dxgi_retry_at {
            match Duplicator::new(&self.name) {
                Ok(dup) => {
                    self.announce(true, "DXGI desktop duplication");
                    self.image.clear();
                    self.dxgi = Some(dup);
                }
                Err(e) => {
                    self.announce(false, &format!("GDI (DXGI unavailable: {e})"));
                    self.dxgi_retry_at = Instant::now() + Duration::from_secs(2);
                }
            }
        }

        if let Some(dup) = self.dxgi.as_mut() {
            let first = self.image.is_empty();
            let grabbed = dup.grab(wait, &mut self.image, first);
            let gpu_copy = dup.last_copy;
            match grabbed {
                dxgi::Frame::Lost => {
                    // Mode change, secure desktop (UAC/lock screen), ...: recreate next time.
                    self.dxgi = None;
                    return Grab::Failed;
                }
                dxgi::Frame::Unchanged if !first => return Grab::Unchanged,
                dxgi::Frame::Unchanged => {
                    // A fresh duplication with nothing to report yet: seed the image from GDI.
                    let mut seed = std::mem::take(&mut self.image);
                    let ok = self.gdi(d, &mut seed, false);
                    self.image = seed;
                    if !ok {
                        return Grab::Failed;
                    }
                }
                dxgi::Frame::New => {}
            }
            if self.image.len() != d.width as usize * d.height as usize * 4 {
                self.dxgi = None; // the monitor changed size under us
                return Grab::Failed;
            }
            let t = Instant::now();
            out.clear();
            out.extend_from_slice(&self.image);
            self.cursor.paint(out, d.width as usize, d.height as usize, d.left, d.top);
            self.copy_time = gpu_copy + t.elapsed();
            return Grab::New;
        }

        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let t = Instant::now();
        if !self.gdi(d, out, true) {
            return Grab::Failed;
        }
        self.copy_time = t.elapsed();
        Grab::New
    }

    fn gdi(&mut self, d: &Display, out: &mut Vec<u8>, with_cursor: bool) -> bool {
        if self.gdi.is_none() {
            self.gdi = Capturer::new();
        }
        self.gdi.as_mut().is_some_and(|c| c.capture(d, out, with_cursor))
    }

    fn announce(&mut self, dxgi: bool, what: &str) {
        if self.announced != Some(dxgi) {
            println!("[server] capture: {what}");
            self.announced = Some(dxgi);
        }
    }
}

/// Reusable screen grabber. Holds the screen DC, a memory DC and a bitmap sized to the last
/// capture; they're only recreated when the capture size changes. Use from one thread.
pub struct Capturer {
    screen: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    w: i32,
    h: i32,
}

impl Capturer {
    pub fn new() -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return None;
            }
            let mem = CreateCompatibleDC(screen);
            Some(Self { screen, mem, bmp: HBITMAP::default(), old: HGDIOBJ::default(), w: 0, h: 0 })
        }
    }

    /// Capture monitor `d` into `out` as top-down BGRA, resizing `out` to `width * height * 4`.
    /// Returns false on failure.
    pub fn capture(&mut self, d: &Display, out: &mut Vec<u8>, with_cursor: bool) -> bool {
        let (w, h) = (d.width, d.height);
        if w <= 0 || h <= 0 {
            return false;
        }
        unsafe {
            if (w, h) != (self.w, self.h) {
                self.release_bitmap();
                self.bmp = CreateCompatibleBitmap(self.screen, w, h);
                if self.bmp.is_invalid() {
                    return false;
                }
                self.old = SelectObject(self.mem, HGDIOBJ(self.bmp.0));
                (self.w, self.h) = (w, h);
            }

            // Source origin is in virtual-screen space; GetDC(None) maps (0,0) to the primary's
            // top-left, so other monitors sit at their signed offsets.
            if BitBlt(self.mem, 0, 0, w, h, self.screen, d.left, d.top, SRCCOPY).is_err() {
                return false;
            }
            // BitBlt never includes the mouse pointer; draw it in ourselves.
            if with_cursor {
                draw_cursor(self.mem, d.left, d.top);
            }

            let mut bi = BITMAPINFO::default();
            bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            bi.bmiHeader.biWidth = w;
            bi.bmiHeader.biHeight = -h; // negative = top-down
            bi.bmiHeader.biPlanes = 1;
            bi.bmiHeader.biBitCount = 32;
            bi.bmiHeader.biCompression = BI_RGB.0 as u32;

            out.resize((w as usize) * (h as usize) * 4, 0);
            GetDIBits(
                self.mem,
                self.bmp,
                0,
                h as u32,
                Some(out.as_mut_ptr() as *mut c_void),
                &mut bi,
                DIB_RGB_COLORS,
            ) != 0
        }
    }

    unsafe fn release_bitmap(&mut self) {
        if !self.bmp.is_invalid() {
            SelectObject(self.mem, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            self.bmp = HBITMAP::default();
        }
        (self.w, self.h) = (0, 0);
    }
}

impl Drop for Capturer {
    fn drop(&mut self) {
        unsafe {
            self.release_bitmap();
            let _ = DeleteDC(self.mem);
            ReleaseDC(None, self.screen);
        }
    }
}

/// Draw the current mouse cursor into `dc`, whose origin sits at (`left`, `top`) of the virtual
/// desktop. Does nothing when the cursor is hidden; off-monitor positions are simply clipped.
unsafe fn draw_cursor(dc: HDC, left: i32, top: i32) {
    let mut ci = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };
    if GetCursorInfo(&mut ci).is_err() || ci.flags.0 & CURSOR_SHOWING.0 == 0 || ci.hCursor.is_invalid() {
        return;
    }
    let icon = HICON(ci.hCursor.0);
    let mut ii = ICONINFO::default();
    if GetIconInfo(icon, &mut ii).is_err() {
        return;
    }
    // GetIconInfo hands us copies of the cursor bitmaps; free them.
    if !ii.hbmMask.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
    }
    if !ii.hbmColor.is_invalid() {
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
    }
    let x = ci.ptScreenPos.x - left - ii.xHotspot as i32;
    let y = ci.ptScreenPos.y - top - ii.yHotspot as i32;
    let _ = DrawIconEx(dc, x, y, icon, 0, 0, 0, None, DI_NORMAL);
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
