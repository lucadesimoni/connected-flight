// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-maint - maintenance page of the Connected Flight Box.
//!
//!   cfb-maint --listen 192.168.77.1:8080 --token-file /etc/cfb/maint.token
//!
//! A separate process with no flight functions (docs/12, docs/13):
//! * reads the status snapshot cfbd writes (`/run/cfb/status.json`);
//! * can do exactly two things, each requiring the maintenance token:
//!   - switch maintenance mode (the `maint` key in the discretes file),
//!   - confirm a software install (creates the OTA confirm file that the
//!     OTA state machine waits for; the update gate still decides).
//!
//! It never talks to radios, links or the update gate. Bind it to the
//! maintenance Ethernet address only.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tiny_http::{Header, Method, Request, Response, Server};

struct Paths {
    status: PathBuf,
    discretes: PathBuf,
    confirm: PathBuf,
}

const MAX_BODY: u64 = 4096;

/// HTML-escape for text and attribute values.
fn esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Constant-time comparison (no early exit on the first differing byte).
fn token_ok(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    if expected.is_empty() {
        return false;
    }
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= (*a.get(i).unwrap_or(&0) ^ *b.get(i).unwrap_or(&0)) as usize;
    }
    diff == 0
}

/// Minimal application/x-www-form-urlencoded parser.
fn form(body: &str) -> Vec<(String, String)> {
    fn dec(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'+' => out.push(b' '),
                b'%' => {
                    let hex = b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                    match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                        Some(v) => {
                            out.push(v);
                            i += 2;
                        }
                        None => out.push(b'%'),
                    }
                }
                c => out.push(c),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    body.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (dec(k), dec(v))).collect()
}

/// Set `maint=0|1` in the discretes file, keep every other key as it is.
fn set_maint(path: &Path, on: bool) -> std::io::Result<()> {
    let old = fs::read_to_string(path).unwrap_or_default();
    let mut parts: Vec<String> = old.split_whitespace().filter(|kv| !kv.starts_with("maint=")).map(String::from).collect();
    parts.push(format!("maint={}", u8::from(on)));
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, parts.join(" ") + "\n")?;
    fs::rename(&tmp, path)
}

