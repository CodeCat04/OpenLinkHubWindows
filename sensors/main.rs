//! OpenLinkHub Sensors: a tiny Windows service that reads the CPU package temperature
//! through the signed PawnIO driver and publishes it in shared memory for OpenLinkHub.
//!
//! Windows only lets kernel drivers read CPU temperature sensors, and PawnIO only accepts
//! SYSTEM or Administrators. Keeping that in this small service lets OpenLinkHub itself run
//! without admin rights. The service takes no input, opens no files, has no network code,
//! and runs from Program Files so non-admin processes can't modify it.
//!
//! Usage (from an elevated prompt): `OpenLinkHubSensors.exe install | uninstall | console`

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "shared.rs"]
mod shared;

use shared::*;
use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType,
    ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::Memory::{CreateFileMappingW, MapViewOfFile, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows_sys::Win32::System::IO::DeviceIoControl;

const SERVICE_NAME: &str = "OpenLinkHubSensors";
const DISPLAY_NAME: &str = "OpenLinkHub Sensors";
const DESCRIPTION: &str =
    "Reads the CPU temperature for OpenLinkHub through the PawnIO driver. Takes no input and has no network access.";
const INTERVAL: Duration = Duration::from_secs(2);
/// How often to retry when PawnIO isn't installed yet or the CPU can't be read.
const RETRY: Duration = Duration::from_secs(30);

const AMD_MODULE: &[u8] = include_bytes!("pawnio/AMDFamily17.bin");
const INTEL_MODULE: &[u8] = include_bytes!("pawnio/IntelMSR.bin");

// PawnIO device type 41394; CTL_CODE(type, fn, METHOD_BUFFERED, FILE_ANY_ACCESS).
const IOCTL_PIO_LOAD_BINARY: u32 = (41394 << 16) | (0x821 << 2);
const IOCTL_PIO_EXECUTE_FN: u32 = (41394 << 16) | (0x841 << 2);
const FN_NAME_LENGTH: usize = 32;

/// AMD SMN register THM_TCON_CUR_TMP (Tctl), same as Linux k10temp.
const SMN_THM_TCON_CUR_TMP: u64 = 0x0005_9800;
const MSR_IA32_TEMPERATURE_TARGET: u64 = 0x1A2;
const MSR_IA32_PACKAGE_THERM_STATUS: u64 = 0x1B1;

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn main() -> ExitCode {
    let arg = std::env::args().nth(1).unwrap_or_default().to_ascii_lowercase();
    let result = match arg.as_str() {
        "install" => install(),
        "uninstall" => uninstall(),
        "console" => {
            let (_tx, rx) = mpsc::channel();
            run(&rx, true);
            Ok(())
        }
        _ => service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| e.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------- service

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    let (tx, rx) = mpsc::channel();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register(SERVICE_NAME, handler) else { return };
    let set = |state, accept| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
    };
    set(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
    run(&rx, false);
    set(ServiceState::Stopped, ServiceControlAccept::empty());
}

/// Main loop: publish a reading every 2 s until told to stop.
fn run(stop: &mpsc::Receiver<()>, console: bool) {
    let Some(block) = SharedBlock::create() else {
        if console {
            eprintln!("Unable to create shared memory (run as administrator)");
        }
        return;
    };
    block.publish(STATUS_STARTING, 0.0, cpu_vendor().0);

    let mut sensor: Option<CpuSensor> = None;
    loop {
        let wait = match sensor.as_mut().map(CpuSensor::read) {
            Some(Ok(t)) => {
                block.publish(STATUS_OK, t, sensor.as_ref().map_or(0, |s| s.vendor));
                if console {
                    println!("CPU {t:.1} °C");
                }
                INTERVAL
            }
            Some(Err(_)) => {
                // The driver may have been updated or removed; reopen next time.
                sensor = None;
                block.publish(STATUS_READ_FAILED, 0.0, cpu_vendor().0);
                RETRY
            }
            None => match CpuSensor::open() {
                Ok(s) => {
                    sensor = Some(s);
                    Duration::ZERO
                }
                Err(status) => {
                    if console {
                        eprintln!("{}", status_text(status));
                    }
                    block.publish(status, 0.0, cpu_vendor().0);
                    RETRY
                }
            },
        };
        match stop.recv_timeout(wait) {
            Err(RecvTimeoutError::Timeout) => {}
            _ => break,
        }
    }
}

// ---------------------------------------------------------------- shared memory

struct SharedBlock {
    _mapping: HANDLE,
    view: *mut Shared,
}

impl SharedBlock {
    fn create() -> Option<SharedBlock> {
        // SYSTEM and Administrators: full access. Authenticated users: read only.
        let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GR;;;AU)");
        let mut sd = std::ptr::null_mut();
        let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) };
        if ok == 0 {
            return None;
        }
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd, bInheritHandle: 0 };
        let name = wide(MAPPING_NAME);
        let mapping = unsafe { CreateFileMappingW(INVALID_HANDLE_VALUE, &sa, PAGE_READWRITE, 0, size_of::<Shared>() as u32, name.as_ptr()) };
        unsafe { LocalFree(sd) };
        if mapping.is_null() {
            return None;
        }
        let view: MEMORY_MAPPED_VIEW_ADDRESS = unsafe { MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, size_of::<Shared>()) };
        if view.Value.is_null() {
            unsafe { CloseHandle(mapping) };
            return None;
        }
        Some(SharedBlock { _mapping: mapping, view: view.Value as *mut Shared })
    }

    fn publish(&self, status: u32, cpu_temp: f32, vendor: u32) {
        use std::sync::atomic::{fence, Ordering};
        unsafe {
            let p = self.view;
            let seq = std::ptr::read_volatile(&(*p).seq);
            std::ptr::write_volatile(&mut (*p).seq, seq.wrapping_add(1) | 1); // odd: writing
            fence(Ordering::Release);
            let data = Shared {
                magic: MAGIC,
                version: VERSION,
                seq: seq.wrapping_add(1) | 1,
                status,
                cpu_temp,
                vendor,
                updated_tick_ms: GetTickCount64(),
                reserved: [0; 8],
            };
            std::ptr::write_volatile(p, data);
            fence(Ordering::Release);
            std::ptr::write_volatile(&mut (*p).seq, (seq.wrapping_add(1) | 1).wrapping_add(1)); // even: done
        }
    }
}

