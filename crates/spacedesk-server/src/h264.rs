//! H.264 encoding via Cisco's prebuilt OpenH264 library, loaded at runtime. Viewers decode this
//! in hardware (e.g. Android MediaCodec), which is far faster than software JPEG decode.
#![cfg(windows)]

use std::path::PathBuf;

use openh264::encoder::{
    BitRate, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, RateControlMode, UsageType,
};
use openh264::formats::{BgraSliceU8, YUVBuffer};
use openh264::OpenH264API;

const DLL_NAME: &str = "openh264-2.6.0-win64.dll";

/// Locate Cisco's prebuilt OpenH264 library. To avoid DLL-planting, we only accept an explicitly
/// configured absolute path (`OPENH264_DLL`) or the file sitting next to our own executable — never
/// the current working directory. The loaded blob's hash is additionally verified by
/// `OpenH264API::from_blob_path` (it rejects anything that isn't a known Cisco build).
fn find_lib() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("OPENH264_DLL") {
        let pb = PathBuf::from(p);
        if pb.is_absolute() && pb.exists() {
            return Some(pb);
        }
    }
    let p = std::env::current_exe().ok()?.parent()?.join(DLL_NAME);
    p.exists().then_some(p)
}

pub struct H264Encoder {
    enc: Encoder,
    yuv: YUVBuffer,
    w: usize,
    h: usize,
}

impl H264Encoder {
    /// `w`/`h` must be even (YUV 4:2:0).
    pub fn new(w: usize, h: usize, bitrate_bps: u32) -> Result<Self, String> {
        let lib = find_lib().ok_or_else(|| {
            format!(
                "{DLL_NAME} not found — put it next to the server, set OPENH264_DLL, or download \
                 it from http://ciscobinary.openh264.org/{DLL_NAME}.bz2"
            )
        })?;
        let api = OpenH264API::from_blob_path(&lib).map_err(|e| format!("loading {}: {e}", lib.display()))?;
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(60.0))
            // Not supported for screen content; OpenH264 turns them off anyway (and warns).
            .adaptive_quantization(false)
            .background_detection(false)
            // Every captured frame must produce output: each message carries one access unit.
            .skip_frames(false)
            // Periodic IDR so a decoder that hiccups recovers within a few seconds.
            .intra_frame_period(IntraFramePeriod::from_num_frames(300));
        let enc = Encoder::with_api_config(api, config).map_err(|e| format!("encoder init failed: {e}"))?;
        Ok(Self { enc, yuv: YUVBuffer::new(w, h), w, h })
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// Encode one top-down BGRA frame of exactly `dimensions()` into `out` as one Annex-B access
    /// unit (SPS/PPS are included with every IDR). Returns false if the encoder produced nothing.
    pub fn encode(&mut self, bgra: &[u8], out: &mut Vec<u8>) -> bool {
        self.yuv.read_bgra8(BgraSliceU8::new(bgra, (self.w, self.h)));
        out.clear();
        match self.enc.encode(&self.yuv) {
            Ok(bs) => bs.write_vec(out),
            Err(_) => return false,
        }
        !out.is_empty()
    }
}

/// View a top-down BGRA frame of `w`x`h` cropped to even dimensions (needed for 4:2:0). Borrows
/// `bgra` directly when the width is already even; otherwise copies the cropped rows to `scratch`.
pub fn even_view<'a>(bgra: &'a [u8], w: usize, h: usize, scratch: &'a mut Vec<u8>) -> (&'a [u8], usize, usize) {
    let (ew, eh) = (w & !1, h & !1);
    if ew == w {
        return (&bgra[..ew * eh * 4], ew, eh);
    }
    scratch.clear();
    for row in bgra.chunks_exact(w * 4).take(eh) {
        scratch.extend_from_slice(&row[..ew * 4]);
    }
    (scratch, ew, eh)
}
