use super::to_px;
use crate::rgb::{elapsed_ms, modify_brightness, ActiveRgb, Color};
use std::time::Instant;

/// HsvToRgb: integer HSV (hue in degrees, s/v in 0..255) packed as 0xRRGGBB.
/// Mirrors Go exactly, including its odd `x` term and the wrap of negative channels.
fn hsv_to_rgb_packed(hue: i64, saturation: i64, value: i64) -> u32 {
    let h = hue % 360;
    let s = saturation as f64 / 255.0;
    let v = value as f64 / 255.0;

    let c = v * s;
    let x = c * (1.0 - (h as f64 / 60.0 - ((h / 60) * 2) as f64).abs());
    let m = v - c;

    let (r, g, b) = if h < 60 {
        (c, x, 0.0)
    } else if h < 120 {
        (x, c, 0.0)
    } else if h < 180 {
        (0.0, c, x)
    } else if h < 240 {
        (0.0, x, c)
    } else if h < 300 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    let ri = ((r + m) * 255.0) as i64;
    let gi = ((g + m) * 255.0) as i64;
    let bi = ((b + m) * 255.0) as i64;
    ((ri << 16) | (gi << 8) | bi) as u32
}

impl ActiveRgb {
    /// Rotator: hue-rotating pattern driven by the start color's channels.
    pub fn rotator(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let hue = elapsed as f64 / self.rgb_mode_speed / 2.0;
        let base = hue as i64;
        let sc = self.start_color;

        // Go keeps only the low byte of the packed value for each channel.
        let channel = |j: usize, value: f64| -> f64 {
            let h = base.wrapping_add((j as i64).wrapping_mul(5));
            (hsv_to_rgb_packed(h, 255, value as i64) as u8) as f64
        };

        let px = (0..self.light_channels)
            .map(|j| {
                let color = Color {
                    red: channel(j, sc.red),
                    green: channel(j, sc.green),
                    blue: channel(j, sc.blue),
                    brightness: self.brightness,
                    ..Color::default()
                };
                to_px(&modify_brightness(color))
            })
            .collect();
        self.set_pixels(px);
    }
}
