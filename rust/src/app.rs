//! Windows integration: per-user install, autostart, Start menu entry, tray icon, browser.
//!
//! Everything is per-user (HKCU, %LOCALAPPDATA%), so nothing here needs administrator rights.

use crate::icon;
use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MessageBoxW, PeekMessageW, TranslateMessage, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK,
    MB_YESNO, MSG, PM_REMOVE, SW_SHOWNORMAL,
};
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

pub const APP_NAME: &str = "OpenLinkHub";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenLinkHub";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

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

pub fn install_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Programs").join(APP_NAME)
}

pub fn installed_exe() -> PathBuf {
    install_dir().join("OpenLinkHub.exe")
}

fn start_menu_shortcut() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\OpenLinkHub.lnk"))
}

pub fn current_exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("OpenLinkHub.exe"))
}

pub fn is_installed_copy() -> bool {
    same_path(&current_exe(), &installed_exe())
}

/// True when this executable is a different build from the installed one (an update).
pub fn differs_from_installed() -> bool {
    match (std::fs::read(current_exe()), std::fs::read(installed_exe())) {
        (Ok(a), Ok(b)) => a != b,
        _ => false,
    }
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

/// Copy this executable to %LOCALAPPDATA%\Programs\OpenLinkHub and register it
/// (autostart, Start menu shortcut, "Apps & features" entry). Returns the installed path.
pub fn install() -> io::Result<PathBuf> {
    let target = installed_exe();
    std::fs::create_dir_all(install_dir())?;
    if !same_path(&current_exe(), &target) {
        std::fs::copy(current_exe(), &target)?;
    }
    let exe = target.display().to_string();

    let (run, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
    run.set_value(APP_NAME, &format!("\"{exe}\" --background"))?;

    let (uninstall, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY)?;
    uninstall.set_value("DisplayName", &"OpenLinkHub")?;
    uninstall.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION"))?;
    uninstall.set_value("Publisher", &"OpenLinkHub contributors")?;
    uninstall.set_value("DisplayIcon", &exe)?;
    uninstall.set_value("InstallLocation", &install_dir().display().to_string())?;
    uninstall.set_value("UninstallString", &format!("\"{exe}\" --uninstall"))?;
    uninstall.set_value("NoModify", &1u32)?;
    uninstall.set_value("NoRepair", &1u32)?;

    if let Some(lnk) = start_menu_shortcut() {
        // WScript.Shell is the only dependency-free way to write a .lnk.
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.Description='Corsair iCUE LINK control';$s.Save()",
            ps_quote(&lnk.display().to_string()),
            ps_quote(&exe),
            ps_quote(&install_dir().display().to_string()),
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .status();
    }
    log::info!("Installed to {exe}");
    Ok(target)
}

fn ps_quote(s: &str) -> String {
    s.replace('\'', "''")
}

/// Remove registry entries and shortcut, then delete the installed executable once this
/// process exits. User settings in %APPDATA%\OpenLinkHub are kept.
pub fn uninstall() -> io::Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_SET_VALUE) {
        let _ = run.delete_value(APP_NAME);
    }
    let _ = hkcu.delete_subkey_all(UNINSTALL_KEY);
    if let Some(lnk) = start_menu_shortcut() {
        let _ = std::fs::remove_file(lnk);
    }
    let dir = install_dir();
    if dir.exists() {
        // A running exe can't delete itself; let a detached cmd do it after we exit.
        // raw_arg: cmd.exe doesn't understand the \" escaping that .arg() would apply.
        let _ = Command::new("cmd")
            .raw_arg(format!("/C ping -n 3 127.0.0.1 >nul & rmdir /S /Q \"{}\"", dir.display()))
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }
    Ok(())
}

pub fn open_url(url: &str) {
    let (verb, file) = (wide("open"), wide(url));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

pub fn message_box(text: &str, question: bool) -> bool {
    let (t, c) = (wide(text), wide(APP_NAME));
    let style = if question { MB_YESNO | MB_ICONQUESTION } else { MB_OK | MB_ICONINFORMATION };
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), style) == IDYES }
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
