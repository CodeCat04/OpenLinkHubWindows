//! iCUE LINK LCD panels (AIO pump cover, XD5 Elite LCD). Port of OpenLinkHub
//! `src/devices/lcd/*.go` and the LCD parts of `src/devices/lsh/lsh.go`.
//!
//! The panel is a separate USB HID device next to the hub. Each [`LcdController`] owns one
//! panel and a render thread that pushes a 480×480 JPEG to it: once a second for data screens,
//! at the GIF's own frame delays for images and animations.
//!
//! Upstream's user-editable Arc / Double Arc / Animation profiles (`database/lcd/*.json`) and
//! the Fahrenheit dashboard option are not ported: the built-in defaults and Celsius are used.

mod media;
mod protocol;
mod render;
mod text;

#[cfg(test)]
pub use media::LcdFrame;
// Optional API (UI choices, failure polling, wire-format tests) that not every caller uses.
#[allow(unused_imports)]
pub use protocol::{packets, PRODUCT_AIO, PRODUCT_XD5, VENDOR_ID};

use hidapi::{HidApi, HidDevice};
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const MODE_LIQUID: u8 = 0;
pub const MODE_PUMP: u8 = 1;
pub const MODE_CPU: u8 = 2;
pub const MODE_GPU: u8 = 3;
pub const MODE_ALL_IN_ONE: u8 = 4;
/// Rendered, but like upstream not offered by [`modes`].
pub const MODE_LIQUID_CPU: u8 = 5;
pub const MODE_CPU_GPU_TEMP: u8 = 6;
pub const MODE_CPU_GPU_LOAD: u8 = 7;
pub const MODE_CPU_GPU_LOAD_TEMP: u8 = 8;
pub const MODE_TIME: u8 = 9;
pub const MODE_IMAGE: u8 = 10;
pub const MODE_ARC: u8 = 100;
pub const MODE_DOUBLE_ARC: u8 = 101;
pub const MODE_ANIMATION: u8 = 102;

/// Data screens refresh once a second (Go: lcdRefreshInterval).
const REFRESH: Duration = Duration::from_millis(1000);
/// Frame delay for GIF frames without one (Go: setupLCDImage sleeps 10 ms).
const MIN_FRAME_DELAY: Duration = Duration::from_millis(10);
/// Consecutive failed frames before [`LcdController::is_failed`] reports the panel gone.
const MAX_FAILURES: u32 = 5;
/// Interface 0 on Linux; Windows reports -1 for a device that isn't composite.
const MAX_INTERFACE: i32 = 0;

/// Values the screen can display, supplied by the hub every frame.
#[derive(Debug, Clone, Default)]
pub struct LcdReadings {
    /// °C, AIO coolant / pump temperature.
    pub liquid_temp: f32,
    pub pump_rpm: i16,
    /// °C, 0 = unavailable.
    pub cpu_temp: f32,
    /// °C, 0 = unavailable.
    pub gpu_temp: f32,
    /// %.
    pub cpu_load: f32,
    /// %.
    pub gpu_load: f32,
}

/// Something that can provide readings (implemented by the hub). The render thread holds it
/// until [`LcdController::stop`].
pub trait LcdSource: Send + Sync {
    fn readings(&self) -> LcdReadings;
}

impl<F: Fn() -> LcdReadings + Send + Sync> LcdSource for F {
    fn readings(&self) -> LcdReadings {
        self()
    }
}

/// A physical LCD panel found on USB.
#[derive(Debug, Clone)]
pub struct LcdInfo {
    pub path: String,
    pub serial: String,
    pub product_id: u16,
    /// AIO pump cover (one per system); otherwise an XD5 Elite LCD.
    pub aio: bool,
}

impl LcdInfo {
    /// Upstream product names (Go: loadLcdDevices).
    pub fn product(&self) -> &'static str {
        if self.aio {
            "iCUE LINK AIO LCD"
        } else {
            "iCUE LINK XD5 LCD"
        }
    }
}

