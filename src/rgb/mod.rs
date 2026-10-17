//! RGB engine. Port of OpenLinkHub `src/rgb`.
//!
//! Every frame, the hub builds a fresh `ActiveRgb` per device from the selected profile
//! and calls one effect method on it. The effect writes `output` (packed R,G,B bytes for
//! `light_channels` LEDs). Cross-frame state lives in `RgbState`, owned by the hub's
//! RGB loop, and the shared `start` instant (some effects reset it to restart a cycle).

pub mod effects;

use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Color {
    #[serde(default)]
    pub red: f64,
    #[serde(default)]
    pub green: f64,
    #[serde(default)]
    pub blue: f64,
    #[serde(default)]
    pub brightness: f64,
    #[serde(default)]
    pub position: f64,
    #[serde(default)]
    pub temperature: f64,
}

impl Color {
    /// True when every channel and brightness is zero (Go: `rgb.Color{} == c`).
    pub fn is_empty(&self) -> bool {
        *self == Color::default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub brightness: f64,
    #[serde(default)]
    pub smoothness: i32,
    #[serde(default, rename = "start")]
    pub start_color: Color,
    #[serde(default, rename = "middle")]
    pub middle_color: Color,
    #[serde(default, rename = "end")]
    pub end_color: Color,
    #[serde(default)]
    pub gradients: BTreeMap<i32, Color>,
    #[serde(default, rename = "minTemp")]
    pub min_temp: f64,
    #[serde(default, rename = "maxTemp")]
    pub max_temp: f64,
    #[serde(default, rename = "profileName")]
    pub profile_name: String,
}

#[derive(Debug, Clone, Default)]
pub struct LastCycle {
    pub start_color: Option<Color>,
    pub end_color: Option<Color>,
    pub last_cycle: i64,
}

/// State that persists across frames for the whole hub (Go: the `d.activeRgb` object).
#[derive(Debug, Clone)]
pub struct RgbState {
    pub start_color: Color,
    pub end_color: Color,
    pub middle_color: Color,
    pub phase: i32,
    pub last_cycle: HashMap<i32, LastCycle>,
    /// Smoothed temperature per channel for temperature effects.
    pub previous_temp: HashMap<i32, f64>,
    pub previous_color: HashMap<i32, Color>,
}

impl RgbState {
    pub fn new() -> Self {
        RgbState {
            start_color: generate_random_color(1.0),
            end_color: generate_random_color(1.0),
            middle_color: Color::default(),
            phase: 0,
            last_cycle: HashMap::new(),
            previous_temp: HashMap::new(),
            previous_color: HashMap::new(),
        }
    }
}

impl Default for RgbState {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-frame effect context (Go: `rgb.ActiveRGB` as built by `generateRgbEffect`).
#[derive(Debug, Clone)]
pub struct ActiveRgb {
    pub light_channels: usize,
    pub rgb_mode_speed: f64,
    pub start_color: Color,
    pub end_color: Color,
    pub middle_color: Color,
    pub brightness: f64,
    pub has_lcd: bool,
    pub is_aio: bool,
    pub min_temp: f64,
    pub max_temp: f64,
    pub channel_id: i32,
    /// Packed R,G,B output for this device.
    pub output: Vec<u8>,
}

impl ActiveRgb {
    pub fn new(light_channels: usize, rgb_mode_speed: f64, brightness: f64) -> Self {
        ActiveRgb {
            light_channels,
            rgb_mode_speed,
            start_color: Color::default(),
            end_color: Color::default(),
            middle_color: Color::default(),
            brightness,
            has_lcd: false,
            is_aio: false,
            min_temp: 0.0,
            max_temp: 0.0,
            channel_id: 0,
            output: Vec::new(),
        }
    }

    /// AIO pumps with an LCD cap have 4 inner LEDs (indices 16..=19) hidden under the screen.
    pub fn masks_led(&self, j: usize) -> bool {
        self.is_aio && self.has_lcd && j > 15 && j < 20
    }

    /// Fill all LEDs with one color, honouring the LCD mask, and set `output`.
    pub fn fill(&mut self, c: Color) {
        let mut buf = BTreeMap::new();
        for j in 0..self.light_channels {
            let px = if self.masks_led(j) { [0, 0, 0] } else { [c.red as u8, c.green as u8, c.blue as u8] };
            buf.insert(j, px);
        }
        self.output = set_color(&buf);
    }

