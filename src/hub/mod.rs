//! iCUE LINK System Hub driver. Port of OpenLinkHub `src/devices/lsh/lsh.go`.
//!
//! Each hub runs three worker threads:
//! - poll  (1 s):  fan RPM, temperatures, PSU rails; AIO over-temperature protection
//! - speed (3 s):  evaluates every channel's fan/pump curve and pushes duty changes
//! - rgb   (20 ms): renders effects for every LED on the chain and streams the frame

pub mod adapters;
pub mod manager;
pub mod metadata;
pub mod transport;

use crate::config;
use crate::lcd::{LcdController, LcdInfo, LcdReadings, LcdSettings};
use crate::rgb::{self, ActiveRgb, Color, FrameInputs, RgbState};
use crate::rgb_profiles;
use crate::sensors;
use crate::temperatures::{self, TemperatureProfile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use transport::*;

pub const VENDOR_ID: u16 = 0x1b1c;
pub const PRODUCT_ID: u16 = 0x0c3f;

const DEFAULT_SPEED: u8 = 70;
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
const SPEED_INTERVAL: Duration = Duration::from_millis(3000);
const FRAME_INTERVAL: Duration = Duration::from_millis(20);
const CRITICAL_AIO_COOLANT_TEMP: f32 = 57.0;
const LCD_LED_CHANNELS: u8 = 24;
/// Hub port LED budgets; above these, brightness is reduced to protect the port (Go: setDeviceProtection).
const PORT_PROTECTION: [(u32, f64); 3] = [(238, 0.66), (340, 0.33), (442, 0.1)];
/// Consecutive I/O failures before the hub is considered gone (unplugged, sleep/resume).
const MAX_IO_FAILURES: u32 = 5;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RgbOverride {
    pub enabled: bool,
    pub start: Color,
    pub middle: Color,
    pub end: Color,
    pub speed: f64,
}

/// Per-hub user settings, persisted to `%APPDATA%\OpenLinkHub\hubs\<serial>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubProfile {
    pub speed_profiles: BTreeMap<u8, String>,
    pub rgb_profiles: BTreeMap<u8, String>,
    pub labels: BTreeMap<u8, String>,
    pub rgb_overrides: BTreeMap<u8, RgbOverride>,
    /// LINK adapter channel → configuration index in `adapters::all()`.
    pub adapters: BTreeMap<u8, u32>,
    /// LINK adapter channel → part index → RGB profile.
    pub sub_rgb_profiles: BTreeMap<u8, BTreeMap<u8, String>>,
    /// Pump channel → LCD screen settings.
    pub lcd: BTreeMap<u8, LcdSettings>,
    /// Pump channel → LCD panel serial (only needed with several XD5/XC7 panels).
    pub lcd_devices: BTreeMap<u8, String>,
    /// LED brightness, 0-100 %.
    pub brightness: u8,
    pub rgb_off: bool,
}

impl Default for HubProfile {
    fn default() -> Self {
        HubProfile {
            speed_profiles: BTreeMap::new(),
            rgb_profiles: BTreeMap::new(),
            labels: BTreeMap::new(),
            rgb_overrides: BTreeMap::new(),
            adapters: BTreeMap::new(),
            sub_rgb_profiles: BTreeMap::new(),
            lcd: BTreeMap::new(),
            lcd_devices: BTreeMap::new(),
            brightness: 100,
            rgb_off: false,
        }
    }
}

/// The lighting part of a hub's settings, saved with each profile (see `crate::profiles`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lighting {
    pub rgb_profiles: BTreeMap<u8, String>,
    pub rgb_overrides: BTreeMap<u8, RgbOverride>,
    pub sub_rgb_profiles: BTreeMap<u8, BTreeMap<u8, String>>,
    pub brightness: u8,
    pub rgb_off: bool,
}

impl Lighting {
    fn capture(p: &HubProfile) -> Lighting {
        Lighting {
            rgb_profiles: p.rgb_profiles.clone(),
            rgb_overrides: p.rgb_overrides.clone(),
            sub_rgb_profiles: p.sub_rgb_profiles.clone(),
            brightness: p.brightness,
            rgb_off: p.rgb_off,
        }
    }

    /// Replace every device's lighting. Channels missing from `self` go back to the defaults.
    fn apply(&self, devices: &mut BTreeMap<u8, LinkDevice>, profile: &mut HubProfile) {
        for d in devices.values_mut() {
            d.rgb_profile = saved_rgb_profile(self.rgb_profiles.get(&d.channel_id));
            d.rgb_override = self.rgb_overrides.get(&d.channel_id).cloned().unwrap_or_default();
            let subs = self.sub_rgb_profiles.get(&d.channel_id);
            for part in &mut d.sub_devices {
                part.rgb_profile = saved_rgb_profile(subs.and_then(|m| m.get(&part.index)));
            }
        }
        profile.rgb_profiles = self.rgb_profiles.clone();
        profile.rgb_overrides = self.rgb_overrides.clone();
        profile.sub_rgb_profiles = self.sub_rgb_profiles.clone();
        profile.brightness = self.brightness.min(100);
        profile.rgb_off = self.rgb_off;
    }
}

/// An LCD panel as shown in the dashboard.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LcdView {
    pub product: String,
    pub settings: LcdSettings,
}

/// A strip or fan behind a LINK adapter.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubDevice {
    pub index: u8,
    pub name: String,
    pub led_channels: u8,
    pub rgb_profile: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkDevice {
    pub channel_id: u8,
    pub type_id: u8,
    pub model: u8,
    pub device_id: String,
    pub name: String,
    pub description: String,
    pub label: String,
    pub led_channels: u8,
    pub contains_pump: bool,
    pub aio: bool,
    pub has_speed: bool,
    pub has_temps: bool,
    pub is_probe: bool,
    pub is_link_adapter: bool,
    pub is_commander_duo: bool,
    pub is_psu: bool,
    pub is_vrm_cooler: bool,
    #[serde(skip)]
    pub titan_aio: bool,
    #[serde(skip)]
    pub port_id: u8,
    /// Hub "device command code" (needed when rewriting the table for LINK adapters).
    #[serde(skip)]
    pub device_code: u8,
    /// LINK adapter configuration index (0 = nothing connected).
    pub adapter: u32,
    pub sub_devices: Vec<SubDevice>,
    /// LCD panel attached to this pump, if any.
    pub lcd: Option<LcdView>,
    pub rpm: i16,
    pub temperature: f32,
    pub speed_profile: String,
    pub rgb_profile: String,
    pub rgb_override: RgbOverride,
    /// LEDs temporarily turned off from the fan speeds list, to find the fan in the case.
    /// Not saved: lighting comes back on restart.
    pub light_off: bool,
    /// Last duty sent to the device, %.
    pub duty: u8,
    pub volts: Vec<f64>,
    pub amps: Vec<f64>,
    pub watts: Vec<f64>,
    pub power_out: f64,
}

