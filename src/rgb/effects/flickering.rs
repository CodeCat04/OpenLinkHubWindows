use super::to_px;
use crate::rgb::{elapsed_ms, interpolate_colors, ActiveRgb};
use rand::Rng;
use std::time::Instant;

impl ActiveRgb {
    /// Flickering: start-to-end gradient with LEDs randomly blanking for a frame.
    pub fn flickering(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let progress = (elapsed as f64 / (self.rgb_mode_speed * 1000.0)) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
        }

        let n = self.light_channels;
        // Go: rand.Intn(LightChannels * int(RgbModeSpeed)), which panics when that is 0.
        let range = (n as i64).saturating_mul(self.rgb_mode_speed as i64);
        let mut rng = rand::thread_rng();

        let px = (0..n)
            .map(|j| {
                let t = j as f64 / n as f64;
                let colors = interpolate_colors(&self.start_color, &self.end_color, t, self.brightness);
                if range > 0 && rng.gen_range(0..range) == 1 {
                    [0, 0, 0]
                } else {
                    to_px(&colors)
                }
            })
            .collect();
        self.set_pixels(px);
    }
}
