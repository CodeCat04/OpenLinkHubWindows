use super::hsb_to_rgb;
use crate::rgb::{elapsed_ms, lerp, ActiveRgb};
use rand::Rng;
use std::f64::consts::PI;
use std::time::Instant;

/// interstellarPalette: flowing blue-violet-cyan space gradient as (h, s, v).
fn interstellar_palette(t: f64) -> (f64, f64, f64) {
    if t < 0.45 {
        (lerp(220.0, 300.0, t / 0.45), 0.9, 0.4 + 0.5 * t)
    } else if t < 0.75 {
        (lerp(300.0, 180.0, (t - 0.45) / 0.30), 0.9, 0.6)
    } else {
        (lerp(180.0, 220.0, (t - 0.75) / 0.25), 0.9, 0.5)
    }
}

/// generateInterstellarColors: scrolling nebula with a rare random twinkle.
fn generate_interstellar_colors(light_channels: usize, progress: f64, brightness: f64) -> Vec<[u8; 3]> {
    let mut colors: Vec<(i32, i32, i32)> = (0..light_channels)
        .map(|i| {
            let pos = i as f64 / light_channels as f64 + progress;
            let t = ((pos * PI * 2.0).sin() + 1.0) * 0.5;
            let (h, s, v) = interstellar_palette(t);
            hsb_to_rgb(h, s, v * brightness * 0.6)
        })
        .collect();

    let mut rng = rand::thread_rng();
    for c in colors.iter_mut() {
        if rng.gen::<f64>() < 0.001 {
            *c = hsb_to_rgb(rng.gen::<f64>() * 360.0, 0.8, brightness);
        }
    }

    colors.into_iter().map(|(r, g, b)| [r as u8, g as u8, b as u8]).collect()
}

impl ActiveRgb {
    /// Nebula: slow space-like gradient with occasional sparks.
    pub fn nebula(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let mut progress = (elapsed as f64 / (self.rgb_mode_speed * 5000.0)) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
            progress = 0.0;
        }
        let px = generate_interstellar_colors(self.light_channels, progress, self.brightness);
        self.set_pixels(px);
    }
}
