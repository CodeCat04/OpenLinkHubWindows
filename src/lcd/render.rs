//! Screen rendering. Port of `GenerateScreenImage`, `GenerateArcScreenImage`,
//! `GenerateDoubleArcScreenImage` and `GenerateAnimationScreenImage` from OpenLinkHub
//! `src/devices/lcd/lcd.go`, with the built-in defaults of `arc.go`, `doublearc.go` and
//! `animation.go` (user-editable profiles are not ported).

use super::text::typeface;
use super::LcdReadings;
use std::f64::consts::PI;
use std::sync::OnceLock;

pub const WIDTH: u32 = 480;
pub const HEIGHT: u32 = 480;
const W: i32 = WIDTH as i32;
const H: i32 = HEIGHT as i32;

/// Go's `jpeg.Encode(..., nil)` default quality.
pub const QUALITY_DEFAULT: u8 = 75;
/// Animation frames (Go: `jpeg.Options{Quality: 90}`).
pub const QUALITY_ANIMATION: u8 = 90;

/// Upstream background for the data screens (`database/lcd/background.jpg`).
static BACKGROUND_JPG: &[u8] = include_bytes!("../../assets/lcd/background.jpg");

pub type Rgb = [u8; 3];
const WHITE: Rgb = [255, 255, 255];
const SEPARATOR: &str = "-------------------------------------------";

/// Built-in defaults of the custom LCD profiles (Go: InitArc / InitDoubleArc / InitAnimation).
mod style {
    use super::Rgb;
    pub const THICKNESS: f64 = 50.0;
    pub const MARGIN: f64 = 20.0;
    pub const GAP_RADIANS: f64 = 0.5;
    pub const BACKGROUND: Rgb = [24, 24, 24];
    pub const BORDER: Rgb = [64, 64, 64];
    /// Single arc: sensor 0 (CPU temperature).
    pub const ARC_START: Rgb = [0, 128, 255];
    pub const ARC_END: Rgb = [0, 255, 255];
    pub const ARC_TEXT: Rgb = [140, 220, 255];
    /// Double arc: left = sensor 0 (CPU temperature), right = sensor 2 (liquid temperature).
    pub const DOUBLE_SEPARATOR: Rgb = [160, 160, 160];
    pub const LEFT_SENSOR: usize = 0;
    pub const LEFT_START: Rgb = [0, 128, 255];
    pub const LEFT_END: Rgb = [0, 255, 255];
    pub const LEFT_TEXT: Rgb = [140, 220, 255];
    pub const RIGHT_SENSOR: usize = 2;
    pub const RIGHT_START: Rgb = [255, 128, 0];
    pub const RIGHT_END: Rgb = [255, 178, 102];
    pub const RIGHT_TEXT: Rgb = [255, 218, 84];
    /// Animation: margin and sensor rows (sensor id, text color), all enabled.
    pub const ANIMATION_MARGIN: i32 = 60;
    pub const ANIMATION_SEPARATOR: Rgb = [0, 255, 255];
    pub const ANIMATION_SENSORS: [(u8, Rgb); 3] = [(0, [255, 255, 0]), (2, [0, 255, 255]), (1, [0, 255, 0])];
}

/// Sensor ids shared by the arc / animation screens (Go: lcdSensors).
const SENSOR_PUMP: u8 = 5;
const SENSOR_DATE: u8 = 20;
const SENSOR_TIME: u8 = 21;

/// An opaque 480×480 RGB frame.
#[derive(Clone)]
pub struct Canvas {
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn solid(c: Rgb) -> Canvas {
        Canvas { px: c.iter().copied().cycle().take((WIDTH * HEIGHT * 3) as usize).collect() }
    }

    /// The upstream background image, decoded once.
    pub fn background() -> Canvas {
        static BG: OnceLock<Canvas> = OnceLock::new();
        BG.get_or_init(|| match image::load_from_memory(BACKGROUND_JPG) {
            Ok(img) => Canvas::from_image(&img, image::imageops::FilterType::CatmullRom),
            Err(e) => {
                log::error!("Unable to decode LCD background: {e}");
                Canvas::solid([0, 0, 0])
            }
        })
        .clone()
    }

    /// Stretch any image to 480×480 and flatten it onto black (Go encodes premultiplied RGBA,
    /// so transparent pixels come out black).
    pub fn from_image(img: &image::DynamicImage, filter: image::imageops::FilterType) -> Canvas {
        let mut rgba = img.to_rgba8();
        if rgba.dimensions() != (WIDTH, HEIGHT) {
            rgba = image::imageops::resize(&rgba, WIDTH, HEIGHT, filter);
        }
        Canvas::from_rgba(rgba.as_raw())
    }

