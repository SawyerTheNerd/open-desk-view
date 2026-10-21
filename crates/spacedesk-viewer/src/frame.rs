//! Shared framebuffer store. The network thread decodes JPEG tiles and composites them here;
//! the UI thread reads it to present. Pixels are packed `0x00RRGGBB` (softbuffer's format).

use std::sync::Mutex;

/// Maximum accepted width/height from the (untrusted) server, per axis.
pub const MAX_DIM: u32 = 16384;

/// Validate server-supplied dimensions and that `w*h` fits without overflow.
fn dims_ok(w: u32, h: u32) -> bool {
    w > 0 && h > 0 && w <= MAX_DIM && h <= MAX_DIM
        && (w as usize).checked_mul(h as usize).is_some()
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
    /// Bumped on every committed frame so the UI can tell whether to repaint.
    pub generation: u64,
}

impl Frame {
    pub fn new() -> Self {
        Frame { width: 0, height: 0, pixels: Vec::new(), generation: 0 }
    }

    /// Resize the backing buffer. Returns false (and leaves the frame unchanged) for
    /// out-of-range or overflowing dimensions.
    fn ensure_size(&mut self, w: u32, h: u32) -> bool {
        if !dims_ok(w, h) {
            return false;
        }
        if self.width != w || self.height != h {
            self.width = w;
            self.height = h;
            self.pixels = vec![0u32; (w as usize) * (h as usize)];
        }
        true
    }

    /// Composite a decoded RGB(8) tile into the full framebuffer at the given rectangle.
    /// `rgb` is tightly packed `rect_w * rect_h * 3` bytes.
    pub fn blit_rgb_tile(
        &mut self,
        full_w: u32,
        full_h: u32,
        left: u32,
        top: u32,
        rect_w: u32,
        rect_h: u32,
        rgb: &[u8],
    ) {
        if !self.ensure_size(full_w, full_h) {
            return;
        }
        for row in 0..rect_h {
            let dy = top + row;
            if dy >= full_h {
                break;
            }
            let dst_row = (dy as usize) * (full_w as usize);
            let src_row = (row as usize) * (rect_w as usize) * 3;
            for col in 0..rect_w {
                let dx = left + col;
                if dx >= full_w {
                    break;
                }
                let s = src_row + (col as usize) * 3;
                if s + 2 >= rgb.len() {
                    break;
                }
                let r = rgb[s] as u32;
                let g = rgb[s + 1] as u32;
                let b = rgb[s + 2] as u32;
                self.pixels[dst_row + dx as usize] = (r << 16) | (g << 8) | b;
            }
        }
    }
}

pub type SharedFrame = std::sync::Arc<Mutex<Frame>>;
