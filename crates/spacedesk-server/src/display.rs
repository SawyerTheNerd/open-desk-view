//! Virtual display management: locate the IddCx virtual monitor (Virtual Display Driver) and
//! switch it to the mode that best matches the connecting viewer's screen, so the viewer gets
//! a true extended desktop at its own resolution.
#![cfg(windows)]

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::*;

/// Adapter string reported by the open-source Virtual Display Driver (MttVDD).
const VDD_ADAPTER: &str = "Virtual Display Driver";
const DISPLAY_DEVICE_ATTACHED_TO_DESKTOP: u32 = 0x1;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// Device name (e.g. `\\.\DISPLAY21`) of the attached Virtual Display Driver monitor.
/// Looked up by adapter name, so it survives reboots and monitor renumbering.
pub fn find_virtual_display() -> Option<String> {
    let mut i = 0;
    loop {
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), i, &mut dd, 0) }.as_bool() {
            return None;
        }
        i += 1;
        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP != 0
            && from_wide(&dd.DeviceString).contains(VDD_ADAPTER)
        {
            return Some(from_wide(&dd.DeviceName));
        }
    }
}

/// Switch `device` to the supported mode closest to `w`x`h` (exact if available; otherwise the
/// closest aspect ratio that fits, then the largest such). Returns the mode now in effect.
pub fn match_mode(device: &str, w: u32, h: u32) -> Option<(u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let name = wide(device);
    let want_aspect = w as f64 / h as f64;

    let mut best: Option<DEVMODEW> = None;
    let mut best_key = (u64::MAX, u64::MAX, 0u32);
    let mut i = 0;
    loop {
        let mut dm = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if !unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_DISPLAY_SETTINGS_MODE(i), &mut dm) }
            .as_bool()
        {
            break;
        }
        i += 1;
        let (mw, mh) = (dm.dmPelsWidth, dm.dmPelsHeight);
        if mw == 0 || mh == 0 || mw > w || mh > h {
            continue; // never pick a mode larger than the viewer's screen
        }
        let aspect_gap = (((mw as f64 / mh as f64) - want_aspect).abs() * 1000.0) as u64;
        let area_gap = (w as u64 * h as u64) - (mw as u64 * mh as u64);
        // Lower aspect gap, then lower area gap, then higher refresh rate.
        let key = (aspect_gap, area_gap, u32::MAX - dm.dmDisplayFrequency);
        if key < best_key {
            best_key = key;
            best = Some(dm);
        }
    }

    let mut dm = best?;
    let (bw, bh) = (dm.dmPelsWidth, dm.dmPelsHeight);

    if let Some((cw, ch)) = current_mode(device) {
        if (cw, ch) == (bw, bh) {
            return Some((bw, bh));
        }
    }

    dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
    let r = unsafe {
        ChangeDisplaySettingsExW(
            PCWSTR(name.as_ptr()),
            Some(&dm),
            HWND::default(),
            CDS_UPDATEREGISTRY,
            None,
        )
    };
    if r == DISP_CHANGE_SUCCESSFUL {
        Some((bw, bh))
    } else {
        eprintln!("[server] could not set {device} to {bw}x{bh} (code {})", r.0);
        current_mode(device)
    }
}

fn current_mode(device: &str) -> Option<(u32, u32)> {
    let name = wide(device);
    let mut dm = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        ..Default::default()
    };
    unsafe { EnumDisplaySettingsW(PCWSTR(name.as_ptr()), ENUM_CURRENT_SETTINGS, &mut dm) }
        .as_bool()
        .then_some((dm.dmPelsWidth, dm.dmPelsHeight))
}
