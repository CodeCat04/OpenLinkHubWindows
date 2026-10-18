//! Application configuration and on-disk locations.
//!
//! All user data lives in `%APPDATA%\OpenLinkHub` so the executable can be placed anywhere
//! (Downloads, Program Files, a USB stick) and upgrades are a simple file swap.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// Open the dashboard window when the app is started by hand (not at sign-in).
    pub open_dashboard: bool,
    /// Don't drive fan curves; leave speeds at their defaults.
    pub manual: bool,
    /// Log raw HID traffic summaries.
    pub debug: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            open_dashboard: true,
            manual: false,
            debug: false,
        }
    }
}

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static CONFIG: OnceLock<RwLock<Config>> = OnceLock::new();

/// `%APPDATA%\OpenLinkHub`, created on first use.
pub fn data_dir() -> &'static Path {
    DATA_DIR.get_or_init(|| {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        let dir = base.join("OpenLinkHub");
        let _ = fs::create_dir_all(dir.join("hubs"));
        dir
    })
}

pub fn path(name: &str) -> PathBuf {
    data_dir().join(name)
}

/// Load `config.json`, writing defaults (including any newly added keys) back to disk.
pub fn init() -> Config {
    CONFIG
        .get_or_init(|| {
            let file = path("config.json");
            let cfg: Config = load_json(&file).unwrap_or_default();
            if let Err(e) = save_json(&file, &cfg) {
                log::warn!("Unable to write {}: {e}", file.display());
            }
            RwLock::new(cfg)
        })
        .read()
        .unwrap()
        .clone()
}

pub fn get() -> Config {
    CONFIG.get().expect("config::init() not called").read().unwrap().clone()
}

/// Modify and persist the configuration.
pub fn update(f: impl FnOnce(&mut Config)) -> std::io::Result<()> {
    let mut cfg = CONFIG.get().expect("config::init() not called").write().unwrap();
    f(&mut cfg);
    save_json(&path("config.json"), &*cfg)
}

pub fn load_json<T: DeserializeOwned>(file: &Path) -> Option<T> {
    let text = fs::read_to_string(file).ok()?;
    match serde_json::from_str(&text) {
        Ok(v) => Some(v),
        Err(e) => {
            log::error!("Ignoring invalid JSON in {}: {e}", file.display());
            None
        }
    }
}

/// Write JSON atomically (temp file + rename) so a crash never leaves a truncated profile.
pub fn save_json<T: Serialize>(file: &Path, value: &T) -> std::io::Result<()> {
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&tmp, file)
}

