// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! The common interface of every air-ground bearer driver.
//!
//! The daemon only sees this trait: it asks for status, hands over encoded
//! CFB frames with their queue message id, and receives acknowledgements and
//! uplink frames. How a driver gets a frame to the ground (UDP in WireGuard,
//! Iridium SBD sessions, …) and what "acknowledged" means for it is the
//! driver's business. Contract:
//!
//! * `send` must not block the 1 Hz loop (queue internally if needed);
//! * a message counts as delivered only when `poll` returns `Rx::Acked(id)`;
//!   until then the daemon keeps it queued and resends after its retry time,
//!   so drivers must tolerate the same id being sent again (deduplicate);
//! * `status().up` means "a send now has a real chance to get through".

use cfb_core::frame::Frame;
use cfb_core::{LinkKind, LinkStatus};
use std::io;

pub enum Rx {
    /// Delivery of the queue message with this id is confirmed.
    Acked(u64),
    /// A frame from the ground (text, OTA announcement, …).
    Uplink(Frame),
}

pub trait LinkDriver {
    fn kind(&self) -> LinkKind;
    fn status(&self, t: f64) -> LinkStatus;
    /// Bytes this link may take in one 1 s tick.
    fn tick_budget(&self) -> usize;
    /// Hand over one encoded frame. Must return quickly.
    fn send(&mut self, frame: &[u8], msg_id: u64, t: f64) -> io::Result<()>;
    /// Collect acknowledgements and uplink frames.
    fn poll(&mut self, t: f64) -> Vec<Rx>;
    /// Periodic housekeeping (keepalives, probes). `seq` is a free frame
    /// sequence number the driver may use. Only called while transmitting is
    /// permitted for this link.
    fn maintain(&mut self, _t: f64, _seq: u16) {}
    /// One-line health summary for logs and the maintenance page.
    fn describe(&self) -> String {
        format!("{:?}", self.kind())
    }
}