// ---------------------------------------------------------------- CPU

/// (vendor, family, model, brand string)
fn cpu_vendor() -> (u32, u32, u32, String) {
    use std::arch::x86_64::__cpuid;
    let v = __cpuid(0);
    let mut id = Vec::with_capacity(12);
    for r in [v.ebx, v.edx, v.ecx] {
        id.extend_from_slice(&r.to_le_bytes());
    }
    let vendor = match &id[..] {
        b"AuthenticAMD" => VENDOR_AMD,
        b"GenuineIntel" => VENDOR_INTEL,
        _ => VENDOR_UNKNOWN,
    };
    let sig = __cpuid(1).eax;
    let base_family = (sig >> 8) & 0xF;
    let family = if base_family == 0xF { base_family + ((sig >> 20) & 0xFF) } else { base_family };
    let model = ((sig >> 4) & 0xF) | (((sig >> 16) & 0xF) << 4);
    let mut brand = Vec::with_capacity(48);
    if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
        for leaf in 0x8000_0002u32..=0x8000_0004 {
            let r = __cpuid(leaf);
            for x in [r.eax, r.ebx, r.ecx, r.edx] {
                brand.extend_from_slice(&x.to_le_bytes());
            }
        }
    }
    let brand = String::from_utf8_lossy(&brand).trim_matches(char::from(0)).trim().to_string();
    (vendor, family, model, brand)
}

struct CpuSensor {
    pawnio: PawnIo,
    vendor: u32,
    /// AMD only: offset between Tctl and the real die temperature on some early Zen parts.
    tctl_offset: f32,
    /// Intel only: TjMax, °C.
    tj_max: f32,
    pci_mutex: HANDLE,
}

