//! Supported LINK devices, embedded from the upstream `database/external/lsh.json`.

use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupportedDevice {
    pub device_id: u8,
    pub device_model: u8,
    pub device_name: String,
    #[serde(default)]
    pub led_channels: u8,
    #[serde(default)]
    pub contains_pump: bool,
    #[serde(default)]
    pub desc: String,
    #[serde(default)]
    pub aio: bool,
    #[serde(default)]
    pub temperature_probe: bool,
    #[serde(default)]
    pub link_adapter: bool,
    #[serde(default)]
    pub has_speed: bool,
    #[serde(default)]
    pub commander_duo: bool,
    #[serde(default)]
    pub psu: bool,
    #[serde(default)]
    pub vrm_cooler: bool,
    #[serde(default)]
    pub v2_device: bool,
}

const LSH_JSON: &str = include_str!("../../../database/external/lsh.json");

pub fn all() -> &'static [SupportedDevice] {
    static DEVICES: OnceLock<Vec<SupportedDevice>> = OnceLock::new();
    DEVICES.get_or_init(|| serde_json::from_str(LSH_JSON).expect("embedded lsh.json is valid"))
}

pub fn lookup(device_id: u8, model: u8) -> Option<&'static SupportedDevice> {
    all().iter().find(|d| d.device_id == device_id && d.device_model == model)
}

#[cfg(test)]
mod tests {
    #[test]
    fn metadata_parses_and_has_qx() {
        let qx = super::lookup(1, 0).expect("QX fan present");
        assert_eq!(qx.led_channels, 34);
        assert!(qx.has_speed);
    }
}
