//! OpenLinkHub for Windows: control Corsair iCUE LINK System Hubs without iCUE.
//!
//! Usage: `OpenLinkHub.exe [--background] [--stop]`
//! - `--background`: don't open the dashboard (used when starting with Windows)
//! - `--stop`: ask a running OpenLinkHub to quit, and wait for it (used by the installer)

// Release builds are a GUI app (tray icon, no console window). Debug builds keep the console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod hub;
mod icon;
mod lcd;
mod logging;
mod profiles;
mod rgb;
mod rgb_profiles;
mod sensors;
mod server;
mod temperatures;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a.eq_ignore_ascii_case(f));

    if flag("--stop") {
        app::stop_running_instance();
        return;
    }

    // Single instance: a second launch just brings up the running dashboard. This happens
    // before logging starts so a second launch never touches the log file.
    let Some(instance) = app::Instance::acquire() else {
        if !flag("--background") {
            app::show_running_instance();
        }
        return;
    };

    let cfg = config::init();
    logging::init(cfg.debug);
    log::info!("OpenLinkHub {} starting", env!("CARGO_PKG_VERSION"));
    let _ = ctrlc::set_handler(app::request_shutdown);

    sensors::start();
    hub::manager::start();
    app::run(instance, cfg.open_dashboard && !flag("--background"));
}
