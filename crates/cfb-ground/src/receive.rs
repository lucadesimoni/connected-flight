// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Receiving: UDP frames from the tunnels, and Iridium SBD messages from
//! the service provider's webhook.

use crate::store::{Origin, Store};
use cfb_core::frame::{encode_ack, Frame, MsgType};
use std::collections::HashSet;
use std::io::Write;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_unix() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Box and bearer from the tunnel source address (`scripts/make-wg.sh`
/// plan: 10.66.<bearer>.<box>). Other addresses (bench, localhost) are
/// identified by the address itself.
pub fn origin_of(addr: &SocketAddr, rx_unix: i64) -> Origin {
    if let IpAddr::V4(v4) = addr.ip() {
        let o = v4.octets();
        if o[0] == 10 && o[1] == 66 && (1..=4).contains(&o[2]) {
            let bearer = ["", "wifi", "cellular", "iridium", "leo_sat"][o[2] as usize];
            return Origin { box_id: format!("box-{}", o[3]), bearer: bearer.into(), rx_unix };
        }
    }
    Origin { box_id: addr.ip().to_string(), bearer: "udp".into(), rx_unix }
}

pub struct UdpReceiver {
    pub sock: UdpSocket,
    pub announce_ota: bool,
    pub log: Option<std::fs::File>,
}

impl UdpReceiver {
    /// Receive loop: ack every valid frame, store it once.
    pub fn run(mut self, store: Arc<Mutex<Store>>) {
        let mut announced: HashSet<IpAddr> = HashSet::new();
        let mut seq: u16 = 0;
        let mut buf = [0u8; 2048];
        loop {
            let Ok((n, from)) = self.sock.recv_from(&mut buf) else { continue };
            let Ok(f) = Frame::decode(&buf[..n]) else { continue };
            // Ack first: the box keeps resending until it gets one.
            let ack = Frame { msg_type: MsgType::Ack, seq, time: 0, payload: encode_ack(&[f.seq]) };
            seq = seq.wrapping_add(1);
            let _ = self.sock.send_to(&ack.encode(), from);
            if f.msg_type == MsgType::Health && f.payload == b"KA" {
                continue; // keepalive
            }
            let o = origin_of(&from, now_unix());
            let new = store.lock().unwrap().insert(&o, &f).unwrap_or_else(|e| {
                eprintln!("cfb-ground: store: {e}");
                false
            });
            if new {
                let line = format!("{} {} #{} {:?}: {}", o.box_id, o.bearer, f.seq, f.msg_type, crate::store::describe(&f));
                println!("{line}");
                if let Some(l) = self.log.as_mut() {
                    let _ = writeln!(l, "{}\t{line}", o.rx_unix);
                }
            }
            if self.announce_ota && announced.insert(from.ip()) {
                let msg = Frame { msg_type: MsgType::Text, seq, time: 0, payload: b"OTA AVAILABLE".to_vec() };
                seq = seq.wrapping_add(1);
                let _ = self.sock.send_to(&msg.encode(), from);
                println!("{from}: announced OTA package");
            }
        }
    }
}

/// Iridium SBD delivery (service-provider webhook, form-encoded, e.g.
/// `imei=…&momsn=…&transmit_time=…&data=<hex>`). Returns frames stored.
pub fn sbd_webhook(store: &Store, form: &[(String, String)]) -> Result<usize, String> {
    let get = |k| cfb_core::urlenc::get(form, k);
    let imei = get("imei");
    if imei.is_empty() || imei.len() > 20 || !imei.bytes().all(|c| c.is_ascii_digit()) {
        return Err("missing or bad imei".into());
    }
    let bytes = cfb_core::sbd::from_hex(get("data"))?;
    let frames = cfb_core::sbd::unpack(&bytes)?;
    let o = Origin { box_id: format!("imei-{imei}"), bearer: "iridium-sbd".into(), rx_unix: now_unix() };
    let mut n = 0;
    for b in frames {
        let f = Frame::decode(b).map_err(|e| format!("{e:?}"))?;
        if store.insert(&o, &f).map_err(|e| e.to_string())? {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfb_core::PositionReport;

    #[test]
    fn origin_from_tunnel_address() {
        let o = origin_of(&"10.66.2.7:5000".parse().unwrap(), 0);
        assert_eq!((o.box_id.as_str(), o.bearer.as_str()), ("box-7", "cellular"));
        let o = origin_of(&"10.66.4.7:5000".parse().unwrap(), 0);
        assert_eq!(o.bearer, "leo_sat");
        let o = origin_of(&"127.0.0.1:5000".parse().unwrap(), 0);
        assert_eq!((o.box_id.as_str(), o.bearer.as_str()), ("127.0.0.1", "udp"));
    }

    #[test]
    fn udp_receive_acks_and_stores_once() {
        let store = Arc::new(Mutex::new(Store::open(":memory:").unwrap()));
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let r = UdpReceiver { sock, announce_ota: false, log: None };
        let s2 = store.clone();
        std::thread::spawn(move || r.run(s2));

        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        let p = PositionReport { lat_deg: 47.0, lon_deg: 8.3, alt_ft: 1400, gs_kt: 0, track_deg: 0.0, vs_fpm: 0, airborne: false, emergency: false };
        let f = Frame { msg_type: MsgType::Position, seq: 42, time: 1000, payload: p.encode().to_vec() }.encode();
        for _ in 0..2 {
            client.send_to(&f, addr).unwrap(); // second = resend after a "lost" ack
            let mut buf = [0u8; 64];
            let (n, _) = client.recv_from(&mut buf).unwrap();
            let ack = Frame::decode(&buf[..n]).unwrap();
            assert_eq!(cfb_core::frame::decode_ack(&ack.payload), vec![42], "every copy is acked");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        let boxes = store.lock().unwrap().boxes().unwrap();
        assert_eq!(boxes[0]["frames"], 1);
        assert_eq!(boxes[0]["duplicates"], 1);
    }

    #[test]
    fn sbd_webhook_stores_frames() {
        let store = Store::open(":memory:").unwrap();
        let p = PositionReport { lat_deg: 47.5, lon_deg: 8.6, alt_ft: 8500, gs_kt: 140, track_deg: 31.0, vs_fpm: 0, airborne: true, emergency: false };
        let f1 = Frame { msg_type: MsgType::Position, seq: 1, time: 500, payload: p.encode().to_vec() }.encode();
        let f2 = Frame { msg_type: MsgType::Text, seq: 2, time: 501, payload: b"TURBULENCE".to_vec() }.encode();
        let (payload, _) = cfb_core::sbd::pack([(1, &f1[..]), (2, &f2[..])], 340);
        let hex: String = payload.iter().map(|b| format!("{b:02x}")).collect();
        let form = cfb_core::urlenc::parse_form(&format!("imei=300234010753370&momsn=12&data={hex}"));
        assert_eq!(sbd_webhook(&store, &form), Ok(2));
        assert_eq!(sbd_webhook(&store, &form), Ok(0), "provider retry: duplicates");
        let b = store.boxes().unwrap();
        assert_eq!(b[0]["box"], "imei-300234010753370");
        assert_eq!(b[0]["bearers"], "iridium-sbd");
        assert!(sbd_webhook(&store, &cfb_core::urlenc::parse_form("imei=abc&data=00")).is_err());
        assert!(sbd_webhook(&store, &cfb_core::urlenc::parse_form("imei=1&data=zz")).is_err());
    }
}