    /// From 480×480 straight-alpha RGBA.
    pub fn from_rgba(rgba: &[u8]) -> Canvas {
        let mut px = Vec::with_capacity((WIDTH * HEIGHT * 3) as usize);
        for p in rgba.as_chunks::<4>().0 {
            let a = u32::from(p[3]);
            px.extend(p[..3].iter().map(|&c| ((u32::from(c) * a + 127) / 255) as u8));
        }
        px.resize((WIDTH * HEIGHT * 3) as usize, 0);
        Canvas { px }
    }

    fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if (0..W).contains(&x) && (0..H).contains(&y) {
            let i = ((y * W + x) * 3) as usize;
            self.px[i..i + 3].copy_from_slice(&c);
        }
    }

    fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if (0..W).contains(&x) && (0..H).contains(&y) {
            let i = ((y * W + x) * 3) as usize;
            for (d, &s) in self.px[i..i + 3].iter_mut().zip(&c) {
                *d = (f32::from(*d) + (f32::from(s) - f32::from(*d)) * a).round() as u8;
            }
        }
    }

    /// Go: drawString / drawColorString (baseline at `x`, `y`).
    fn text(&mut self, x: i32, y: i32, size: f32, text: &str, c: Rgb) {
        if let Some(tf) = typeface() {
            tf.draw(x, y, size, text, |px, py, a| self.blend(px, py, c, a));
        }
    }

    /// Encode as a baseline 4:2:0 JPEG, like Go's `image/jpeg`.
    pub fn jpeg(&self, quality: u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 * 1024);
        let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
        enc.set_chroma_subsampling_method(jpeg_encoder::ChromaSubsamplingMethod::Average);
        if let Err(e) = enc.encode(&self.px, WIDTH as u16, HEIGHT as u16, jpeg_encoder::ColorType::Rgb) {
            log::error!("Unable to encode LCD image: {e}");
            return Vec::new();
        }
        out
    }
}

/// Text recorded once and replayed onto many frames (animation backgrounds).
#[derive(Default)]
pub struct Overlay {
    ops: Vec<(i32, i32, Rgb, f32)>,
}

impl Overlay {
    fn text(&mut self, x: i32, y: i32, size: f32, text: &str, c: Rgb) {
        if let Some(tf) = typeface() {
            tf.draw(x, y, size, text, |px, py, a| self.ops.push((px, py, c, a)));
        }
    }

    pub fn apply(&self, canvas: &mut Canvas) {
        for &(x, y, c, a) in &self.ops {
            canvas.blend(x, y, c, a);
        }
    }
}

/// Go: calculateStringXY — centre `text` horizontally; `y` is the baseline that centres it
/// vertically, minus 10.
fn center(size: f32, text: &str) -> (i32, i32) {
    let (w, h) = typeface().map_or((0, 0), |tf| tf.bounds(size, text));
    ((W - w) / 2, (H + h) / 2 - 10)
}

/// Go: sensorMaximumValue.
fn sensor_max(sensor: u8) -> f32 {
    match sensor {
        1 => 90.0,
        2 => 60.0,
        SENSOR_PUMP => 5000.0,
        _ => 100.0,
    }
}

fn is_temperature(sensor: u8) -> bool {
    sensor <= 2
}

/// Go: Dashboard.TemperatureToString (Celsius).
fn temperature_text(celsius: f32) -> String {
    format!("{:.1} °C", (f64::from(celsius) * 100.0).round() / 100.0)
}

/// Go: common.GetDate ("02, Jan 2006") and common.GetTime ("15:04"), local time.
pub fn date_time() -> (String, String) {
    let now = chrono::Local::now();
    (now.format("%d, %b %Y").to_string(), now.format("%H:%M").to_string())
}

/// Sensor values by upstream sensor id: CPU temp, GPU temp, liquid temp, CPU load, GPU load,
/// pump RPM.
fn sensor_values(r: &LcdReadings) -> [f32; 6] {
    [r.cpu_temp, r.gpu_temp, r.liquid_temp, r.cpu_load, r.gpu_load, f32::from(r.pump_rpm)]
}

// ---------------------------------------------------------------- data screens

