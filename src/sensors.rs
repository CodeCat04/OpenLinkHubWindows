//! Host sensors: CPU/GPU temperature and load. No network, no child processes.
//!
//! - CPU temperature: published by the OpenLinkHub Sensors service (see `sensors/`), which
//!   reads it through the PawnIO driver. Windows only lets kernel drivers read it.
//! - CPU load: `GetSystemTimes`.
//! - NVIDIA GPU temperature and load: NVML (`nvml.dll`, installed with the NVIDIA driver).
//!
//! Everything refreshes every 2 s on one background thread.

#[path = "../sensors/shared.rs"]
mod shared;

use serde::Serialize;
use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::sync::{OnceLock, RwLock};
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, MEMORY_MAPPED_VIEW_ADDRESS};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::GetSystemTimes;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// A service reading older than this is considered stale (service stopped).
const STALE_MS: u64 = 10_000;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub cpu: f32,
    pub gpu: f32,
    pub cpu_load: f32,
    pub gpu_load: f32,
    /// The sensor service is installed and running.
    pub cpu_service: bool,
    /// Human-readable CPU sensor state, e.g. "ok" or "PawnIO driver not installed".
    pub cpu_status: String,
    pub nvidia: bool,
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

pub fn start() {
    thread::Builder::new()
        .name("sensors".into())
        .spawn(|| {
            let mut cpu_reader = CpuReader::default();
            let mut load = CpuLoad::default();
            let nvml = Nvml::load();
            let mut logged = String::new();
            loop {
                let mut snap = Snapshot { cpu_status: "not installed".into(), ..Snapshot::default() };
                if let Some(s) = cpu_reader.read() {
                    snap.cpu_service = true;
                    snap.cpu_status = shared::status_text(s.status).into();
                    if s.status == shared::STATUS_OK {
                        snap.cpu = s.cpu_temp;
                    }
                }
                if snap.cpu_status != logged {
                    match snap.cpu_status.as_str() {
                        "ok" => log::info!("CPU temperature available from the sensor service"),
                        status => log::warn!("CPU temperature unavailable: sensor service {status}. Fan curves using CPU assume 50 °C."),
                    }
                    logged = snap.cpu_status.clone();
                }
                snap.cpu_load = load.sample();
                if let Some(n) = &nvml {
                    snap.nvidia = true;
                    (snap.gpu, snap.gpu_load) = n.read();
                }
                *snapshot_lock().write().unwrap() = snap;
                thread::sleep(POLL_INTERVAL);
            }
        })
        .expect("spawn sensors thread");
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Reads the service's shared memory, reopening it if the service restarts.
#[derive(Default)]
struct CpuReader {
    mapping: Option<(HANDLE, *const shared::Shared)>,
}

// The view is only read through volatile copies from this one thread.
unsafe impl Send for CpuReader {}

impl CpuReader {
    fn open(&mut self) -> bool {
        let name = wide(shared::MAPPING_NAME);
        let h = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if h.is_null() {
            return false;
        }
        let view: MEMORY_MAPPED_VIEW_ADDRESS = unsafe { MapViewOfFile(h, FILE_MAP_READ, 0, 0, size_of::<shared::Shared>()) };
        if view.Value.is_null() {
            unsafe { CloseHandle(h) };
            return false;
        }
        self.mapping = Some((h, view.Value as *const shared::Shared));
        true
    }

    fn close(&mut self) {
        if let Some((h, view)) = self.mapping.take() {
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: view as *mut c_void });
                CloseHandle(h);
            }
        }
    }

    fn read(&mut self) -> Option<shared::Shared> {
        if self.mapping.is_none() && !self.open() {
            return None;
        }
        let (_, view) = self.mapping?;
        for _ in 0..4 {
            use std::sync::atomic::{fence, Ordering};
            let s1 = unsafe { std::ptr::read_volatile(&(*view).seq) };
            fence(Ordering::Acquire);
            let data = unsafe { std::ptr::read_volatile(view) };
            fence(Ordering::Acquire);
            let s2 = unsafe { std::ptr::read_volatile(&(*view).seq) };
            if s1 != s2 || s1 & 1 == 1 {
                thread::sleep(Duration::from_millis(1));
                continue;
            }
            if data.magic != shared::MAGIC || data.version != shared::VERSION {
                return None;
            }
            if unsafe { GetTickCount64() }.saturating_sub(data.updated_tick_ms) > STALE_MS && data.status != shared::STATUS_NO_PAWNIO {
                // Service stopped or restarted with a new block; reopen next time.
                self.close();
                return None;
            }
            return Some(data);
        }
        None
    }
}

