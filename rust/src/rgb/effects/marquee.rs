use super::to_px;
use crate::rgb::{elapsed_ms, interpolate_colors, ActiveRgb, Color};
use std::time::Instant;

/// generateMarqueeColors: chasing blocks of 2 lit / 2 dark LEDs in a start/end blend.
fn generate_marquee_colors(light_channels: usize, progress: f64, start: &Color, end: &Color, brightness: f64) -> Vec<[u8; 3]> {
    let block_size: i64 = 2;
    let gap_size: i64 = 2;
    let pattern_size = block_size + gap_size;
    let offset = (progress * pattern_size as f64) as i64;

    let shift = if progress < 0.5 { progress * 2.0 } else { (1.0 - progress) * 2.0 };
    let base = to_px(&interpolate_colors(start, end, shift, brightness));

    (0..light_channels)
        .map(|i| {
            let led_id = (i as i64).wrapping_add(offset) % pattern_size;
            if led_id < block_size {
                base
            } else {
                [0, 0, 0]
            }
        })
        .collect()
}

impl ActiveRgb {
    /// Marquee: theatre-style chasing blocks.
    pub fn marquee(&mut self, start: &mut Instant) {
        let elapsed = elapsed_ms(start);
        let mut progress = (elapsed as f64 / (self.rgb_mode_speed * 1000.0)) % 1.0;
        if progress >= 1.0 {
            *start = Instant::now();
            progress = 0.0;
        }
        let px = generate_marquee_colors(self.light_channels, progress, &self.start_color, &self.end_color, self.brightness);
        self.set_pixels(px);
    }
}