/// Go: GenerateScreenImage. `v..v3` are the integer values upstream passes for each mode.
pub fn screen(mode: u8, v: i32, v1: i32, v2: i32, v3: i32) -> Canvas {
    let mut img = Canvas::background();
    let mut s = |x: i32, y: i32, size: f32, text: &str| img.text(x, y, size, text, WHITE);
    // freetype Context.PointToFixed(24) >> 6 at 72 DPI.
    const P: i32 = 24;
    match mode {
        super::MODE_LIQUID | super::MODE_CPU | super::MODE_GPU => {
            let label = match mode {
                super::MODE_LIQUID => "LIQUID TEMP",
                super::MODE_CPU => "CPU TEMP",
                _ => "GPU TEMP",
            };
            let (x, y) = center(40.0, label);
            s(x, y - 120, 40.0, label);
            let (x, y) = center(40.0, "[ °C ]");
            s(x, y + 120, 40.0, "[ °C ]");
            let t = v.to_string();
            let (x, y) = center(240.0, &t);
            s(x, y, 240.0, &t);
        }
        super::MODE_PUMP => {
            let (x, y) = center(40.0, "PUMP SPEED");
            s(x, y - 120, 40.0, "PUMP SPEED");
            let (x, y) = center(40.0, "[ RPM ]");
            s(x, y + 120, 40.0, "[ RPM ]");
            let t = v.to_string();
            let (x, y) = center(200.0, &t);
            s(x, y, 200.0, &t);
        }
        super::MODE_ALL_IN_ONE => {
            let (x, y) = center(40.0, "LIQUID");
            s(x - 80, y - 110, 40.0, "LIQUID");
            let (x, y) = center(40.0, "CPU");
            s(x + 80, y - 110, 40.0, "CPU");
            let (x, y) = center(40.0, "PUMP");
            s(x, y + 130, 40.0, "PUMP");
            let (t, t1, t2) = (v.to_string(), v1.to_string(), v2.to_string());
            let (x, y) = center(100.0, &t);
            s(x - 80, y - 40, 100.0, &t);
            let (x, y) = center(100.0, &t1);
            s(x + 80, y - 40, 100.0, &t1);
            let (x, y) = center(100.0, &t2);
            s(x, y + 60, 100.0, &t2);
        }
        super::MODE_LIQUID_CPU => {
            s(120 + P, 110 + P, 40.0, "LIQUID");
            s(280 + P, 110 + P, 40.0, "CPU");
            s(190 + P, 350 + P, 40.0, "[ °C ]");
            s(95 + P, 270 + P, 160.0, &v.to_string());
            s(250 + P, 270 + P, 160.0, &v1.to_string());
        }
        super::MODE_CPU_GPU_TEMP => {
            s(120 + P, 120 + P, 40.0, "CPU");
            s(270 + P, 120 + P, 40.0, "GPU");
            s(190 + P, 350 + P, 40.0, "[ °C ]");
            s(90 + P, 270 + P, 160.0, &v.to_string());
            s(240 + P, 270 + P, 160.0, &v1.to_string());
        }
        super::MODE_CPU_GPU_LOAD_TEMP => {
            s(130 + P, 140 + P, 40.0, "CPU");
            s(270 + P, 140 + P, 40.0, "GPU");
            s(190 + P, 90 + P, 40.0, "[ °C ]");
            s(190 + P, 350 + P, 40.0, "[ % ]");
            s(120 + P, 220 + P, 80.0, &format!("{v:02}"));
            s(260 + P, 220 + P, 80.0, &format!("{v1:02}"));
            s(120 + P, 290 + P, 80.0, &format!("{v2:02}"));
            s(260 + P, 290 + P, 80.0, &format!("{v3:02}"));
        }
        super::MODE_CPU_GPU_LOAD => {
            s(120 + P, 120 + P, 40.0, "CPU");
            s(270 + P, 120 + P, 40.0, "GPU");
            s(190 + P, 350 + P, 40.0, "[ % ]");
            // Upstream measures the digits with basicfont.Face7x13 (7 px advance, 6 px glyph
            // box), and checks the CPU value for 100 on both columns.
            let width7x13 = |n: i32| 7 * n.to_string().chars().count() as i32 - 1;
            let reduce = if v == 100 { 30 } else { 0 };
            s(100 + width7x13(v) - reduce, 270 + P, 160.0, &format!("{v:02}"));
            s(240 + width7x13(v1) + 15 - reduce, 270 + P, 160.0, &format!("{v1:02}"));
        }
        super::MODE_TIME => {
            let (date, time) = date_time();
            let (x, y) = center(70.0, &date);
            s(x, y - 50, 70.0, &date);
            let (x, y) = center(130.0, &time);
            s(x, y + 50, 130.0, &time);
        }
        _ => {}
    }
    img
}

// ---------------------------------------------------------------- arcs

