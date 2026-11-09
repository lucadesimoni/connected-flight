// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Web view, JSON API and the Iridium SBD webhook.
//!
//! | Route | Auth | |
//! |---|---|---|
//! | `GET /` | view token | all boxes: last contact, last position, last health |
//! | `GET /box?id=…` | view token | map (track) + events of one box |
//! | `GET /api/boxes`, `/api/track?box=&limit=`, `/api/events?box=&limit=` | view token | JSON |
//! | `POST /sbd?token=…` | webhook token | Iridium service-provider delivery |
//!
//! View token: `?token=` or `Authorization: Bearer`; none configured = open
//! (bind to localhost / tunnel then). The webhook always needs its own
//! token. Put a TLS reverse proxy in front for anything public.

use crate::store::Store;
use cfb_core::urlenc;
use serde_json::Value;
use std::io::Read;
use std::sync::{Arc, Mutex};
use tiny_http::{Header, Method, Response, Server};

pub struct WebConfig {
    pub view_token: Option<String>,
    pub webhook_token: Option<String>,
}

pub struct Reply {
    pub code: u16,
    pub content_type: &'static str,
    pub body: String,
}

const MAX_BODY: u64 = 16 * 1024;

fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            c => c.to_string(),
        })
        .collect()
}

fn token_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut d = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        d |= (*a.get(i).unwrap_or(&0) ^ *b.get(i).unwrap_or(&0)) as usize;
    }
    d == 0 && !b.is_empty()
}

fn fmt_time(unix: i64) -> String {
    // UTC, without a date library: days since epoch -> civil date.
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

const STYLE: &str = "body{font:15px system-ui,sans-serif;margin:16px;max-width:1100px;color:#111;background:#fafafa}\
table{border-collapse:collapse;width:100%}td,th{border-bottom:1px solid #ddd;padding:4px 6px;text-align:left;vertical-align:top}\
#map{height:420px;margin:12px 0;border:1px solid #ccc}a{color:#05c}";

fn overview(store: &Store, q: &str) -> rusqlite::Result<String> {
    let mut rows = String::new();
    for b in store.boxes()? {
        let id = b["box"].as_str().unwrap_or("");
        let pos = &b["last_position"];
        let pos_txt = if pos.is_null() {
            "–".to_string()
        } else {
            format!(
                "{:.5}, {:.5} · {} ft · {} kt{}",
                pos["lat"].as_f64().unwrap_or(0.0),
                pos["lon"].as_f64().unwrap_or(0.0),
                pos["alt_ft"],
                pos["gs_kt"],
                if pos["airborne"].as_bool() == Some(true) { " · airborne" } else { "" }
            )
        };
        rows.push_str(&format!(
            "<tr><td><a href=\"/box?id={}{q}\">{}</a></td><td>{}</td><td>{}</td><td>{}</td><td>{} ({} dup)</td><td>{}</td></tr>",
            urlenc_component(id),
            esc(id),
            fmt_time(b["last_rx"].as_i64().unwrap_or(0)),
            esc(&pos_txt),
            esc(b["last_health"].as_str().unwrap_or("–")),
            b["frames"],
            b["duplicates"],
            esc(b["bearers"].as_str().unwrap_or("")),
        ));
    }
    Ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta http-equiv=\"refresh\" content=\"30\"><title>CFB ground</title><style>{STYLE}</style></head><body>\
<h1>Connected Flight Box – fleet</h1><table><tr><th>Box</th><th>Last contact</th><th>Last position</th><th>Last health</th>\
<th>Frames</th><th>Links</th></tr>{rows}</table></body></html>"
    ))
}

