use crate::rgb::{elapsed_ms, interpolate_color, ActiveRgb};
use std::f64::consts::PI;
use std::time::Instant;

impl ActiveRgb {
    /// Wave: sine wave of start/end color travelling along the LEDs.
    pub fn wave(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let speed = self.rgb_mode_speed;
        let progress = elapsed as f64 / (speed * 100.0);

        let px = (0..self.light_channels)
            .map(|i| {
                let wave_pos = (progress + i as f64) / speed;
                let intensity = 0.5 * (1.0 + (2.0 * PI * wave_pos).sin());
                let m = interpolate_color(&self.start_color, &self.end_color, intensity, self.brightness);
                [(m.red * intensity) as u8, (m.green * intensity) as u8, (m.blue * intensity) as u8]
            })
            .collect();
        self.set_pixels(px);
    }
}
