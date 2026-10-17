use super::to_px;
use crate::rgb::{elapsed_ms, interpolate_colors, set_color, ActiveRgb};
use std::time::Instant;

impl ActiveRgb {
    /// RotaryStack: a rotor fills the ring each cycle and leaves one more trail LED stacked at the end.
    pub fn rotary_stack(&mut self, start: &mut Instant) {
        let mut elapsed = elapsed_ms(start);
        let cycle = self.rgb_mode_speed * 1000.0;
        let mut progress = (elapsed as f64 / cycle) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
            elapsed = 0;
            progress = 0.0;
        }

        let led_count = self.light_channels as i64;
        let total_cycles = (elapsed as f64 / cycle) as i64;
        let trail_count = total_cycles.min(led_count);

        let free = led_count - trail_count;
        let mut rotor_pos = (progress * free as f64) as i64;
        if rotor_pos >= free {
            rotor_pos = free - 1;
        }

        let rotor = to_px(&interpolate_colors(&self.start_color, &self.start_color, 0.0, self.brightness));
        let trail = to_px(&interpolate_colors(&self.end_color, &self.end_color, 0.0, self.brightness));

        // Go writes this effect straight to output without the LCD inner-LED mask.
        let raw = (0..self.light_channels)
            .map(|i| {
                let i = i as i64;
                let px = if i >= free {
                    trail
                } else if i <= rotor_pos {
                    rotor
                } else {
                    [0, 0, 0]
                };
                (i as usize, px)
            })
            .collect();
        self.output = set_color(&raw);

        if trail_count >= led_count {
            *start = Instant::now();
        }
    }
}
