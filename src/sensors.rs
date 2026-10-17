//! Host temperature sensors on Windows.
//!
//! Windows has no unprivileged CPU temperature API, so CPU/GPU readings come from
//! LibreHardwareMonitor's "Remote Web Server" (`data.json`), which ships its own driver.
//! GPU falls back to `nvidia-smi` when LHM isn't running. Readings refresh every 2 s.

use crate::config;
use serde::Serialize;
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::{OnceLock, RwLock};
use std::thread;
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sensor {
    pub id: String,
    pub hardware: String,
    pub name: String,
    pub value: f32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub cpu: f32,
    pub gpu: f32,
    pub lhm_available: bool,
    pub nvidia_smi: bool,
    pub sensors: Vec<Sensor>,
}

fn snapshot_lock() -> &'static RwLock<Snapshot> {
    static S: OnceLock<RwLock<Snapshot>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(Snapshot::default()))
}

pub fn snapshot() -> Snapshot {
    snapshot_lock().read().unwrap().clone()
}

pub fn cpu() -> f32 {
    snapshot_lock().read().unwrap().cpu
}

pub fn gpu() -> f32 {
    snapshot_lock().read().unwrap().gpu
}

/// Temperature of a LibreHardwareMonitor sensor by id, or 0 when unavailable.
pub fn by_id(id: &str) -> f32 {
    snapshot_lock().read().unwrap().sensors.iter().find(|s| s.id == id).map_or(0.0, |s| s.value)
}

pub fn start() {
    refresh();
    thread::Builder::new()
        .name("sensors".into())
        .spawn(|| loop {
            thread::sleep(POLL_INTERVAL);
            refresh();
        })
        .expect("spawn sensors thread");
}

fn refresh() {
    let mut snap = Snapshot::default();
    let mut warned = WARNED.get_or_init(|| RwLock::new(false)).write().unwrap();

    let url = config::get().lhm_url;
    let fetched = if config::is_loopback_url(&url) {
        fetch_lhm(&url)
    } else {
        Err("URL is not a local (127.0.0.1 / localhost) address, so it is never contacted".into())
    };
    match fetched {
        Ok(root) => {
            snap.lhm_available = true;
            collect(&root, None, &mut snap.sensors);
            snap.cpu = pick_cpu(&snap.sensors);
            snap.gpu = pick_gpu(&snap.sensors);
            *warned = false;
        }
        Err(e) => {
            if !*warned {
                log::warn!(
                    "LibreHardwareMonitor not reachable ({e}). CPU temperature is unavailable; \
                     fan curves using CPU will assume 50 °C. See README for setup."
                );
                *warned = true;
            }
        }
    }

    if snap.gpu == 0.0 {
        if let Some(t) = nvidia_smi_temp() {
            snap.gpu = t;
            snap.nvidia_smi = true;
        }
    }
    *snapshot_lock().write().unwrap() = snap;
}

static WARNED: OnceLock<RwLock<bool>> = OnceLock::new();

fn fetch_lhm(url: &str) -> Result<Value, String> {
    // No redirects: a local service must not be able to bounce us to another host.
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_millis(800)).redirects(0).build();
    agent.get(url).call().map_err(|e| e.to_string())?.into_json::<Value>().map_err(|e| e.to_string())
}

/// Walk the LHM tree collecting temperature sensors. The hardware name is the nearest
/// ancestor node with a device icon (cpu.png, nvidia.png, ...). The top-level computer-name
/// node is skipped so the machine name never reaches the UI or logs.
fn collect(node: &Value, hardware: Option<(&str, &str)>, out: &mut Vec<Sensor>) {
    let text = node.get("Text").and_then(Value::as_str).unwrap_or("");
    let image = node.get("ImageURL").and_then(Value::as_str).unwrap_or("");
    let is_hardware = !image.is_empty()
        && !image.contains("computer")
        && !image.contains("temperature")
        && !image.contains("transparent")
        && node.get("Value").and_then(Value::as_str).is_none_or(str::is_empty);
    let hw = if is_hardware && is_device_icon(image) { Some((text, image)) } else { hardware };

    if let Some(value) = node.get("Value").and_then(Value::as_str) {
        let kind = node.get("Type").and_then(Value::as_str).unwrap_or("");
        let is_temp = kind == "Temperature" || value.contains('°');
        if is_temp {
            if let (Some(v), Some((hw_name, hw_image))) = (parse_celsius(value), hw) {
                let id = node
                    .get("SensorId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{hw_name}/{text}"));
                out.push(Sensor { id, hardware: format!("{}|{hw_name}", icon_kind(hw_image)), name: text.to_string(), value: v });
            }
        }
    }
    if let Some(children) = node.get("Children").and_then(Value::as_array) {
        for child in children {
            collect(child, hw, out);
        }
    }
}