impl CpuSensor {
    /// Open PawnIO and load the module for this CPU. Errors are shared-memory status codes.
    fn open() -> Result<CpuSensor, u32> {
        let (vendor, family, _model, brand) = cpu_vendor();
        let module = match vendor {
            VENDOR_AMD if (0x17..=0x1A).contains(&family) => AMD_MODULE,
            VENDOR_INTEL => INTEL_MODULE,
            _ => return Err(STATUS_UNSUPPORTED_CPU),
        };
        let pawnio = PawnIo::open().map_err(|_| STATUS_NO_PAWNIO)?;
        pawnio.load(module).map_err(|_| STATUS_MODULE_FAILED)?;

        let mut sensor = CpuSensor { pawnio, vendor, tctl_offset: 0.0, tj_max: 100.0, pci_mutex: std::ptr::null_mut() };
        if vendor == VENDOR_AMD {
            // Same offsets as Linux k10temp (only Zen 1 / Zen+ X-series report a raised Tctl).
            if family == 0x17 {
                sensor.tctl_offset = [("1600X", 20.0), ("1700X", 20.0), ("1800X", 20.0), ("2700X", 10.0), ("Threadripper 19", 27.0), ("Threadripper 29", 27.0)]
                    .iter()
                    .find(|(name, _)| brand.contains(name))
                    .map_or(0.0, |(_, o)| *o);
            }
            // SMN access is shared with other tools (LibreHardwareMonitor, BIOS utilities); they
            // all serialise on this global mutex.
            let name = wide("Global\\Access_PCI");
            sensor.pci_mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        } else {
            let mut out = [0u64; 1];
            if sensor.pawnio.execute("ioctl_read_msr", &[MSR_IA32_TEMPERATURE_TARGET], &mut out).is_ok() {
                let tj = ((out[0] >> 16) & 0xFF) as f32;
                if tj > 0.0 {
                    sensor.tj_max = tj;
                }
            }
        }
        sensor.read().map_err(|_| STATUS_READ_FAILED)?;
        Ok(sensor)
    }

    fn read(&mut self) -> io::Result<f32> {
        let mut out = [0u64; 1];
        let temp = if self.vendor == VENDOR_AMD {
            let locked = !self.pci_mutex.is_null()
                && matches!(unsafe { WaitForSingleObject(self.pci_mutex, 200) }, WAIT_OBJECT_0 | WAIT_ABANDONED);
            let r = self.pawnio.execute("ioctl_read_smn", &[SMN_THM_TCON_CUR_TMP], &mut out);
            if locked {
                unsafe { ReleaseMutex(self.pci_mutex) };
            }
            r?;
            let v = out[0] as u32;
            let mut t = ((v >> 21) & 0x7FF) as f32 * 0.125;
            if v & (1 << 19) != 0 {
                t -= 49.0; // extended range
            }
            t - self.tctl_offset
        } else {
            self.pawnio.execute("ioctl_read_msr", &[MSR_IA32_PACKAGE_THERM_STATUS], &mut out)?;
            self.tj_max - ((out[0] >> 16) & 0x7F) as f32
        };
        if !(1.0..150.0).contains(&temp) {
            return Err(io::Error::other(format!("implausible temperature {temp}")));
        }
        Ok(temp)
    }
}

impl Drop for CpuSensor {
    fn drop(&mut self) {
        if !self.pci_mutex.is_null() {
            unsafe { CloseHandle(self.pci_mutex) };
        }
    }
}

/// Minimal PawnIO client (same IOCTLs as PawnIOLib).
struct PawnIo(HANDLE);