/// Enumerate LCD panels (Go: checkForLcd/loadLcdDevices). Uses the api's current device
/// list, so call `HidApi::refresh_devices` first when hot-plugging.
pub fn discover(api: &HidApi) -> Vec<LcdInfo> {
    let mut found: Vec<LcdInfo> = Vec::new();
    for d in api.device_list() {
        let pid = d.product_id();
        if d.vendor_id() != VENDOR_ID || !(pid == PRODUCT_AIO || pid == PRODUCT_XD5) || d.interface_number() > MAX_INTERFACE {
            continue;
        }
        let path = d.path().to_string_lossy().into_owned();
        let serial = d.serial_number().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| path.clone());
        // Windows lists each top-level collection separately; keep one entry per panel
        // (`LcdController::start` opens all of them).
        match found.iter_mut().find(|i| i.serial == serial && i.product_id == pid) {
            Some(existing) if path < existing.path => existing.path = path,
            Some(_) => {}
            None => found.push(LcdInfo { path, serial, product_id: pid, aio: pid == PRODUCT_AIO }),
        }
    }
    found.sort_by(|a, b| (!a.aio, &a.serial).cmp(&(!b.aio, &b.serial)));
    found
}

/// Display modes for the UI, in order: (id, label). Upstream offers the same list for AIO and
/// XD5 panels (Go: lsh.go LCDModes).
pub fn modes(_aio: bool) -> Vec<(u8, &'static str)> {
    vec![
        (MODE_LIQUID, "Liquid Temperature"),
        (MODE_PUMP, "Pump Speed"),
        (MODE_CPU, "CPU Temperature"),
        (MODE_GPU, "GPU Temperature"),
        (MODE_ALL_IN_ONE, "Combined"),
        (MODE_CPU_GPU_TEMP, "CPU / GPU Temp"),
        (MODE_CPU_GPU_LOAD, "CPU / GPU Load"),
        (MODE_CPU_GPU_LOAD_TEMP, "CPU / GPU Load/Temp"),
        (MODE_TIME, "Time"),
        (MODE_IMAGE, "Image / GIF"),
        (MODE_ARC, "Arc"),
        (MODE_DOUBLE_ARC, "Double Arc"),
        (MODE_ANIMATION, "Animation"),
    ]
}

/// Rotation choices for the UI (Go: LCDRotations).
#[allow(dead_code)]
pub fn rotations() -> Vec<(u8, &'static str)> {
    vec![(0, "default"), (1, "90 degrees"), (2, "180 degrees"), (3, "270 degrees")]
}

/// Brightness choices for the UI (Go: LCDBrightnessLevels). Any 0-100 value is accepted.
#[allow(dead_code)]
pub fn brightness_levels() -> Vec<(u8, &'static str)> {
    vec![(0, "Off"), (33, "33 %"), (66, "66 %"), (100, "100 %")]
}

/// Image/GIF files available for "Image / GIF" mode: files in `config::path("lcd")`
/// (created if missing). JPEG, PNG, BMP, WebP and GIF are supported.
pub fn images() -> Vec<String> {
    media::list()
}

/// The folder [`images`] reads.
pub fn images_dir() -> std::path::PathBuf {
    media::dir()
}