fn urlenc_component(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn box_page(store: &Store, id: &str, q: &str) -> rusqlite::Result<String> {
    let track = store.track(id, 5000)?;
    let events = store.events(id, 300)?;
    let pts: Vec<Value> = track.iter().map(|p| serde_json::json!([p["lat"], p["lon"]])).collect();
    // JSON inside <script>: escape '<' so data can't close the script tag.
    let pts_js = serde_json::to_string(&pts).unwrap_or_else(|_| "[]".into()).replace('<', "\\u003c");
    let mut ev = String::new();
    for e in &events {
        ev.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            fmt_time(e["time"].as_i64().unwrap_or(0)),
            esc(e["type"].as_str().unwrap_or("")),
            esc(e["bearer"].as_str().unwrap_or("")),
            esc(e["body"].as_str().unwrap_or(""))
        ));
    }
    Ok(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>CFB {id_e}</title><style>{STYLE}</style>\
<link rel=\"stylesheet\" href=\"https://cdnjs.cloudflare.com/ajax/libs/leaflet/1.9.4/leaflet.min.css\">\
<script src=\"https://cdnjs.cloudflare.com/ajax/libs/leaflet/1.9.4/leaflet.min.js\"></script></head><body>\
<p><a href=\"/?{qs}\">← fleet</a></p><h1>{id_e}</h1><p>{n} positions (newest 5000)</p><div id=\"map\"></div>\
<script>var pts={pts_js};if(window.L&&pts.length){{var m=L.map('map');\
L.tileLayer('https://tile.openstreetmap.org/{{z}}/{{x}}/{{y}}.png',{{maxZoom:18,attribution:'&copy; OpenStreetMap contributors'}}).addTo(m);\
var l=L.polyline(pts,{{color:'#c03'}}).addTo(m);L.circleMarker(pts[pts.length-1],{{radius:6}}).addTo(m);m.fitBounds(l.getBounds(),{{maxZoom:13}});}}\
else{{document.getElementById('map').textContent=pts.length?'map library not loaded (offline?)':'no positions yet';}}</script>\
<h2>Events (newest first)</h2><table><tr><th>Time</th><th>Type</th><th>Link</th><th>Content</th></tr>{ev}</table></body></html>",
        id_e = esc(id),
        qs = q.trim_start_matches('&'),
        n = track.len(),
    ))
}

/// Request handling without I/O (testable).
pub fn handle(store: &Store, cfg: &WebConfig, method: &Method, url: &str, bearer: Option<&str>, body: &str) -> Reply {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let qf = urlenc::parse_form(query);
    let html = |code, body| Reply { code, content_type: "text/html; charset=utf-8", body };
    let json = |v: Value| Reply { code: 200, content_type: "application/json", body: v.to_string() };

    if (method, path) == (&Method::Post, "/sbd") {
        let ok = cfg.webhook_token.as_deref().is_some_and(|t| token_eq(urlenc::get(&qf, "token"), t));
        if !ok {
            return html(403, "forbidden".into());
        }
        let form = urlenc::parse_form(body);
        return match crate::receive::sbd_webhook(store, &form) {
            Ok(n) => Reply { code: 200, content_type: "text/plain", body: format!("OK {n}") },
            // 200 with an error note: a 4xx would make the provider retry
            // a message that will never parse.
            Err(e) => Reply { code: 200, content_type: "text/plain", body: format!("IGNORED {e}") },
        };
    }

    if let Some(t) = &cfg.view_token {
        let given = bearer.unwrap_or_else(|| urlenc::get(&qf, "token"));
        if !token_eq(given, t) {
            return html(401, "token required".into());
        }
    }
    // Keep the view token on generated links.
    let q = match &cfg.view_token {
        Some(t) => format!("&token={}", urlenc_component(t)),
        None => String::new(),
    };
    let limit = urlenc::get(&qf, "limit").parse::<i64>().unwrap_or(1000).clamp(1, 20_000);
    let r = match (method, path) {
        (Method::Get, "/") => overview(store, &q).map(|b| html(200, b)),
        (Method::Get, "/box") => box_page(store, urlenc::get(&qf, "id"), &q).map(|b| html(200, b)),
        (Method::Get, "/api/boxes") => store.boxes().map(|v| json(Value::Array(v))),
        (Method::Get, "/api/track") => store.track(urlenc::get(&qf, "box"), limit).map(|v| json(Value::Array(v))),
        (Method::Get, "/api/events") => store.events(urlenc::get(&qf, "box"), limit).map(|v| json(Value::Array(v))),
        _ => Ok(html(404, "not found".into())),
    };
    r.unwrap_or_else(|e| html(500, format!("database error: {}", esc(&e.to_string()))))
}

