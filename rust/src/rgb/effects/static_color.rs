use crate::rgb::{modify_brightness, ActiveRgb};

impl ActiveRgb {
    /// Static: every LED shows the start color at its brightness.
    pub fn static_color(&mut self) {
        let c = modify_brightness(self.start_color);
        self.fill(c);
    }
}
