use super::{hsb_to_rgb, scaled_px};
use crate::rgb::{elapsed_secs, ActiveRgb};
use std::time::Instant;

/// watercolorColor: low-saturation hue wheel at `position` (0..1).
fn watercolor_color(position: f64) -> (i32, i32, i32) {
    let position = position % 1.0;
    hsb_to_rgb(position * 360.0, 0.4, 1.0)
}

impl ActiveRgb {
    /// Watercolor: fast-moving pastel hue wheel across the LEDs.
    pub fn watercolor(&mut self, start: Instant) {
        let elapsed = elapsed_secs(&start) / self.rgb_mode_speed;
        let n = self.light_channels;
        let px = (0..n)
            .map(|i| {
                let position = ((i as f64 / n as f64) + (elapsed * 4.0)) % 1.0;
                let (r, g, b) = watercolor_color(position);
                scaled_px(r as f64, g as f64, b as f64, self.brightness)
            })
            .collect();
        self.set_pixels(px);
    }
}