    /// Set `output` from a per-LED map, honouring the LCD mask.
    pub fn set_output(&mut self, mut buf: BTreeMap<usize, [u8; 3]>) {
        for j in 0..self.light_channels {
            if self.masks_led(j) {
                buf.insert(j, [0, 0, 0]);
            }
        }
        self.output = set_color(&buf);
    }
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

pub fn clamp_f64(v: f64, min: f64, max: f64) -> f64 {
    v.max(min).min(max)
}

pub fn clamp_float01(v: f64) -> f64 {
    clamp_f64(v, 0.0, 1.0)
}

/// Go `math.Mod` semantics (result has the sign of `a`), same as Rust `%` on f64.
pub fn fmod(a: f64, b: f64) -> f64 {
    a % b
}

/// ModifyBrightness: scale channels by brightness (clamped 0..1), truncating like Go's uint8().
pub fn modify_brightness(c: Color) -> Color {
    let b = clamp_float01(c.brightness);
    Color {
        red: ((c.red * b) as u8) as f64,
        green: ((c.green * b) as u8) as f64,
        blue: ((c.blue * b) as u8) as f64,
        ..Color::default()
    }
}

/// SetColor: pack a sorted per-LED map into R,G,B bytes.
pub fn set_color(data: &BTreeMap<usize, [u8; 3]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 3);
    for px in data.values() {
        out.extend_from_slice(px);
    }
    out
}

pub fn brightness_value_float(mode: u8) -> f64 {
    mode as f64 / 100.0
}

pub fn generate_random_color(bts: f64) -> Color {
    let mut rng = rand::thread_rng();
    let c = Color {
        red: rng.gen_range(0..256) as f64,
        green: rng.gen_range(0..256) as f64,
        blue: rng.gen_range(0..256) as f64,
        brightness: bts,
        ..Color::default()
    };
    modify_brightness(c)
}

/// interpolateColor: lerp channels then apply brightness `bts`.
pub fn interpolate_color(c1: &Color, c2: &Color, t: f64, bts: f64) -> Color {
    let c = Color {
        red: lerp(c1.red, c2.red, t),
        green: lerp(c1.green, c2.green, t),
        blue: lerp(c1.blue, c2.blue, t),
        brightness: bts,
        ..Color::default()
    };
    modify_brightness(c)
}

/// interpolateColors: blend with truncation to u8 before applying brightness `bts`.
pub fn interpolate_colors(c1: &Color, c2: &Color, t: f64, bts: f64) -> Color {
    let r = (c1.red * (1.0 - t) + c2.red * t) as u8;
    let g = (c1.green * (1.0 - t) + c2.green * t) as u8;
    let b = (c1.blue * (1.0 - t) + c2.blue * t) as u8;
    modify_brightness(Color { red: r as f64, green: g as f64, blue: b as f64, brightness: bts, ..Color::default() })
}

/// interpolateColorsWithBrightness: blend pre-multiplied colors and their brightness.
pub fn interpolate_colors_with_brightness(c1: Color, c2: Color, t: f64) -> Color {
    let bts = c1.brightness * (1.0 - t) + c2.brightness * t;
    let (r1, g1, b1) = (c1.red * c1.brightness, c1.green * c1.brightness, c1.blue * c1.brightness);
    let (r2, g2, b2) = (c2.red * c2.brightness, c2.green * c2.brightness, c2.blue * c2.brightness);
    modify_brightness(Color {
        red: r1 * (1.0 - t) + r2 * t,
        green: g1 * (1.0 - t) + g2 * t,
        blue: b1 * (1.0 - t) + b2 * t,
        brightness: bts,
        ..Color::default()
    })
}

/// interpolate: fraction blend of 0..1 floats, scaled to 0..255 ints.
pub fn interpolate(r1: f64, g1: f64, b1: f64, r2: f64, g2: f64, b2: f64, fraction: f64) -> (i32, i32, i32) {
    let r = r1 + fraction * (r2 - r1);
    let g = g1 + fraction * (g2 - g1);
    let b = b1 + fraction * (b2 - b1);
    ((r * 255.0) as i32, (g * 255.0) as i32, (b * 255.0) as i32)
}

pub fn wrap_distance(a: f64, b: f64) -> f64 {
    let d = (a - b).abs();
    if d > 0.5 {
        1.0 - d
    } else {
        d
    }
}

pub fn arc_mask(position: f64, center: f64, width: f64) -> f64 {
    let d = wrap_distance(position, center);
    if d >= width {
        return 0.0;
    }
    let x = d / width;
    0.5 * (1.0 + (std::f64::consts::PI * x).cos())
}

pub fn lerp_color(c1: Color, c2: Color, t: f64) -> (i32, i32, i32) {
    let t = clamp_float01(t);
    (
        lerp(c1.red, c2.red, t).round() as i32,
        lerp(c1.green, c2.green, t).round() as i32,
        lerp(c1.blue, c2.blue, t).round() as i32,
    )
}

pub fn random01(values: &[f64]) -> f64 {
    let mut sum = 0.0;
    for (i, v) in values.iter().enumerate() {
        sum += (v * 12.9898 + i as f64 * 78.233).sin() * 43758.5453;
    }
    fmod(sum, 1.0).abs()
}

/// Milliseconds elapsed since `start` (Go: `time.Since(start).Milliseconds()`).
pub fn elapsed_ms(start: &Instant) -> i64 {
    start.elapsed().as_millis() as i64
}

/// Seconds elapsed since `start` as f64 (Go: `time.Since(start).Seconds()`).
pub fn elapsed_secs(start: &Instant) -> f64 {
    start.elapsed().as_secs_f64()
}

/// Names of the effects the hub supports, in UI order.
pub const RGB_MODES: &[&str] = &[
    "arc",
    "circle",
    "circleshift",
    "colorpulse",
    "colorshift",
    "colorwarp",
    "cpu-temperature",
    "flickering",
    "gpu-temperature",
    "gradient",
    "liquid-temperature",
    "marquee",
    "nebula",
    "off",
    "probe-temperature",
    "rain",
    "rainbow",
    "pastelrainbow",
    "rotarystack",
    "rotator",
    "sequential",
    "spinner",
    "spiralrainbow",
    "pastelspiralrainbow",
    "static",
    "storm",
    "watercolor",
    "wave",
];

/// Inputs to one effect frame that come from the hub rather than the profile.
pub struct FrameInputs<'a> {
    pub start: &'a mut Instant,
    pub state: &'a mut RgbState,
    pub profile: &'a Profile,
    /// Temperature to visualise for temperature effects (liquid/cpu/gpu/probe).
    pub temperature: f64,
}

/// Run the named effect on `r`. Unknown names produce black.
pub fn run_effect(name: &str, r: &mut ActiveRgb, inp: FrameInputs) {
    let start = inp.start;
    match name {
        "off" => r.fill(Color::default()),
        "static" => r.static_color(),
        "rainbow" => r.rainbow(*start),
        "pastelrainbow" => r.pastel_rainbow(*start),
        "spiralrainbow" => r.spiral_rainbow(*start),
        "pastelspiralrainbow" => r.pastel_spiral_rainbow(*start),
        "arc" => r.arc(*start),
        "rain" => r.rain(*start),
        "watercolor" => r.watercolor(*start),
        "gradient" => r.colorshift_gradient(*start, &inp.profile.gradients, inp.profile.speed),
        "liquid-temperature" | "cpu-temperature" | "gpu-temperature" | "probe-temperature" => {
            r.temperature(inp.temperature, inp.state)
        }
        "colorpulse" => r.colorpulse(start),
        "rotator" => r.rotator(start),
        "wave" => r.wave(start),
        "storm" => r.storm(),
        "flickering" => r.flickering(start),
        "colorshift" => r.colorshift(start, inp.state),
        "circleshift" => r.circle_shift(start),
        "circle" => r.circle(start),
        "spinner" => r.spinner(start),
        "colorwarp" => r.colorwarp(start, inp.state),
        "nebula" => r.nebula(start),
        "marquee" => r.marquee(start),
        "rotarystack" => r.rotary_stack(start),
        "sequential" => r.sequential(start),
        _ => r.fill(Color::default()),
    }
    // Safety net: an effect must always produce exactly one RGB triple per LED.
    r.output.resize(r.light_channels * 3, 0);
}