impl LinkDevice {
    /// Fans follow the global fan profile switcher; pumps and the PSU keep their own curve.
    pub fn follows_fan_profile(&self) -> bool {
        self.has_speed && !self.contains_pump && !self.is_psu
    }
}

struct HubState {
    devices: BTreeMap<u8, LinkDevice>,
    profile: HubProfile,
    has_lcd: bool,
    /// Port-protection brightness cap (0 = no cap).
    global_brightness: f64,
    critical: bool,
    has_psu: bool,
    led_tables: adapters::LedTables,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubView {
    pub serial: String,
    pub product: String,
    pub firmware: String,
    pub brightness: u8,
    pub rgb_off: bool,
    pub brightness_cap: f64,
    pub critical: bool,
    pub devices: Vec<LinkDevice>,
}

pub struct Hub {
    pub serial: String,
    /// Last 4 characters of the serial, used in logs so they can be shared safely.
    pub tag: String,
    pub product: String,
    pub firmware: String,
    io: Mutex<Transport>,
    state: RwLock<HubState>,
    exit: AtomicBool,
    failed: AtomicBool,
    io_failures: AtomicU64,
    rgb_generation: AtomicU64,
    threads: Mutex<Vec<JoinHandle<()>>>,
    /// Pump channel → (panel, product name).
    lcds: Mutex<BTreeMap<u8, (Arc<LcdController>, &'static str)>>,
}

impl Hub {
    /// Open the hub, switch it to software mode, discover the LINK chain and start workers.
    pub fn start(dev: hidapi::HidDevice, serial: String) -> io::Result<Arc<Hub>> {
        let tag = short_serial(&serial);
        let t = Transport::new(dev, config::get().debug);
        let product = t.product().replace("CORSAIR ", "");

        let fw = t.transfer(CMD_GET_FIRMWARE, &[], 0)?;
        let firmware_major = fw[4];
        let firmware = format!("{}.{}.{}", fw[4], fw[5], u16::from_le_bytes([fw[6], fw[7]]));

        t.transfer(CMD_SOFTWARE_MODE, &[], 0)?;
        thread::sleep(Duration::from_millis(500));

        // Device command codes and LED counts, needed to configure LINK adapters.
        let codes = t.read_device_data(CMD_DEVICE_COMMAND_CODES)?;
        let leds = t.read_device_data(CMD_DEVICE_COMMAND_LEDS)?;
        let led_tables = adapters::LedTables::parse(&codes, &leds);

        let profile: HubProfile = config::load_json(&profile_path(&serial)).unwrap_or_default();
        let (devices, has_lcd) = discover_devices(&t, &profile, firmware_major, &led_tables)?;
        let mut devices = devices;
        apply_commander_duo_leds(&t, &mut devices);
        t.open_color_endpoint()?;

        let has_psu = devices.values().any(|d| d.is_psu);
        let mut state = HubState { devices, profile, has_lcd, global_brightness: 0.0, critical: false, has_psu, led_tables };
        state.global_brightness = port_protection(&state.devices);
        if state.global_brightness != 0.0 {
            log::warn!(
                "[{tag}] LED brightness capped at {:.0}% due to port power draw",
                state.global_brightness * 100.0
            );
        }

        let hub = Arc::new(Hub {
            serial: serial.clone(),
            tag: tag.clone(),
            product,
            firmware,
            io: Mutex::new(t),
            state: RwLock::new(state),
            exit: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            io_failures: AtomicU64::new(0),
            rgb_generation: AtomicU64::new(1),
            threads: Mutex::new(Vec::new()),
            lcds: Mutex::new(BTreeMap::new()),
        });

        // Defaults: every channel to 70 % until the first curve evaluation.
        let defaults: BTreeMap<u8, u8> = hub.state.read().unwrap().devices.keys().map(|&k| (k, DEFAULT_SPEED)).collect();
        hub.set_speed(&defaults);
        hub.save_profile();

        let mut threads = Vec::new();
        threads.push(hub.spawn("poll", Hub::poll_loop));
        if config::get().manual {
            log::info!("[{tag}] Manual mode: fan curves are not evaluated");
        } else {
            threads.push(hub.spawn("speed", Hub::speed_loop));
        }
        threads.push(hub.spawn("rgb", Hub::rgb_loop));
        *hub.threads.lock().unwrap() = threads;

        let s = hub.state.read().unwrap();
        log::info!(
            "[{}] {} firmware {} ready with {} device(s): {}",
            hub.tag,
            hub.product,
            hub.firmware,
            s.devices.len(),
            s.devices.values().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", ")
        );
        drop(s);
        Ok(hub)
    }

    fn spawn(self: &Arc<Self>, name: &str, f: fn(&Hub)) -> JoinHandle<()> {
        let hub = Arc::clone(self);
        thread::Builder::new()
            .name(format!("{name}-{}", self.serial))
            .spawn(move || f(&hub))
            .expect("spawn hub worker")
    }

    pub fn is_failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    fn running(&self) -> bool {
        !self.exit.load(Ordering::Relaxed) && !self.is_failed()
    }

    /// Sleep in short slices so shutdown is never blocked on a long interval.
    fn sleep(&self, d: Duration) {
        let until = Instant::now() + d;
        while self.running() && Instant::now() < until {
            thread::sleep(Duration::from_millis(25).min(until - Instant::now()));
        }
    }

    /// Run an I/O sequence under the device lock, tracking consecutive failures.
    fn with_io<T>(&self, what: &str, f: impl FnOnce(&Transport) -> io::Result<T>) -> Option<T> {
        if !self.running() {
            return None;
        }
        let t = self.io.lock().unwrap();
        match f(&t) {
            Ok(v) => {
                self.io_failures.store(0, Ordering::Relaxed);
                Some(v)
            }
            Err(e) => {
                let n = self.io_failures.fetch_add(1, Ordering::Relaxed) + 1;
                log::warn!("[{}] {what} failed ({n}/{MAX_IO_FAILURES}): {e}", self.tag);
                if n >= MAX_IO_FAILURES as u64 {
                    log::error!("[{}] Hub stopped responding; it will be re-initialised when it reappears", self.tag);
                    self.failed.store(true, Ordering::Relaxed);
                }
                None
            }
        }
    }