/// Whole-system CPU utilisation from successive `GetSystemTimes` samples.
#[derive(Default)]
struct CpuLoad {
    last: Option<(u64, u64)>,
}

impl CpuLoad {
    fn sample(&mut self) -> f32 {
        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return 0.0;
        }
        // Kernel time includes idle time.
        let (busy_total, idle) = (ft(kernel) + ft(user), ft(idle));
        let load = match self.last {
            Some((prev_total, prev_idle)) if busy_total > prev_total => {
                let total = (busy_total - prev_total) as f64;
                let idle = idle.saturating_sub(prev_idle) as f64;
                ((1.0 - idle / total) * 100.0).clamp(0.0, 100.0) as f32
            }
            _ => 0.0,
        };
        self.last = Some((busy_total, idle));
        load
    }
}

type NvmlInit = unsafe extern "C" fn() -> i32;
type NvmlHandleByIndex = unsafe extern "C" fn(u32, *mut *mut c_void) -> i32;
type NvmlGetTemperature = unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32;
type NvmlGetUtilization = unsafe extern "C" fn(*mut c_void, *mut [u32; 2]) -> i32;
type Farproc = unsafe extern "system" fn() -> isize;

/// Minimal NVML binding for GPU 0, loaded at runtime so non-NVIDIA systems just skip it.
struct Nvml {
    device: *mut c_void,
    get_temperature: NvmlGetTemperature,
    get_utilization: NvmlGetUtilization,
}

unsafe impl Send for Nvml {}

impl Nvml {
    fn load() -> Option<Nvml> {
        let lib = unsafe { LoadLibraryW(wide("nvml.dll").as_ptr()) };
        if lib.is_null() {
            return None;
        }
        unsafe {
            let sym = |name: &[u8]| GetProcAddress(lib, name.as_ptr());
            // NVML exports use the C ABI, which is the same as "system" on x86-64.
            let init = std::mem::transmute::<Farproc, NvmlInit>(sym(b"nvmlInit_v2\0")?);
            let by_index = std::mem::transmute::<Farproc, NvmlHandleByIndex>(sym(b"nvmlDeviceGetHandleByIndex_v2\0")?);
            let get_temperature = std::mem::transmute::<Farproc, NvmlGetTemperature>(sym(b"nvmlDeviceGetTemperature\0")?);
            let get_utilization = std::mem::transmute::<Farproc, NvmlGetUtilization>(sym(b"nvmlDeviceGetUtilizationRates\0")?);
            if init() != 0 {
                return None;
            }
            let mut device = std::ptr::null_mut();
            if by_index(0, &mut device) != 0 {
                return None;
            }
            log::info!("NVIDIA GPU sensors available");
            Some(Nvml { device, get_temperature, get_utilization })
        }
    }

    /// (temperature °C, utilisation %)
    fn read(&self) -> (f32, f32) {
        let (mut temp, mut util) = (0u32, [0u32; 2]);
        unsafe {
            if (self.get_temperature)(self.device, 0, &mut temp) != 0 {
                temp = 0;
            }
            if (self.get_utilization)(self.device, &mut util) != 0 {
                util = [0, 0];
            }
        }
        (temp as f32, util[0] as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_load_is_a_percentage() {
        let mut l = CpuLoad::default();
        assert_eq!(l.sample(), 0.0);
        thread::sleep(Duration::from_millis(50));
        let v = l.sample();
        assert!((0.0..=100.0).contains(&v));
    }
}
