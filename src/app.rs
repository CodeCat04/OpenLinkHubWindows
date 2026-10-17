//! Windows integration: autostart, tray icon, browser.
//!
//! Installation (Start menu, Apps entry, uninstall) is handled by the Inno Setup installer
//! in `installer/`. Everything here is per-user (HKCU), so nothing needs administrator rights.

use crate::icon;
use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, SW_SHOWNORMAL};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

pub const APP_NAME: &str = "OpenLinkHub";
/// Same value name the installer's "Start with Windows" task writes.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

pub fn shutdown_requested() -> bool {
    SHUTDOWN.load(Ordering::SeqCst)
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

/// Where the installer puts the app (Inno Setup `{autopf}` for a per-user install).
fn installed_exe() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Programs").join(APP_NAME).join("OpenLinkHub.exe")
}

fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("OpenLinkHub.exe"))
}

pub fn is_installed_copy() -> bool {
    same_path(&current_exe(), &installed_exe())
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.as_os_str().eq_ignore_ascii_case(b.as_os_str()),
    }
}

pub fn autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(APP_NAME))
        .is_ok()
}

/// Start with Windows, pointing at the running executable.
pub fn set_autostart(enabled: bool) -> io::Result<()> {
    let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    if enabled {
        run.set_value(APP_NAME, &format!("\"{}\" --background", current_exe().display()))
    } else {
        match run.delete_value(APP_NAME) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        }
    }
}

pub fn open_url(url: &str) {
    let (verb, file) = (wide("open"), wide(url));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

/// Run the tray icon and Win32 message loop on the calling (main) thread until shutdown.
pub fn run_tray(dashboard_url: &str) {
    let open = MenuItem::new("Open dashboard", true, None);
    let autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None);
    let quit = MenuItem::new("Quit (return lighting to hardware mode)", true, None);
    let menu = Menu::new();
    let _ = menu.append_items(&[&open, &PredefinedMenuItem::separator(), &autostart, &PredefinedMenuItem::separator(), &quit]);

    let (rgba, size) = icon::rgba(32);
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("OpenLinkHub")
        .with_icon(Icon::from_rgba(rgba, size, size).expect("valid icon"))
        .build();
    let _tray = match tray {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("Tray icon unavailable: {e}");
            None
        }
    };

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while !shutdown_requested() {
        unsafe {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == open.id() {
                open_url(dashboard_url);
            } else if event.id == quit.id() {
                request_shutdown();
            } else if event.id == autostart.id() {
                if let Err(e) = set_autostart(autostart.is_checked()) {
                    log::error!("Unable to change autostart: {e}");
                }
                autostart.set_checked(autostart_enabled());
            }
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event {
                open_url(dashboard_url);
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
}
