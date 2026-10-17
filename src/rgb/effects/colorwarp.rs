use crate::rgb::{elapsed_ms, generate_random_color, interpolate_color, ActiveRgb, RgbState};
use std::time::Instant;

impl ActiveRgb {
    /// Colorwarp: per channel, fade to a new random color every cycle.
    pub fn colorwarp(&mut self, start: &mut Instant, state: &mut RgbState) {
        let elapsed = elapsed_ms(start);
        let speed = if self.rgb_mode_speed == 0.0 { 1.0 } else { self.rgb_mode_speed };

        let total_progress = elapsed as f64 / (speed * 1000.0);
        let progress = total_progress % 1.0;
        let current_cycle = total_progress as i64;

        let bts = self.brightness;
        let lc = state.last_cycle.entry(self.channel_id).or_default();
        let mut start_color = *lc.start_color.get_or_insert_with(|| generate_random_color(bts));
        let mut end_color = *lc.end_color.get_or_insert_with(|| generate_random_color(bts));

        if current_cycle != lc.last_cycle {
            lc.last_cycle = current_cycle;
            start_color = end_color;
            end_color = generate_random_color(bts);
            lc.start_color = Some(start_color);
            lc.end_color = Some(end_color);
        }

        let color = interpolate_color(&start_color, &end_color, progress, bts);
        self.fill(color);
    }
}
