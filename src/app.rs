//! Windows integration: tray icon, dashboard window, single instance, autostart.
//!
//! The dashboard is a native window hosting WebView2, which talks to `server::handle`
//! in-process through a custom scheme. Instances coordinate through named Windows objects
//! (a mutex and two events), so the app has no network listener of any kind.

use crate::{hub, icon, server};
use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, OpenMutexW, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
    EVENT_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows};

pub const APP_NAME: &str = "OpenLinkHub";
/// Same value name the installer's "Start with Windows" task writes.
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const DASHBOARD_URL: &str = "http://olh.localhost/";
/// The only page outside the app the dashboard may open (in the default browser).
const PROJECT_URL: &str = "https://github.com/CodeCat04/OpenLinkHubWindows";

const INSTANCE_MUTEX: &str = "Local\\OpenLinkHub.Instance";
const SHOW_EVENT: &str = "Local\\OpenLinkHub.Show";
const STOP_EVENT: &str = "Local\\OpenLinkHub.Stop";

/// Edge features that would otherwise contact Microsoft services in the background.
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
     --disable-background-networking --disable-component-update --disable-domain-reliability --no-pings";

#[derive(Debug)]
enum UserEvent {
    Menu(MenuEvent),
    Tray(TrayIconEvent),
    Show,
    Quit,
}

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static PROXY: OnceLock<Mutex<EventLoopProxy<UserEvent>>> = OnceLock::new();

/// Ask the app to quit (tray menu, dashboard, Ctrl+C, `--stop`).
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
    if let Some(p) = PROXY.get() {
        let _ = p.lock().unwrap().send_event(UserEvent::Quit);
    }
}

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------- single instance

/// Held for the life of the primary instance.
pub struct Instance {
    _mutex: HANDLE,
    show: HANDLE,
    stop: HANDLE,
}

unsafe impl Send for Instance {}

impl Instance {
    /// Become the primary instance, or `None` if OpenLinkHub is already running.
    pub fn acquire() -> Option<Instance> {
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 1, wide(INSTANCE_MUTEX).as_ptr());
            if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
                if !mutex.is_null() {
                    CloseHandle(mutex);
                }
                return None;
            }
            let show = CreateEventW(std::ptr::null(), 0, 0, wide(SHOW_EVENT).as_ptr());
            let stop = CreateEventW(std::ptr::null(), 0, 0, wide(STOP_EVENT).as_ptr());
            Some(Instance { _mutex: mutex, show, stop })
        }
    }
}

fn signal(event: &str) -> bool {
    unsafe {
        let h = OpenEventW(EVENT_MODIFY_STATE, 0, wide(event).as_ptr());
        if h.is_null() {
            return false;
        }
        SetEvent(h);
        CloseHandle(h);
        true
    }
}

/// Bring the running instance's dashboard to the front.
pub fn show_running_instance() {
    signal(SHOW_EVENT);
}

/// Ask a running instance to quit and wait (up to 15 s) until it has exited, so its files
/// can be replaced and the hubs are back in hardware mode. Used by the installer.
pub fn stop_running_instance() {
    if !signal(STOP_EVENT) {
        return; // not running
    }
    unsafe {
        let m = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, 0, wide(INSTANCE_MUTEX).as_ptr());
        if !m.is_null() {
            // Returns WAIT_ABANDONED once the owning process exits.
            WaitForSingleObject(m, 15_000);
            CloseHandle(m);
        }
    }
    // The mutex is released as the process tears down; give it a moment to unlock the exe.
    std::thread::sleep(Duration::from_millis(500));
}

// ---------------------------------------------------------------- autostart

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