    /// Stop workers and hand the hub back to hardware (firmware) control.
    pub fn stop(&self) {
        if self.exit.swap(true, Ordering::SeqCst) {
            return;
        }
        log::info!("[{}] Stopping...", self.tag);
        for t in self.threads.lock().unwrap().drain(..) {
            let _ = t.join();
        }
        for (lcd, _) in std::mem::take(&mut *self.lcds.lock().unwrap()).into_values() {
            lcd.stop(self.is_failed());
        }
        if !self.is_failed() {
            let t = self.io.lock().unwrap();
            if let Err(e) = t.send(CMD_HARDWARE_MODE) {
                log::warn!("[{}] Unable to switch to hardware mode: {e}", self.tag);
            }
        }
        log::info!("[{}] Stopped", self.tag);
    }

    // ---------------------------------------------------------------- polling

    fn poll_loop(&self) {
        while self.running() {
            self.poll_data();
            self.protect_liquid_cooler();
            self.sleep(POLL_INTERVAL);
        }
    }

    fn poll_data(&self) {
        let has_psu = self.state.read().unwrap().has_psu;
        let Some((speeds, temps, rails)) = self.with_io("poll", |t| {
            let speeds = t.read(MODE_GET_SPEEDS, DATA_TYPE_GET_SPEEDS, 0)?;
            let temps = t.read(MODE_GET_TEMPERATURES, DATA_TYPE_GET_TEMPERATURES, 0)?;
            let rails = if has_psu {
                Some((t.read(MODE_GET_PSU_VOLTS, &[], PSU_HEADER)?, t.read(MODE_GET_PSU_AMPS, &[], PSU_HEADER)?))
            } else {
                None
            };
            Ok((speeds, temps, rails))
        }) else {
            return;
        };

        let mut s = self.state.write().unwrap();

        // Speeds: [6]=sensor count, then 3 bytes per channel (status, rpm LE). Index 0 is the hub.
        // Upstream only trusts the packet when the hub slot reports "not connected" (0x01).
        if speeds.get(7) == Some(&0x01) {
            let amount = speeds[6] as usize;
            for i in 0..amount {
                let Some(sensor) = speeds.get(7 + i * 3..7 + i * 3 + 3) else { break };
                if sensor[0] != 0x00 {
                    continue;
                }
                if let Some(d) = s.devices.get_mut(&(i as u8)) {
                    let rpm = i16::from_le_bytes([sensor[1], sensor[2]]);
                    if rpm > 1 || d.is_psu {
                        d.rpm = rpm.max(0);
                    }
                }
            }
        }

        // Temperatures: same layout, value in tenths of °C.
        if temps.get(3) == Some(&0x00) && matches!(temps.get(7), Some(0x00 | 0x01)) {
            let amount = temps[6] as usize;
            for i in 0..amount {
                let Some(sensor) = temps.get(7 + i * 3..7 + i * 3 + 3) else { break };
                if sensor[0] != 0x00 {
                    continue;
                }
                if let Some(d) = s.devices.get_mut(&(i as u8)) {
                    let temp = i16::from_le_bytes([sensor[1], sensor[2]]) as f32 / 10.0;
                    if temp > 1.0 {
                        d.temperature = temp;
                    }
                }
            }
        }

        if let Some((volts, amps)) = rails {
            if let Some(psu) = s.devices.values_mut().find(|d| d.is_psu) {
                psu.volts = parse_rails(&volts, 1000.0);
                psu.amps = parse_rails(&amps, 100.0);
                psu.watts = psu.volts.iter().zip(&psu.amps).map(|(v, a)| round2(v * a)).collect();
                psu.power_out = round2(psu.watts.iter().sum());
            }
        }
    }

    /// Run fans and pump at 100 % and pulse red while AIO coolant is above 57 °C;
    /// restore once it drops below 52 °C (Go: protectLiquidCooler).
    fn protect_liquid_cooler(&self) {
        let mut s = self.state.write().unwrap();
        let Some(liquid) = s.devices.values().find(|d| d.aio).map(|d| d.temperature) else { return };
        let was = s.critical;
        if liquid > CRITICAL_AIO_COOLANT_TEMP {
            s.critical = true;
        } else if liquid < CRITICAL_AIO_COOLANT_TEMP - 5.0 {
            s.critical = false;
        }
        if s.critical != was {
            if s.critical {
                log::error!("[{}] AIO coolant at {liquid:.1} °C, forcing 100% cooling", self.tag);
            } else {
                log::info!("[{}] AIO coolant back to {liquid:.1} °C, restoring profiles", self.tag);
            }
            self.rgb_generation.fetch_add(1, Ordering::Relaxed);
        }
    }

    // ---------------------------------------------------------------- speed

    fn speed_loop(&self) {
        let mut last: BTreeMap<u8, u8> = BTreeMap::new();
        while self.running() {
            let wanted = self.compute_duties();
            if !wanted.is_empty() && wanted != last {
                if config::get().debug {
                    log::debug!("[{}] duties {wanted:?}", self.tag);
                }
                if self.set_speed(&wanted) {
                    last = wanted;
                }
            }
            self.sleep(SPEED_INTERVAL);
        }
    }

    fn compute_duties(&self) -> BTreeMap<u8, u8> {
        let s = self.state.read().unwrap();
        let liquid = liquid_temperature(&s.devices);
        let psu = s.devices.values().find(|d| d.is_psu).map_or(0.0, |d| d.temperature);
        let mut out = BTreeMap::new();
        for d in s.devices.values() {
            let name = if s.critical { temperatures::CRITICAL_PROFILE } else { d.speed_profile.as_str() };
            let profile = temperatures::get(name).or_else(|| temperatures::get("Normal")).expect("Normal profile");
            let temp = sensor_temperature(&profile, &self.serial, &s.devices, liquid, psu);
            out.insert(d.channel_id, duty_for(d, &profile, temp));
        }
        out
    }

    /// Send duties for every channel (Go: setSpeed). Retries while the hub reports busy.
    fn set_speed(&self, duties: &BTreeMap<u8, u8>) -> bool {
        let mut buffer = vec![0u8; duties.len() * 4 + 1];
        buffer[0] = duties.len() as u8;
        for (i, (&ch, &duty)) in duties.iter().enumerate() {
            buffer[1 + i * 4] = ch;
            buffer[1 + i * 4 + 1] = 0; // mode: percent
            buffer[1 + i * 4 + 2] = duty;
        }
        for attempt in 0..=20 {
            let Some(reply) = self.with_io("set speed", |t| t.write(MODE_SET_SPEED, DATA_TYPE_SET_SPEED, &buffer)) else {
                return false;
            };
            if reply[3] == 0x00 {
                let mut s = self.state.write().unwrap();
                for (ch, duty) in duties {
                    if let Some(d) = s.devices.get_mut(ch) {
                        d.duty = *duty;
                    }
                }
                return true;
            }
            if attempt < 20 {
                thread::sleep(Duration::from_millis(100));
            }
        }
        log::warn!("[{}] Hub rejected speed update", self.tag);
        false
    }

