use crate::rgb::{interpolate_color, ActiveRgb, Color, RgbState};

/// Per-frame smoothing factor for both the temperature and the displayed color.
const SMOOTHING: f64 = 0.1;

fn clamp(v: f64, min: f64, max: f64) -> f64 {
    min.max(max.min(v))
}

fn smooth_color(old: &Color, new: &Color, smoothing: f64) -> Color {
    Color {
        red: old.red + smoothing * (new.red - old.red),
        green: old.green + smoothing * (new.green - old.green),
        blue: old.blue + smoothing * (new.blue - old.blue),
        ..Color::default()
    }
}

fn map_temperature_in_range(temp: f64, min_temp: f64, max_temp: f64) -> f64 {
    if max_temp == min_temp {
        return 0.5;
    }
    let clamped = clamp(temp, min_temp.min(max_temp), min_temp.max(max_temp));
    (clamped - min_temp) / (max_temp - min_temp)
}

/// interpolateTemperatureColor: three stops (start, middle, end) sorted by their `temperature`.
fn interpolate_temperature_color(start: &Color, middle: &Color, end: &Color, current: f64, brightness: f64) -> Color {
    let (mut c1, mut c2, mut c3) = (start, middle, end);
    if c1.temperature > c2.temperature {
        std::mem::swap(&mut c1, &mut c2);
    }
    if c2.temperature > c3.temperature {
        std::mem::swap(&mut c2, &mut c3);
    }
    if c1.temperature > c2.temperature {
        std::mem::swap(&mut c1, &mut c2);
    }

    if current <= c1.temperature {
        return interpolate_color(c1, c1, 0.0, brightness);
    }
    if current <= c2.temperature {
        let t = map_temperature_in_range(current, c1.temperature, c2.temperature);
        return interpolate_color(c1, c2, t, brightness);
    }
    if current <= c3.temperature {
        let t = map_temperature_in_range(current, c2.temperature, c3.temperature);
        return interpolate_color(c2, c3, t, brightness);
    }
    interpolate_color(c3, c3, 0.0, brightness)
}

impl ActiveRgb {
    /// Temperature: color mapped from `current_temp` across start/middle/end stops, eased per channel.
    pub fn temperature(&mut self, current_temp: f64, state: &mut RgbState) {
        let ch = self.channel_id;

        let smoothed_temp = match state.previous_temp.get(&ch) {
            Some(&prev) if prev.is_finite() => prev + SMOOTHING * (current_temp - prev),
            _ => current_temp,
        };
        state.previous_temp.insert(ch, smoothed_temp);

        let target = interpolate_temperature_color(
            &self.start_color,
            &self.middle_color,
            &self.end_color,
            smoothed_temp,
            self.brightness,
        );

        let smoothed = match state.previous_color.get(&ch) {
            Some(prev) if prev.red.is_finite() && prev.green.is_finite() && prev.blue.is_finite() => {
                smooth_color(prev, &target, SMOOTHING)
            }
            _ => target,
        };
        state.previous_color.insert(ch, smoothed);

        self.fill(smoothed);
    }
}
