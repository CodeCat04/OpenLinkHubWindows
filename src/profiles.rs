//! Profiles: a fan curve for every fan plus the lighting saved with it. Quiet, Normal and
//! Performance always exist; every curve the user creates is a profile too.
//!
//! Lighting edits always go straight to the hub files and `rgb.json`, which hold the current
//! state. On a switch, the current lighting is saved under the profile being left and the new
//! profile's saved lighting is put back. A profile that hasn't been used yet keeps whatever
//! lighting is current. Saved in `%APPDATA%\OpenLinkHub\profiles.json`.

use crate::hub::{manager, Lighting};
use crate::rgb::Profile;
use crate::temperatures::{self, FAN_PRESETS};
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
        Mutex::new(config::load_json(&config::path("profiles.json")).unwrap_or_default())
    })
}

fn save(s: &Store) {
    if let Err(e) = config::save_json(&config::path("profiles.json"), s) {
        log::error!("Unable to save profiles: {e}");
    }
}

/// The user's own curves (custom profiles).
pub fn custom() -> Vec<String> {
    temperatures::custom_names()
}

/// Every profile: Quiet, Normal and Performance, then the custom ones.
pub fn names() -> Vec<String> {
    FAN_PRESETS.iter().map(|p| p.to_string()).chain(custom()).collect()
}

fn is_profile(name: &str) -> bool {
    FAN_PRESETS.contains(&name) || temperatures::get(name).is_some_and(|p| !p.builtin && !p.hidden)
}

fn current(s: &Store) -> Option<String> {
    s.active.clone().filter(|a| is_profile(a)).or_else(|| manager::common_fan_curve().filter(|c| is_profile(c)))
}

/// The selected profile. Before the first switch, the curve every fan is already on.
pub fn active() -> Option<String> {
    current(&store().lock().unwrap())
}

pub fn switch(name: &str) -> Result<(), String> {
    if !is_profile(name) {
        return Err(format!("Unknown profile '{name}'"));
    }
    let hubs = manager::all();
    if hubs.is_empty() {
        return Err("No iCUE LINK hub connected".into());
    }
    let mut s = store().lock().unwrap();
    let leaving = current(&s);
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
    manager::set_fan_curve(name)?;
    s.active = Some(name.to_string());
    save(&s);
    drop(s);
    log::info!("Profile: {name}");
    app::profile_changed();
    Ok(())
}

/// Drop the lighting saved with a custom profile whose curve was deleted.
pub fn forget(name: &str) {
    let mut s = store().lock().unwrap();
    s.lighting.remove(name);
    if s.active.as_deref() == Some(name) {
        s.active = None;
    }
    save(&s);
}
