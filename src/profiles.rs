//! Profiles (Quiet / Normal / Performance): a fan preset plus the lighting saved with it.
//!
//! Lighting edits always go straight to the hub files and `rgb.json`, which hold the current
//! state. On a switch, the current lighting is saved under the profile being left and the new
//! profile's saved lighting is put back. A profile that hasn't been used yet keeps whatever
//! lighting is current. Saved in `%APPDATA%\OpenLinkHub\profiles.json`.

use crate::hub::{manager, Lighting};
use crate::rgb::Profile;
use crate::temperatures::FAN_PRESETS;
use crate::{app, config, rgb_profiles};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Store {
    active: Option<String>,
    lighting: BTreeMap<String, SavedLighting>,
}

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SavedLighting {
    /// RGB effect settings (colors, speed) by effect.
    effects: BTreeMap<String, Profile>,
    /// Hub serial → device lighting. Hubs that were unplugged at the time keep their entry.
    hubs: BTreeMap<String, Lighting>,
}

fn store() -> &'static Mutex<Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    STORE.get_or_init(|| {
        let mut s: Store = config::load_json(&config::path("profiles.json")).unwrap_or_default();
        s.active = s.active.filter(|a| FAN_PRESETS.contains(&a.as_str()));
        Mutex::new(s)
    })
}

/// The selected profile. Before the first switch, the preset every fan is already on.
pub fn active() -> Option<String> {
    let s = store().lock().unwrap();
    s.active.clone().or_else(|| manager::active_fan_preset().map(String::from))
}

pub fn switch(name: &str) -> Result<(), String> {
    if !FAN_PRESETS.contains(&name) {
        return Err(format!("Unknown profile '{name}'"));
    }
    let hubs = manager::all();
    if hubs.is_empty() {
        return Err("No iCUE LINK hub connected".into());
    }
    let mut s = store().lock().unwrap();
    let leaving = s.active.clone().or_else(|| manager::active_fan_preset().map(String::from));
    if let Some(leaving) = leaving.filter(|l| l != name) {
        let saved = s.lighting.entry(leaving).or_default();
        saved.effects = rgb_profiles::all();
        for hub in &hubs {
            saved.hubs.insert(hub.serial.clone(), hub.lighting());
        }
        if let Some(target) = s.lighting.get(name) {
            if let Err(e) = rgb_profiles::restore(&target.effects) {
                log::error!("Unable to restore lighting effects: {e}");
            }
            for hub in &hubs {
                if let Some(l) = target.hubs.get(&hub.serial) {
                    hub.apply_lighting(l);
                }
            }
        }
    }
    manager::set_fan_preset(name)?;
    s.active = Some(name.to_string());
    if let Err(e) = config::save_json(&config::path("profiles.json"), &*s) {
        log::error!("Unable to save profiles: {e}");
    }
    drop(s);
    log::info!("Profile: {name}");
    app::profile_changed();
    Ok(())
}
