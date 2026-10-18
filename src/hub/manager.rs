//! Discovers iCUE LINK hubs and keeps them running across hot-plug and sleep/resume.
//!
//! Windows invalidates HID handles on resume and on unplug. A hub whose I/O starts failing
//! marks itself failed; the next scan drops it and re-initialises it once it enumerates again.

use super::{Hub, HubView, PRODUCT_ID, VENDOR_ID};
use hidapi::HidApi;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

const SCAN_INTERVAL: Duration = Duration::from_secs(3);
/// Interface 0 carries the control protocol; interface 1 is unused here.
const CONTROL_INTERFACE: i32 = 0;

static STOPPING: AtomicBool = AtomicBool::new(false);

fn hubs() -> &'static RwLock<BTreeMap<String, Arc<Hub>>> {
    static HUBS: OnceLock<RwLock<BTreeMap<String, Arc<Hub>>>> = OnceLock::new();
    HUBS.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn api() -> &'static Mutex<Option<HidApi>> {
    static API: OnceLock<Mutex<Option<HidApi>>> = OnceLock::new();
    API.get_or_init(|| Mutex::new(None))
}

pub fn start() {
    match HidApi::new() {
        Ok(a) => *api().lock().unwrap() = Some(a),
        Err(e) => {
            log::error!("Unable to initialise HID: {e}");
            return;
        }
    }
    scan();
    if hubs().read().unwrap().is_empty() {
        log::warn!("No iCUE LINK System Hub found. Is it connected, and is iCUE closed?");
    }
    thread::Builder::new()
        .name("hotplug".into())
        .spawn(|| {
            while !STOPPING.load(Ordering::Relaxed) {
                thread::sleep(SCAN_INTERVAL);
                if !STOPPING.load(Ordering::Relaxed) {
                    scan();
                }
            }
        })
        .expect("spawn hotplug thread");
}

/// One enumeration pass: drop failed or vanished hubs, start new ones.
fn scan() {
    let mut guard = api().lock().unwrap();
    let Some(api) = guard.as_mut() else { return };
    if let Err(e) = api.refresh_devices() {
        log::warn!("HID enumeration failed: {e}");
        return;
    }

    let present: Vec<(String, String)> = api
        .device_list()
        .filter(|d| d.vendor_id() == VENDOR_ID && d.product_id() == PRODUCT_ID && d.interface_number() == CONTROL_INTERFACE)
        .map(|d| {
            let path = d.path().to_string_lossy().into_owned();
            let serial = d.serial_number().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| path.clone());
            (serial, path)
        })
        .collect();

    let stale: Vec<Arc<Hub>> = {
        let mut map = hubs().write().unwrap();
        let gone: Vec<String> = map
            .iter()
            .filter(|(serial, hub)| hub.is_failed() || !present.iter().any(|(s, _)| s == *serial))
            .map(|(serial, _)| serial.clone())
            .collect();
        gone.iter().filter_map(|s| map.remove(s)).collect()
    };
    for hub in stale {
        log::info!("[{}] Hub disconnected", hub.tag);
        hub.stop();
    }

    for (serial, path) in present.iter().cloned() {
        if STOPPING.load(Ordering::Relaxed) || hubs().read().unwrap().contains_key(&serial) {
            continue;
        }
        let dev = match api.open_path(&std::ffi::CString::new(path.clone()).unwrap_or_default()) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("Unable to open {}: {e}. Close iCUE or other RGB software and it will be retried.", super::short_serial(&serial));
                continue;
            }
        };
        match Hub::start(dev, serial.clone()) {
            Ok(hub) => {
                hubs().write().unwrap().insert(serial, hub);
            }
            Err(e) => log::warn!("Unable to initialise {}: {e}. Will retry.", super::short_serial(&serial)),
        }
    }

    // LCD panels (pump caps, XD5/XC7 screens) are separate USB devices attached to a hub's pump.
    let panels = crate::lcd::discover(api);
    let hubs_now = all();
    for hub in &hubs_now {
        hub.prune_lcds(&panels);
    }
    for info in &panels {
        if STOPPING.load(Ordering::Relaxed) || hubs_now.iter().any(|h| h.has_lcd_panel(&info.serial)) {
            continue;
        }
        if let Some((hub, ch)) = hubs_now.iter().find_map(|h| h.lcd_channel_for(info).map(|ch| (h, ch))) {
            hub.attach_lcd(api, info, ch);
        }
    }
}

pub fn get(serial: &str) -> Option<Arc<Hub>> {
    hubs().read().unwrap().get(serial).cloned()
}

pub fn all() -> Vec<Arc<Hub>> {
    hubs().read().unwrap().values().cloned().collect()
}

pub fn views() -> Vec<HubView> {
    all().iter().map(|h| h.view()).collect()
}

/// Temperature of a probe on any hub (fan curves can follow a probe on another hub).
pub fn device_temperature(serial: &str, channel: u8) -> f32 {
    get(serial).map_or(0.0, |h| h.device_temperature(channel))
}

/// Re-render RGB on every hub (after an RGB profile edit).
pub fn refresh_rgb() {
    for hub in all() {
        hub.refresh_rgb();
    }
}

/// Stop every hub and return them to hardware mode. Safe to call more than once.
pub fn stop_all() {
    STOPPING.store(true, Ordering::SeqCst);
    let all: Vec<Arc<Hub>> = std::mem::take(&mut *hubs().write().unwrap()).into_values().collect();
    for hub in all {
        hub.stop();
    }
}
