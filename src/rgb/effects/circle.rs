use crate::rgb::{elapsed_ms, interpolate_colors, ActiveRgb};
use std::f64::consts::PI;
use std::time::Instant;

impl ActiveRgb {
    /// Circle: a glowing point colored by the start/end gradient travels around the ring.
    pub fn circle(&mut self, start: &mut Instant) {
        self.circle_frame(start);
    }

    /// CircleShift: identical to `circle` in the Go source.
    pub fn circle_shift(&mut self, start: &mut Instant) {
        self.circle_frame(start);
    }

    fn circle_frame(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let mut progress = (elapsed as f64 / (self.rgb_mode_speed * 1000.0)) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
            progress = 0.0;
        }
        let progress = progress % 1.0;

        let fade_factor = 6.0;
        let n = self.light_channels;
        let px = (0..n)
            .map(|j| {
                let pos = j as f64 / n as f64;
                let mut distance = (progress - pos).abs();
                if distance > 0.5 {
                    distance = 1.0 - distance;
                }
                let fade = (distance * PI * fade_factor).cos().max(0.0);
                let colors = interpolate_colors(&self.start_color, &self.end_color, pos, self.brightness);
                [(colors.red * fade) as u8, (colors.green * fade) as u8, (colors.blue * fade) as u8]
            })
            .collect();
        self.set_pixels(px);
    }
}
