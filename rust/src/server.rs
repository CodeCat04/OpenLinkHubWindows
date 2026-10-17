//! Local web dashboard and JSON API.
//!
//! The server binds to loopback only. State-changing requests must carry an
//! `X-OpenLinkHub` header (which a cross-origin web page can't send without a CORS
//! preflight we never approve) and a localhost `Host` header (blocks DNS rebinding),
//! so websites you visit can't change your fan speeds.

use crate::hub::{manager, RgbOverride};
use crate::rgb::{Profile, RGB_MODES};
use crate::temperatures::{self, TemperatureProfile};
use crate::{app, config, logging, rgb_profiles, sensors};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::Arc;
use std::thread;
use tiny_http::{Header, Method, Request, Response, Server};

const INDEX_HTML: &str = include_str!("../web/index.html");
const WORKERS: usize = 4;
const MAX_BODY: usize = 64 * 1024;

pub fn bind(address: &str, port: u16) -> Result<Arc<Server>, String> {
    Server::http((address, port)).map(Arc::new).map_err(|e| e.to_string())
}

pub fn start(server: Arc<Server>) {
    for i in 0..WORKERS {
        let server = Arc::clone(&server);
        thread::Builder::new()
            .name(format!("http-{i}"))
            .spawn(move || loop {
                match server.recv() {
                    Ok(rq) => handle(rq),
                    Err(e) => {
                        log::warn!("HTTP server stopped: {e}");
                        break;
                    }
                }
            })
            .expect("spawn http worker");
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

fn respond_json(rq: Request, status: u16, body: &Value) {
    let resp = Response::from_string(body.to_string())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"))
        .with_header(header("Cache-Control", "no-store"));
    let _ = rq.respond(resp);
}

fn ok(rq: Request) {
    respond_json(rq, 200, &json!({ "ok": true }));
}

fn fail(rq: Request, status: u16, message: &str) {
    respond_json(rq, status, &json!({ "ok": false, "error": message }));
}

fn host_is_local(rq: &Request) -> bool {
    rq.headers().iter().find(|h| h.field.equiv("Host")).is_some_and(|h| {
        let host = h.value.as_str();
        let name = host.rsplit_once(':').map_or(host, |(n, _)| n);
        matches!(name, "127.0.0.1" | "localhost" | "[::1]")
    })
}

fn has_api_header(rq: &Request) -> bool {
    rq.headers().iter().any(|h| h.field.equiv("X-OpenLinkHub"))
}

fn handle(mut rq: Request) {
    if !host_is_local(&rq) {
        return fail(rq, 403, "Forbidden host");
    }
    let url = rq.url().split('?').next().unwrap_or("/").to_string();
    let method = rq.method().clone();

    match (method, url.as_str()) {
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            let resp = Response::from_string(INDEX_HTML)
                .with_header(header("Content-Type", "text/html; charset=utf-8"))
                .with_header(header("Content-Security-Policy", "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; img-src 'self' data:"))
                .with_header(header("X-Frame-Options", "DENY"));
            let _ = rq.respond(resp);
        }
        (Method::Get, "/api/state") => respond_json(rq, 200, &state()),
        (Method::Get, "/api/logs") => respond_json(rq, 200, &json!(logging::tail())),
        (Method::Post, path) if path.starts_with("/api/") => {
            if !has_api_header(&rq) {
                return fail(rq, 403, "Missing X-OpenLinkHub header");
            }
            let mut body = String::new();
            if rq.as_reader().take(MAX_BODY as u64).read_to_string(&mut body).is_err() {
                return fail(rq, 400, "Unreadable body");
            }
            let body: Value = match serde_json::from_str(if body.is_empty() { "{}" } else { &body }) {
                Ok(v) => v,
                Err(e) => return fail(rq, 400, &format!("Invalid JSON: {e}")),
            };
            match post(path, body) {
                Ok(()) => ok(rq),
                Err(e) => fail(rq, 400, &e),
            }
        }
        _ => fail(rq, 404, "Not found"),
    }
}

fn state() -> Value {
    let cfg = config::get();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "hubs": manager::views(),
        "sensors": sensors::snapshot(),
        "speedProfiles": temperatures::all(),
        "rgbProfiles": rgb_profiles::all(),
        "rgbModes": RGB_MODES,
        "settings": {
            "autostart": app::autostart_enabled(),
            "openBrowser": cfg.open_browser,
            "lhmUrl": cfg.lhm_url,
            "manual": cfg.manual,
            "installed": app::is_installed_copy(),
            "dataDir": "%APPDATA%\\OpenLinkHub",
        },
    })
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("Invalid request: {e}"))
}