/// Open the project page in the default browser. Nothing else may leave the app.
fn open_external(url: &str) {
    if !url.starts_with(PROJECT_URL) {
        log::warn!("Blocked navigation to {url}");
        return;
    }
    let (verb, file) = (wide("open"), wide(url));
    unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

// ---------------------------------------------------------------- event loop

struct Dashboard {
    webview: WebView,
    window: Window,
}

fn open_dashboard(target: &EventLoopWindowTarget<UserEvent>, context: &mut WebContext) -> Option<Dashboard> {
    let (rgba, size) = icon::rgba(32);
    let window = WindowBuilder::new()
        .with_title("OpenLinkHub")
        .with_inner_size(LogicalSize::new(1240.0, 900.0))
        .with_min_inner_size(LogicalSize::new(720.0, 520.0))
        .with_window_icon(tao::window::Icon::from_rgba(rgba, size, size).ok())
        .build(target)
        .map_err(|e| log::error!("Unable to create the dashboard window: {e}"))
        .ok()?;
    let webview = WebViewBuilder::new_with_web_context(context)
        .with_custom_protocol("olh".into(), |_id, request| server::handle(request))
        .with_url(DASHBOARD_URL)
        .with_navigation_handler(|url| {
            if url.starts_with(DASHBOARD_URL) {
                return true;
            }
            open_external(&url);
            false
        })
        .with_new_window_req_handler(|url, _features| {
            open_external(&url);
            NewWindowResponse::Deny
        })
        .with_devtools(cfg!(debug_assertions))
        .with_background_color((15, 16, 19, 255))
        .with_additional_browser_args(BROWSER_ARGS)
        .with_default_context_menus(cfg!(debug_assertions))
        .build(&window)
        .map_err(|e| {
            log::error!("Unable to start WebView2 ({e}). Is the Microsoft Edge WebView2 Runtime installed?");
        })
        .ok()?;
    Some(Dashboard { webview, window })
}

fn build_tray(autostart: &CheckMenuItem, open: &MenuItem, quit: &MenuItem) -> Option<TrayIcon> {
    let menu = Menu::new();
    let _ = menu.append_items(&[open, &PredefinedMenuItem::separator(), autostart, &PredefinedMenuItem::separator(), quit]);
    let (rgba, size) = icon::rgba(32);
    TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip("OpenLinkHub")
        .with_icon(tray_icon::Icon::from_rgba(rgba, size, size).expect("valid icon"))
        .build()
        .map_err(|e| log::warn!("Tray icon unavailable: {e}"))
        .ok()
}

/// Run the tray and window event loop on the main thread. Never returns.
pub fn run(instance: Instance, show_dashboard: bool) -> ! {
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let _ = PROXY.set(Mutex::new(proxy.clone()));
    if SHUTDOWN.load(Ordering::SeqCst) {
        let _ = proxy.send_event(UserEvent::Quit);
    }

    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Menu(e));
        }));
        let p = proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Tray(e));
        }));
    }

    // Other instances (a second launch, or the installer's --stop) signal named events.
    let (show_ev, stop_ev) = (instance.show as usize, instance.stop as usize);
    std::thread::Builder::new()
        .name("instance".into())
        .spawn(move || loop {
            let handles = [show_ev as HANDLE, stop_ev as HANDLE];
            let r = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            if r == WAIT_OBJECT_0 {
                let _ = proxy.send_event(UserEvent::Show);
            } else if r == WAIT_OBJECT_0 + 1 {
                request_shutdown();
                break;
            } else {
                break;
            }
        })
        .expect("spawn instance thread");

    let mut context = WebContext::new(Some(crate::config::path("webview")));
    let open = MenuItem::new("Open dashboard", true, None);
    let autostart = CheckMenuItem::new("Start with Windows", true, autostart_enabled(), None);
    let quit = MenuItem::new("Quit (return lighting to hardware mode)", true, None);
    let mut tray: Option<TrayIcon> = None;
    let mut dashboard: Option<Dashboard> = None;
    let _instance = instance;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                tray = build_tray(&autostart, &open, &quit);
                if show_dashboard {
                    dashboard = open_dashboard(target, &mut context);
                }
            }
            Event::UserEvent(UserEvent::Show) => match &dashboard {
                Some(d) => {
                    d.window.set_minimized(false);
                    d.window.set_visible(true);
                    d.window.set_focus();
                }
                None => dashboard = open_dashboard(target, &mut context),
            },
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == open.id() {
                    let _ = PROXY.get().map(|p| p.lock().unwrap().send_event(UserEvent::Show));
                } else if e.id == quit.id() {
                    request_shutdown();
                } else if e.id == autostart.id() {
                    if let Err(err) = set_autostart(autostart.is_checked()) {
                        log::error!("Unable to change autostart: {err}");
                    }
                    autostart.set_checked(autostart_enabled());
                }
            }
            Event::UserEvent(UserEvent::Tray(TrayIconEvent::DoubleClick { button: MouseButton::Left, .. })) => {
                let _ = PROXY.get().map(|p| p.lock().unwrap().send_event(UserEvent::Show));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                // Destroy the WebView so the browser engine's memory is released while the
                // dashboard is closed; the app keeps running in the tray.
                if let Some(d) = dashboard.take() {
                    drop(d.webview);
                    drop(d.window);
                }
            }
            Event::UserEvent(UserEvent::Quit) => {
                dashboard = None;
                tray = None;
                log::info!("Shutting down");
                hub::manager::stop_all();
                log::logger().flush();
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
        let _ = &tray;
    })
}
