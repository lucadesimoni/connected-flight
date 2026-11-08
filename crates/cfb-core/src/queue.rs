// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Bounded, prioritised store-and-forward queue.
//!
//! Messages wait here until the link manager hands them to a link that may
//! carry them. The queue has a hard byte budget so it can never fill the
//! storage. When the budget is exceeded, the oldest message of the lowest
//! priority is evicted first (CFB-HLR-030).
//!
//! The queue is persisted as an atomic snapshot (write to temp file, fsync,
//! rename), so a power cut during write never corrupts it. Snapshots are taken
//! on a timer and on power-down warning, not per message, to limit flash wear.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Large, deferrable data: flight data files, logs. Ground links only by default.
    Bulk = 0,
    /// Engine trend, maintenance data.
    Normal = 1,
    /// Operational messages: OOOI times, crew messages.
    High = 2,
    /// Position reports and alerts. May use any allowed link.
    Critical = 3,
}

impl Priority {
    pub const ALL_DESC: [Priority; 4] =
        [Priority::Critical, Priority::High, Priority::Normal, Priority::Bulk];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: u64,
    pub priority: Priority,
    /// Monotonic seconds when queued.
    pub created_t: u64,
    /// Drop the message after this many seconds; 0 = never expires.
    pub ttl_s: u64,
    pub payload: Vec<u8>,
    /// When it was last handed to a link and is waiting for the ground
    /// ack. Not persisted: after a restart everything unacked is resent.
    pub sent_t: Option<u64>,
}

#[derive(Debug)]
pub struct StoreForwardQueue {
    lanes: [VecDeque<Message>; 4],
    bytes: usize,
    max_bytes: usize,
    next_id: u64,
    pub evicted: u64,
}

