use crate::rgb::{elapsed_secs, interpolate_colors_with_brightness, ActiveRgb, Color};
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::time::Instant;

impl ActiveRgb {
    /// ColorshiftGradient: all LEDs cycle through the profile's gradient stops over `duration_seconds`.
    pub fn colorshift_gradient(&mut self, start: Instant, gradients: &BTreeMap<i32, Color>, duration_seconds: f64) {
        let duration = if duration_seconds <= 0.0 { 5.0 } else { duration_seconds };
        let global_progress = (elapsed_secs(&start) / duration) % 1.0;

        let num_colors = gradients.len();
        if num_colors < 2 {
            // Go returns without touching the output; send black instead of nothing.
            self.fill(Color::default());
            return;
        }

        // Go reads keys 0..len; a missing key yields a zero color.
        let mut grad: Vec<Color> = (0..num_colors as i32).map(|i| gradients.get(&i).copied().unwrap_or_default()).collect();
        grad.sort_by(|a, b| a.position.partial_cmp(&b.position).unwrap_or(Ordering::Equal));

        let mut color_a = Color::default();
        let mut color_b = Color::default();
        let mut segment_progress = 0.0;

        for w in grad.windows(2) {
            if global_progress >= w[0].position && global_progress < w[1].position {
                color_a = w[0];
                color_b = w[1];
                segment_progress = (global_progress - color_a.position) / (color_b.position - color_a.position);
                break;
            }
        }

        // Wrap-around from the last stop to the first.
        let last = grad[grad.len() - 1];
        if global_progress >= last.position {
            color_a = last;
            color_b = grad[0];
            let segment_range = 1.0 - color_a.position + color_b.position;
            segment_progress = (global_progress - color_a.position) / segment_range;
        }

        let color = interpolate_colors_with_brightness(color_a, color_b, segment_progress);
        self.fill(color);
    }
}
