//! Screen capture via DXGI Desktop Duplication. The GPU tells us when the desktop actually
//! changes, so a still screen costs (almost) nothing, and copying a changed frame takes a few
//! milliseconds instead of the tens GDI needs to read the composed screen back.
#![cfg(windows)]

use std::time::Duration;

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::*;

pub enum Frame {
    /// A new desktop image (or pointer move) was copied into the buffer.
    New,
    /// Nothing changed within the wait.
    Unchanged,
    /// Duplication was lost (mode change, secure desktop, ...); recreate it.
    Lost,
}

pub struct Duplicator {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    staging: Option<ID3D11Texture2D>,
    w: usize,
    h: usize,
    /// Time the last image copy took (GPU -> CPU), excluding the wait for a change.
    pub last_copy: Duration,
}

impl Duplicator {
    /// Start duplicating the monitor whose GDI device name is `name` (e.g. `\\.\DISPLAY21`).
    pub fn new(name: &str) -> Result<Self, String> {
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(|e| format!("DXGI factory: {e}"))?;
            let (adapter, output) = find_output(&factory, name).ok_or_else(|| format!("{name} is not a DXGI output"))?;

            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| format!("D3D11 device: {e}"))?;
            let device = device.ok_or("no D3D11 device")?;
            let context = context.ok_or("no D3D11 context")?;

            let output1: IDXGIOutput1 = output.cast().map_err(|e| format!("IDXGIOutput1: {e}"))?;
            let dup = output1.DuplicateOutput(&device).map_err(|e| format!("DuplicateOutput: {e}"))?;
            let desc = dup.GetDesc();
            Ok(Self {
                device,
                context,
                dup,
                staging: None,
                w: desc.ModeDesc.Width as usize,
                h: desc.ModeDesc.Height as usize,
                last_copy: Duration::ZERO,
            })
        }
    }

    /// Wait up to `wait` for a change. On a desktop-image change, copy it (top-down BGRA, no
    /// cursor) into `image`; a pointer-only change returns `New` with `image` untouched.
    /// `force` copies the current image even if this update was pointer-only.
    pub fn grab(&mut self, wait: Duration, image: &mut Vec<u8>, force: bool) -> Frame {
        unsafe {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;
            match self.dup.AcquireNextFrame(wait.as_millis() as u32, &mut info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Frame::Unchanged,
                Err(_) => return Frame::Lost,
            }
            let image_changed = info.LastPresentTime != 0 || force;
            let t = std::time::Instant::now();
            let copied = if image_changed {
                resource.and_then(|r| r.cast::<ID3D11Texture2D>().ok()).is_some_and(|tex| self.copy(&tex, image))
            } else {
                true
            };
            self.last_copy = t.elapsed();
            let _ = self.dup.ReleaseFrame();
            if copied {
                Frame::New
            } else {
                Frame::Lost
            }
        }
    }

    /// GPU texture -> CPU staging texture -> `image` (tightly packed rows).
    unsafe fn copy(&mut self, tex: &ID3D11Texture2D, image: &mut Vec<u8>) -> bool {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        let (w, h) = (desc.Width as usize, desc.Height as usize);
        if self.staging.is_none() || (w, h) != (self.w, self.h) {
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: desc.Width,
                Height: desc.Height,
                MipLevels: 1,
                ArraySize: 1,
                Format: desc.Format,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };
            let mut staging = None;
            if self.device.CreateTexture2D(&staging_desc, None, Some(&mut staging)).is_err() {
                return false;
            }
            self.staging = staging;
            (self.w, self.h) = (w, h);
        }
        let Some(staging) = &self.staging else { return false };
        self.context.CopyResource(staging, tex);

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        if self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).is_err() {
            return false;
        }
        let row = w * 4;
        image.resize(row * h, 0);
        let src = mapped.pData as *const u8;
        for y in 0..h {
            std::ptr::copy_nonoverlapping(src.add(y * mapped.RowPitch as usize), image.as_mut_ptr().add(y * row), row);
        }
        self.context.Unmap(staging, 0);
        true
    }
}

/// The adapter and output whose GDI device name matches `name`.
unsafe fn find_output(factory: &IDXGIFactory1, name: &str) -> Option<(IDXGIAdapter1, IDXGIOutput)> {
    for a in 0.. {
        let adapter = factory.EnumAdapters1(a).ok()?;
        for o in 0.. {
            let Ok(output) = adapter.EnumOutputs(o) else { break };
            if let Ok(desc) = output.GetDesc() {
                let n = desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(desc.DeviceName.len());
                if String::from_utf16_lossy(&desc.DeviceName[..n]) == name {
                    return Some((adapter, output));
                }
            }
        }
    }
    None
}
