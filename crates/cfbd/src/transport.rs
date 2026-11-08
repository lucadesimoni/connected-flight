//! Ground transport over IP bearers.
//!
//! Each bearer (Starlink/LEO, Iridium Certus, cellular, Wi-Fi) carries one
//! WireGuard tunnel to the ground service, with its own tunnel address.
//! Binding a UDP socket to that address plus policy routing by source
//! address sends the frames over that bearer, and only that one. WireGuard
//! does encryption, peer authentication and roaming; this layer adds
//! end-to-end acknowledgement:
//!
//! * the box sends a CFB frame in one UDP datagram;
//! * the ground replies with an `Ack` frame listing the received sequence
//!   numbers; only then is the message removed from the queue;
//! * the bearer counts as **up** while its interface is up and:
//!   - with keepalive (`keepalive_s > 0`, unmetered: LEO, cellular, Wi-Fi):
//!     the ground has answered within `down_after_s`;
//!   - without keepalive (metered: Iridium): no frame has gone unanswered
//!     for longer than `down_after_s` (optimistic, costs no idle bytes).
//!
//! A bearer that is down sends a sparse probe so it notices recovery.
//!
//! For Starlink this means the link state comes from what actually gets
//! through, not from the dish's own status, so obstructions, handovers and
//! dish reboots are all handled the same way.

use cfb_core::config::BearerConfig;
use cfb_core::frame::{decode_ack, frame_seq, Frame, MsgType};
use cfb_core::{LinkKind, LinkStatus};
use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};

pub struct Bearer {
    pub kind: LinkKind,
    sock: UdpSocket,
    ground: SocketAddr,
    cfg: BearerConfig,
    last_rx: Option<f64>,
    last_tx: f64,
    /// First send since the last reply.
    unanswered_since: Option<f64>,
    /// frame seq -> queue message id
    inflight: HashMap<u16, u64>,
    pub tx_bytes: u64,
    pub rx_frames: u64,
}

pub enum Rx {
    Acked(u64),
    Uplink(Frame),
}

fn iface_up(iface: &Option<String>) -> bool {
    match iface {
        None => true,
        Some(i) => std::fs::read_to_string(format!("/sys/class/net/{i}/operstate"))
            // WireGuard interfaces report "unknown" when up.
            .map(|s| matches!(s.trim(), "up" | "unknown"))
            .unwrap_or(false),
    }
}

impl Bearer {
    pub fn open(kind: LinkKind, cfg: &BearerConfig, ground: SocketAddr) -> io::Result<Self> {
        let local: SocketAddr = cfg.local.parse().map_err(|_| io::Error::other("bad local address"))?;
        let sock = UdpSocket::bind(local)?;
        sock.set_nonblocking(true)?;
        Ok(Self {
            kind,
            sock,
            ground,
            cfg: cfg.clone(),
            last_rx: None,
            last_tx: f64::NEG_INFINITY,
            unanswered_since: None,
            inflight: HashMap::new(),
            tx_bytes: 0,
            rx_frames: 0,
        })
    }

    fn alive(&self, t: f64) -> bool {
        let down = self.cfg.down_after_s as f64;
        if self.cfg.keepalive_s > 0 {
            self.last_rx.is_some_and(|r| t - r <= down)
        } else {
            self.unanswered_since.is_none_or(|t0| t - t0 <= down)
        }
    }

    pub fn status(&self, t: f64) -> LinkStatus {
        LinkStatus { kind: self.kind, up: self.alive(t) && iface_up(&self.cfg.iface), est_bps: self.cfg.nominal_bps }
    }

    fn sent(&mut self, n: usize, t: f64) {
        self.tx_bytes += n as u64;
        self.last_tx = t;
        self.unanswered_since.get_or_insert(t);
    }

    /// Bytes this bearer may send in one 1-second tick.
    pub fn tick_budget(&self) -> usize {
        (self.cfg.nominal_bps as usize / 8).max(64)
    }

    pub fn send(&mut self, frame: &[u8], msg_id: u64, t: f64) -> io::Result<()> {
        self.sock.send_to(frame, self.ground)?;
        if let Some(seq) = frame_seq(frame) {
            self.inflight.insert(seq, msg_id);
        }
        self.sent(frame.len(), t);
        Ok(())
    }

