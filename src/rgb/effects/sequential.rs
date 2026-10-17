use crate::rgb::{elapsed_ms, modify_brightness, set_color, ActiveRgb, Color};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::time::Instant;

/// GenerateRandomColorSeeded: deterministic random color for `seed`, scaled by `bts`.
/// Uses `StdRng`, so the colors differ from Go's `math/rand` stream but are equally stable per seed.
fn generate_random_color_seeded(seed: i64, bts: f64) -> Color {
    let mut rng = StdRng::seed_from_u64(seed as u64);
    modify_brightness(Color {
        red: rng.gen_range(0..256) as f64,
        green: rng.gen_range(0..256) as f64,
        blue: rng.gen_range(0..256) as f64,
        brightness: bts,
        ..Color::default()
    })
}

impl ActiveRgb {
    /// Sequential: LEDs fill one by one with the next color each cycle (random colors when start == end).
    pub fn sequential(&mut self, start: &mut Instant) {
        let random = self.start_color == self.end_color;

        let elapsed = elapsed_ms(start);
        let mut cycle_duration = self.rgb_mode_speed * 1000.0;
        if cycle_duration <= 0.0 {
            cycle_duration = 1000.0;
        }

        let total_cycles = (elapsed as f64 / cycle_duration) as i64;
        let progress = (elapsed as f64 / cycle_duration) % 1.0;

        let led_count = self.light_channels as i64;
        if led_count <= 0 {
            self.output = Vec::new();
            return;
        }

        let step = ((progress * led_count as f64) as i64).min(led_count - 1);

        let (current, prev) = if random {
            let curr = generate_random_color_seeded(total_cycles, self.brightness);
            let prev = if total_cycles > 0 {
                generate_random_color_seeded(total_cycles - 1, self.brightness)
            } else {
                Color::default()
            };
            (curr, prev)
        } else if total_cycles % 2 == 0 {
            (self.start_color, self.end_color)
        } else {
            (self.end_color, self.start_color)
        };

        let current = [current.red as u8, current.green as u8, current.blue as u8];
        let prev = [prev.red as u8, prev.green as u8, prev.blue as u8];

        // Go writes this effect straight to output without the LCD inner-LED mask.
        let raw = (0..self.light_channels)
            .map(|i| {
                let px = if i as i64 <= step {
                    current
                } else if total_cycles > 0 || !random {
                    prev
                } else {
                    [0, 0, 0]
                };
                (i, px)
            })
            .collect();
        self.output = set_color(&raw);
    }
}
