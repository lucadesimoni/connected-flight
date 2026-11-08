// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-ground - minimal ground service, for bench tests and as the
//! reference for the air-ground protocol.
//!
//!   cfb-ground --listen 10.66.0.1:7400 [--log /var/lib/cfb-ground/rx.log] [--announce-ota]
//!
//! Receives CFB frames (one per UDP datagram, normally inside the WireGuard
//! tunnels), prints them decoded, and acknowledges each one. With
//! `--announce-ota` it tells every new aircraft once that an update is
//! published ("OTA AVAILABLE").
//!
//! Iridium SBD messages arrive from the service provider (usually as hex in
//! an HTTP POST); decode one with
//!   cfb-ground --decode-sbd <hex>

use cfb_core::frame::{encode_ack, Frame, MsgType, EPOCH_2020};
use cfb_core::PositionReport;
use std::collections::HashSet;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};
use std::net::UdpSocket;

fn describe(f: &Frame) -> String {
    match f.msg_type {
        MsgType::Position => PositionReport::decode(&f.payload)
            .map(|p| {
                format!(
                    "{:.5},{:.5} {}ft {}kt trk {:.0} vs {} air={}",
                    p.lat_deg, p.lon_deg, p.alt_ft, p.gs_kt, p.track_deg, p.vs_fpm, p.airborne
                )
            })
            .unwrap_or_else(|| "bad position".into()),
        MsgType::FileChunk => format!("{} bytes", f.payload.len()),
        _ => String::from_utf8_lossy(&f.payload).into_owned(),
    }
}

/// Decode one SBD message (hex) into frame descriptions.
fn decode_sbd(hex: &str) -> Result<Vec<String>, String> {
    let bytes = cfb_core::sbd::from_hex(hex)?;
    cfb_core::sbd::unpack(&bytes)?
        .into_iter()
        .map(|b| {
            Frame::decode(b)
                .map(|f| format!("t={} #{} {:?}: {}", f.time as u64 + EPOCH_2020, f.seq, f.msg_type, describe(&f)))
                .map_err(|e| format!("{e:?}"))
        })
        .collect()
}

fn main() {
    let mut listen = "0.0.0.0:7400".to_string();
    let mut announce = false;
    let mut log_path: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().unwrap_or(listen),
            "--announce-ota" => announce = true,
            "--log" => log_path = args.next(),
            "--decode-sbd" => {
                match decode_sbd(&args.next().unwrap_or_default()) {
                    Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
                    Err(e) => {
                        eprintln!("cfb-ground: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            _ => {
                eprintln!("usage: cfb-ground [--listen ADDR] [--log FILE] [--announce-ota] | --decode-sbd HEX");
                std::process::exit(2);
            }
        }
    }
    let sock = UdpSocket::bind(&listen).unwrap_or_else(|e| {
        eprintln!("cfb-ground: bind {listen}: {e}");
        std::process::exit(1)
    });
    println!("cfb-ground listening on {listen}");
    // Append-only receive log: one line per frame (unix time received,
    // sender, frame time, seq, type, decoded body). Duplicates (resends
    // after a lost ack) are logged again; dedupe on (sender, seq, frame time).
    let mut log = log_path.map(|p| {
        std::fs::OpenOptions::new().create(true).append(true).open(&p).unwrap_or_else(|e| {
            eprintln!("cfb-ground: log {p}: {e}");
            std::process::exit(1)
        })
    });
    let mut announced = HashSet::new();
    let mut seq: u16 = 0;
    let mut buf = [0u8; 2048];
    loop {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let f = match Frame::decode(&buf[..n]) {
            Ok(f) => f,
            Err(e) => {
                println!("{from}: bad frame {e:?}");
                continue;
            }
        };
        let ack = Frame { msg_type: MsgType::Ack, seq, time: 0, payload: encode_ack(&[f.seq]) };
        seq = seq.wrapping_add(1);
        let _ = sock.send_to(&ack.encode(), from);

        let when = f.time as u64 + EPOCH_2020;
        let body = describe(&f);
        if !(f.msg_type == MsgType::Health && f.payload == b"KA") {
            println!("{from} t={when} #{} {:?}: {body}", f.seq, f.msg_type);
            if let Some(l) = log.as_mut() {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                let _ = writeln!(l, "{now}\t{from}\t{when}\t{}\t{:?}\t{}", f.seq, f.msg_type, body.replace(['\t', '\n'], " "));
                let _ = l.flush();
            }
        }

        if announce && announced.insert(from.ip()) {
            let msg = Frame { msg_type: MsgType::Text, seq, time: 0, payload: b"OTA AVAILABLE".to_vec() };
            seq = seq.wrapping_add(1);
            let _ = sock.send_to(&msg.encode(), from);
            println!("{from}: announced OTA package");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_batched_sbd_message() {
        let p = PositionReport {
            lat_deg: 47.0,
            lon_deg: 8.3,
            alt_ft: 8500,
            gs_kt: 140,
            track_deg: 31.0,
            vs_fpm: 0,
            airborne: true,
            emergency: false,
        };
        let f1 = Frame { msg_type: MsgType::Position, seq: 1, time: 100, payload: p.encode().to_vec() }.encode();
        let f2 = Frame { msg_type: MsgType::Text, seq: 2, time: 101, payload: b"HELLO".to_vec() }.encode();
        let (payload, ids) = cfb_core::sbd::pack([(1, &f1[..]), (2, &f2[..])], 340);
        assert_eq!(ids, vec![1, 2]);
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        let lines = decode_sbd(&hex).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("47.00000,8.30000 8500ft 140kt"), "{}", lines[0]);
        assert!(lines[1].ends_with("Text: HELLO"), "{}", lines[1]);
        assert!(decode_sbd("zz").is_err());
    }
}