    // ---------------------------------------------------------------- rgb

    fn rgb_loop(&self) {
        let mut generation = 0;
        let mut start = Instant::now();
        let mut state = RgbState::new();
        let mut last_frame: Option<Vec<u8>> = None;

        while self.running() {
            let current = self.rgb_generation.load(Ordering::Relaxed);
            if current != generation {
                generation = current;
                // Upstream sends an all-black frame and waits 40 ms before a new effect,
                // otherwise QX fans mixed with RX fans sometimes stay dark.
                let leds = self.total_leds();
                if leds > 0 {
                    self.with_io("reset color", |t| t.write_color(&vec![0u8; leds * 3]));
                    thread::sleep(Duration::from_millis(40));
                }
                start = Instant::now();
                state = RgbState::new();
                last_frame = None;
            }

            if let Some(frame) = self.render_frame(&mut start, &mut state) {
                if last_frame.as_ref() != Some(&frame) && self.with_io("write color", |t| t.write_color(&frame)).is_some() {
                    last_frame = Some(frame);
                }
            }
            self.sleep(FRAME_INTERVAL);
        }
    }

    fn total_leds(&self) -> usize {
        self.state.read().unwrap().devices.values().map(|d| d.led_channels as usize).sum()
    }

    /// Build one frame for every LED on the chain, in channel order. LINK adapters
    /// contribute one segment per connected strip or fan, each with its own effect.
    fn render_frame(&self, start: &mut Instant, state: &mut RgbState) -> Option<Vec<u8>> {
        let s = self.state.read().unwrap();
        let total: usize = s.devices.values().map(|d| d.led_channels as usize).sum();
        if total == 0 {
            return None;
        }
        if s.profile.rgb_off {
            return Some(vec![0u8; total * 3]);
        }

        let ctx = FrameContext {
            brightness: if s.global_brightness != 0.0 {
                s.global_brightness
            } else {
                rgb::brightness_value_float(s.profile.brightness)
            },
            liquid: liquid_temperature(&s.devices) as f64,
            probe: s.devices.values().find(|d| d.is_probe && d.temperature > 0.0).map_or(0.0, |d| d.temperature) as f64,
            has_lcd: s.has_lcd,
            critical: s.critical,
        };

        let mut frame = Vec::with_capacity(total * 3);
        for d in s.devices.values().filter(|d| d.led_channels > 0) {
            if d.light_off {
                frame.resize(frame.len() + d.led_channels as usize * 3, 0);
            } else if d.is_link_adapter {
                for part in &d.sub_devices {
                    // Upstream keys adapter parts' effect state as channel 48 + part index.
                    let seg = Segment { leds: part.led_channels, channel_id: 48 + part.index as i32, aio: false, probe_temp: None, rgb_override: None };
                    frame.extend_from_slice(&render_segment(&ctx, &part.rgb_profile, &seg, start, state));
                }
            } else {
                let seg = Segment {
                    leds: d.led_channels,
                    channel_id: d.channel_id as i32,
                    aio: d.aio,
                    probe_temp: d.is_probe.then_some(d.temperature as f64),
                    rgb_override: d.rgb_override.enabled.then_some(&d.rgb_override),
                };
                frame.extend_from_slice(&render_segment(&ctx, &d.rgb_profile, &seg, start, state));
            }
        }
        // Keep the frame exactly as long as the hub expects even if a part list is short.
        frame.resize(total * 3, 0);
        Some(frame)
    }

    // ---------------------------------------------------------------- API

    pub fn view(&self) -> HubView {
        let lcds: BTreeMap<u8, LcdView> = self
            .lcds
            .lock()
            .unwrap()
            .iter()
            .map(|(&ch, (c, product))| (ch, LcdView { product: product.to_string(), settings: c.settings() }))
            .collect();
        let s = self.state.read().unwrap();
        HubView {
            serial: self.serial.clone(),
            product: self.product.clone(),
            firmware: self.firmware.clone(),
            brightness: s.profile.brightness,
            rgb_off: s.profile.rgb_off,
            brightness_cap: s.global_brightness,
            critical: s.critical,
            devices: s
                .devices
                .values()
                .cloned()
                .map(|mut d| {
                    d.lcd = lcds.get(&d.channel_id).cloned();
                    d
                })
                .collect(),
        }
    }

    // ---------------------------------------------------------------- LCD

    /// The pump channel a newly found LCD panel belongs to, if any (Go: `getDevices`).
    /// AIO panels go to the AIO pump; XD5/XC7 panels to a non-AIO pump, preferring the
    /// channel the user assigned this panel to.
    pub fn lcd_channel_for(&self, info: &LcdInfo) -> Option<u8> {
        let s = self.state.read().unwrap();
        if !s.has_lcd {
            return None;
        }
        let taken = self.lcds.lock().unwrap();
        let free: Vec<&LinkDevice> =
            s.devices.values().filter(|d| d.contains_pump && d.aio == info.aio && !taken.contains_key(&d.channel_id)).collect();
        if let Some(d) = free.iter().find(|d| s.profile.lcd_devices.get(&d.channel_id) == Some(&info.serial)) {
            return Some(d.channel_id);
        }
        free.iter().find(|d| !s.profile.lcd_devices.contains_key(&d.channel_id)).map(|d| d.channel_id)
    }

    pub fn has_lcd_panel(&self, serial: &str) -> bool {
        self.lcds.lock().unwrap().values().any(|(c, _)| c.serial() == serial)
    }

    /// Start driving `info` as the screen of pump `channel`.
    pub fn attach_lcd(self: &Arc<Self>, api: &hidapi::HidApi, info: &LcdInfo, channel: u8) {
        let settings = self.state.read().unwrap().profile.lcd.get(&channel).cloned().unwrap_or_default();
        let weak: Weak<Hub> = Arc::downgrade(self);
        let source = Arc::new(move || weak.upgrade().map(|h| h.lcd_readings(channel)).unwrap_or_default());
        match LcdController::start(api, info, settings, source) {
            Ok(c) => {
                log::info!("[{}] {} attached to channel {channel}", self.tag, info.product());
                self.lcds.lock().unwrap().insert(channel, (c, info.product()));
                self.state.write().unwrap().profile.lcd_devices.insert(channel, info.serial.clone());
                self.save_profile();
            }
            Err(e) => log::warn!("[{}] Unable to open {}: {e}", self.tag, info.product()),
        }
    }

