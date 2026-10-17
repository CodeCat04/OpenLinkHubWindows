use super::scaled_px;
use crate::rgb::{clamp_float01, elapsed_secs, random01, ActiveRgb, Color};
use std::time::Instant;

/// randomRainColor: deterministic saturated HSV color for a drop (slot, channel).
fn random_rain_color(slot: f64, channel: f64) -> (f64, f64, f64) {
    let h = random01(&[slot, channel, 1.0]);
    let s = 0.85 + 0.15 * random01(&[slot, channel, 2.0]);
    let v = 0.90 + 0.10 * random01(&[slot, channel, 3.0]);

    let mut h = h % 1.0;
    if h < 0.0 {
        h += 1.0;
    }

    let i = (h * 6.0) as i64;
    let f = h * 6.0 - i as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);

    match i % 6 {
        0 => (v * 255.0, t * 255.0, p * 255.0),
        1 => (q * 255.0, v * 255.0, p * 255.0),
        2 => (p * 255.0, v * 255.0, t * 255.0),
        3 => (p * 255.0, q * 255.0, v * 255.0),
        4 => (t * 255.0, p * 255.0, v * 255.0),
        _ => (v * 255.0, p * 255.0, q * 255.0),
    }
}

/// generateRainColors: per LED, the brightest of the drops spawned in the last 6 slots.
fn generate_rain_colors(
    light_channels: usize,
    elapsed: f64,
    brightness: f64,
    speed: f64,
    random: bool,
    start: &Color,
    end: &Color,
) -> Vec<[u8; 3]> {
    let speed = if speed <= 0.0 { 1.0 } else { speed };
    let step_duration = 0.115 / speed;
    let drop_duration = 0.55 / speed.sqrt();
    let spawn_chance = clamp_float01(0.18 + (speed - 1.0) * 0.035);
    let current_slot = (elapsed / step_duration).floor() as i64;

    (0..light_channels)
        .map(|i| {
            let (mut best_r, mut best_g, mut best_b) = (0.0, 0.0, 0.0);
            let mut best_intensity = 0.0;

            for back in 0..6 {
                let slot = current_slot - back;
                if slot < 0 {
                    continue;
                }
                let slot_f = slot as f64;
                let channel_f = (i + 1) as f64;

                if random01(&[slot_f, channel_f, 10.0]) > spawn_chance {
                    continue;
                }

                let jitter = random01(&[slot_f, channel_f, 20.0]) * step_duration;
                let drop_start = slot_f * step_duration + jitter;
                let age = elapsed - drop_start;
                if age < 0.0 || age > drop_duration {
                    continue;
                }

                let norm = age / drop_duration;
                let mut intensity = (1.0 - norm).powf(1.9);
                intensity *= 0.78 + 0.22 * random01(&[slot_f, channel_f, 30.0]);

                let (rr, gg, bb) = if random {
                    random_rain_color(slot_f, channel_f)
                } else if norm < 0.5 {
                    (start.red, start.green, start.blue)
                } else {
                    (end.red, end.green, end.blue)
                };

                if intensity > best_intensity {
                    best_intensity = intensity;
                    best_r = rr * intensity;
                    best_g = gg * intensity;
                    best_b = bb * intensity;
                }
            }

            scaled_px(best_r, best_g, best_b, brightness)
        })
        .collect()
}

impl ActiveRgb {
    /// Rain: fading drops; random hues when start and end colors match.
    pub fn rain(&mut self, start: Instant) {
        let elapsed = elapsed_secs(&start);
        let speed_factor = if self.rgb_mode_speed == 1.0 {
            0.5
        } else if self.rgb_mode_speed == 2.0 {
            0.3
        } else if self.rgb_mode_speed == 3.0 {
            0.1
        } else {
            1.0
        };

        let random = self.start_color == self.end_color;
        let (sc, ec) = (self.start_color, self.end_color);
        let px = generate_rain_colors(self.light_channels, elapsed, self.brightness, speed_factor, random, &sc, &ec);
        self.set_pixels(px);
    }
}
