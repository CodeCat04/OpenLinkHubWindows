use super::scaled_px;
use crate::rgb::{arc_mask, elapsed_secs, lerp_color, ActiveRgb, Color};
use std::f64::consts::PI;
use std::time::Instant;

fn wrap01(x: f64) -> f64 {
    let x = x % 1.0;
    if x < 0.0 {
        x + 1.0
    } else {
        x
    }
}

/// arcColor: slowly drifting HSV hue along the travel direction.
fn arc_color(travel_pos: f64, elapsed: f64, hue_offset: f64) -> (i32, i32, i32) {
    let h = wrap01((elapsed / 40.0) + hue_offset + (travel_pos * 0.30));
    let s = 0.92;
    let v = 1.0;

    let i = (h * 6.0) as i64;
    let f = h * 6.0 - i as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));

    let (r, g, b) = match i % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r * 255.0).round() as i32, (g * 255.0).round() as i32, (b * 255.0).round() as i32)
}

/// gradientArcColor: oscillate between start and end color along the ring.
fn gradient_arc_color(start: Color, end: Color, position: f64, elapsed: f64, phase_offset: f64) -> (i32, i32, i32) {
    let t = 0.5 + 0.5 * ((2.0 * PI * elapsed / 40.0) + (2.0 * PI * (position + phase_offset))).sin();
    lerp_color(start, end, t)
}

/// arcColors: a bright arc on the outer ring (first 24 LEDs) and a second one on the inner LEDs.
fn arc_colors(light_channels: usize, elapsed: f64, bts: f64, random: bool, start: Color, end: Color) -> Vec<[u8; 3]> {
    let n = light_channels as i64;
    let (outer_count, inner_count) = if n >= 16 { (24, n - 24) } else { (n, 0) };

    let outer_center = (elapsed / 10.0) % 1.0;
    let inner_center = (0.18 + (elapsed / 10.0)) % 1.0;
    let outer_width = 0.23;
    let inner_width = 0.36;
    let ambient_floor = 0.02;
    let pulse = 1.0;

    (0..n)
        .map(|i| {
            let mut position = i as f64 / n as f64;
            let mut mask = 0.0;
            let mut hue_offset = 0.0;
            let mut phase_offset = 0.0;

            if i < outer_count {
                let outer_pos = i as f64 / outer_count as f64;
                mask = arc_mask(outer_pos, outer_center, outer_width);
                position = outer_pos;
            } else if inner_count > 0 {
                let inner_pos = (i - outer_count) as f64 / inner_count as f64;
                mask = arc_mask(inner_pos, inner_center, inner_width);
                hue_offset = 0.38;
                phase_offset = 0.18;
                position = inner_pos;
            }

            let intensity = (ambient_floor + (mask * pulse)).min(1.0);
            let (r, g, b) = if random {
                arc_color(position, elapsed, hue_offset)
            } else {
                gradient_arc_color(start, end, position, elapsed, phase_offset)
            };
            scaled_px(r as f64 * intensity, g as f64 * intensity, b as f64 * intensity, bts)
        })
        .collect()
}

impl ActiveRgb {
    /// Arc: travelling arcs of light; rainbow hues when start and end colors match, else a start/end gradient.
    pub fn arc(&mut self, start: Instant) {
        let speed_factor = if self.rgb_mode_speed > 0.0 { 10.0 / self.rgb_mode_speed } else { 10.0 };
        let elapsed = elapsed_secs(&start) * speed_factor;

        let (sc, ec) = (self.start_color, self.end_color);
        let random = sc.red == ec.red && sc.green == ec.green && sc.blue == ec.blue;

        let px = arc_colors(self.light_channels, elapsed, self.brightness, random, sc, ec);
        self.set_pixels(px);
    }
}
