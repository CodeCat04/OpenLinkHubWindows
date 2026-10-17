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
mod logging;
mod rgb;
mod rgb_profiles;
mod sensors;
mod server;
mod temperatures;

use std::process::ExitCode;
use std::time::{Duration, Instant};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a.eq_ignore_ascii_case(f));
    let cfg = config::init();

    if flag("--stop") {
        stop_running_instance(&cfg);
        return ExitCode::SUCCESS;
    }

    let dashboard = format!("http://127.0.0.1:{}/", cfg.listen_port);

    // Single instance: if the port is taken we're already running, so just show the dashboard.
    // This check happens before logging starts so a second launch never touches the log file.
    let server = match server::bind(&cfg.listen_address, cfg.listen_port) {
        Ok(s) => s,
        Err(_) => {
            if !flag("--background") {
                app::open_url(&dashboard);
            }
            return ExitCode::SUCCESS;
        }
    };

    logging::init(cfg.debug);
    log::info!("OpenLinkHub {} starting", env!("CARGO_PKG_VERSION"));
    let _ = ctrlc::set_handler(app::request_shutdown);

    sensors::start();
    hub::manager::start();
    server::start(server);
    log::info!("Dashboard at {dashboard}");

    if cfg.open_browser && !flag("--background") {
        app::open_url(&dashboard);
    }

    app::run_tray(&dashboard);

    log::info!("Shutting down");
    hub::manager::stop_all();
    log::logger().flush();
    ExitCode::SUCCESS
}

/// Ask a running instance to quit and wait until it has released the hubs, its port and
/// its executable (so the installer can replace or remove it).
fn stop_running_instance(cfg: &config::Config) {
    let url = format!("http://127.0.0.1:{}/api/quit", cfg.listen_port);
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).redirects(0).build();
    if agent.post(&url).set("X-OpenLinkHub", "1").send_string("{}").is_err() {
        return; // not running
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && server::bind(&cfg.listen_address, cfg.listen_port).is_err() {
        std::thread::sleep(Duration::from_millis(200));
    }
    // The port is released just before the process exits; give it a moment to unlock the exe.
    std::thread::sleep(Duration::from_millis(800));
}