/// Go: drawCircle (filled, no anti-aliasing).
fn circle(img: &mut Canvas, cx: f64, cy: f64, r: f64, c: Rgb) {
    let (min_x, max_x) = ((cx - r) as i32, (cx + r) as i32);
    let (min_y, max_y) = ((cy - r) as i32, (cy + r) as i32);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
            if dx * dx + dy * dy <= r * r {
                img.set(x, y, c);
            }
        }
    }
}

/// Go: interpolateColor.
fn lerp(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let f = |i: usize| (f64::from(a[i]) * (1.0 - t) + f64::from(b[i]) * t) as u8;
    [f(0), f(1), f(2)]
}

/// Go: drawSmoothArcGradient — a chain of discs every 0.01 rad along the arc's centre line.
#[allow(clippy::too_many_arguments)]
fn arc_gradient(img: &mut Canvas, cx: f64, cy: f64, inner: f64, outer: f64, start: f64, end: f64, c0: Rgb, c1: Rgb) {
    let range = end - start;
    let radius = (inner + outer) / 2.0;
    let mut angle = start;
    while angle <= end {
        let col = lerp(c0, c1, (angle - start) / range);
        circle(img, cx + radius * angle.cos(), cy + radius * angle.sin(), (outer - inner) / 2.0, col);
        angle += 0.01;
    }
}

/// Go: drawArcOutline — 3 px discs along the arc's centre line.
#[allow(clippy::too_many_arguments)]
fn arc_outline(img: &mut Canvas, cx: f64, cy: f64, inner: f64, outer: f64, start: f64, end: f64, c: Rgb) {
    let radius = (inner + outer) / 2.0;
    let mut angle = start;
    while angle <= end {
        circle(img, cx + radius * angle.cos(), cy + radius * angle.sin(), 3.0, c);
        angle += 0.01;
    }
}

/// Go: GenerateArcScreenImage with the default arc profile (CPU temperature, max 100).
pub fn arc(value: i32) -> Canvas {
    use style::*;
    // setupLCD: sensor 0 → arcType 0, clamped to 100.
    let value = value.min(100);
    let max_value = f64::from(sensor_max(0));
    let mut img = Canvas::solid(BACKGROUND);
    let (cx, cy) = (f64::from(W) / 2.0, f64::from(H) / 2.0);
    let outer = f64::from(W) / 2.0 - MARGIN;
    let inner = outer - THICKNESS;
    let arc_start = PI / 2.0 + GAP_RADIANS / 2.0;
    let arc_end = arc_start + f64::from(value) / max_value * (PI * 2.0 - GAP_RADIANS);
    // InitArc sets the package-level startAngle to π/2 + gap; the border is a full circle.
    let border_start = PI / 2.0 + GAP_RADIANS;
    arc_outline(&mut img, cx, cy, inner, outer, border_start, border_start + 2.0 * PI, BORDER);
    if value > 0 {
        arc_gradient(&mut img, cx, cy, inner, outer, arc_start, arc_end, ARC_START, ARC_END);
    }
    // Dashboard.Temperature: "%.0f" and the unit.
    let v = format!("{:.0}", f64::from(value));
    let (x, y) = center(250.0, &v);
    img.text(x, y, 250.0, &v, ARC_TEXT);
    let (x, y) = center(40.0, "[ °C ]");
    img.text(x, y + 120, 40.0, "[ °C ]", ARC_TEXT);
    img
}

/// Go: GenerateDoubleArcScreenImage with the default profile (CPU temp left, liquid right).
pub fn double_arc(r: &LcdReadings) -> Canvas {
    use style::*;
    let values = sensor_values(r);
    let mut img = Canvas::solid(BACKGROUND);
    let outer = f64::from(W) / 2.0 - MARGIN;
    let inner = outer - THICKNESS;
    let cy = f64::from(H) / 2.0;
    let half = PI - GAP_RADIANS;

    let value_text = |sensor: usize, v: f32| {
        if is_temperature(sensor as u8) {
            temperature_text(v)
        } else {
            format!("{v:.1} %")
        }
    };

    // Left arc grows up from the bottom along the left side.
    let left_cx = MARGIN + outer;
    let left_max = sensor_max(LEFT_SENSOR as u8);
    let left = values[LEFT_SENSOR].min(left_max);
    let start = PI / 2.0 + GAP_RADIANS / 2.0;
    let end = start + f64::from(left) / f64::from(left_max) * half;
    arc_outline(&mut img, left_cx, cy, inner, outer, start, start + half, BORDER);
    arc_gradient(&mut img, left_cx, cy, inner, outer, start, end, LEFT_START, LEFT_END);
    let t = value_text(LEFT_SENSOR, left);
    let (x, y) = center(100.0, &t);
    img.text(x, y - 80, 100.0, &t, LEFT_TEXT);

    let (x, y) = center(20.0, SEPARATOR);
    img.text(x, y, 20.0, SEPARATOR, DOUBLE_SEPARATOR);

    // Right arc grows up from the bottom along the right side (colors reversed).
    let right_cx = f64::from(W) - MARGIN - outer;
    let right_max = sensor_max(RIGHT_SENSOR as u8);
    let right = values[RIGHT_SENSOR].min(right_max);
    let end = PI / 2.0 - GAP_RADIANS / 2.0;
    let start = end - f64::from(right) / f64::from(right_max) * half;
    arc_outline(&mut img, right_cx, cy, inner, outer, end - half, end, BORDER);
    arc_gradient(&mut img, right_cx, cy, inner, outer, start, end, RIGHT_END, RIGHT_START);
    let t = value_text(RIGHT_SENSOR, right);
    let (x, y) = center(100.0, &t);
    img.text(x, y + 80, 100.0, &t, RIGHT_TEXT);
    img
}

