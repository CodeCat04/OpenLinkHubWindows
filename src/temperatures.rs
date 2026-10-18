//! Fan/pump curve profiles. Port of OpenLinkHub `src/temperatures` (graph mode).
//!
//! A profile maps a sensor reading to a pump duty (`points[0]`) and a fan duty (`points[1]`)
//! by linear interpolation between user-defined points.

use crate::config;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

pub const SENSOR_CPU: u8 = 0;
pub const SENSOR_GPU: u8 = 1;
pub const SENSOR_LIQUID: u8 = 2;
pub const SENSOR_PROBE: u8 = 4;
pub const SENSOR_CPU_GPU: u8 = 5;
pub const SENSOR_PSU: u8 = 10;

pub const CRITICAL_PROFILE: &str = "aioCriticalTemperature";
/// Built-in curves offered by the fan profile switcher (dashboard header and tray menu).
pub const FAN_PRESETS: [&str; 3] = ["Quiet", "Normal", "Performance"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point {
    /// Temperature, °C.
    pub x: f32,
    /// Duty, %.
    pub y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemperatureProfile {
    pub sensor: u8,
    #[serde(default)]
    pub zero_rpm: bool,
    /// 0 = pump curve, 1 = fan curve.
    pub points: BTreeMap<u8, Vec<Point>>,
    /// Probe channel id (sensor = Probe).
    #[serde(default)]
    pub channel_id: u8,
    /// Hub serial the probe belongs to (sensor = Probe).
    #[serde(default)]
    pub device: String,
    #[serde(default, skip_deserializing)]
    pub builtin: bool,
    /// A built-in curve the user has edited; `reset` puts the default back.
    #[serde(default, skip_deserializing)]
    pub modified: bool,
    #[serde(default, skip_serializing)]
    pub hidden: bool,
}

impl TemperatureProfile {
    pub fn pump_points(&self) -> &[Point] {
        self.points.get(&0).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn fan_points(&self) -> &[Point] {
        self.points.get(&1).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Build graph points from the legacy temperature bands (Go `upgradeGraphProfiles`).
/// Each band is (min, max, fans, pump).
fn from_bands(sensor: u8, zero_rpm: bool, bands: &[(f32, f32, f32, f32)]) -> TemperatureProfile {
    let mut pump = Vec::new();
    let mut fans = Vec::new();
    if bands.len() == 1 {
        let (min, max, f, p) = bands[0];
        pump.extend([Point { x: min, y: p }, Point { x: max, y: p }]);
        fans.extend([Point { x: min, y: f }, Point { x: max, y: f }]);
    } else {
        for (i, &(min, max, f, p)) in bands.iter().enumerate() {
            let x = if i == 0 { min } else { max };
            pump.push(Point { x, y: p });
            fans.push(Point { x, y: f });
        }
    }
    TemperatureProfile {
        sensor,
        zero_rpm,
        points: BTreeMap::from([(0, pump), (1, fans)]),
        channel_id: 0,
        device: String::new(),
        builtin: true,
        modified: false,
        hidden: false,
    }
}

fn builtin_profiles() -> BTreeMap<String, TemperatureProfile> {
    let quiet = [
        (0., 30., 30., 50.), (30., 40., 40., 50.), (40., 50., 40., 50.), (50., 60., 40., 50.),
        (60., 70., 40., 50.), (70., 80., 70., 80.), (80., 90., 90., 90.), (90., 200., 100., 100.),
    ];
    let normal = [
        (0., 30., 30., 70.), (30., 40., 40., 70.), (40., 50., 40., 70.), (50., 60., 45., 70.),
        (60., 70., 55., 70.), (70., 80., 70., 80.), (80., 90., 90., 90.), (90., 200., 100., 100.),
    ];
    let performance = [
        (0., 30., 70., 70.), (30., 40., 70., 70.), (40., 50., 70., 70.), (50., 60., 70., 70.),
        (60., 70., 70., 70.), (70., 80., 70., 80.), (80., 90., 90., 90.), (90., 200., 100., 100.),
    ];
    let liquid = [
        (0., 30., 30., 50.), (30., 32., 30., 50.), (32., 34., 40., 50.), (34., 36., 40., 50.),
        (36., 38., 40., 60.), (38., 40., 40., 60.), (40., 42., 50., 60.), (42., 44., 60., 70.),
        (44., 46., 70., 80.), (46., 48., 80., 90.), (48., 50., 90., 90.), (50., 60., 100., 100.),
    ];
    let psu = [
        (0., 45., 0., 0.), (45., 50., 0., 0.), (50., 55., 20., 0.), (55., 60., 35., 0.),
        (60., 65., 50., 0.), (65., 70., 60., 0.), (70., 75., 75., 0.), (75., 85., 80., 0.),
        (85., 100., 100., 0.),
    ];
    let mut critical = from_bands(SENSOR_LIQUID, false, &[(0., 65., 100., 100.)]);
    critical.hidden = true;

    BTreeMap::from([
        ("Quiet".to_string(), from_bands(SENSOR_CPU, false, &quiet)),
        ("Normal".to_string(), from_bands(SENSOR_CPU, false, &normal)),
        ("Performance".to_string(), from_bands(SENSOR_CPU, false, &performance)),
        ("Liquid Temperature".to_string(), from_bands(SENSOR_LIQUID, false, &liquid)),
        ("PSU".to_string(), from_bands(SENSOR_PSU, true, &psu)),
        (CRITICAL_PROFILE.to_string(), critical),
    ])
}

fn store() -> &'static RwLock<BTreeMap<String, TemperatureProfile>> {
    static STORE: OnceLock<RwLock<BTreeMap<String, TemperatureProfile>>> = OnceLock::new();
    STORE.get_or_init(|| RwLock::new(merge(config::load_json(&config::path("temperatures.json")).unwrap_or_default())))
}

/// Built-in curves, with the user's edits to them and the user's own curves on top.
fn merge(user: BTreeMap<String, TemperatureProfile>) -> BTreeMap<String, TemperatureProfile> {
    let mut all = builtin_profiles();
    for (name, mut profile) in user {
        match all.get(&name) {
            Some(existing) if existing.hidden => {}
            Some(_) => {
                profile.builtin = true;
                profile.modified = true;
                all.insert(name, profile);
            }
            None => {
                all.insert(name, profile);
            }
        }
    }
    all
}

fn persist(all: &BTreeMap<String, TemperatureProfile>) {
    let user: BTreeMap<&String, &TemperatureProfile> = all.iter().filter(|(_, p)| !p.builtin || p.modified).collect();
    if let Err(e) = config::save_json(&config::path("temperatures.json"), &user) {
        log::error!("Unable to save temperature profiles: {e}");
    }
}

pub fn get(name: &str) -> Option<TemperatureProfile> {
    store().read().unwrap().get(name).cloned()
}

/// Visible profiles for the UI.
pub fn all() -> BTreeMap<String, TemperatureProfile> {
    store().read().unwrap().iter().filter(|(_, p)| !p.hidden).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The user's own curves, which also act as profiles (see `crate::profiles`).
pub fn custom_names() -> Vec<String> {
    store().read().unwrap().iter().filter(|(_, p)| !p.builtin && !p.hidden).map(|(k, _)| k.clone()).collect()
}

/// Create or replace a curve. Editing a built-in curve keeps it built-in, and `reset`
/// restores its default.
pub fn upsert(name: &str, mut profile: TemperatureProfile) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 64 {
        return Err("Profile name must be 1-64 characters".into());
    }
    let mut all = store().write().unwrap();
    if all.get(name).is_some_and(|p| p.hidden) {
        return Err("That name is reserved".into());
    }
    let builtin = all.get(name).is_some_and(|p| p.builtin);
    for pts in profile.points.values_mut() {
        pts.retain(|p| p.x.is_finite() && p.y.is_finite());
        for p in pts.iter_mut() {
            p.x = p.x.clamp(0.0, 150.0);
            p.y = p.y.clamp(0.0, 100.0);
        }
        pts.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    if profile.pump_points().is_empty() || profile.fan_points().is_empty() {
        return Err("Profile needs at least one pump point and one fan point".into());
    }
    profile.builtin = builtin;
    profile.modified = builtin;
    profile.hidden = false;
    all.insert(name.to_string(), profile);
    persist(&all);
    Ok(())
}

/// Put a built-in curve back to its default.
pub fn reset(name: &str) -> Result<(), String> {
    let default = builtin_profiles().remove(name).filter(|p| !p.hidden).ok_or("Only built-in curves can be reset")?;
    let mut all = store().write().unwrap();
    all.insert(name.to_string(), default);
    persist(&all);
    Ok(())
}

pub fn delete(name: &str) -> Result<(), String> {
    let mut all = store().write().unwrap();
    match all.get(name) {
        None => Err("No such profile".into()),
        Some(p) if p.builtin => Err("Built-in profiles can't be deleted".into()),
        Some(_) => {
            all.remove(name);
            persist(&all);
            Ok(())
        }
    }
}

/// Linear interpolation over points sorted by temperature, clamped at both ends.
pub fn interpolate(points: &[Point], temp: f32) -> f32 {
    let Some(first) = points.first() else { return 0.0 };
    let last = points[points.len() - 1];
    if temp <= first.x {
        return first.y;
    }
    if temp >= last.x {
        return last.y;
    }
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if temp >= a.x && temp <= b.x {
            if b.x == a.x {
                return b.y;
            }
            return a.y + (temp - a.x) / (b.x - a.x) * (b.y - a.y);
        }
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolation_clamps_and_blends() {
        let pts = [Point { x: 30.0, y: 20.0 }, Point { x: 50.0, y: 60.0 }];
        assert_eq!(interpolate(&pts, 10.0), 20.0);
        assert_eq!(interpolate(&pts, 90.0), 60.0);
        assert_eq!(interpolate(&pts, 40.0), 40.0);
        assert_eq!(interpolate(&[], 40.0), 0.0);
    }

    #[test]
    fn user_edits_layer_over_builtins() {
        let mut quiet = builtin_profiles()["Quiet"].clone();
        quiet.points.insert(1, vec![Point { x: 40.0, y: 25.0 }]);
        let max = from_bands(SENSOR_LIQUID, false, &[(0.0, 100.0, 100.0, 100.0)]);
        let critical = from_bands(SENSOR_CPU, true, &[(0.0, 100.0, 0.0, 0.0)]);
        let all = merge(BTreeMap::from([
            ("Quiet".to_string(), quiet),
            ("MAX".to_string(), TemperatureProfile { builtin: false, ..max }),
            (CRITICAL_PROFILE.to_string(), critical),
        ]));
        assert!(all["Quiet"].builtin && all["Quiet"].modified);
        assert_eq!(all["Quiet"].fan_points()[0].y, 25.0);
        assert!(!all["Normal"].modified);
        assert!(!all["MAX"].builtin);
        // The critical-temperature curve can never be replaced.
        assert_eq!(all[CRITICAL_PROFILE].sensor, SENSOR_LIQUID);
        assert!(all[CRITICAL_PROFILE].hidden);
    }

    #[test]
    fn normal_profile_matches_go_graph_upgrade() {
        let p = &builtin_profiles()["Normal"];
        let fans: Vec<(f32, f32)> = p.fan_points().iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(fans[0], (0.0, 30.0));
        assert_eq!(fans[1], (40.0, 40.0));
        assert_eq!(fans[7], (200.0, 100.0));
        assert_eq!(interpolate(p.fan_points(), 50.0), 40.0);
    }
}