fn known_mode(mode: u8) -> bool {
    mode == MODE_LIQUID_CPU || modes(true).iter().any(|(m, _)| *m == mode)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LcdSettings {
    pub mode: u8,
    /// 0-3, see [`rotations`].
    pub rotation: u8,
    /// Backlight, 0-100 %.
    pub brightness: u8,
    /// File name in [`images_dir`] for Image / GIF mode. In Animation mode a GIF here
    /// replaces the default background.
    pub image: String,
}

impl Default for LcdSettings {
    fn default() -> Self {
        LcdSettings { mode: MODE_LIQUID, rotation: 0, brightness: 100, image: String::new() }
    }
}

impl LcdSettings {
    /// Check new settings, filling in the first image for Image mode without one
    /// (Go: UpdateDeviceLcd / UpdateDeviceLcdImage).
    fn validated(mut self) -> Result<LcdSettings, String> {
        if !known_mode(self.mode) {
            return Err(format!("unknown LCD mode {}", self.mode));
        }
        if self.rotation > 3 {
            return Err(format!("invalid LCD rotation {}", self.rotation));
        }
        if self.brightness > 100 {
            return Err(format!("invalid LCD brightness {}", self.brightness));
        }
        if self.mode == MODE_IMAGE {
            if self.image.is_empty() {
                self.image = images().into_iter().next().ok_or_else(|| format!("no images in {}", images_dir().display()))?;
            } else {
                let (file, _) = media::resolve(&self.image).ok_or_else(|| format!("image \"{}\" not found", self.image))?;
                self.image = file;
            }
        }
        Ok(self)
    }

    /// Repair stored settings at startup instead of failing (Go: getLcdImages falls back to
    /// the first image, then to the liquid screen).
    fn sanitized(self) -> LcdSettings {
        let mut s = self;
        if !known_mode(s.mode) {
            s.mode = MODE_LIQUID;
        }
        s.rotation = if s.rotation > 3 { 0 } else { s.rotation };
        s.brightness = s.brightness.min(100);
        if s.mode == MODE_IMAGE && media::resolve(&s.image).is_none() {
            s.image.clear();
        }
        match s.clone().validated() {
            Ok(v) => v,
            Err(_) => LcdSettings { mode: MODE_LIQUID, image: String::new(), ..s },
        }
    }
}

#[cfg(test)]
/// Render one frame as the panel would show it: the JPEG for data screens, the first frame
/// for Image / GIF and Animation. Falls back to the liquid screen if an image can't be loaded.
pub fn render_frame(settings: &LcdSettings, readings: &LcdReadings) -> Vec<u8> {
    render_frames(settings, readings).into_iter().next().map(|f| f.jpeg).unwrap_or_default()
}

#[cfg(test)]
/// Every frame of one pass of the current mode (one for data screens).
pub fn render_frames(settings: &LcdSettings, readings: &LcdReadings) -> Vec<LcdFrame> {
    match settings.mode {
        MODE_IMAGE => match media::ImageCache::default().get(&settings.image) {
            Ok(frames) => return frames.to_vec(),
            Err(e) => log::warn!("LCD image: {e}"),
        },
        MODE_ANIMATION => {
            let gif = media::AnimationCache::default().get(&settings.image);
            let overlay = render::animation_overlay(readings);
            let mut frames = Vec::new();
            media::animation_frames(&gif, &overlay, |f| {
                frames.push(f);
                true
            });
            return frames;
        }
        _ => {}
    }
    let mode = if known_mode(settings.mode) && settings.mode != MODE_IMAGE { settings.mode } else { MODE_LIQUID };
    let jpeg = render::data_screen(mode, readings).unwrap_or_default();
    vec![LcdFrame { jpeg, delay_ms: 0 }]
}

// ---------------------------------------------------------------- device

/// Raw access to a panel: output reports (frames) and feature reports (settings). Both take
/// the report id as byte 0. Implemented for the HID panel; other implementations are for
/// tests.
pub trait LcdIo: Send {
    fn write(&mut self, report: &[u8]) -> io::Result<()>;
    fn send_feature_report(&mut self, report: &[u8]) -> io::Result<()>;
}

/// The panel's HID handles. Windows exposes each top-level collection as its own path, so
/// every collection of the panel is opened and each report goes to the first handle that
/// accepts it (with a single collection this is just one handle, as on Linux).
struct Panel {
    handles: Vec<HidDevice>,
    output: usize,
    feature: usize,
}

impl Panel {
    fn route(handles: &[HidDevice], preferred: &mut usize, f: impl Fn(&HidDevice) -> hidapi::HidResult<()>) -> io::Result<()> {
        let mut last = None;
        for k in 0..handles.len() {
            let i = (*preferred + k) % handles.len();
            match f(&handles[i]) {
                Ok(()) => {
                    *preferred = i;
                    return Ok(());
                }
                Err(e) => last = Some(e),
            }
        }
        Err(last.map_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "LCD not open"), io::Error::other))
    }

}

impl LcdIo for Panel {
    fn write(&mut self, report: &[u8]) -> io::Result<()> {
        Self::route(&self.handles, &mut self.output, |d| d.write(report).map(|_| ()))
    }

