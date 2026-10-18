//! RGB effect profiles (speed, colors, temperature range per effect).
//!
//! Defaults are embedded from `data/rgb.json` (shared with upstream OpenLinkHub); edits made in the UI are
//! saved to `%APPDATA%\OpenLinkHub\rgb.json` and layered on top.

use crate::config;
use crate::rgb::{Profile, RGB_MODES};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

const RGB_JSON: &str = include_str!("../data/rgb.json");

#[derive(Deserialize)]
struct RgbFile {
    profiles: BTreeMap<String, serde_json::Value>,
}

fn defaults() -> BTreeMap<String, Profile> {
    let file: RgbFile = serde_json::from_str(RGB_JSON).expect("embedded rgb.json is valid");
    let mut out: BTreeMap<String, Profile> = file
        .profiles
        .into_iter()
        .filter(|(name, _)| RGB_MODES.contains(&name.as_str()))
        .filter_map(|(name, v)| serde_json::from_value(v).ok().map(|p| (name, p)))
        .collect();
    out.insert("off".into(), Profile { profile_name: "Off".into(), ..Profile::default() });
    out
}

fn store() -> &'static RwLock<BTreeMap<String, Profile>> {
    static STORE: OnceLock<RwLock<BTreeMap<String, Profile>>> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut all = defaults();
        let user: BTreeMap<String, Profile> = config::load_json(&config::path("rgb.json")).unwrap_or_default();
        for (name, profile) in user {
            if all.contains_key(&name) {
                all.insert(name, profile);
            }
        }
        RwLock::new(all)
    })
}

pub fn get(name: &str) -> Option<Profile> {
    store().read().unwrap().get(name).cloned()
}

pub fn all() -> BTreeMap<String, Profile> {
    store().read().unwrap().clone()
}

pub fn update(name: &str, mut profile: Profile) -> Result<(), String> {
    let mut all = store().write().unwrap();
    let Some(existing) = all.get(name) else { return Err("Unknown RGB profile".into()) };
    profile.profile_name = existing.profile_name.clone();
    profile.speed = profile.speed.clamp(0.1, 10.0);
    profile.smoothness = profile.smoothness.clamp(1, 100);
    all.insert(name.to_string(), profile);
    config::save_json(&config::path("rgb.json"), &*all).map_err(|e| e.to_string())
}

/// Put back effect settings saved with a profile. Effects it doesn't mention are left alone.
pub fn restore(saved: &BTreeMap<String, Profile>) -> Result<(), String> {
    let mut all = store().write().unwrap();
    for (name, profile) in saved {
        if let Some(existing) = all.get_mut(name) {
            *existing = profile.clone();
        }
    }
    config::save_json(&config::path("rgb.json"), &*all).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_mode_has_a_profile() {
        let all = super::defaults();
        for mode in crate::rgb::RGB_MODES {
            assert!(all.contains_key(*mode), "missing default profile for {mode}");
        }
    }
}
