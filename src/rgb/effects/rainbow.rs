use super::scaled_px;
use crate::rgb::{elapsed_secs, interpolate, ActiveRgb};
use std::time::Instant;

/// rainbowColor: RGB (0..255) at `position` (0..1) on a red-yellow-green-cyan-blue-red wheel.
fn rainbow_color(position: f64) -> (i32, i32, i32) {
    let position = position % 1.0;
    if position < 0.2 {
        interpolate(1.0, 0.0, 0.0, 1.0, 1.0, 0.0, position / 0.2)
    } else if position < 0.4 {
        interpolate(1.0, 1.0, 0.0, 0.0, 1.0, 0.0, (position - 0.2) / 0.2)
    } else if position < 0.6 {
        interpolate(0.0, 1.0, 0.0, 0.0, 1.0, 1.0, (position - 0.4) / 0.2)
    } else if position < 0.8 {
        interpolate(0.0, 1.0, 1.0, 0.0, 0.0, 1.0, (position - 0.6) / 0.2)
    } else {
        interpolate(0.0, 0.0, 1.0, 1.0, 0.0, 0.0, (position - 0.8) / 0.2)
    }
}

impl ActiveRgb {
    /// Rainbow: the full wheel spread across the LEDs, rotating over time.
    pub fn rainbow(&mut self, start: Instant) {
        let speed_factor = if self.rgb_mode_speed > 0.0 { 4.0 / self.rgb_mode_speed } else { 4.0 };
        let elapsed = elapsed_secs(&start) * speed_factor;
        let n = self.light_channels;
        let px = (0..n)
            .map(|i| {
                let position = ((i as f64 / n as f64) + (elapsed / 4.0)) % 1.0;
                let (r, g, b) = rainbow_color(position);
                scaled_px(r as f64, g as f64, b as f64, self.brightness)
            })
            .collect();
        self.set_pixels(px);
    }

    /// SpiralRainbow: rainbow with the wheel repeated `3` times across the LEDs.
    pub fn spiral_rainbow(&mut self, start: Instant) {
        let speed_factor = if self.rgb_mode_speed > 0.0 { 4.0 / self.rgb_mode_speed } else { 4.0 };
        let spiral_density = 3.0;
        let elapsed = elapsed_secs(&start) * speed_factor;
        let n = self.light_channels;
        let px = (0..n)
            .map(|i| {
                let position = ((elapsed / 4.0) + (i as f64 * spiral_density / n as f64)) % 1.0;
                let (r, g, b) = rainbow_color(position);
                scaled_px(r as f64, g as f64, b as f64, self.brightness)
            })
            .collect();
        self.set_pixels(px);
    }
}