impl StoreForwardQueue {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            lanes: Default::default(),
            bytes: 0,
            max_bytes,
            next_id: 1,
            evicted: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.lanes.iter().map(|l| l.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len_of(&self, p: Priority) -> usize {
        self.lanes[p as usize].len()
    }

    /// Queue a payload. Returns its id, or `None` if the payload alone is
    /// larger than the whole budget.
    pub fn push(&mut self, priority: Priority, payload: Vec<u8>, now: u64, ttl_s: u64) -> Option<u64> {
        if payload.len() > self.max_bytes {
            return None;
        }
        // CFB-HLR-030: make room by evicting lowest priority, oldest first.
        // A message never evicts one of higher priority. Check first that
        // enough evictable bytes exist, so a refused push evicts nothing.
        let evictable: usize = self.lanes[..=priority as usize]
            .iter()
            .flat_map(|l| l.iter().map(|m| m.payload.len()))
            .sum();
        if self.bytes - evictable + payload.len() > self.max_bytes {
            return None;
        }
        while self.bytes + payload.len() > self.max_bytes {
            let victim = (0..=priority as usize).find(|&p| !self.lanes[p].is_empty())?;
            let m = self.lanes[victim].pop_front().expect("non-empty lane");
            self.bytes -= m.payload.len();
            self.evicted += 1;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.bytes += payload.len();
        self.lanes[priority as usize].push_back(Message { id, priority, created_t: now, ttl_s, payload, sent_t: None });
        Some(id)
    }

    /// Remove expired messages (CFB-HLR-031).
    pub fn expire(&mut self, now: u64) -> usize {
        let mut n = 0;
        for lane in &mut self.lanes {
            let before = lane.len();
            lane.retain(|m| {
                let keep = m.ttl_s == 0 || now.saturating_sub(m.created_t) < m.ttl_s;
                if !keep {
                    self.bytes -= m.payload.len();
                }
                keep
            });
            n += before - lane.len();
        }
        n
    }

    /// Next message to send, highest priority first, among the priorities
    /// `allowed` accepts. The message stays queued until `ack`.
    pub fn peek(&self, allowed: impl Fn(Priority) -> bool) -> Option<&Message> {
        Priority::ALL_DESC
            .iter()
            .filter(|p| allowed(**p))
            .find_map(|p| self.lanes[*p as usize].front())
    }

    /// Like `peek`, but skips messages already sent and still waiting for
    /// their ack, unless `retry_s` has passed (CFB-HLR-033).
    pub fn peek_ready(&self, allowed: impl Fn(Priority) -> bool, now: u64, retry_s: u64) -> Option<&Message> {
        Priority::ALL_DESC
            .iter()
            .filter(|p| allowed(**p))
            .find_map(|p| {
                self.lanes[*p as usize]
                    .iter()
                    .find(|m| m.sent_t.is_none_or(|s| now.saturating_sub(s) >= retry_s))
            })
    }

    /// Record that a message was handed to a link.
    pub fn mark_sent(&mut self, id: u64, now: u64) {
        for lane in &mut self.lanes {
            if let Some(m) = lane.iter_mut().find(|m| m.id == id) {
                m.sent_t = Some(now);
                return;
            }
        }
    }

    /// Remove a message once the far end confirms delivery.
    pub fn ack(&mut self, id: u64) -> bool {
        for lane in &mut self.lanes {
            if let Some(pos) = lane.iter().position(|m| m.id == id) {
                let m = lane.remove(pos).expect("position valid");
                self.bytes -= m.payload.len();
                return true;
            }
        }
        false
    }

    // ---- persistence -----------------------------------------------------
    // Format: "CFQ1" | next_id u64 | count u32 | { id u64, prio u8,
    // created u64, ttl u64, len u32, payload } ... | crc32c u32 (over all
    // preceding bytes). Little endian.

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes + 64 * self.len() + 16);
        out.extend_from_slice(b"CFQ1");
        out.extend_from_slice(&self.next_id.to_le_bytes());
        out.extend_from_slice(&(self.len() as u32).to_le_bytes());
        for lane in &self.lanes {
            for m in lane {
                out.extend_from_slice(&m.id.to_le_bytes());
                out.push(m.priority as u8);
                out.extend_from_slice(&m.created_t.to_le_bytes());
                out.extend_from_slice(&m.ttl_s.to_le_bytes());
                out.extend_from_slice(&(m.payload.len() as u32).to_le_bytes());
                out.extend_from_slice(&m.payload);
            }
        }
        let crc = crate::frame::crc32c(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    pub fn from_bytes(buf: &[u8], max_bytes: usize) -> io::Result<Self> {
        let bad = |s: &str| io::Error::new(io::ErrorKind::InvalidData, s.to_string());
        if buf.len() < 20 || &buf[..4] != b"CFQ1" {
            return Err(bad("bad queue header"));
        }
        let (body, crc) = buf.split_at(buf.len() - 4);
        if crate::frame::crc32c(body) != u32::from_le_bytes(crc.try_into().unwrap()) {
            return Err(bad("queue snapshot CRC mismatch"));
        }
        let mut r = Reader { b: body, p: 4 };
        let next_id = r.u64()?;
        let count = r.u32()?;
        let mut q = StoreForwardQueue::new(max_bytes);
        q.next_id = next_id;
        for _ in 0..count {
            let id = r.u64()?;
            let prio = match r.u8()? {
                0 => Priority::Bulk,
                1 => Priority::Normal,
                2 => Priority::High,
                3 => Priority::Critical,
                _ => return Err(bad("bad priority")),
            };
            let created_t = r.u64()?;
            let ttl_s = r.u64()?;
            let len = r.u32()? as usize;
            let payload = r.take(len)?.to_vec();
            q.bytes += payload.len();
            q.lanes[prio as usize].push_back(Message { id, priority: prio, created_t, ttl_s, payload, sent_t: None });
        }
        Ok(q)
    }

    /// Atomic snapshot: write temp file, fsync, rename over the old one.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let tmp = path.with_extension("tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&self.to_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)?;
        if let Some(dir) = path.parent() {
            if let Ok(d) = File::open(dir) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    }

    /// Load a snapshot. A missing or corrupt file gives an empty queue
    /// (CFB-HLR-032): losing queued data is acceptable, failing to start is not.
    pub fn load_or_new(path: &Path, max_bytes: usize) -> Self {
        let mut buf = Vec::new();
        match File::open(path).and_then(|mut f| f.read_to_end(&mut buf)) {
            Ok(_) => Self::from_bytes(&buf, max_bytes).unwrap_or_else(|_| Self::new(max_bytes)),
            Err(_) => Self::new(max_bytes),
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.p + n > self.b.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated"));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_priority_first_fifo_within_lane() {
        let mut q = StoreForwardQueue::new(1000);
        let a = q.push(Priority::Normal, vec![1], 0, 0).unwrap();
        let _b = q.push(Priority::Normal, vec![2], 0, 0).unwrap();
        let c = q.push(Priority::Critical, vec![3], 0, 0).unwrap();
        assert_eq!(q.peek(|_| true).unwrap().id, c);
        q.ack(c);
        assert_eq!(q.peek(|_| true).unwrap().id, a);
    }

    #[test]
    fn filter_skips_disallowed_priorities() {
        let mut q = StoreForwardQueue::new(1000);
        q.push(Priority::Bulk, vec![0; 10], 0, 0);
        assert!(q.peek(|p| p >= Priority::Normal).is_none());
        assert!(q.peek(|_| true).is_some());
    }

    #[test]
    fn eviction_lowest_priority_oldest_first() {
        let mut q = StoreForwardQueue::new(30);
        let bulk1 = q.push(Priority::Bulk, vec![0; 10], 0, 0).unwrap();
        let bulk2 = q.push(Priority::Bulk, vec![0; 10], 1, 0).unwrap();
        q.push(Priority::High, vec![0; 10], 2, 0).unwrap();
        q.push(Priority::Critical, vec![0; 10], 3, 0).unwrap();
        assert_eq!(q.bytes(), 30);
        assert_eq!(q.evicted, 1);
        assert!(!q.ack(bulk1), "oldest bulk must be gone");
        assert!(q.ack(bulk2));
    }

    #[test]
    fn low_priority_never_evicts_higher() {
        let mut q = StoreForwardQueue::new(20);
        q.push(Priority::Critical, vec![0; 20], 0, 0).unwrap();
        assert!(q.push(Priority::Bulk, vec![0; 5], 1, 0).is_none());
        assert_eq!(q.len_of(Priority::Critical), 1);
    }

    #[test]
    fn sent_messages_wait_for_ack_then_retry() {
        let mut q = StoreForwardQueue::new(1000);
        let a = q.push(Priority::High, vec![1], 0, 0).unwrap();
        let b = q.push(Priority::High, vec![2], 0, 0).unwrap();
        q.mark_sent(a, 10);
        assert_eq!(q.peek_ready(|_| true, 11, 15).unwrap().id, b);
        q.mark_sent(b, 11);
        assert!(q.peek_ready(|_| true, 12, 15).is_none());
        assert_eq!(q.peek_ready(|_| true, 25, 15).unwrap().id, a, "retry after timeout");
        assert!(q.ack(a));
        assert_eq!(q.peek_ready(|_| true, 26, 15).unwrap().id, b);
    }

    #[test]
    fn ttl_expiry() {
        let mut q = StoreForwardQueue::new(100);
        q.push(Priority::High, vec![0; 4], 0, 10);
        q.push(Priority::High, vec![0; 4], 0, 0);
        assert_eq!(q.expire(9), 0);
        assert_eq!(q.expire(10), 1);
        assert_eq!(q.len(), 1);
        assert_eq!(q.bytes(), 4);
    }

    #[test]
    fn snapshot_roundtrip_and_corruption() {
        let mut q = StoreForwardQueue::new(1000);
        q.push(Priority::Critical, b"pos".to_vec(), 5, 60);
        q.push(Priority::Bulk, vec![7; 100], 6, 0);
        let bytes = q.to_bytes();
        let r = StoreForwardQueue::from_bytes(&bytes, 1000).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.bytes(), q.bytes());
        assert_eq!(r.peek(|_| true).unwrap().payload, b"pos");

        let mut bad = bytes.clone();
        bad[30] ^= 0xFF;
        assert!(StoreForwardQueue::from_bytes(&bad, 1000).is_err());
    }

    #[test]
    fn save_and_load_file() {
        let dir = std::env::temp_dir().join(format!("cfbq-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queue.bin");
        let mut q = StoreForwardQueue::new(1000);
        q.push(Priority::High, b"hello".to_vec(), 1, 0);
        q.save(&path).unwrap();
        let r = StoreForwardQueue::load_or_new(&path, 1000);
        assert_eq!(r.len(), 1);
        std::fs::write(&path, b"garbage").unwrap();
        assert!(StoreForwardQueue::load_or_new(&path, 1000).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