    /// Drop panels that stopped responding or were unplugged so they can be re-attached.
    pub fn prune_lcds(&self, present: &[LcdInfo]) {
        let mut lcds = self.lcds.lock().unwrap();
        let gone: Vec<u8> = lcds
            .iter()
            .filter(|(_, (c, _))| c.is_failed() || !present.iter().any(|p| p.serial == c.serial()))
            .map(|(&ch, _)| ch)
            .collect();
        for ch in gone {
            if let Some((c, product)) = lcds.remove(&ch) {
                log::info!("[{}] {product} on channel {ch} disconnected", self.tag);
                c.stop(true);
            }
        }
    }

    fn lcd_readings(&self, channel: u8) -> LcdReadings {
        let snap = sensors::snapshot();
        let s = self.state.read().unwrap();
        let (liquid, pump) = match s.devices.get(&channel) {
            Some(d) if d.contains_pump => (d.temperature, d.rpm),
            _ => (liquid_temperature(&s.devices), 0),
        };
        LcdReadings { liquid_temp: liquid, pump_rpm: pump, cpu_temp: snap.cpu, gpu_temp: snap.gpu, cpu_load: snap.cpu_load, gpu_load: snap.gpu_load }
    }

    pub fn set_lcd(&self, channel: u8, settings: LcdSettings) -> Result<(), String> {
        let controller = self.lcds.lock().unwrap().get(&channel).map(|(c, _)| c.clone()).ok_or("No LCD on that channel")?;
        controller.update(settings)?;
        // Re-read: the panel may adjust settings (e.g. pick the first image).
        self.state.write().unwrap().profile.lcd.insert(channel, controller.settings());
        self.save_profile();
        Ok(())
    }

    pub fn device_temperature(&self, channel: u8) -> f32 {
        self.state.read().unwrap().devices.get(&channel).map_or(0.0, |d| d.temperature)
    }

    pub fn set_speed_profile(&self, channels: &[u8], profile: &str) -> Result<(), String> {
        if temperatures::get(profile).is_none() {
            return Err(format!("Unknown speed profile '{profile}'"));
        }
        self.update_channels(channels, |p, d| {
            if d.is_psu || !d.has_speed && !d.contains_pump {
                return false;
            }
            d.speed_profile = profile.to_string();
            p.speed_profiles.insert(d.channel_id, profile.to_string());
            true
        })
    }

