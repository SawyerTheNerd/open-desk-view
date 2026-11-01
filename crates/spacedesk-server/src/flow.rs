//! Flow control for one session: how many frames may be unacknowledged, when the viewer counts
//! as stalled, and when it needs the current screen again as a keyframe. Pure state, so the rules
//! are tested here without a socket, a GPU or a viewer; the session loop feeds it acks and sends.

use std::time::Duration;

/// Unacknowledged H.264 frames allowed in flight (covers the viewer decoder's buffering).
pub const H264_WINDOW: u32 = 3;
/// JPEG viewers ack each frame as soon as it's painted.
pub const JPEG_WINDOW: u32 = 1;
/// No ack for this long means the viewer stalled (hidden, decoder stuck, or it dropped frames it
/// couldn't decode). Doubles while it stays stalled, up to the max.
const STALL_TIMEOUT: Duration = Duration::from_secs(1);
const MAX_STALL_TIMEOUT: Duration = Duration::from_secs(4);

pub struct FlowControl {
    window: u32,
    in_flight: u32,
    stall_timeout: Duration,
    refresh: bool,
}

impl FlowControl {
    pub fn new(window: u32) -> Self {
        FlowControl { window: window.max(1), in_flight: 0, stall_timeout: STALL_TIMEOUT, refresh: false }
    }

    /// Change the window, e.g. when the session falls back from H.264 to JPEG.
    pub fn set_window(&mut self, window: u32) {
        self.window = window.max(1);
    }

    /// May another frame go out now?
    pub fn can_send(&self) -> bool {
        self.in_flight < self.window
    }

    /// How long to wait for an ack while the window is full before calling the viewer stalled.
    pub fn stall_timeout(&self) -> Duration {
        self.stall_timeout
    }

    /// Must the next frame be the current screen as a keyframe, even if nothing changed?
    pub fn needs_refresh(&self) -> bool {
        self.refresh
    }

    pub fn on_ack(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.stall_timeout = STALL_TIMEOUT;
    }

    pub fn on_sent(&mut self) {
        self.in_flight += 1;
        self.refresh = false;
    }

    /// The window stayed full for [`Self::stall_timeout`]. What's in flight is presumed lost:
    /// allow exactly one keyframe probe of the current screen (the window reopens only when it's
    /// acked), so the viewer recovers the moment it can decode again, and back off while it stays
    /// stalled (e.g. hidden in the background).
    pub fn on_stall(&mut self) {
        self.in_flight = self.window - 1;
        self.refresh = true;
        self.stall_timeout = (self.stall_timeout * 2).min(MAX_STALL_TIMEOUT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_limits_unacked_frames() {
        let mut f = FlowControl::new(H264_WINDOW);
        for _ in 0..3 {
            assert!(f.can_send());
            f.on_sent();
        }
        assert!(!f.can_send());
        f.on_ack();
        assert!(f.can_send());
    }

    #[test]
    fn stall_allows_one_keyframe_probe() {
        let mut f = FlowControl::new(H264_WINDOW);
        (0..3).for_each(|_| f.on_sent());
        f.on_stall();
        assert!(f.can_send() && f.needs_refresh());
        f.on_sent();
        assert!(!f.can_send(), "only the probe goes out until the viewer acks");
        assert!(!f.needs_refresh());
    }

    #[test]
    fn stall_timeout_backs_off_and_resets_on_ack() {
        let mut f = FlowControl::new(JPEG_WINDOW);
        let waits: Vec<_> = (0..4)
            .map(|_| {
                f.on_sent();
                let t = f.stall_timeout();
                f.on_stall();
                t.as_secs()
            })
            .collect();
        assert_eq!(waits, [1, 2, 4, 4]);
        f.on_ack();
        assert_eq!(f.stall_timeout(), STALL_TIMEOUT);
    }

    #[test]
    fn late_acks_never_underflow() {
        let mut f = FlowControl::new(H264_WINDOW);
        f.on_ack();
        f.on_ack();
        assert!(f.can_send());
    }
}
