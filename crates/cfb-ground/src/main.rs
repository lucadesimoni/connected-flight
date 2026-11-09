// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-ground - ground service of the Connected Flight Box.
//!
//!   cfb-ground --listen 10.66.0.1:7400 --db /var/lib/cfb-ground/cfb.db \
//!              --http 127.0.0.1:8090 [--view-token-file F] [--webhook-token-file F] \
//!              [--log F] [--announce-ota]
//!
//! * UDP receiver (inside the WireGuard tunnels): acks every frame, stores
//!   each one once (SQLite, duplicates counted);
//! * web view (fleet, per-box map and events) + JSON API;
//! * `POST /sbd?token=…`: Iridium SBD deliveries from the service provider.
//!
//! Tools:
//!   cfb-ground --decode-sbd <hex>       decode an SBD message
//!   cfb-ground --encode-sbd "<text>"    MT payload (hex) for the provider API
//!
//! Not airborne software: no certification constraints, but the same
//! review rules (docs/12, docs/13).

mod receive;
mod store;
mod web;

use cfb_core::frame::{Frame, MsgType, EPOCH_2020};
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};

fn read_token(path: &Option<String>) -> Option<String> {
    let p = path.as_ref()?;
    match std::fs::read_to_string(p) {
        Ok(s) if s.trim().len() >= 8 => Some(s.trim().to_string()),
        _ => {
            eprintln!("cfb-ground: token file {p} missing or shorter than 8 characters");
            std::process::exit(1)
        }
    }
}

/// MT payload (hex) carrying one text frame.
fn encode_sbd(text: &str) -> Result<String, String> {
    let now = receive::now_unix() as u64;
    let f = Frame { msg_type: MsgType::Text, seq: 0, time: now.saturating_sub(EPOCH_2020) as u32, payload: text.as_bytes().to_vec() }
        .encode();
    let (p, ids) = cfb_core::sbd::pack([(0, &f[..])], 270);
    if ids.is_empty() {
        return Err("text too long for one MT message".into());
    }
    Ok(p.iter().map(|b| format!("{b:02x}")).collect())
}

/// Decode one SBD message (hex) into frame descriptions.
fn decode_sbd(hex: &str) -> Result<Vec<String>, String> {
    let bytes = cfb_core::sbd::from_hex(hex)?;
    cfb_core::sbd::unpack(&bytes)?
        .into_iter()
        .map(|b| {
            Frame::decode(b)
                .map(|f| format!("t={} #{} {:?}: {}", f.time as u64 + EPOCH_2020, f.seq, f.msg_type, store::describe(&f)))
                .map_err(|e| format!("{e:?}"))
        })
        .collect()
}

fn main() {
    let mut listen = "0.0.0.0:7400".to_string();
    let mut http: Option<String> = None;
    let mut db = "cfb-ground.db".to_string();
    let mut view_tok: Option<String> = None;
    let mut hook_tok: Option<String> = None;
    let mut log_path: Option<String> = None;
    let mut announce = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().unwrap_or_default();
        match a.as_str() {
            "--listen" => listen = val(),
            "--http" => http = Some(val()),
            "--db" => db = val(),
            "--view-token-file" => view_tok = Some(val()),
            "--webhook-token-file" => hook_tok = Some(val()),
            "--log" => log_path = Some(val()),
            "--announce-ota" => announce = true,
            "--encode-sbd" | "--decode-sbd" => {
                let r = if a == "--encode-sbd" { encode_sbd(&val()).map(|h| vec![h]) } else { decode_sbd(&val()) };
                match r {
                    Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
                    Err(e) => {
                        eprintln!("cfb-ground: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            _ => {
                eprintln!(
                    "usage: cfb-ground [--listen ADDR] [--db FILE] [--http ADDR] [--view-token-file F] \
                     [--webhook-token-file F] [--log F] [--announce-ota] | --decode-sbd HEX | --encode-sbd TEXT"
                );
                std::process::exit(2);
            }
        }
    }
    let store = Arc::new(Mutex::new(store::Store::open(&db).unwrap_or_else(|e| {
        eprintln!("cfb-ground: database {db}: {e}");
        std::process::exit(1)
    })));
    let sock = UdpSocket::bind(&listen).unwrap_or_else(|e| {
        eprintln!("cfb-ground: bind {listen}: {e}");
        std::process::exit(1)
    });
    let log = log_path.map(|p| {
        std::fs::OpenOptions::new().create(true).append(true).open(&p).unwrap_or_else(|e| {
            eprintln!("cfb-ground: log {p}: {e}");
            std::process::exit(1)
        })
    });
    println!("cfb-ground: UDP {listen}, database {db}");
    if let Some(addr) = http {
        let cfg = web::WebConfig { view_token: read_token(&view_tok), webhook_token: read_token(&hook_tok) };
        let server = tiny_http::Server::http(&addr).unwrap_or_else(|e| {
            eprintln!("cfb-ground: http {addr}: {e}");
            std::process::exit(1)
        });
        println!("cfb-ground: web on http://{addr} (view token: {}, SBD webhook: {})",
            if cfg.view_token.is_some() { "required" } else { "none" },
            if cfg.webhook_token.is_some() { "enabled" } else { "disabled" });
        let s2 = store.clone();
        std::thread::spawn(move || web::serve(server, s2, cfg));
    }
    receive::UdpReceiver { sock, announce_ota: announce, log }.run(store);
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfb_core::PositionReport;

    #[test]
    fn sbd_encode_decode_roundtrip() {
        let p = PositionReport { lat_deg: 47.0, lon_deg: 8.3, alt_ft: 8500, gs_kt: 140, track_deg: 31.0, vs_fpm: 0, airborne: true, emergency: false };
        let f1 = Frame { msg_type: MsgType::Position, seq: 1, time: 100, payload: p.encode().to_vec() }.encode();
        let f2 = Frame { msg_type: MsgType::Text, seq: 2, time: 101, payload: b"HELLO".to_vec() }.encode();
        let (payload, _) = cfb_core::sbd::pack([(1, &f1[..]), (2, &f2[..])], 340);
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        let lines = decode_sbd(&hex).unwrap();
        assert!(lines[0].contains("47.00000,8.30000 8500ft 140kt"));
        assert!(lines[1].ends_with("Text: HELLO"));
        assert!(decode_sbd("zz").is_err());
        let h = encode_sbd("OTA AVAILABLE").unwrap();
        assert!(decode_sbd(&h).unwrap()[0].ends_with("Text: OTA AVAILABLE"));
        assert!(encode_sbd(&"x".repeat(300)).is_err());
    }
}
