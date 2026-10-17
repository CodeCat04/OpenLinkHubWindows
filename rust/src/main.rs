//! OpenLinkHub for Windows: control Corsair iCUE LINK System Hubs without iCUE.
//!
//! Usage: `OpenLinkHub.exe [--background] [--install] [--uninstall]`

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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a.eq_ignore_ascii_case(f));

    let cfg = config::init();
    logging::init(cfg.debug);
    log::info!("OpenLinkHub {} starting", env!("CARGO_PKG_VERSION"));

    let quiet = flag("--quiet");
    let notify = |text: &str| {
        if !quiet {
            app::message_box(text, false);
        }
    };

    if flag("--uninstall") {
        stop_running_instance(&cfg);
        return match app::uninstall() {
            Ok(()) => {
                notify("OpenLinkHub has been uninstalled.\n\nYour settings are kept in %APPDATA%\\OpenLinkHub; delete that folder to remove them.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                notify(&format!("Uninstall failed: {e}"));
                ExitCode::FAILURE
            }
        };
    }

    // Install / update: explicit flag, or offered when this copy isn't the installed one.
    if !app::is_installed_copy() {
        let installed = app::installed_exe();
        let wants_install = if flag("--install") {
            true
        } else if quiet || flag("--background") {
            false
        } else if installed.exists() {
            app::differs_from_installed()
                && app::message_box(
                    &format!(
                        "Update the installed OpenLinkHub to version {}?\n\n\
                         The running copy will be stopped and replaced. Your settings are kept.",
                        env!("CARGO_PKG_VERSION")
                    ),
                    true,
                )
        } else if !cfg.install_declined {
            let yes = app::message_box(
                "Install OpenLinkHub?\n\n\
                 It will be copied to your user profile, start automatically with Windows, \
                 and appear in the Start menu and in Settings > Apps for easy removal. \
                 No administrator rights are needed.\n\n\
                 Choose No to just run it from here (you won't be asked again).",
                true,
            );
            if !yes {
                let _ = config::update(|c| c.install_declined = true);
            }
            yes
        } else {
            false
        };

        if wants_install {
            stop_running_instance(&cfg);
            match app::install() {
                Ok(path) => {
                    let mut cmd = std::process::Command::new(&path);
                    if quiet {
                        cmd.arg("--background");
                    }
                    let _ = cmd.spawn();
                    return ExitCode::SUCCESS;
                }
                Err(e) => {
                    log::error!("Install failed: {e}");
                    notify(&format!("Install failed: {e}\n\nOpenLinkHub will run without installing."));
                }
            }
        }
    }

    let dashboard = format!("http://127.0.0.1:{}/", cfg.listen_port);

    // Single instance: if the port is taken we're already running, so just show the dashboard.
    let server = match server::bind(&cfg.listen_address, cfg.listen_port) {
        Ok(s) => s,
        Err(e) => {
            log::info!("Port {} unavailable ({e}); assuming OpenLinkHub is already running", cfg.listen_port);
            if !flag("--background") {
                app::open_url(&dashboard);
            }
            return ExitCode::SUCCESS;
        }
    };

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

/// Ask a running instance to quit (before install/uninstall) and wait until it has released
/// the hubs, its port and its executable.
fn stop_running_instance(cfg: &config::Config) {
    use std::time::{Duration, Instant};
    let url = format!("http://127.0.0.1:{}/api/quit", cfg.listen_port);
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(2)).redirects(0).build();
    if agent.post(&url).set("X-OpenLinkHub", "1").send_string("{}").is_err() {
        return; // not running
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && server::bind(&cfg.listen_address, cfg.listen_port).is_err() {
        std::thread::sleep(Duration::from_millis(200));
    }
    // The port is released before the process fully exits; give it a moment to unlock the exe.
    std::thread::sleep(Duration::from_millis(800));
}
