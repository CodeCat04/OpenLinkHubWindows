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
    /// Address the web UI binds to. Keep it on loopback unless you know what you're doing.
    pub listen_address: String,
    pub listen_port: u16,
    /// Open the dashboard in the default browser when the app starts.
    pub open_browser: bool,
    /// LibreHardwareMonitor remote web server JSON endpoint, used for CPU/GPU temperatures.
    pub lhm_url: String,
    /// Don't drive fan curves; leave speeds at their defaults.
    pub manual: bool,
    /// Log raw HID traffic summaries.
    pub debug: bool,
    /// The user chose not to install; don't ask again.
    pub install_declined: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen_address: "127.0.0.1".into(),
            listen_port: 27003,
            open_browser: true,
            lhm_url: "http://127.0.0.1:8085/data.json".into(),
            manual: false,
            debug: false,
            install_declined: false,
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

/// The app never talks to anything beyond this computer, so the only URL it will fetch
/// (LibreHardwareMonitor) must be plain HTTP on a loopback address.
pub fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url.trim().strip_prefix("http://") else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    matches!(host.to_ascii_lowercase().as_str(), "127.0.0.1" | "localhost" | "::1")
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

#[cfg(test)]
mod tests {
    use super::is_loopback_url;

    #[test]
    fn only_loopback_urls_are_allowed() {
        assert!(is_loopback_url("http://127.0.0.1:8085/data.json"));
        assert!(is_loopback_url("http://localhost:8085/data.json"));
        assert!(is_loopback_url("http://[::1]:8085/data.json"));
        assert!(!is_loopback_url("https://127.0.0.1:8085/data.json"));
        assert!(!is_loopback_url("http://192.168.1.10:8085/data.json"));
        assert!(!is_loopback_url("http://example.com/data.json"));
        assert!(!is_loopback_url("http://127.0.0.1.example.com/data.json"));
        assert!(!is_loopback_url("http://127.0.0.1@example.com/data.json"));
        assert!(!is_loopback_url("http://localhost.evil:80/"));
    }
}