    /// Keepalive / probe. Also how a bearer that is down finds out it is back.
    pub fn maybe_keepalive(&mut self, t: f64, seq: u16) {
        let every = match (self.alive(t), self.cfg.keepalive_s) {
            (true, 0) => return,                 // metered and fine: no idle traffic
            (true, k) => k as f64,
            (false, 0) => (self.cfg.down_after_s as f64).max(60.0),
            (false, k) => k.max(5) as f64,
        };
        if t - self.last_tx < every {
            return;
        }
        let f = Frame { msg_type: MsgType::Health, seq, time: 0, payload: b"KA".to_vec() };
        let b = f.encode();
        if self.sock.send_to(&b, self.ground).is_ok() {
            self.sent(b.len(), t);
        }
    }

    /// Drain received datagrams.
    pub fn poll(&mut self, t: f64) -> Vec<Rx> {
        let mut out = Vec::new();
        let mut buf = [0u8; 2048];
        while let Ok((n, from)) = self.sock.recv_from(&mut buf) {
            if from != self.ground {
                continue; // only the ground service may talk to us
            }
            let Ok(f) = Frame::decode(&buf[..n]) else { continue };
            self.last_rx = Some(t);
            self.unanswered_since = None;
            self.rx_frames += 1;
            if f.msg_type == MsgType::Ack {
                for seq in decode_ack(&f.payload) {
                    if let Some(id) = self.inflight.remove(&seq) {
                        out.push(Rx::Acked(id));
                    }
                }
            } else {
                out.push(Rx::Uplink(f));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cfb_core::frame::encode_ack;

    fn cfg() -> BearerConfig {
        BearerConfig { local: "127.0.0.1:0".into(), iface: None, nominal_bps: 1_000_000, keepalive_s: 10, down_after_s: 30 }
    }

    #[test]
    fn send_ack_and_uplink_roundtrip() {
        let ground = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut b = Bearer::open(LinkKind::LeoSat, &cfg(), ground.local_addr().unwrap()).unwrap();
        assert!(!b.status(0.0).up, "down until the ground answers");

        let f = Frame { msg_type: MsgType::Position, seq: 42, time: 0, payload: vec![1; 17] };
        b.send(&f.encode(), 7, 0.0).unwrap();

        let mut buf = [0u8; 2048];
        let (n, box_addr) = ground.recv_from(&mut buf).unwrap();
        assert_eq!(Frame::decode(&buf[..n]).unwrap().seq, 42);
        let ack = Frame { msg_type: MsgType::Ack, seq: 0, time: 0, payload: encode_ack(&[42, 99]) };
        ground.send_to(&ack.encode(), box_addr).unwrap();
        let txt = Frame { msg_type: MsgType::Text, seq: 1, time: 0, payload: b"OTA AVAILABLE".to_vec() };
        ground.send_to(&txt.encode(), box_addr).unwrap();

        let mut rx = Vec::new();
        for _ in 0..200 {
            rx.extend(b.poll(1.0));
            if rx.len() >= 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(rx[0], Rx::Acked(7)), "seq 99 unknown, ignored");
        assert!(matches!(&rx[1], Rx::Uplink(f) if f.payload == b"OTA AVAILABLE"));
        assert!(b.status(1.0).up);
        assert!(!b.status(40.0).up, "no reply for > down_after_s");
    }

    #[test]
    fn metered_bearer_optimistic_until_unanswered() {
        let ground = UdpSocket::bind("127.0.0.1:0").unwrap();
        let c = BearerConfig { keepalive_s: 0, down_after_s: 30, ..cfg() };
        let mut b = Bearer::open(LinkKind::Iridium, &c, ground.local_addr().unwrap()).unwrap();
        assert!(b.status(0.0).up, "metered: up until proven otherwise");
        b.maybe_keepalive(100.0, 1);
        assert_eq!(b.tx_bytes, 0, "no idle keepalive on metered link");
        let f = Frame { msg_type: MsgType::Position, seq: 5, time: 0, payload: vec![0; 17] };
        b.send(&f.encode(), 1, 100.0).unwrap();
        assert!(b.status(120.0).up);
        assert!(!b.status(131.0).up, "unanswered > 30 s");
        let before = b.tx_bytes;
        b.maybe_keepalive(161.0, 2);
        assert!(b.tx_bytes > before, "sparse probe while down");
    }

    #[test]
    fn foreign_sender_ignored() {
        let ground = UdpSocket::bind("127.0.0.1:0").unwrap();
        let stranger = UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut b = Bearer::open(LinkKind::Cellular, &cfg(), ground.local_addr().unwrap()).unwrap();
        let addr = b.sock.local_addr().unwrap();
        let f = Frame { msg_type: MsgType::Text, seq: 1, time: 0, payload: b"hi".to_vec() };
        stranger.send_to(&f.encode(), addr).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(b.poll(0.0).is_empty());
        assert!(!b.status(0.0).up);
    }
}