pub fn serve(server: Server, store: Arc<Mutex<Store>>, cfg: WebConfig) {
    for mut req in server.incoming_requests() {
        let mut body = String::new();
        let _ = req.as_reader().take(MAX_BODY).read_to_string(&mut body);
        let bearer = req
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .and_then(|h| h.value.as_str().strip_prefix("Bearer "))
            .map(str::to_string);
        let r = {
            let s = store.lock().unwrap();
            handle(&s, &cfg, req.method(), req.url(), bearer.as_deref(), &body)
        };
        let resp = Response::from_string(r.body)
            .with_status_code(r.code)
            .with_header(Header::from_bytes("Content-Type", r.content_type).unwrap())
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
            .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap());
        let _ = req.respond(resp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Origin;
    use cfb_core::frame::{Frame, MsgType};
    use cfb_core::PositionReport;

    fn store() -> Store {
        let s = Store::open(":memory:").unwrap();
        let o = Origin { box_id: "box-2".into(), bearer: "wifi".into(), rx_unix: 1_794_000_000 };
        let p = PositionReport { lat_deg: 47.0, lon_deg: 8.3, alt_ft: 1400, gs_kt: 0, track_deg: 0.0, vs_fpm: 0, airborne: false, emergency: false };
        s.insert(&o, &Frame { msg_type: MsgType::Position, seq: 1, time: 10, payload: p.encode().to_vec() }).unwrap();
        s.insert(&o, &Frame { msg_type: MsgType::Text, seq: 2, time: 11, payload: b"<script>x</script>".to_vec() }).unwrap();
        s
    }

    #[test]
    fn pages_render_and_escape() {
        let s = store();
        let cfg = WebConfig { view_token: None, webhook_token: None };
        let r = handle(&s, &cfg, &Method::Get, "/", None, "");
        assert_eq!(r.code, 200);
        assert!(r.body.contains("box-2") && r.body.contains("47.00000, 8.30000"));
        let r = handle(&s, &cfg, &Method::Get, "/box?id=box-2", None, "");
        assert!(r.body.contains("[[47.0,8.3]]"), "track embedded");
        assert!(r.body.contains("&lt;script&gt;x&lt;/script&gt;") && !r.body.contains("<script>x"));
        let r = handle(&s, &cfg, &Method::Get, "/api/track?box=box-2&limit=5", None, "");
        assert_eq!(r.content_type, "application/json");
        assert_eq!(serde_json::from_str::<Value>(&r.body).unwrap().as_array().unwrap().len(), 1);
        assert_eq!(handle(&s, &cfg, &Method::Get, "/nope", None, "").code, 404);
    }

    #[test]
    fn view_token_enforced_and_kept_on_links() {
        let s = store();
        let cfg = WebConfig { view_token: Some("viewtok1".into()), webhook_token: None };
        assert_eq!(handle(&s, &cfg, &Method::Get, "/", None, "").code, 401);
        assert_eq!(handle(&s, &cfg, &Method::Get, "/?token=wrong", None, "").code, 401);
        let r = handle(&s, &cfg, &Method::Get, "/?token=viewtok1", None, "");
        assert_eq!(r.code, 200);
        assert!(r.body.contains("/box?id=box-2&token=viewtok1"));
        assert_eq!(handle(&s, &cfg, &Method::Get, "/api/boxes", Some("viewtok1"), "").code, 200);
    }

    #[test]
    fn webhook_needs_its_own_token() {
        let s = store();
        let body = "imei=300234010753370&data=00";
        let no_tok = WebConfig { view_token: None, webhook_token: None };
        assert_eq!(handle(&s, &no_tok, &Method::Post, "/sbd", None, body).code, 403, "fail closed");
        let cfg = WebConfig { view_token: None, webhook_token: Some("hooktok1".into()) };
        assert_eq!(handle(&s, &cfg, &Method::Post, "/sbd?token=bad", None, body).code, 403);
        let r = handle(&s, &cfg, &Method::Post, "/sbd?token=hooktok1", None, body);
        assert_eq!(r.code, 200);
        assert!(r.body.starts_with("IGNORED"), "unparseable payload acknowledged, not stored: {}", r.body);
    }

    #[test]
    fn utc_formatting() {
        assert_eq!(fmt_time(0), "1970-01-01 00:00:00Z");
        assert_eq!(fmt_time(1_794_139_361), "2026-11-08 12:02:41Z");
        assert_eq!(fmt_time(951_782_400), "2000-02-29 00:00:00Z");
    }
}