fn page(status: &serde_json::Value, msg: &str) -> String {
    let g = |k: &str| match &status[k] {
        serde_json::Value::Null => "–".to_string(),
        serde_json::Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    let lights = &status["lights"];
    let light = |c: &str, color: &str| {
        let v = lights[c].as_str().unwrap_or("Off");
        let style = match v {
            "On" => format!("background:{color}"),
            "Blink" => format!("background:{color};animation:b 1s steps(1) infinite"),
            _ => "background:#555".into(),
        };
        format!(r#"<span class="led" style="{style}" title="{c}: {v}"></span>"#)
    };
    let mut links = String::new();
    if let Some(arr) = status["links"].as_array() {
        for l in arr {
            links.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td><small>{}</small></td></tr>",
                esc(l["kind"].as_str().unwrap_or("")),
                if l["up"].as_bool() == Some(true) { "up" } else { "down" },
                if l["permitted"].as_bool() == Some(true) { "allowed" } else { "inhibited" },
                esc(l["detail"].as_str().unwrap_or(""))
            ));
        }
    }
    let list = |k: &str| {
        status[k]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str()).map(esc).collect::<Vec<_>>().join(", "))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "none".into())
    };
    let maint_on = status["maintenance_mode"].as_bool() == Some(true);
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="refresh" content="3"><title>CFB maintenance</title>
<style>body{{font:15px system-ui,sans-serif;margin:16px;max-width:900px;background:#fafafa;color:#111}}
table{{border-collapse:collapse;width:100%}}td,th{{border-bottom:1px solid #ddd;padding:4px 6px;text-align:left}}
.led{{display:inline-block;width:18px;height:18px;border-radius:50%;margin-right:6px;vertical-align:middle}}
@keyframes b{{50%{{background:#555}}}}form{{display:inline-block;margin:4px 8px 4px 0}}
.msg{{padding:8px;background:#ffe;border:1px solid #cc9}}</style></head><body>
<h1>CFB {reg} <small>v{ver}</small></h1>
{msg}
<p>{lg}{la}{lr} faults: <b>{faults}</b></p>
<table>
<tr><th>Phase</th><td>{phase}</td><th>Maintenance mode</th><td>{maint}</td></tr>
<tr><th>Position</th><td>{lat}, {lon} ({ps})</td><th>Altitude</th><td>{alt} ft ({als})</td></tr>
<tr><th>Ground speed</th><td>{gs} kt</td><th>Engine</th><td>{eng} ({es})</td></tr>
<tr><th>Radio switch</th><td>{sw}</td><th>Power</th><td>{pw}</td></tr>
<tr><th>Queue</th><td>{qm} msgs, {qb} / {qx} bytes, {qe} evicted</td><th>Modules</th><td>{mods}</td></tr>
<tr><th>Software update</th><td>{ota}</td><th>Update gate</th><td>{gate}</td></tr>
<tr><th>Edge apps</th><td colspan="3">{apps}</td></tr>
</table>
<h2>Links</h2><table><tr><th>Link</th><th>State</th><th>Radio</th><th>Detail</th></tr>{links}</table>
<h2>Actions</h2>
<form method="post" action="/maint"><input type="hidden" name="on" value="{maint_next}">
<input type="password" name="token" placeholder="token" required> <button>{maint_label}</button></form>
<form method="post" action="/confirm"><input type="password" name="token" placeholder="token" required>
<button>Confirm software install</button></form>
<p><small>Install starts only when the update gate is open (parked, engine off, maintenance mode, supply OK) and a verified package is waiting. Status as of t={t} s.</small></p>
</body></html>"#,
        reg = esc(&g("registration")),
        ver = esc(&g("version")),
        msg = if msg.is_empty() { String::new() } else { format!(r#"<p class="msg">{}</p>"#, esc(msg)) },
        lg = light("green", "#2a2"),
        la = light("amber", "#e90"),
        lr = light("red", "#d22"),
        faults = list("faults"),
        phase = esc(&g("phase")),
        maint = if maint_on { "ON" } else { "off" },
        lat = esc(&g("lat")),
        lon = esc(&g("lon")),
        ps = esc(&g("pos_source")),
        alt = esc(&g("alt_ft")),
        als = esc(&g("alt_source")),
        gs = esc(&g("gs_kt")),
        eng = esc(&g("engine_running")),
        es = esc(&g("engine_source")),
        sw = if status["rf_switch_off"].as_bool() == Some(true) { "OFF" } else { "on / not fitted" },
        pw = if status["power_fail"].as_bool() == Some(true) { "FAILING" } else { "ok" },
        qm = esc(&g("queue_messages")),
        qb = esc(&g("queue_bytes")),
        qx = esc(&g("queue_max_bytes")),
        qe = esc(&g("queue_evicted")),
        mods = list("modules"),
        ota = esc(&g("ota_state")),
        gate = esc(&g("update_gate")),
        apps = list("edge_apps"),
        links = links,
        maint_next = if maint_on { "0" } else { "1" },
        maint_label = if maint_on { "Maintenance mode OFF" } else { "Maintenance mode ON" },
        t = esc(&g("t")),
    )
}

fn read_status(p: &Path) -> serde_json::Value {
    fs::read(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({"phase": "no status (cfbd not running?)"}))
}

fn html(body: String, code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_string(body)
        .with_status_code(code)
        .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
        .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
        .with_header(Header::from_bytes("X-Frame-Options", "DENY").unwrap())
}

/// Handle one request; returns (status code, html). Separate from I/O for tests.
fn handle(method: &Method, url: &str, body: &str, token: &str, p: &Paths) -> (u16, String) {
    let status = read_status(&p.status);
    match (method, url) {
        (Method::Get, "/") => (200, page(&status, "")),
        (Method::Get, "/status.json") => (200, status.to_string()),
        (Method::Post, "/maint") | (Method::Post, "/confirm") => {
            let f = form(body);
            let get = |k: &str| f.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str()).unwrap_or("");
            if !token_ok(get("token"), token) {
                std::thread::sleep(Duration::from_secs(1)); // slow down guessing
                return (403, page(&status, "Wrong token."));
            }
            let msg = if url == "/maint" {
                let on = get("on") == "1";
                match set_maint(&p.discretes, on) {
                    Ok(()) => format!("Maintenance mode {}.", if on { "ON" } else { "OFF" }),
                    Err(e) => format!("Could not change maintenance mode: {e}"),
                }
            } else {
                match fs::write(&p.confirm, b"confirmed\n") {
                    Ok(()) => "Install confirmed. It starts when the update gate is open and a verified package is waiting.".into(),
                    Err(e) => format!("Could not confirm: {e}"),
                }
            };
            (200, page(&read_status(&p.status), &msg))
        }
        _ => (404, "<!doctype html><title>404</title>not found".into()),
    }
}

fn serve(server: Server, token: String, p: Paths) {
    for mut req in server.incoming_requests() {
        let mut body = String::new();
        let _ = req.as_reader().take(MAX_BODY).read_to_string(&mut body);
        let url = req.url().split('?').next().unwrap_or("/").to_string();
        let (code, out) = handle(req.method(), &url, &body, &token, &p);
        let resp = if url == "/status.json" && code == 200 {
            Response::from_string(out).with_header(Header::from_bytes("Content-Type", "application/json").unwrap())
        } else {
            html(out, code)
        };
        let _ = respond(req, resp);
    }
}

fn respond(req: Request, resp: Response<std::io::Cursor<Vec<u8>>>) -> std::io::Result<()> {
    req.respond(resp)
}

fn main() {
    let mut listen = "127.0.0.1:8080".to_string();
    let mut token_file = "/etc/cfb/maint.token".to_string();
    let mut p = Paths {
        status: "/run/cfb/status.json".into(),
        discretes: "/run/cfb/discretes".into(),
        confirm: "/run/cfb/ota/confirm".into(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_default();
        match a.as_str() {
            "--listen" => listen = val(),
            "--token-file" => token_file = val(),
            "--status" => p.status = val().into(),
            "--discretes" => p.discretes = val().into(),
            "--confirm" => p.confirm = val().into(),
            _ => {
                eprintln!("usage: cfb-maint [--listen ADDR] [--token-file F] [--status F] [--discretes F] [--confirm F]");
                std::process::exit(2);
            }
        }
    }
    let token = fs::read_to_string(&token_file).map(|s| s.trim().to_string()).unwrap_or_default();
    if token.len() < 8 {
        eprintln!("cfb-maint: token file {token_file} missing or shorter than 8 characters; actions disabled");
    }
    let server = Server::http(&listen).unwrap_or_else(|e| {
        eprintln!("cfb-maint: listen {listen}: {e}");
        std::process::exit(1)
    });
    println!("cfb-maint on http://{listen}");
    let token = if token.len() >= 8 { token } else { String::new() };
    serve(server, token, p);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(dir: &Path) -> Paths {
        Paths { status: dir.join("status.json"), discretes: dir.join("discretes"), confirm: dir.join("confirm") }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cfb-maint-{name}-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn escaping_and_token_compare() {
        assert_eq!(esc(r#"<a href="x">&'"#), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
        assert!(token_ok("secret123", "secret123"));
        assert!(!token_ok("secret12", "secret123"));
        assert!(!token_ok("", ""), "empty expected token never matches");
        assert!(!token_ok("secret124", "secret123"));
    }

    #[test]
    fn form_decoding() {
        let f = form("token=a%2Bb+c&on=1&x=%zz");
        assert_eq!(f[0], ("token".into(), "a+b c".into()));
        assert_eq!(f[1], ("on".into(), "1".into()));
        assert_eq!(f[2], ("x".into(), "%zz".into()));
    }

    #[test]
    fn status_page_renders_and_escapes() {
        let d = tmp("page");
        let p = paths(&d);
        fs::write(&p.status, r#"{"registration":"<script>","phase":"OnGround","lights":{"green":"On","amber":"Off","red":"Off"},"faults":[],"links":[{"kind":"Wifi","up":true,"permitted":true,"detail":"x"}]}"#).unwrap();
        let (code, html) = handle(&Method::Get, "/", "", "tok12345", &p);
        assert_eq!(code, 200);
        assert!(html.contains("&lt;script&gt;") && !html.contains("<script>"));
        assert!(html.contains("OnGround") && html.contains("Wifi"));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn actions_need_token_and_only_touch_their_file() {
        let d = tmp("act");
        let p = paths(&d);
        fs::write(&p.discretes, "wow=1 eng=0 maint=0 v=13.0\n").unwrap();
        let (code, _) = handle(&Method::Post, "/maint", "on=1&token=wrong", "tok12345", &p);
        assert_eq!(code, 403);
        assert!(fs::read_to_string(&p.discretes).unwrap().contains("maint=0"));
        let (code, _) = handle(&Method::Post, "/maint", "on=1&token=tok12345", "tok12345", &p);
        assert_eq!(code, 200);
        let disc = fs::read_to_string(&p.discretes).unwrap();
        assert!(disc.contains("maint=1") && disc.contains("wow=1 eng=0") && disc.contains("v=13.0"), "{disc}");
        assert!(!p.confirm.exists());
        let (code, _) = handle(&Method::Post, "/confirm", "token=tok12345", "tok12345", &p);
        assert_eq!(code, 200);
        assert!(p.confirm.exists());
        assert_eq!(handle(&Method::Get, "/nope", "", "tok12345", &p).0, 404);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn real_http_roundtrip() {
        let d = tmp("http");
        let p = paths(&d);
        fs::write(&p.status, r#"{"phase":"Airborne"}"#).unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        std::thread::spawn(move || serve(server, "tok12345".into(), p));
        use std::io::Write;
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        s.write_all(b"GET /status.json HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
        let mut resp = String::new();
        s.read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
        assert!(resp.contains("Airborne"));
        fs::remove_dir_all(&d).unwrap();
    }
}