/// Go: GenerateAnimationScreenImage's per-frame text, built once per pass over the frames.
pub fn animation_overlay(r: &LcdReadings) -> Overlay {
    use style::*;
    let values = sensor_values(r);
    let sensors = &ANIMATION_SENSORS;
    let mut o = Overlay::default();
    let mut padding = -(sensors.len() as i32 * 125) / 2 + ANIMATION_MARGIN;
    let (date, time) = date_time();
    for (m, &(sensor, color)) in sensors.iter().enumerate() {
        let value = match sensor {
            SENSOR_DATE => date.clone(),
            SENSOR_TIME => time.clone(),
            _ => {
                let v = values.get(usize::from(sensor)).copied().unwrap_or(0.0).min(sensor_max(sensor));
                if is_temperature(sensor) {
                    temperature_text(v)
                } else if sensor == SENSOR_PUMP {
                    format!("{v:.0} RPM")
                } else {
                    format!("{v:.1} %")
                }
            }
        };
        let (x, y) = center(80.0, &value);
        o.text(x, y + padding, 80.0, &value, color);
        let label = sensor_label(sensor);
        let (x, y) = center(40.0, label);
        o.text(x, y + padding + 55, 40.0, label, color);
        if m != sensors.len() - 1 {
            let (x, y) = center(20.0, SEPARATOR);
            o.text(x, y + padding + 88, 20.0, SEPARATOR, ANIMATION_SEPARATOR);
        }
        padding += 125;
    }
    o
}

/// Go: lcdSensors, upper-cased (sensorTextCache).
fn sensor_label(sensor: u8) -> &'static str {
    match sensor {
        0 => "CPU TEMP",
        1 => "GPU TEMP",
        2 => "LIQUID TEMP",
        3 => "CPU LOAD",
        4 => "GPU LOAD",
        SENSOR_PUMP => "PUMP RPM",
        SENSOR_DATE => "DATE",
        SENSOR_TIME => "TIME",
        _ => "",
    }
}

/// One frame of a data screen (everything except Image / GIF and Animation), as JPEG.
/// Values are truncated to integers as upstream's `setupLCD` does.
pub fn data_screen(mode: u8, r: &LcdReadings) -> Option<Vec<u8>> {
    use super::*;
    let (liquid, cpu, gpu) = (r.liquid_temp as i32, r.cpu_temp as i32, r.gpu_temp as i32);
    let (cpu_load, gpu_load) = (r.cpu_load as i32, r.gpu_load as i32);
    let pump = i32::from(r.pump_rpm);
    let canvas = match mode {
        MODE_LIQUID => screen(mode, liquid, 0, 0, 0),
        MODE_PUMP => screen(mode, pump, 0, 0, 0),
        MODE_CPU => screen(mode, cpu, 0, 0, 0),
        MODE_GPU => screen(mode, gpu, 0, 0, 0),
        MODE_ALL_IN_ONE => screen(mode, liquid, cpu, pump, 0),
        MODE_LIQUID_CPU => screen(mode, liquid, cpu, 0, 0),
        MODE_CPU_GPU_TEMP => screen(mode, cpu, gpu, 0, 0),
        MODE_CPU_GPU_LOAD => screen(mode, cpu_load, gpu_load, 0, 0),
        MODE_CPU_GPU_LOAD_TEMP => screen(mode, cpu, gpu, cpu_load, gpu_load),
        MODE_TIME => screen(mode, 0, 0, 0, 0),
        MODE_ARC => arc(cpu),
        MODE_DOUBLE_ARC => double_arc(r),
        _ => return None,
    };
    Some(canvas.jpeg(QUALITY_DEFAULT))
}