impl PawnIo {
    fn open() -> io::Result<PawnIo> {
        for path in [r"\\?\GLOBALROOT\Device\PawnIO", r"\\.\PawnIO"] {
            let p = wide(path);
            let h = unsafe {
                CreateFileW(
                    p.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if h != INVALID_HANDLE_VALUE {
                return Ok(PawnIo(h));
            }
        }
        Err(io::Error::last_os_error())
    }

    fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> io::Result<usize> {
        let mut returned = 0u32;
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.as_ptr() as *const c_void,
                input.len() as u32,
                output.as_mut_ptr() as *mut c_void,
                output.len() as u32,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(returned as usize)
    }

    fn load(&self, module: &[u8]) -> io::Result<()> {
        self.ioctl(IOCTL_PIO_LOAD_BINARY, module, &mut []).map(|_| ())
    }

    fn execute(&self, function: &str, input: &[u64], output: &mut [u64]) -> io::Result<usize> {
        let mut buf = vec![0u8; FN_NAME_LENGTH + input.len() * 8];
        let name = function.as_bytes();
        buf[..name.len().min(FN_NAME_LENGTH - 1)].copy_from_slice(&name[..name.len().min(FN_NAME_LENGTH - 1)]);
        for (i, v) in input.iter().enumerate() {
            buf[FN_NAME_LENGTH + i * 8..FN_NAME_LENGTH + i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        let mut out = vec![0u8; output.len() * 8];
        let n = self.ioctl(IOCTL_PIO_EXECUTE_FN, &buf, &mut out)? / 8;
        for (i, v) in output.iter_mut().enumerate().take(n) {
            *v = u64::from_le_bytes(out[i * 8..i * 8 + 8].try_into().unwrap());
        }
        Ok(n)
    }
}

impl Drop for PawnIo {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

// ---------------------------------------------------------------- install

fn service_dir() -> PathBuf {
    let pf = std::env::var_os("ProgramW6432").or_else(|| std::env::var_os("ProgramFiles")).unwrap_or_else(|| "C:\\Program Files".into());
    PathBuf::from(pf).join("OpenLinkHub")
}

/// Remove any existing service and wait until Windows has actually deleted it.
fn remove_service(manager: &ServiceManager) -> Result<(), String> {
    let access = ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE;
    let Ok(service) = manager.open_service(SERVICE_NAME, access) else { return Ok(()) };
    if service.query_status().map(|s| s.current_state != ServiceState::Stopped).unwrap_or(false) {
        let _ = service.stop();
        for _ in 0..50 {
            if service.query_status().map(|s| s.current_state == ServiceState::Stopped).unwrap_or(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    service.delete().map_err(|e| format!("Unable to remove the existing service: {e}"))?;
    drop(service);
    for _ in 0..50 {
        if manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).is_err() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Copy this exe to Program Files and register it as an auto-start LocalSystem service.
fn install() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE)
        .map_err(|e| format!("Run this as administrator ({e})"))?;
    remove_service(&manager)?;

    let dir = service_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let target = dir.join("OpenLinkHubSensors.exe");
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    if source.canonicalize().ok() != target.canonicalize().ok() {
        let mut copied = Err(String::new());
        for _ in 0..20 {
            copied = std::fs::copy(&source, &target).map(|_| ()).map_err(|e| e.to_string());
            if copied.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250)); // old service process still exiting
        }
        copied.map_err(|e| format!("Unable to copy to {}: {e}", target.display()))?;
    }

    let info = ServiceInfo {
        name: SERVICE_NAME.into(),
        display_name: DISPLAY_NAME.into(),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: target,
        launch_arguments: vec![],
        dependencies: vec![],
        account_name: None, // LocalSystem: PawnIO only admits SYSTEM and Administrators
        account_password: None,
    };
    let service = manager
        .create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .map_err(|e| format!("Unable to create the service: {e}"))?;
    let _ = service.set_description(DESCRIPTION);
    service.start::<&str>(&[]).map_err(|e| format!("Unable to start the service: {e}"))?;
    Ok(())
}

fn uninstall() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|e| format!("Run this as administrator ({e})"))?;
    remove_service(&manager)?;
    let dir = service_dir();
    for _ in 0..20 {
        let _ = std::fs::remove_file(dir.join("OpenLinkHubSensors.exe"));
        if !dir.join("OpenLinkHubSensors.exe").exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let _ = std::fs::remove_dir(&dir); // only if empty
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_codes_match_pawnio_header() {
        // CTL_CODE(41394, 0x821/0x841, METHOD_BUFFERED, FILE_ANY_ACCESS)
        assert_eq!(IOCTL_PIO_LOAD_BINARY, 0xA1B2_2084);
        assert_eq!(IOCTL_PIO_EXECUTE_FN, 0xA1B2_2104);
    }

    #[test]
    fn shared_layout_is_stable() {
        assert_eq!(size_of::<Shared>(), 64);
    }

    #[test]
    fn this_cpu_is_detected() {
        let (vendor, family, _, brand) = cpu_vendor();
        assert_ne!(vendor, VENDOR_UNKNOWN);
        assert!(family > 0);
        assert!(!brand.is_empty());
    }
}
