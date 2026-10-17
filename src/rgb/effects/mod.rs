//! Effect implementations, one file per effect (port of OpenLinkHub `src/rgb/*.go`).

mod arc;
mod circle;
mod colorpulse;
mod colorshift;
mod colorwarp;
mod flickering;
mod gradient;
mod marquee;
mod nebula;
mod pastelrainbow;
mod rain;
mod rainbow;
mod rotarystack;
mod rotator;
mod sequential;
mod spinner;
mod static_color;
mod storm;
mod temperature;
mod watercolor;
mod wave;

use super::{modify_brightness, ActiveRgb, Color};

impl ActiveRgb {
    /// Set `output` from one RGB triple per LED (index = LED), honouring the LCD mask.
    pub(crate) fn set_pixels(&mut self, px: Vec<[u8; 3]>) {
        self.set_output(px.into_iter().enumerate().collect());
    }
}

/// Go `[]byte{byte(c.Red), byte(c.Green), byte(c.Blue)}`.
pub(crate) fn to_px(c: &Color) -> [u8; 3] {
    [c.red as u8, c.green as u8, c.blue as u8]
}

/// Go `ModifyBrightness(Color{r, g, b, bts})` packed as bytes.
pub(crate) fn scaled_px(red: f64, green: f64, blue: f64, bts: f64) -> [u8; 3] {
    to_px(&modify_brightness(Color { red, green, blue, brightness: bts, ..Color::default() }))
}

/// HSBToRGB / hsvToRGB: hue in degrees, saturation and value in 0..1, result in 0..255.
pub(crate) fn hsb_to_rgb(h: f64, s: f64, v: f64) -> (i32, i32, i32) {
    let h = h % 360.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;

    let (r, g, b) = if (0.0..60.0).contains(&h) {
        (c, x, 0.0)
    } else if (60.0..120.0).contains(&h) {
        (x, c, 0.0)
    } else if (120.0..180.0).contains(&h) {
        (0.0, c, x)
    } else if (180.0..240.0).contains(&h) {
        (0.0, x, c)
    } else if (240.0..300.0).contains(&h) {
        (x, 0.0, c)
    } else if (300.0..360.0).contains(&h) {
        (c, 0.0, x)
    } else {
        (0.0, 0.0, 0.0)
    };

    (((r + m) * 255.0) as i32, ((g + m) * 255.0) as i32, ((b + m) * 255.0) as i32)
}
