use crate::rgb::{elapsed_ms, interpolate_color, ActiveRgb, RgbState};
use std::time::Instant;

impl ActiveRgb {
    /// Colorshift: all LEDs fade start->end, then end->start, alternating each cycle.
    pub fn colorshift(&mut self, start: &mut Instant, state: &mut RgbState) {
        let elapsed = elapsed_ms(start);
        let speed = if self.rgb_mode_speed == 0.0 { 1.0 } else { self.rgb_mode_speed };

        let total_progress = elapsed as f64 / (speed * 1000.0);
        let progress = total_progress % 1.0;

        let cycle_count = total_progress as i64;
        state.phase = if cycle_count % 2 == 0 { 0 } else { 1 };

        let color = if state.phase == 0 {
            interpolate_color(&self.start_color, &self.end_color, progress, self.brightness)
        } else {
            interpolate_color(&self.end_color, &self.start_color, progress, self.brightness)
        };
        self.fill(color);
    }
}