    fn send_feature_report(&mut self, report: &[u8]) -> io::Result<()> {
        Self::route(&self.handles, &mut self.feature, |d| d.send_feature_report(report))
    }
}

struct Shared {
    serial: String,
    tag: String,
    panel: Mutex<Option<Box<dyn LcdIo>>>,
    settings: RwLock<LcdSettings>,
    /// Bumped on every settings change so the render thread restarts its current pass.
    generation: AtomicU64,
    exit: AtomicBool,
    failures: AtomicU32,
}

impl Shared {
    fn running(&self) -> bool {
        !self.exit.load(Ordering::Relaxed)
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Sleep until `until` in short slices. False if stopped or the settings changed.
    fn wait_until(&self, until: Instant, generation: u64) -> bool {
        loop {
            if !self.running() || self.generation() != generation {
                return false;
            }
            let now = Instant::now();
            if now >= until {
                return true;
            }
            thread::sleep(Duration::from_millis(25).min(until - now));
        }
    }

    fn feature(&self, report: &[u8], what: &str) {
        let mut panel = self.panel.lock().unwrap();
        let Some(p) = panel.as_mut() else { return };
        if let Err(e) = p.send_feature_report(report) {
            log::warn!("[{}] Unable to {what}: {e}", self.tag);
        }
    }

    /// Go: setLcdBrightness. A brightness of 0 also switches the backlight off.
    fn apply_brightness(&self, brightness: u8) {
        self.feature(&protocol::brightness_report(brightness), "change LCD brightness");
        if brightness == 0 {
            self.feature(&protocol::REPORT_OFF, "turn off LCD backlight");
        }
    }

    /// Go: setLcdRotation.
    fn apply_rotation(&self, rotation: u8) {
        self.feature(&protocol::rotation_report(rotation), "change LCD rotation");
    }

    /// Go: transferToLcd. Returns false if the frame didn't make it.
    fn send(&self, jpeg: &[u8]) -> bool {
        if jpeg.is_empty() {
            return false;
        }
        let result = {
            let mut panel = self.panel.lock().unwrap();
            let Some(p) = panel.as_mut() else { return false };
            let mut result = Ok(());
            for packet in protocol::packets(jpeg) {
                if !self.running() {
                    break;
                }
                result = p.write(&packet);
                if result.is_err() {
                    break;
                }
            }
            result
        };
        match result {
            Ok(()) => {
                if self.failures.swap(0, Ordering::Relaxed) >= MAX_FAILURES {
                    log::info!("[{}] LCD responding again", self.tag);
                }
                true
            }
            Err(e) => {
                let n = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
                if n == 1 {
                    log::warn!("[{}] Unable to write to LCD: {e}", self.tag);
                } else if n == MAX_FAILURES {
                    log::error!("[{}] LCD stopped responding: {e}", self.tag);
                }
                false
            }
        }
    }

    /// Image can't be shown: switch to the liquid screen like upstream, unless the user
    /// changed the settings meanwhile.
    fn fall_back(&self, generation: u64, why: &str) {
        log::warn!("[{}] {why}; showing liquid temperature instead", self.tag);
        let mut s = self.settings.write().unwrap();
        if self.generation() == generation {
            s.mode = MODE_LIQUID;
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A running LCD panel.
pub struct LcdController {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl LcdController {
    /// Open the panel, apply rotation and brightness, and start the render thread (Go:
    /// setLcdRotation, setLcdBrightness, setupLCD, setupLCDImage).
    pub fn start(api: &HidApi, info: &LcdInfo, settings: LcdSettings, source: Arc<dyn LcdSource>) -> io::Result<Arc<LcdController>> {
        let mut paths: Vec<String> = api
            .device_list()
            .filter(|d| d.vendor_id() == VENDOR_ID && d.product_id() == info.product_id && d.interface_number() <= MAX_INTERFACE)
            .filter(|d| d.serial_number().is_some_and(|s| s == info.serial))
            .map(|d| d.path().to_string_lossy().into_owned())
            .filter(|p| *p != info.path)
            .collect();
        paths.sort();
        paths.insert(0, info.path.clone());

        let mut handles = Vec::new();
        let mut first_error = None;
        for path in &paths {
            let Ok(cpath) = std::ffi::CString::new(path.as_str()) else { continue };
            match api.open_path(&cpath) {
                Ok(d) => handles.push(d),
                Err(e) => {
                    first_error.get_or_insert(e);
                }
            }
        }
        if handles.is_empty() {
            let e = first_error.map_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LCD not found"), io::Error::other);
            return Err(e);
        }
        let controller = Self::start_with_io(Box::new(Panel { handles, output: 0, feature: 0 }), &info.serial, settings, source)?;
        log::info!("[{}] {} ready", controller.shared.tag, info.product());
        Ok(controller)
    }

    /// [`start`](Self::start) on an already open panel.
    #[allow(dead_code)]
    pub fn start_with_io(io: Box<dyn LcdIo>, serial: &str, settings: LcdSettings, source: Arc<dyn LcdSource>) -> io::Result<Arc<LcdController>> {
        let tag = format!("lcd …{}", serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>());
        let settings = settings.sanitized();
        let shared = Arc::new(Shared {
            serial: serial.to_string(),
            tag,
            panel: Mutex::new(Some(io)),
            settings: RwLock::new(settings.clone()),
            generation: AtomicU64::new(0),
            exit: AtomicBool::new(false),
            failures: AtomicU32::new(0),
        });
        shared.apply_rotation(settings.rotation);
        shared.apply_brightness(settings.brightness);

        let worker = Arc::clone(&shared);
        let thread = thread::Builder::new().name(format!("lcd-{serial}")).spawn(move || render_loop(&worker, source.as_ref()))?;
        Ok(Arc::new(LcdController { shared, thread: Mutex::new(Some(thread)) }))
    }

    pub fn serial(&self) -> &str {
        &self.shared.serial
    }

    /// Current settings. Can differ from what was passed in: Image mode picks the first image
    /// when none is set, and falls back to mode 0 when the image can't be shown.
    pub fn settings(&self) -> LcdSettings {
        self.shared.settings.read().unwrap().clone()
    }

    /// Several frames in a row failed (unplugged, or the handle died on resume).
    #[allow(dead_code)]
    pub fn is_failed(&self) -> bool {
        self.shared.failures.load(Ordering::Relaxed) >= MAX_FAILURES
    }

    /// Apply new settings (mode/rotation/brightness/image) live.
    pub fn update(&self, settings: LcdSettings) -> Result<(), String> {
        if !self.shared.running() {
            return Err("LCD is stopped".into());
        }
        let settings = settings.validated()?;
        let old = {
            let mut current = self.shared.settings.write().unwrap();
            let old = std::mem::replace(&mut *current, settings.clone());
            self.shared.generation.fetch_add(1, Ordering::Relaxed);
            old
        };
        if old.rotation != settings.rotation {
            self.shared.apply_rotation(settings.rotation);
        }
        if old.brightness != settings.brightness {
            self.shared.apply_brightness(settings.brightness);
        }
        Ok(())
    }

    /// Stop the thread and send upstream's shutdown feature reports, then close the panel
    /// (Go: Stop; `dirty` = StopDirty, which leaves the brightness alone). Safe to call twice.
    pub fn stop(&self, dirty: bool) {
        if self.shared.exit.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
        let Some(mut panel) = self.shared.panel.lock().unwrap().take() else { return };
        let reports: &[[u8; 4]] = if dirty { &protocol::REPORTS_STOP_DIRTY } else { &protocol::REPORTS_STOP };
        for report in reports {
            if let Err(e) = panel.send_feature_report(report) {
                log::warn!("[{}] Unable to send report to LCD: {e}", self.shared.tag);
            }
        }
        drop(panel);
        log::info!("[{}] LCD stopped", self.shared.tag);
    }
}

impl Drop for LcdController {
    fn drop(&mut self) {
        // Without stop() the thread just exits; the panel closes with the last reference.
        self.shared.exit.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------- render thread

fn render_loop(shared: &Shared, source: &dyn LcdSource) {
    let mut images = media::ImageCache::default();
    let mut animations = media::AnimationCache::default();
    while shared.running() {
        let generation = shared.generation();
        let settings = shared.settings.read().unwrap().clone();
        let tick = Instant::now();
        match settings.mode {
            MODE_IMAGE => play_image(shared, &mut images, &settings.image, generation),
            MODE_ANIMATION => {
                let gif = animations.get(&settings.image);
                let overlay = render::animation_overlay(&source.readings());
                play_animation(shared, &gif, &overlay, generation);
                shared.wait_until(tick + REFRESH, generation);
            }
            mode => {
                match render::data_screen(mode, &source.readings()) {
                    Some(jpeg) => {
                        shared.send(&jpeg);
                    }
                    None => shared.fall_back(generation, &format!("Unknown LCD mode {mode}")),
                }
                shared.wait_until(tick + REFRESH, generation);
            }
        }
    }
}

/// Go: setupLCDImage. Animated images play at their frame delays; a still image is
/// re-sent once a second (upstream re-sends it every 10 ms).
fn play_image(shared: &Shared, cache: &mut media::ImageCache, name: &str, generation: u64) {
    let frames = match cache.get(name) {
        Ok(f) => f,
        Err(e) => {
            shared.fall_back(generation, &format!("Unable to show LCD image: {e}"));
            return;
        }
    };
    if frames.len() == 1 {
        let tick = Instant::now();
        shared.send(&frames[0].jpeg);
        shared.wait_until(tick + REFRESH, generation);
        return;
    }
    for frame in frames.iter() {
        let tick = Instant::now();
        let ok = shared.send(&frame.jpeg);
        let delay = if ok { Duration::from_millis(u64::from(frame.delay_ms)).max(MIN_FRAME_DELAY) } else { REFRESH };
        if !shared.wait_until(tick + delay, generation) {
            return;
        }
    }
}

/// Go: setupLCD's DisplayAnimation case: one pass over the background with the readings
/// taken at the start of the pass, no delay after the last frame.
fn play_animation(shared: &Shared, gif: &[u8], overlay: &render::Overlay, generation: u64) {
    let mut next = Instant::now();
    media::animation_frames(gif, overlay, |frame| {
        if !shared.wait_until(next, generation) {
            return false;
        }
        if !shared.send(&frame.jpeg) {
            // Don't spin through the remaining frames of a dead panel.
            shared.wait_until(Instant::now() + REFRESH, generation);
            return false;
        }
        next = Instant::now() + Duration::from_millis(u64::from(frame.delay_ms));
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readings() -> LcdReadings {
        LcdReadings { liquid_temp: 31.4, pump_rpm: 2460, cpu_temp: 54.2, gpu_temp: 47.9, cpu_load: 23.0, gpu_load: 81.0 }
    }

    #[test]
    fn every_mode_renders_a_480px_jpeg() {
        let mut ids: Vec<u8> = modes(true).into_iter().map(|(id, _)| id).collect();
        ids.push(MODE_LIQUID_CPU);
        for mode in ids {
            let settings = LcdSettings { mode, ..LcdSettings::default() };
            let jpeg = render_frame(&settings, &readings());
            let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
                .unwrap_or_else(|e| panic!("mode {mode} did not produce a JPEG: {e}"));
            assert_eq!((img.width(), img.height()), (480, 480), "mode {mode}");
        }
    }

    #[test]
    fn frame_packets_match_upstream_framing() {
        let jpeg: Vec<u8> = (0..2100u32).map(|i| i as u8).collect();
        let packets = protocol::packets(&jpeg);
        assert_eq!(packets.len(), 3);
        assert!(packets.iter().all(|p| p.len() == protocol::BUFFER_SIZE));
        // 02 05 01 <last> <index> 00 <len u16 LE>
        assert_eq!(&packets[0][..8], &[0x02, 0x05, 0x01, 0x00, 0x00, 0x00, 0xF8, 0x03]); // 1016
        assert_eq!(&packets[2][..8], &[0x02, 0x05, 0x01, 0x01, 0x02, 0x00, 0x44, 0x00]); // 68, last
        assert_eq!(&packets[2][8..12], &jpeg[2032..2036]);
    }
}
