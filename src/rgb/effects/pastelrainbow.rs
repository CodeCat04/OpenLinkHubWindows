use super::scaled_px;
use crate::rgb::{elapsed_secs, ActiveRgb};
use std::time::Instant;

/// Pastel palette stops, evenly spaced around the wheel.
const PASTEL_COLORS: [(f64, f64, f64); 20] = [
    (244.0, 253.0, 255.0),
    (255.0, 218.0, 247.0),
    (255.0, 140.0, 230.0),
    (254.0, 96.0, 219.0),
    (255.0, 17.0, 206.0),
    (254.0, 28.0, 206.0),
    (255.0, 106.0, 225.0),
    (255.0, 184.0, 241.0),
    (255.0, 230.0, 248.0),
    (255.0, 255.0, 203.0),
    (255.0, 255.0, 155.0),
    (255.0, 255.0, 80.0),
    (255.0, 255.0, 33.0),
    (216.0, 250.0, 52.0),
    (161.0, 244.0, 119.0),
    (105.0, 237.0, 185.0),
    (14.0, 208.0, 250.0),
    (37.0, 210.0, 255.0),
    (114.0, 227.0, 255.0),
    (159.0, 235.0, 253.0),
];

/// pastelRainbowColor: RGB (0..255) at `position` (0..1) on the pastel wheel.
fn pastel_rainbow_color(position: f64) -> (i32, i32, i32) {
    let position = position - position.floor();
    let len = PASTEL_COLORS.len();
    let pos = position * len as f64;
    let idx = pos as usize;
    let frac = pos - idx as f64;
    // `position` can round up to exactly 1.0; Go would index out of range there.
    let idx = idx % len;
    let next_idx = (idx + 1) % len;

    let c1 = PASTEL_COLORS[idx];
    let c2 = PASTEL_COLORS[next_idx];
    let r = c1.0 + (c2.0 - c1.0) * frac;
    let g = c1.1 + (c2.1 - c1.1) * frac;
    let b = c1.2 + (c2.2 - c1.2) * frac;
    (r as i32, g as i32, b as i32)
}

impl ActiveRgb {
    /// PastelRainbow: pastel wheel spread across the LEDs, rotating over time.
    pub fn pastel_rainbow(&mut self, start: Instant) {
        let speed_factor = if self.rgb_mode_speed > 0.0 { 4.0 / self.rgb_mode_speed } else { 4.0 };
        let elapsed = elapsed_secs(&start) * speed_factor;
        let n = self.light_channels;
        let px = (0..n)
            .map(|i| {
                let position = ((i as f64 / n as f64) + (elapsed / 4.0)) % 1.0;
                let (r, g, b) = pastel_rainbow_color(position);
                scaled_px(r as f64, g as f64, b as f64, self.brightness)
            })
            .collect();
        self.set_pixels(px);
    }

    /// PastelSpiralRainbow: pastel wheel repeated `5` times across the LEDs.
    pub fn pastel_spiral_rainbow(&mut self, start: Instant) {
        let speed_factor = if self.rgb_mode_speed > 0.0 { 2.0 / self.rgb_mode_speed } else { 2.0 };
        let spiral_density = 5.0;
        let elapsed = elapsed_secs(&start) * speed_factor;
        let n = self.light_channels;
        let px = (0..n)
            .map(|i| {
                let position = ((elapsed / 4.0) + (i as f64 * spiral_density / n as f64)) % 1.0;
                let (r, g, b) = pastel_rainbow_color(position);
                scaled_px(r as f64, g as f64, b as f64, self.brightness)
            })
            .collect();
        self.set_pixels(px);
    }
}
