use super::scaled_px;
use crate::rgb::{ActiveRgb, Color};
use rand::Rng;

/// stormColorEffect: start color, with a rare flash of the end color.
fn storm_color_effect(c1: &Color, c2: &Color, bts: f64) -> [u8; 3] {
    let c = if rand::thread_rng().gen::<f32>() < 0.001 { c2 } else { c1 };
    scaled_px(c.red, c.green, c.blue, bts)
}

impl ActiveRgb {
    /// Storm: start color with random single-frame end-color flashes per LED.
    pub fn storm(&mut self) {
        let (sc, ec) = (self.start_color, self.end_color);
        let px = (0..self.light_channels).map(|_| storm_color_effect(&sc, &ec, sc.brightness)).collect();
        self.set_pixels(px);
    }
}
