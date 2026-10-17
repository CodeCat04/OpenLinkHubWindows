use crate::rgb::{elapsed_ms, interpolate_colors, ActiveRgb};
use std::time::Instant;

impl ActiveRgb {
    /// Colorpulse: all LEDs fade from start to end color every `rgb_mode_speed` seconds.
    pub fn colorpulse(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let mut progress = (elapsed as f64 / (self.rgb_mode_speed * 1000.0)) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
            progress = 0.0;
        }
        let color = interpolate_colors(&self.start_color, &self.end_color, progress, self.brightness);
        self.fill(color);
    }
}