fn is_device_icon(image: &str) -> bool {
    ["cpu", "nvidia", "ati", "amd", "intel", "gpu", "hdd", "nic", "mainboard", "chip", "ram", "battery", "psu", "fan", "water"]
        .iter()
        .any(|k| image.contains(k))
}

fn icon_kind(image: &str) -> &'static str {
    if image.contains("cpu") {
        "cpu"
    } else if image.contains("nvidia") || image.contains("ati") || image.contains("amd") || image.contains("gpu") {
        "gpu"
    } else if image.contains("intel") {
        "igpu"
    } else {
        "other"
    }
}

/// Parse "45.3 °C", "45,3 °C" (locale decimal comma) or "113.5 °F".
fn parse_celsius(s: &str) -> Option<f32> {
    let num: String = s
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',' || *c == '-')
        .map(|c| if c == ',' { '.' } else { c })
        .collect();
    let v: f32 = num.parse().ok()?;
    let c = if s.contains("°F") { (v - 32.0) * 5.0 / 9.0 } else { v };
    (c.is_finite() && c > 0.0 && c < 150.0).then_some(c)
}

fn pick(sensors: &[Sensor], kind: &str, preferred: &[&str]) -> f32 {
    let of_kind: Vec<&Sensor> = sensors.iter().filter(|s| s.hardware.starts_with(kind)).collect();
    for name in preferred {
        if let Some(s) = of_kind.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            return s.value;
        }
    }
    of_kind.first().map_or(0.0, |s| s.value)
}

fn pick_cpu(sensors: &[Sensor]) -> f32 {
    pick(sensors, "cpu|", &["CPU Package", "Core (Tctl/Tdie)", "Core (Tctl)", "Tctl/Tdie", "Tctl", "Core Max", "Core Average", "CPU Cores"])
}

fn pick_gpu(sensors: &[Sensor]) -> f32 {
    let dgpu = pick(sensors, "gpu|", &["GPU Core", "GPU Hot Spot"]);
    if dgpu > 0.0 {
        dgpu
    } else {
        pick(sensors, "igpu|", &["GPU Core"])
    }
}

fn nvidia_smi_temp() -> Option<f32> {
    let out = Command::new("nvidia-smi")
        .args(["-i", "0", "--query-gpu=temperature.gpu", "--format=csv,noheader,nounits"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lhm_values() {
        assert_eq!(parse_celsius("45.5 °C"), Some(45.5));
        assert_eq!(parse_celsius("45,5 °C"), Some(45.5));
        assert_eq!(parse_celsius("122.0 °F"), Some(50.0));
        assert_eq!(parse_celsius("0.0 °C"), None);
        assert_eq!(parse_celsius("n/a"), None);
    }

    #[test]
    fn collects_cpu_and_gpu_and_hides_machine_name() {
        let tree = serde_json::json!({
            "Text": "Sensor", "Children": [{
                "Text": "MY-PC", "ImageURL": "images_icon/computer.png", "Value": "", "Children": [
                    {"Text": "AMD Ryzen 7 7800X3D", "ImageURL": "images_icon/cpu.png", "Value": "", "Children": [
                        {"Text": "Temperatures", "ImageURL": "images_icon/temperature.png", "Value": "", "Children": [
                            {"Text": "Core (Tctl/Tdie)", "Value": "61.4 °C", "SensorId": "/amdcpu/0/temperature/2", "Type": "Temperature", "ImageURL": "images/transparent.png"}
                        ]}
                    ]},
                    {"Text": "NVIDIA GeForce RTX 4080", "ImageURL": "images_icon/nvidia.png", "Value": "", "Children": [
                        {"Text": "Temperatures", "ImageURL": "images_icon/temperature.png", "Value": "", "Children": [
                            {"Text": "GPU Core", "Value": "48,0 °C", "ImageURL": "images/transparent.png"}
                        ]}
                    ]}
                ]
            }]
        });
        let mut sensors = Vec::new();
        collect(&tree, None, &mut sensors);
        assert_eq!(sensors.len(), 2);
        assert_eq!(pick_cpu(&sensors), 61.4);
        assert_eq!(pick_gpu(&sensors), 48.0);
        assert!(sensors.iter().all(|s| !s.hardware.contains("MY-PC") && !s.id.contains("MY-PC")));
    }
}