fn hub(serial: &str) -> Result<Arc<crate::hub::Hub>, String> {
    manager::get(serial).ok_or_else(|| "Hub not found (disconnected?)".to_string())
}

#[derive(Deserialize)]
struct ChannelsRequest {
    serial: String,
    channels: Vec<u8>,
    profile: String,
}

#[derive(Deserialize)]
struct LabelRequest {
    serial: String,
    channel: u8,
    label: String,
}

#[derive(Deserialize)]
struct OverrideRequest {
    serial: String,
    channel: u8,
    #[serde(rename = "override")]
    value: RgbOverride,
}

#[derive(Deserialize)]
struct BrightnessRequest {
    serial: String,
    value: u8,
}

#[derive(Deserialize)]
struct RgbOffRequest {
    serial: String,
    off: bool,
}

#[derive(Deserialize)]
struct SpeedProfileRequest {
    name: String,
    profile: TemperatureProfile,
}

#[derive(Deserialize)]
struct NameRequest {
    name: String,
}

#[derive(Deserialize)]
struct RgbProfileRequest {
    name: String,
    profile: Profile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsRequest {
    autostart: Option<bool>,
    open_browser: Option<bool>,
    lhm_url: Option<String>,
}

fn post(path: &str, body: Value) -> Result<(), String> {
    match path {
        "/api/hub/speed" => {
            let r: ChannelsRequest = parse(body)?;
            hub(&r.serial)?.set_speed_profile(&r.channels, &r.profile)
        }
        "/api/hub/rgb" => {
            let r: ChannelsRequest = parse(body)?;
            hub(&r.serial)?.set_rgb_profile(&r.channels, &r.profile)
        }
        "/api/hub/override" => {
            let r: OverrideRequest = parse(body)?;
            hub(&r.serial)?.set_rgb_override(r.channel, r.value)
        }
        "/api/hub/label" => {
            let r: LabelRequest = parse(body)?;
            hub(&r.serial)?.set_label(r.channel, &r.label)
        }
        "/api/hub/brightness" => {
            let r: BrightnessRequest = parse(body)?;
            hub(&r.serial)?.set_brightness(r.value);
            Ok(())
        }
        "/api/hub/rgbOff" => {
            let r: RgbOffRequest = parse(body)?;
            hub(&r.serial)?.set_rgb_off(r.off);
            Ok(())
        }
        "/api/speedProfile" => {
            let r: SpeedProfileRequest = parse(body)?;
            temperatures::upsert(&r.name, r.profile)
        }
        "/api/speedProfile/delete" => {
            let r: NameRequest = parse(body)?;
            if manager::views().iter().flat_map(|h| &h.devices).any(|d| d.speed_profile == r.name) {
                return Err("Profile is in use by a device; switch those devices first".into());
            }
            temperatures::delete(&r.name)
        }
        "/api/rgbProfile" => {
            let r: RgbProfileRequest = parse(body)?;
            rgb_profiles::update(&r.name, r.profile)?;
            manager::refresh_rgb();
            Ok(())
        }
        "/api/settings" => {
            let r: SettingsRequest = parse(body)?;
            if r.lhm_url.as_deref().is_some_and(|u| !config::is_loopback_url(u)) {
                return Err("Only local addresses are allowed, e.g. http://127.0.0.1:8085/data.json. OpenLinkHub never connects to other machines.".into());
            }
            if let Some(a) = r.autostart {
                app::set_autostart(a).map_err(|e| e.to_string())?;
            }
            config::update(|c| {
                if let Some(v) = r.open_browser {
                    c.open_browser = v;
                }
                if let Some(v) = r.lhm_url {
                    c.lhm_url = v.trim().to_string();
                }
            })
            .map_err(|e| e.to_string())
        }
        "/api/quit" => {
            app::request_shutdown();
            Ok(())
        }
        _ => Err("Unknown endpoint".into()),
    }
}
