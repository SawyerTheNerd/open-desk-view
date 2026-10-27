//! Resource-usage settings. A profile sets every knob; individual command-line flags override it.
//! The server's values are ceilings: a viewer may ask for a lower frame rate or quality, never more.

use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Frame-rate ceiling.
    pub max_fps: u32,
    /// H.264 bitrate budget in bits per pixel per second, before the viewer's quality scaling.
    pub h264_bits_per_pixel: u32,
    /// Fixed H.264 bitrate in kbps, overriding the per-pixel budget.
    pub bitrate_kbps: Option<u32>,
    /// JPEG quality ceiling (1-100).
    pub jpeg_quality: u8,
    /// Once the screen has been still for a moment, how often to look for changes.
    pub idle_poll: Duration,
}

pub const PROFILE_NAMES: &str = "eco, balanced, performance";

impl Settings {
    /// `eco`: least CPU/bandwidth/battery. `balanced` (default). `performance`: smoothest.
    pub fn profile(name: &str) -> Option<Self> {
        let (max_fps, bpp, jpeg_quality, idle_ms) = match name {
            "eco" => (24, 4, 60, 250),
            "balanced" => (30, 6, 75, 100),
            "performance" => (60, 10, 85, 16),
            _ => return None,
        };
        Some(Settings {
            max_fps,
            h264_bits_per_pixel: bpp,
            bitrate_kbps: None,
            jpeg_quality,
            idle_poll: Duration::from_millis(idle_ms),
        })
    }

    /// Frame rate for a viewer that asked for `requested` fps (0 = no preference).
    pub fn fps_for(&self, requested: u32) -> u32 {
        let fps = if requested == 0 { self.max_fps } else { requested.min(self.max_fps) };
        fps.max(1)
    }

    /// JPEG quality for a viewer that asked for `requested` (outside 1-100 = no preference).
    pub fn jpeg_quality_for(&self, requested: u32) -> u8 {
        match requested {
            1..=100 => (requested as u8).min(self.jpeg_quality),
            _ => self.jpeg_quality,
        }
    }

    /// H.264 bitrate (bps) for a `w`x`h` stream, scaled by the viewer's quality request (1-100).
    pub fn h264_bitrate_for(&self, w: usize, h: usize, requested_quality: u32) -> u32 {
        if let Some(kbps) = self.bitrate_kbps {
            return kbps.saturating_mul(1000);
        }
        let q = match requested_quality {
            1..=100 => requested_quality as u64,
            _ => 100,
        };
        ((w * h) as u64 * self.h264_bits_per_pixel as u64 * q / 100).clamp(1_000_000, 25_000_000) as u32
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings::profile("balanced").unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewer_can_lower_but_not_raise() {
        let s = Settings::profile("balanced").unwrap();
        assert_eq!(s.fps_for(0), 30);
        assert_eq!(s.fps_for(24), 24);
        assert_eq!(s.fps_for(120), 30);
        assert_eq!(s.jpeg_quality_for(50), 50);
        assert_eq!(s.jpeg_quality_for(95), 75);
        assert_eq!(s.jpeg_quality_for(0), 75);
    }

    #[test]
    fn bitrate_scales_and_overrides() {
        let mut s = Settings::profile("balanced").unwrap();
        // 1366x768 at 6 bpp = ~6.3 Mbps; half quality halves it.
        assert_eq!(s.h264_bitrate_for(1366, 768, 100), 6_294_528);
        assert_eq!(s.h264_bitrate_for(1366, 768, 50), 3_147_264);
        s.bitrate_kbps = Some(4000);
        assert_eq!(s.h264_bitrate_for(1920, 1080, 10), 4_000_000);
    }
}
