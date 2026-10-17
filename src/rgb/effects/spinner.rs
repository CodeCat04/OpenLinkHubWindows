use super::to_px;
use crate::rgb::{elapsed_ms, interpolate_colors, ActiveRgb};
use std::time::Instant;

impl ActiveRgb {
    /// Spinner: a single lit LED runs around the ring once per cycle.
    pub fn spinner(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let mut progress = (elapsed as f64 / (self.rgb_mode_speed * 1000.0)) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
            progress = 0.0;
        }

        let n = self.light_channels;
        let active = (progress * n as f64) as i64;
        let px = (0..n)
            .map(|j| {
                if j as i64 == active {
                    let t = active as f64 / 40.0;
                    to_px(&interpolate_colors(&self.start_color, &self.end_color, t, self.brightness))
                } else {
                    [0, 0, 0]
                }
            })
            .collect();
        self.set_pixels(px);
    }
}
