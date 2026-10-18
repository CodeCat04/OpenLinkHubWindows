//! Dashboard page and JSON API, served to the dashboard window through a WebView2 custom
//! scheme (`http://olh.localhost/`). Requests go straight from WebView2 to this function
//! in-process: no port is opened and nothing touches the network stack.

use crate::hub::{manager, RgbOverride};
use crate::rgb::{Profile, RGB_MODES};
use crate::temperatures::{self, TemperatureProfile};
use crate::{app, config, logging, rgb_profiles, sensors};
use serde::Deserialize;
use serde_json::{json, Value};
use std::borrow::Cow;
use wry::http::{Method, Request, Response};

const INDEX_HTML: &str = include_str!("../web/index.html");
const MAX_BODY: usize = 64 * 1024;
const CSP: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'; img-src 'self' data:";

type Reply = Response<Cow<'static, [u8]>>;

fn reply(status: u16, content_type: &str, body: Vec<u8>) -> Reply {
    Response::builder()
        .status(status)
        .header("Content-Type", content_type)
        .header("Cache-Control", "no-store")
        .header("Content-Security-Policy", CSP)
        .body(Cow::Owned(body))
        .expect("valid response")
}

fn json_reply(status: u16, body: &Value) -> Reply {
    reply(status, "application/json", body.to_string().into_bytes())
}

fn fail(status: u16, message: &str) -> Reply {
    json_reply(status, &json!({ "ok": false, "error": message }))
}

/// Handle one request from the dashboard window.
pub fn handle(request: Request<Vec<u8>>) -> Reply {
    let path = request.uri().path().to_string();
    match (request.method(), path.as_str()) {
        (&Method::GET, "/") | (&Method::GET, "/index.html") => reply(200, "text/html; charset=utf-8", INDEX_HTML.as_bytes().to_vec()),
        (&Method::GET, "/api/state") => json_reply(200, &state()),
        (&Method::GET, "/api/logs") => json_reply(200, &json!(logging::tail())),
        (&Method::POST, p) if p.starts_with("/api/") => {
            // Defence in depth: only the dashboard's own fetch() sets this header.
            if !request.headers().contains_key("X-OpenLinkHub") {
                return fail(403, "Missing X-OpenLinkHub header");
            }
            let body = request.body();
            if body.len() > MAX_BODY {
                return fail(413, "Request too large");
            }
            let body: Value = match serde_json::from_slice(if body.is_empty() { b"{}" } else { body }) {
                Ok(v) => v,
                Err(e) => return fail(400, &format!("Invalid JSON: {e}")),
            };
            match post(p, body) {
                Ok(()) => json_reply(200, &json!({ "ok": true })),
                Err(e) => fail(400, &e),
            }
        }
        _ => fail(404, "Not found"),
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
        "linkAdapters": crate::hub::adapters::choices(),
        "lcd": {
            "modes": crate::lcd::modes(true).into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "rotations": crate::lcd::rotations().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "brightness": crate::lcd::brightness_levels().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect::<Vec<_>>(),
            "images": crate::lcd::images(),
        },
        "settings": {
            "autostart": app::autostart_enabled(),
            "openDashboard": cfg.open_dashboard,
            "installed": app::is_installed_copy(),
            "dataDir": "%APPDATA%\\OpenLinkHub",
        },
    })
}

fn parse<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("Invalid request: {e}"))
}

fn hub(serial: &str) -> Result<std::sync::Arc<crate::hub::Hub>, String> {
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
struct AdapterRequest {
    serial: String,
    channel: u8,
    adapter: u32,
}

#[derive(Deserialize)]
struct SubRgbRequest {
    serial: String,
    channel: u8,
    part: u8,
    profile: String,
}

#[derive(Deserialize)]
struct LcdRequest {
    serial: String,
    channel: u8,
    settings: crate::lcd::LcdSettings,
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
    open_dashboard: Option<bool>,
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
        "/api/hub/adapter" => {
            let r: AdapterRequest = parse(body)?;
            hub(&r.serial)?.set_link_adapter(r.channel, r.adapter)
        }
        "/api/hub/subRgb" => {
            let r: SubRgbRequest = parse(body)?;
            hub(&r.serial)?.set_sub_rgb_profile(r.channel, r.part, &r.profile)
        }
        "/api/hub/lcd" => {
            let r: LcdRequest = parse(body)?;
            hub(&r.serial)?.set_lcd(r.channel, r.settings)
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
            if let Some(a) = r.autostart {
                app::set_autostart(a).map_err(|e| e.to_string())?;
            }
            config::update(|c| {
                if let Some(v) = r.open_dashboard {
                    c.open_dashboard = v;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn post_req(path: &str, body: &str, with_header: bool) -> Request<Vec<u8>> {
        let mut b = Request::builder().method("POST").uri(format!("http://olh.localhost{path}"));
        if with_header {
            b = b.header("X-OpenLinkHub", "1");
        }
        b.body(body.as_bytes().to_vec()).unwrap()
    }

    #[test]
    fn serves_page_and_rejects_bad_requests() {
        let page = handle(Request::get("http://olh.localhost/").body(Vec::new()).unwrap());
        assert_eq!(page.status(), 200);
        assert!(std::str::from_utf8(page.body()).unwrap().contains("<title>OpenLinkHub</title>"));
        assert_eq!(handle(post_req("/api/quit", "{}", false)).status(), 403);
        assert_eq!(handle(post_req("/api/hub/rgb", "{bad", true)).status(), 400);
        assert_eq!(handle(Request::get("http://olh.localhost/nope").body(Vec::new()).unwrap()).status(), 404);
    }
}