    pub fn set_rgb_profile(&self, channels: &[u8], profile: &str) -> Result<(), String> {
        if rgb_profiles::get(profile).is_none() {
            return Err(format!("Unknown RGB profile '{profile}'"));
        }
        self.update_channels(channels, |p, d| {
            if d.led_channels == 0 {
                return false;
            }
            if d.is_link_adapter {
                // Applies to every strip/fan behind the adapter.
                let subs = p.sub_rgb_profiles.entry(d.channel_id).or_default();
                for part in &mut d.sub_devices {
                    part.rgb_profile = profile.to_string();
                    subs.insert(part.index, profile.to_string());
                }
            } else {
                d.rgb_profile = profile.to_string();
                p.rgb_profiles.insert(d.channel_id, profile.to_string());
            }
            true
        })?;
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Set the RGB profile of one strip/fan behind a LINK adapter.
    pub fn set_sub_rgb_profile(&self, channel: u8, part: u8, profile: &str) -> Result<(), String> {
        if rgb_profiles::get(profile).is_none() {
            return Err(format!("Unknown RGB profile '{profile}'"));
        }
        self.update_channels(&[channel], |p, d| {
            let Some(sub) = d.sub_devices.iter_mut().find(|s| s.index == part) else { return false };
            sub.rgb_profile = profile.to_string();
            p.sub_rgb_profiles.entry(d.channel_id).or_default().insert(part, profile.to_string());
            true
        })?;
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Choose what is connected to a LINK adapter (Go: `UpdateLinkAdapter`). Rewrites the
    /// hub's device command code and LED count tables and power-cycles the LED ports.
    pub fn set_link_adapter(&self, channel: u8, adapter: u32) -> Result<(), String> {
        let def = adapters::get(adapter).ok_or("Unknown LINK adapter configuration")?;
        let (table, leds) = {
            let s = self.state.read().unwrap();
            let dev = s.devices.get(&channel).filter(|d| d.is_link_adapter).ok_or("That device is not a LINK adapter")?;
            let max_channel = s.devices.values().filter(|d| !d.is_psu).map(|d| d.channel_id).max().unwrap_or(0);
            // Same safety check as upstream: never rewrite the table if it doesn't match the chain.
            if s.led_tables.length != max_channel + 1 {
                return Err(format!(
                    "The hub reports {} channels but {} were found; not changing the adapter to avoid misconfiguring the hub",
                    s.led_tables.length,
                    max_channel + 1
                ));
            }
            let kinds: BTreeMap<u8, adapters::ChannelKind> = s
                .devices
                .values()
                .map(|d| {
                    let kind = if d.channel_id == dev.channel_id {
                        adapters::ChannelKind::Adapter(adapter)
                    } else if d.is_link_adapter {
                        adapters::ChannelKind::Adapter(d.adapter)
                    } else if d.is_commander_duo {
                        adapters::ChannelKind::CommanderDuo
                    } else {
                        adapters::ChannelKind::Device(d.device_code)
                    };
                    (d.channel_id, kind)
                })
                .collect();
            let mut leds = s.led_tables.leds.clone();
            if !adapters::set_led_count(&mut leds, channel, def.total_leds()) {
                return Err("The hub's LED table is too short for this channel".into());
            }
            (adapters::command_table(&kinds, max_channel), leds)
        };

        self.with_io("configure LINK adapter", |t| {
            t.write(CMD_DEVICE_COMMAND_CODES, DATA_TYPE_COMMAND_MODE, &table)?;
            t.write(CMD_DEVICE_COMMAND_LEDS, DATA_TYPE_LED_COUNT, &leds)?;
            t.transfer(CMD_RESET_LED_POWER, &[], 0)?;
            Ok(())
        })
        .ok_or("The hub did not accept the adapter configuration")?;

        {
            let mut s = self.state.write().unwrap();
            s.led_tables.leds = leds;
            let HubState { devices, profile, .. } = &mut *s;
            profile.adapters.insert(channel, adapter);
            profile.sub_rgb_profiles.remove(&channel);
            if let Some(d) = devices.get_mut(&channel) {
                d.adapter = adapter;
                d.sub_devices = adapter_parts(adapter, None);
                d.led_channels = def.total_leds();
            }
            s.global_brightness = port_protection(&s.devices);
        }
        self.save_profile();
        log::info!("[{}] LINK adapter on channel {channel} set to {}", self.tag, def.name);
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub fn set_rgb_override(&self, channel: u8, value: RgbOverride) -> Result<(), String> {
        self.update_channels(&[channel], |p, d| {
            d.rgb_override = value.clone();
            p.rgb_overrides.insert(d.channel_id, value.clone());
            true
        })?;
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Turn one device's LEDs off (or back to its lighting) without saving anything.
    /// The RGB loop picks it up on the next frame.
    pub fn set_light_off(&self, channel: u8, off: bool) -> Result<(), String> {
        let mut s = self.state.write().unwrap();
        let d = s.devices.get_mut(&channel).filter(|d| d.led_channels > 0).ok_or("That device has no lights")?;
        d.light_off = off;
        Ok(())
    }

    pub fn set_label(&self, channel: u8, label: &str) -> Result<(), String> {
        let label: String = label.trim().chars().take(32).collect();
        self.update_channels(&[channel], |p, d| {
            d.label = label.clone();
            p.labels.insert(d.channel_id, label.clone());
            true
        })
    }

    pub fn set_brightness(&self, value: u8) {
        self.state.write().unwrap().profile.brightness = value.min(100);
        self.save_profile();
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_rgb_off(&self, off: bool) {
        self.state.write().unwrap().profile.rgb_off = off;
        self.save_profile();
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
    }

    pub fn lighting(&self) -> Lighting {
        Lighting::capture(&self.state.read().unwrap().profile)
    }

    pub fn apply_lighting(&self, l: &Lighting) {
        {
            let mut s = self.state.write().unwrap();
            let HubState { devices, profile, .. } = &mut *s;
            l.apply(devices, profile);
        }
        self.save_profile();
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Re-render RGB after an RGB profile's colors or speed changed.
    pub fn refresh_rgb(&self) {
        self.rgb_generation.fetch_add(1, Ordering::Relaxed);
    }

    fn update_channels(&self, channels: &[u8], mut f: impl FnMut(&mut HubProfile, &mut LinkDevice) -> bool) -> Result<(), String> {
        {
            let mut s = self.state.write().unwrap();
            let HubState { devices, profile, .. } = &mut *s;
            let mut changed = 0;
            for ch in channels {
                if let Some(d) = devices.get_mut(ch) {
                    if f(profile, d) {
                        changed += 1;
                    }
                }
            }
            if changed == 0 {
                return Err("No matching device on this hub".into());
            }
        }
        self.save_profile();
        Ok(())
    }

    fn save_profile(&self) {
        let profile = self.state.read().unwrap().profile.clone();
        if let Err(e) = config::save_json(&profile_path(&self.serial), &profile) {
            log::error!("[{}] Unable to save hub profile: {e}", self.tag);
        }
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.exit.store(true, Ordering::SeqCst);
    }
}

pub fn short_serial(serial: &str) -> String {
    let tail: String = serial.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("hub …{tail}")
}

/// Hub-wide inputs shared by every segment of a frame.
struct FrameContext {
    brightness: f64,
    liquid: f64,
    /// First temperature probe on the hub, for probe-temperature effects on other devices.
    probe: f64,
    has_lcd: bool,
    critical: bool,
}

/// A run of LEDs driven by one effect: a device, or one part behind a LINK adapter.
struct Segment<'a> {
    leds: u8,
    channel_id: i32,
    aio: bool,
    /// The segment's own probe reading, if it is a temperature probe.
    probe_temp: Option<f64>,
    rgb_override: Option<&'a RgbOverride>,
}

/// Render one segment (Go: `generateRgbEffect`).
fn render_segment(ctx: &FrameContext, rgb_profile: &str, seg: &Segment, start: &mut Instant, state: &mut RgbState) -> Vec<u8> {
    let mode = if ctx.critical { "colorpulse" } else { rgb_profile };
    let profile = rgb_profiles::get(mode).unwrap_or_default();
    let custom = !(profile.start_color.is_empty() || profile.end_color.is_empty());

    let mut r = ActiveRgb::new(seg.leds as usize, profile.speed.clamp(0.1, 10.0), profile.brightness);
    r.has_lcd = ctx.has_lcd;
    r.is_aio = seg.aio;
    r.channel_id = seg.channel_id;
    if custom {
        (r.start_color, r.end_color, r.middle_color) = (profile.start_color, profile.end_color, profile.middle_color);
    } else {
        (r.start_color, r.end_color, r.middle_color) = (state.start_color, state.end_color, state.middle_color);
    }
    if let Some(o) = seg.rgb_override.filter(|_| !ctx.critical) {
        (r.start_color, r.end_color, r.middle_color) = (o.start, o.end, o.middle);
        if o.speed > 0.0 {
            r.rgb_mode_speed = o.speed.clamp(0.1, 10.0);
        }
    }
    r.brightness = ctx.brightness;
    r.start_color.brightness = ctx.brightness;
    r.end_color.brightness = ctx.brightness;
    r.middle_color.brightness = ctx.brightness;
    r.min_temp = profile.min_temp;
    r.max_temp = profile.max_temp;

    let temperature = match mode {
        "liquid-temperature" => ctx.liquid,
        "cpu-temperature" => sensors::cpu() as f64,
        "gpu-temperature" => sensors::gpu() as f64,
        "probe-temperature" => {
            // Upstream defaults the probe gradient range to 0-60 °C per device.
            r.min_temp = 0.0;
            r.max_temp = 60.0;
            r.start_color.temperature = 0.0;
            r.end_color.temperature = 60.0;
            seg.probe_temp.unwrap_or(ctx.probe)
        }
        _ => 0.0,
    };
    rgb::run_effect(mode, &mut r, FrameInputs { start, state, profile: &profile, temperature });
    r.output
}

fn profile_path(serial: &str) -> std::path::PathBuf {
    let safe: String = serial.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    config::path("hubs").join(format!("{safe}.json"))
}

/// Parse the LINK chain (Go: getDevices). Returns devices by channel id and whether an LCD is present.
fn discover_devices(
    t: &Transport,
    profile: &HubProfile,
    firmware_major: u8,
    led_tables: &adapters::LedTables,
) -> io::Result<(BTreeMap<u8, LinkDevice>, bool)> {
    let response = t.read(MODE_GET_DEVICES, DATA_TYPE_GET_DEVICES, 0)?;
    let channels = response[6];
    let data = &response[7..];
    let mut devices = BTreeMap::new();
    let mut lcd = false;
    let mut pos = 0usize;
    let mut duo_port = 1;

    for i in 1..=channels {
        let Some(&id_len) = data.get(pos + 7) else { break };
        let id_len = id_len as usize;
        if id_len == 0 {
            pos += 8;
            continue;
        }
        let Some(type_model) = data.get(pos..pos + 8) else { break };
        let (type_id, model) = (type_model[2], type_model[3]);
        if type_id == 6 || type_id == 14 {
            lcd = true; // iCUE LINK Cooler Pump LCD / XD5 Elite LCD
        }
        let device_id = data.get(pos + 8..pos + 8 + id_len).map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        pos += 8 + id_len;

        let Some(meta) = metadata::lookup(type_id, model) else {
            log::warn!("Unknown LINK device on channel {i}: type {type_id} model {model}");
            continue;
        };

        let mut d = LinkDevice {
            channel_id: i,
            type_id,
            model,
            device_id,
            name: meta.device_name.clone(),
            description: meta.desc.clone(),
            label: profile.labels.get(&i).cloned().unwrap_or_default(),
            led_channels: if meta.link_adapter { 0 } else { meta.led_channels },
            contains_pump: meta.contains_pump,
            aio: meta.aio,
            has_speed: meta.has_speed,
            has_temps: !meta.link_adapter && !meta.vrm_cooler,
            is_probe: meta.temperature_probe,
            is_link_adapter: meta.link_adapter,
            is_commander_duo: meta.commander_duo,
            is_psu: meta.psu,
            is_vrm_cooler: meta.vrm_cooler,
            titan_aio: meta.device_id == 17,
            speed_profile: profile
                .speed_profiles
                .get(&i)
                .filter(|p| temperatures::get(p).is_some())
                .cloned()
                .unwrap_or_else(|| default_speed_profile(meta)),
            rgb_profile: saved_rgb_profile(profile.rgb_profiles.get(&i)),
            rgb_override: profile.rgb_overrides.get(&i).cloned().unwrap_or_default(),
            duty: DEFAULT_SPEED,
            ..LinkDevice::default()
        };
        if meta.psu {
            d.speed_profile = "PSU".into();
        }
        d.device_code = led_tables.code_for(i);
        if meta.link_adapter {
            d.adapter = profile.adapters.get(&i).copied().filter(|&a| adapters::get(a).is_some()).unwrap_or(0);
            d.sub_devices = adapter_parts(d.adapter, profile.sub_rgb_profiles.get(&i));
            d.led_channels = d.sub_devices.iter().map(|p| p.led_channels as u32).sum::<u32>().min(255) as u8;
        }
        if meta.commander_duo {
            d.name = format!("{} - Channel {duo_port}", d.name);
            duo_port += 1;
        }
        if meta.v2_device {
            if let Ok(raw) = t.read(MODE_GET_DEVICE_MODELS, &[], i) {
                match parse_v2_device_model(&raw) {
                    Ok((name, leds)) => {
                        d.name = name;
                        d.led_channels = leds;
                    }
                    Err(e) => log::warn!("Unable to parse V2 device model on channel {i}: {e}"),
                }
            }
        }
        d.port_id = if i >= 13 || (firmware_major < 2 && i >= 7) { 1 } else { 0 };
        devices.insert(i, d);
    }

    if lcd {
        for d in devices.values_mut().filter(|d| d.contains_pump) {
            if d.aio {
                d.led_channels += LCD_LED_CHANNELS; // LCD cap ring
            }
            d.name.push_str(" LCD");
        }
    }
    Ok((devices, lcd))
}

/// Parts behind a LINK adapter configuration, with their saved RGB profiles (default static).
fn adapter_parts(adapter: u32, saved: Option<&BTreeMap<u8, String>>) -> Vec<SubDevice> {
    let Some(def) = adapters::get(adapter).filter(|_| adapter > 0) else { return Vec::new() };
    def.devices
        .iter()
        .map(|(&index, part)| SubDevice {
            index,
            name: part.name.clone(),
            led_channels: part.total,
            rgb_profile: saved_rgb_profile(saved.and_then(|m| m.get(&index))),
        })
        .collect()
}

/// A saved RGB profile name if it still exists, else the default (static).
fn saved_rgb_profile(saved: Option<&String>) -> String {
    saved.filter(|p| rgb_profiles::get(p).is_some()).cloned().unwrap_or_else(|| "static".into())
}

/// AIOs default to their own coolant sensor, which works without LibreHardwareMonitor.
fn default_speed_profile(meta: &metadata::SupportedDevice) -> String {
    if meta.aio { "Liquid Temperature" } else { "Normal" }.to_string()
}

/// Commander DUO channels report their LED count dynamically (Go: getLedDevices).
fn apply_commander_duo_leds(t: &Transport, devices: &mut BTreeMap<u8, LinkDevice>) {
    if !devices.values().any(|d| d.is_commander_duo) {
        return;
    }
    let Ok(buf) = t.read(MODE_GET_LEDS, &[], 0) else { return };
    let channels = buf[6] as usize;
    let data = &buf[7..];
    for i in 1..=channels {
        let (Some(c), Some(n)) = (data.get(i * 4..i * 4 + 2), data.get(i * 4 + 2..i * 4 + 4)) else { break };
        if u16::from_le_bytes([c[0], c[1]]) != 2 {
            continue;
        }
        if let Some(d) = devices.get_mut(&(i as u8)).filter(|d| d.is_commander_duo) {
            d.led_channels = u16::from_le_bytes([n[0], n[1]]).min(50) as u8;
        }
    }
}

/// V2 devices describe themselves with TLV fields: 3 = model name, 12 = one byte per LED.
fn parse_v2_device_model(data: &[u8]) -> Result<(String, u8), String> {
    const OFFSET: usize = 10;
    if data.len() < OFFSET {
        return Err("packet too short".into());
    }
    let len = u16::from_le_bytes([data[8], data[9]]) as usize;
    let end = OFFSET + len;
    if end > data.len() {
        return Err(format!("payload length {len} exceeds packet"));
    }
    let (mut name, mut leds) = (None, None);
    let mut off = OFFSET;
    while off + 2 <= end {
        let (field, flen) = (data[off], data[off + 1] as usize);
        off += 2;
        if off + flen > end {
            return Err(format!("field {field} overruns payload"));
        }
        let value = &data[off..off + flen];
        off += flen;
        match field {
            3 => name = Some(String::from_utf8_lossy(value).trim_end_matches('\0').to_string()),
            12 => leds = Some(flen as u8),
            _ => {}
        }
    }
    Ok((name.ok_or("model name missing")?, leds.ok_or("LED field missing")?))
}

fn parse_rails(data: &[u8], scale: f64) -> Vec<f64> {
    let Some(n) = data.get(6..8).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize) else { return Vec::new() };
    if n > 3 {
        return Vec::new();
    }
    (0..n)
        .map_while(|i| data.get(8 + i * 4 + 2..8 + i * 4 + 4))
        .map(|b| round2(u16::from_le_bytes([b[0], b[1]]) as f64 / scale))
        .collect()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn port_protection(devices: &BTreeMap<u8, LinkDevice>) -> f64 {
    let mut per_port: BTreeMap<u8, u32> = BTreeMap::new();
    for d in devices.values() {
        *per_port.entry(d.port_id).or_default() += d.led_channels as u32;
    }
    let mut cap = 0.0;
    for leds in per_port.values() {
        for (limit, factor) in PORT_PROTECTION {
            if *leds > limit {
                cap = factor;
            }
        }
    }
    cap
}

fn liquid_temperature(devices: &BTreeMap<u8, LinkDevice>) -> f32 {
    devices.values().find(|d| d.aio || d.contains_pump).map_or(0.0, |d| d.temperature)
}

fn sensor_temperature(p: &TemperatureProfile, serial: &str, devices: &BTreeMap<u8, LinkDevice>, liquid: f32, psu: f32) -> f32 {
    let temp = match p.sensor {
        temperatures::SENSOR_CPU => sensors::cpu(),
        temperatures::SENSOR_GPU => {
            let g = sensors::gpu();
            if g > 0.0 { g } else { sensors::cpu() }
        }
        temperatures::SENSOR_LIQUID => liquid,
        temperatures::SENSOR_PSU => psu,
        temperatures::SENSOR_CPU_GPU => {
            let g = sensors::gpu();
            sensors::cpu().max(if g > 0.0 { g } else { 50.0 })
        }
        temperatures::SENSOR_PROBE => {
            if p.device.is_empty() || p.device == serial {
                devices.get(&p.channel_id).map_or(0.0, |d| d.temperature)
            } else {
                manager::device_temperature(&p.device, p.channel_id)
            }
        }
        _ => 0.0,
    };
    // All sources failed: assume a moderate load rather than stopping fans (upstream behaviour).
    if temp == 0.0 { 50.0 } else { temp }
}

/// Map a temperature to this device's duty with upstream's safety floors.
fn duty_for(d: &LinkDevice, p: &TemperatureProfile, temp: f32) -> u8 {
    let mut pump = temperatures::interpolate(p.pump_points(), temp).round() as i32;
    let mut fans = temperatures::interpolate(p.fan_points(), temp).round() as i32;
    if fans < 20 && !p.zero_rpm {
        fans = 20;
    }
    if d.aio {
        if d.titan_aio {
            pump = pump.max(31);
        } else if pump < 50 {
            pump = 70;
        }
    } else if pump < 20 {
        pump = 30;
    }
    (if d.contains_pump { pump } else { fans }).clamp(0, 100) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_round_trips_and_resets_unsaved_channels() {
        let quiet = HubProfile { brightness: 40, rgb_off: true, ..HubProfile::default() };
        let saved = Lighting::capture(&quiet);

        let mut profile = HubProfile::default();
        profile.rgb_profiles.insert(1, "rainbow".into());
        profile.rgb_overrides.insert(1, RgbOverride { enabled: true, ..RgbOverride::default() });
        let part = SubDevice { index: 0, name: "strip".into(), led_channels: 10, rgb_profile: "wave".into() };
        let mut devices = BTreeMap::from([
            (1, LinkDevice { channel_id: 1, rgb_profile: "rainbow".into(), rgb_override: profile.rgb_overrides[&1].clone(), ..fan() }),
            (2, LinkDevice { channel_id: 2, is_link_adapter: true, sub_devices: vec![part], ..LinkDevice::default() }),
        ]);
        let normal = Lighting::capture(&profile);

        saved.apply(&mut devices, &mut profile);
        assert_eq!(devices[&1].rgb_profile, "static");
        assert!(!devices[&1].rgb_override.enabled);
        assert_eq!(devices[&2].sub_devices[0].rgb_profile, "static");
        assert_eq!((profile.brightness, profile.rgb_off), (40, true));
        assert!(profile.rgb_profiles.is_empty());

        normal.apply(&mut devices, &mut profile);
        assert_eq!(devices[&1].rgb_profile, "rainbow");
        assert!(devices[&1].rgb_override.enabled);
        assert_eq!((profile.brightness, profile.rgb_off), (100, false));
    }

    fn fan() -> LinkDevice {
        LinkDevice { has_speed: true, ..LinkDevice::default() }
    }

    #[test]
    fn duty_floors_match_upstream() {
        let normal = temperatures::get("Normal").unwrap();
        assert_eq!(duty_for(&fan(), &normal, 50.0), 40);
        assert_eq!(duty_for(&fan(), &normal, 250.0), 100);

        let aio = LinkDevice { aio: true, contains_pump: true, ..LinkDevice::default() };
        let quiet = temperatures::get("Quiet").unwrap();
        // Quiet pump is 50 %, which is allowed for AIOs; below 50 jumps to 70.
        assert_eq!(duty_for(&aio, &quiet, 40.0), 50);

        let mut silent = normal.clone();
        silent.points.insert(1, vec![temperatures::Point { x: 0.0, y: 0.0 }]);
        assert_eq!(duty_for(&fan(), &silent, 40.0), 20, "fans never stop without zero-RPM");
        silent.zero_rpm = true;
        assert_eq!(duty_for(&fan(), &silent, 40.0), 0);
    }

    #[test]
    fn v2_model_parsing() {
        let mut pkt = vec![0u8; 10];
        let payload = [3u8, 4, b'Q', b'X', b'2', 0, 12, 3, 0, 0, 0];
        pkt[8..10].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        pkt.extend_from_slice(&payload);
        assert_eq!(parse_v2_device_model(&pkt).unwrap(), ("QX2".to_string(), 3));
        assert!(parse_v2_device_model(&pkt[..12]).is_err());
    }

    #[test]
    fn port_protection_thresholds() {
        let qx = |ch| LinkDevice { channel_id: ch, led_channels: 34, ..LinkDevice::default() };
        let seven: BTreeMap<u8, LinkDevice> = (1..=7).map(|c| (c, qx(c))).collect();
        assert_eq!(port_protection(&seven), 0.0);
        let eight: BTreeMap<u8, LinkDevice> = (1..=8).map(|c| (c, qx(c))).collect();
        assert_eq!(port_protection(&eight), 0.66);
    }

    #[test]
    fn rails_parse() {
        let mut v = vec![0u8; 20];
        v[6] = 2;
        v[10..12].copy_from_slice(&12050u16.to_le_bytes());
        v[14..16].copy_from_slice(&5010u16.to_le_bytes());
        assert_eq!(parse_rails(&v, 1000.0), vec![12.05, 5.01]);
    }
}
