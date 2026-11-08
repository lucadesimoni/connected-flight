// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-ground - minimal ground service, for bench tests and as the
//! reference for the air-ground protocol.
//!
//!   cfb-ground --listen 0.0.0.0:7400 [--announce-ota]
//!
//! Receives CFB frames (one per UDP datagram, normally inside the WireGuard
//! tunnels), prints them decoded, and acknowledges each one. With
//! `--announce-ota` it tells every new aircraft once that an update is
//! published ("OTA AVAILABLE").

use cfb_core::frame::{encode_ack, Frame, MsgType, EPOCH_2020};
use cfb_core::PositionReport;
use std::collections::HashSet;
use std::net::UdpSocket;

fn main() {
    let mut listen = "0.0.0.0:7400".to_string();
    let mut announce = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--listen" => listen = args.next().unwrap_or(listen),
            "--announce-ota" => announce = true,
            _ => {
                eprintln!("usage: cfb-ground [--listen ADDR] [--announce-ota]");
                std::process::exit(2);
            }
        }
    }
    let sock = UdpSocket::bind(&listen).unwrap_or_else(|e| {
        eprintln!("cfb-ground: bind {listen}: {e}");
        std::process::exit(1)
    });
    println!("cfb-ground listening on {listen}");
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
        let body = match f.msg_type {
            MsgType::Position => PositionReport::decode(&f.payload)
                .map(|p| format!("{:.5},{:.5} {}ft {}kt trk {:.0} vs {} air={}", p.lat_deg, p.lon_deg, p.alt_ft, p.gs_kt, p.track_deg, p.vs_fpm, p.airborne))
                .unwrap_or_else(|| "bad position".into()),
            MsgType::FileChunk => format!("{} bytes", f.payload.len()),
            _ => String::from_utf8_lossy(&f.payload).into_owned(),
        };
        if !(f.msg_type == MsgType::Health && f.payload == b"KA") {
            println!("{from} t={when} #{} {:?}: {body}", f.seq, f.msg_type);
        }

        if announce && announced.insert(from.ip()) {
            let msg = Frame { msg_type: MsgType::Text, seq, time: 0, payload: b"OTA AVAILABLE".to_vec() };
            seq = seq.wrapping_add(1);
            let _ = sock.send_to(&msg.encode(), from);
            println!("{from}: announced OTA package");
        }
    }
}
