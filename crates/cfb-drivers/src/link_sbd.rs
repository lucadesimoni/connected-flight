// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Iridium SBD bearer (9602/9603 transceivers, e.g. RockBLOCK 9603).
//!
//! Three layers:
//! * [`Modem`]: the AT-command protocol over any `Read + Write` (serial port
//!   in service, simulated modem in tests);
//! * a worker thread that runs sessions (they take tens of seconds), packs
//!   several CFB frames into one MO message (`cfb_core::sbd::pack`), backs
//!   off on bad signal or failures, and reopens the port after I/O errors;
//! * [`SbdLink`]: the [`LinkDriver`] the daemon sees. "Acked" means the MO
//!   message reached the Iridium gateway (`+SBDIX` MO status 0–4).
//!
//! Not yet: reading mobile-terminated (uplink) messages (`AT+SBDRB`);
//! they stay queued at the gateway.

use crate::link::{LinkDriver, Rx};
use cfb_core::sbd::{checksum, pack, parse_csq, parse_sbdix, Sbdix};
use cfb_core::{LinkKind, LinkStatus};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ------------------------------------------------------------ protocol

pub struct Modem<P: Read + Write> {
    port: P,
    buf: Vec<u8>,
    /// How long one empty read takes (the port's read timeout).
    tick: Duration,
}

impl<P: Read + Write> Modem<P> {
    pub fn new(port: P, tick: Duration) -> Self {
        Self { port, buf: Vec::new(), tick }
    }

    fn read_line(&mut self, deadline: Instant) -> io::Result<Option<String>> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n' || *b == b'\r') {
                let line: Vec<u8> = self.buf.drain(..=i).collect();
                let s = String::from_utf8_lossy(&line).trim().to_string();
                if s.is_empty() {
                    continue;
                }
                return Ok(Some(s));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            let mut tmp = [0u8; 256];
            let n = self.port.read(&mut tmp)?;
            if n == 0 {
                std::thread::sleep(self.tick.min(Duration::from_millis(50)));
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() > 4096 {
                // Junk without line ends: drop it (security review finding S3).
                self.buf.clear();
            }
        }
    }

    /// Send a command; collect lines until one of `until`, or ERROR.
    fn cmd(&mut self, c: &str, until: &[&str], timeout: Duration) -> io::Result<Vec<String>> {
        self.port.write_all(format!("{c}\r").as_bytes())?;
        self.port.flush()?;
        self.collect(c, until, timeout)
    }

    fn collect(&mut self, echo: &str, until: &[&str], timeout: Duration) -> io::Result<Vec<String>> {
        let deadline = Instant::now() + timeout;
        let mut lines = Vec::new();
        while let Some(l) = self.read_line(deadline)? {
            if l == echo {
                continue; // command echo (before ATE0 takes effect)
            }
            if l == "ERROR" {
                return Err(io::Error::other(format!("{echo}: ERROR")));
            }
            let done = until.contains(&l.as_str());
            lines.push(l);
            if done {
                return Ok(lines);
            }
        }
        Err(io::Error::new(io::ErrorKind::TimedOut, format!("{echo}: timeout")))
    }

    pub fn init(&mut self) -> io::Result<()> {
        self.cmd("ATE0", &["OK"], Duration::from_secs(3))?;
        self.cmd("AT&K0", &["OK"], Duration::from_secs(3))?; // no hardware flow control
        Ok(())
    }

    pub fn signal(&mut self) -> io::Result<u8> {
        let l = self.cmd("AT+CSQ", &["OK"], Duration::from_secs(60))?;
        l.iter().find_map(|x| parse_csq(x)).ok_or_else(|| io::Error::other("no +CSQ"))
    }

    /// Write the MO buffer, run a session, clear the buffer on success.
    pub fn send_mo(&mut self, payload: &[u8]) -> io::Result<Sbdix> {
        let c = format!("AT+SBDWB={}", payload.len());
        self.cmd(&c, &["READY"], Duration::from_secs(5))?;
        self.port.write_all(payload)?;
        self.port.write_all(&checksum(payload))?;
        self.port.flush()?;
        let l = self.collect("", &["OK"], Duration::from_secs(10))?;
        match l.first().map(String::as_str) {
            Some("0") => {}
            other => return Err(io::Error::other(format!("SBDWB rejected: {other:?}"))),
        }
        let l = self.cmd("AT+SBDIX", &["OK"], Duration::from_secs(90))?;
        let r = l.iter().find_map(|x| parse_sbdix(x)).ok_or_else(|| io::Error::other("no +SBDIX"))?;
        if r.mo_ok() {
            let _ = self.cmd("AT+SBDD0", &["OK"], Duration::from_secs(5));
        }
        Ok(r)
    }
}

// ------------------------------------------------------------ worker

#[derive(Debug, Clone, Copy)]
pub struct SbdTiming {
    /// Wait for new frames when idle.
    pub idle: Duration,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// Signal check interval while idle.
    pub signal_every: Duration,
    /// Port read timeout (stty `time`).
    pub port_tick: Duration,
}

impl Default for SbdTiming {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(1),
            backoff_min: Duration::from_secs(20),
            backoff_max: Duration::from_secs(300),
            signal_every: Duration::from_secs(60),
            port_tick: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Shared {
    signal: Option<u8>,
    consecutive_failures: u32,
    sessions_ok: u64,
    sessions_failed: u64,
    last_error: Option<String>,
}

type Opener<P> = Box<dyn Fn() -> io::Result<P> + Send>;

fn worker<P: Read + Write>(
    open: Opener<P>,
    jobs: Receiver<(u64, Vec<u8>)>,
    acks: Sender<Vec<u64>>,
    shared: Arc<Mutex<Shared>>,
    min_signal: u8,
    max_mo: usize,
    tm: SbdTiming,
) {
    let mut pending: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut modem: Option<Modem<P>> = None;
    let mut backoff = tm.backoff_min;
    let mut not_before = Instant::now();
    let mut last_signal = Instant::now() - tm.signal_every;
    let fail = |s: &Arc<Mutex<Shared>>, e: String| {
        let mut g = s.lock().unwrap();
        g.consecutive_failures += 1;
        g.sessions_failed += 1;
        g.last_error = Some(e);
    };
    loop {
        // Collect new frames (deduplicated by message id).
        let wait = if pending.is_empty() { tm.idle } else { Duration::from_millis(1) };
        match jobs.recv_timeout(wait) {
            Ok((id, f)) => {
                if !pending.iter().any(|(i, _)| *i == id) {
                    pending.push((id, f));
                }
                while let Ok((id, f)) = jobs.try_recv() {
                    if !pending.iter().any(|(i, _)| *i == id) {
                        pending.push((id, f));
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if Instant::now() < not_before {
            continue;
        }
        if modem.is_none() {
            match open().map(|p| Modem::new(p, tm.port_tick)).and_then(|mut m| m.init().map(|_| m)) {
                Ok(m) => modem = Some(m),
                Err(e) => {
                    shared.lock().unwrap().last_error = Some(format!("open: {e}"));
                    not_before = Instant::now() + tm.backoff_min;
                    continue;
                }
            }
        }
        let m = modem.as_mut().unwrap();
        if pending.is_empty() {
            if last_signal.elapsed() >= tm.signal_every {
                last_signal = Instant::now();
                match m.signal() {
                    Ok(s) => shared.lock().unwrap().signal = Some(s),
                    Err(_) => modem = None,
                }
            }
            continue;
        }
        let sig = match m.signal() {
            Ok(s) => s,
            Err(e) => {
                fail(&shared, format!("signal: {e}"));
                modem = None;
                not_before = Instant::now() + backoff;
                continue;
            }
        };
        shared.lock().unwrap().signal = Some(sig);
        if sig < min_signal {
            not_before = Instant::now() + tm.backoff_min;
            continue;
        }
        let (payload, ids) = pack(pending.iter().map(|(i, f)| (*i, f.as_slice())), max_mo);
        if ids.is_empty() {
            pending.clear(); // only unpackable frames: drop (link refuses them up front)
            continue;
        }
        match m.send_mo(&payload) {
            Ok(r) if r.mo_ok() => {
                pending.retain(|(i, _)| !ids.contains(i));
                let mut g = shared.lock().unwrap();
                g.consecutive_failures = 0;
                g.sessions_ok += 1;
                drop(g);
                let _ = acks.send(ids);
                backoff = tm.backoff_min;
            }
            Ok(r) => {
                fail(&shared, format!("SBDIX MO status {}", r.mo_status));
                not_before = Instant::now() + backoff;
                backoff = (backoff * 2).min(tm.backoff_max);
            }
            Err(e) => {
                fail(&shared, e.to_string());
                modem = None;
                not_before = Instant::now() + backoff;
                backoff = (backoff * 2).min(tm.backoff_max);
            }
        }
    }
}

// ------------------------------------------------------------ link

pub struct SbdLink {
    jobs: Sender<(u64, Vec<u8>)>,
    acks: Receiver<Vec<u64>>,
    shared: Arc<Mutex<Shared>>,
    /// Message ids handed to the worker and not yet acked, with sizes.
    inflight: HashMap<u64, usize>,
    min_signal: u8,
    max_mo: usize,
}

impl SbdLink {
    /// Serial modem: `stty` setup, then the worker thread.
    pub fn open_serial(device: &str, min_signal: u8, max_mo: usize) -> io::Result<Self> {
        crate::serial::configure(device, 19_200, 10)?;
        let dev = device.to_string();
        Ok(Self::with_opener(Box::new(move || crate::serial::open_rw(&dev)), min_signal, max_mo, SbdTiming::default()))
    }

    pub fn with_opener<P: Read + Write + 'static>(
        open: Opener<P>,
        min_signal: u8,
        max_mo: usize,
        tm: SbdTiming,
    ) -> Self {
        let (jtx, jrx) = channel();
        let (atx, arx) = channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let s2 = shared.clone();
        std::thread::spawn(move || worker(open, jrx, atx, s2, min_signal, max_mo, tm));
        Self { jobs: jtx, acks: arx, shared, inflight: HashMap::new(), min_signal, max_mo }
    }
}

impl LinkDriver for SbdLink {
    fn kind(&self) -> LinkKind {
        LinkKind::Iridium
    }

    fn status(&self, _t: f64) -> LinkStatus {
        let g = self.shared.lock().unwrap();
        // Optimistic until a signal reading says otherwise (metered link,
        // no probing traffic); down after repeated session failures.
        let up = g.signal.is_none_or(|s| s >= self.min_signal) && g.consecutive_failures < 5;
        LinkStatus { kind: LinkKind::Iridium, up, est_bps: 2_400 }
    }

    fn tick_budget(&self) -> usize {
        self.max_mo
    }

    fn send(&mut self, frame: &[u8], msg_id: u64, _t: f64) -> io::Result<()> {
        if frame.is_empty() || frame.len() > 255 || frame.len() + 1 > self.max_mo {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large for SBD"));
        }
        if self.inflight.contains_key(&msg_id) {
            return Ok(()); // already with the worker (daemon retry): no duplicate
        }
        if self.inflight.values().sum::<usize>() + frame.len() > 4 * self.max_mo {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "SBD backlog full"));
        }
        self.inflight.insert(msg_id, frame.len());
        self.jobs.send((msg_id, frame.to_vec())).map_err(|_| io::Error::other("SBD worker stopped"))
    }

    fn poll(&mut self, _t: f64) -> Vec<Rx> {
        let mut out = Vec::new();
        while let Ok(ids) = self.acks.try_recv() {
            for id in ids {
                self.inflight.remove(&id);
                out.push(Rx::Acked(id));
            }
        }
        out
    }

    fn describe(&self) -> String {
        let g = self.shared.lock().unwrap();
        format!(
            "Iridium SBD signal={:?} ok={} failed={} inflight={} last_error={:?}",
            g.signal, g.sessions_ok, g.sessions_failed, self.inflight.len(), g.last_error
        )
    }
}

// ------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Simulated 9603: answers AT commands like the real modem.
    #[derive(Clone)]
    struct FakeModem(Arc<Mutex<FakeState>>);

    struct FakeState {
        out: VecDeque<u8>,
        line: Vec<u8>,
        binary_left: usize,
        binary: Vec<u8>,
        csq: u8,
        mo_status: u8,
        momsn: u32,
        delivered: Vec<Vec<u8>>,
        buffer: Vec<u8>,
    }

    impl FakeModem {
        fn new(csq: u8, mo_status: u8) -> Self {
            FakeModem(Arc::new(Mutex::new(FakeState {
                out: VecDeque::new(),
                line: vec![],
                binary_left: 0,
                binary: vec![],
                csq,
                mo_status,
                momsn: 0,
                delivered: vec![],
                buffer: vec![],
            })))
        }
    }

    impl Read for FakeModem {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            let mut s = self.0.lock().unwrap();
            let n = b.len().min(s.out.len());
            for x in b.iter_mut().take(n) {
                *x = s.out.pop_front().unwrap();
            }
            Ok(n)
        }
    }

    impl Write for FakeModem {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            let mut s = self.0.lock().unwrap();
            for &byte in data {
                if s.binary_left > 0 {
                    s.binary.push(byte);
                    s.binary_left -= 1;
                    if s.binary_left == 0 {
                        let n = s.binary.len() - 2;
                        let (p, cs) = s.binary.split_at(n);
                        let ok = checksum(p) == [cs[0], cs[1]];
                        let p = p.to_vec();
                        if ok {
                            s.buffer = p;
                        }
                        s.out.extend(if ok { b"\r\n0\r\n\r\nOK\r\n".to_vec() } else { b"\r\n2\r\n\r\nOK\r\n".to_vec() });
                        s.binary.clear();
                    }
                    continue;
                }
                if byte != b'\r' {
                    s.line.push(byte);
                    continue;
                }
                let cmd = String::from_utf8(std::mem::take(&mut s.line)).unwrap();
                let resp: Vec<u8> = if let Some(n) = cmd.strip_prefix("AT+SBDWB=") {
                    s.binary_left = n.parse::<usize>().unwrap() + 2;
                    b"READY\r\n".to_vec()
                } else {
                    match cmd.as_str() {
                        "AT+CSQ" => format!("\r\n+CSQ:{}\r\n\r\nOK\r\n", s.csq).into_bytes(),
                        "AT+SBDIX" => {
                            s.momsn += 1;
                            if s.mo_status <= 4 {
                                let b = s.buffer.clone();
                                s.delivered.push(b);
                            }
                            format!("\r\n+SBDIX: {}, {}, 0, 0, 0, 0\r\n\r\nOK\r\n", s.mo_status, s.momsn).into_bytes()
                        }
                        "AT+SBDD0" => b"\r\n0\r\n\r\nOK\r\n".to_vec(),
                        _ => b"\r\nOK\r\n".to_vec(),
                    }
                };
                s.out.extend(resp);
            }
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn fast() -> SbdTiming {
        SbdTiming {
            idle: Duration::from_millis(5),
            backoff_min: Duration::from_millis(20),
            backoff_max: Duration::from_millis(100),
            signal_every: Duration::from_millis(50),
            port_tick: Duration::from_millis(1),
        }
    }

    #[test]
    fn modem_protocol_session() {
        let fake = FakeModem::new(4, 0);
        let mut m = Modem::new(fake.clone(), Duration::from_millis(1));
        m.init().unwrap();
        assert_eq!(m.signal().unwrap(), 4);
        let r = m.send_mo(b"\x05hello").unwrap();
        assert!(r.mo_ok());
        assert_eq!(fake.0.lock().unwrap().delivered, vec![b"\x05hello".to_vec()]);
    }

    fn wait_acks(link: &mut SbdLink, n: usize) -> Vec<u64> {
        wait_acks_for(link, n, Duration::from_secs(5))
    }

    fn wait_acks_for(link: &mut SbdLink, n: usize, max: Duration) -> Vec<u64> {
        let mut ids = Vec::new();
        let t0 = Instant::now();
        while ids.len() < n && t0.elapsed() < max {
            for r in link.poll(0.0) {
                if let Rx::Acked(id) = r {
                    ids.push(id);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        ids
    }

    #[test]
    fn link_batches_frames_and_acks_after_gateway_ok() {
        let fake = FakeModem::new(4, 0);
        let f2 = fake.clone();
        let mut link = SbdLink::with_opener(Box::new(move || Ok(f2.clone())), 2, 340, fast());
        link.send(&[0xCF; 30], 1, 0.0).unwrap();
        link.send(&[0xCF; 30], 2, 0.0).unwrap();
        link.send(&[0xCF; 30], 1, 0.0).unwrap(); // daemon retry: deduplicated
        let mut ids = wait_acks(&mut link, 2);
        ids.sort();
        assert_eq!(ids, vec![1, 2]);
        let delivered = fake.0.lock().unwrap().delivered.clone();
        let frames: usize = delivered.iter().map(|d| cfb_core::sbd::unpack(d).unwrap().len()).sum();
        assert_eq!(frames, 2, "each frame delivered exactly once");
        assert!(link.status(0.0).up);
    }

    #[test]
    fn no_ack_without_gateway_success_and_link_goes_down() {
        let fake = FakeModem::new(4, 32); // 32 = no network service
        let f2 = fake.clone();
        let mut link = SbdLink::with_opener(Box::new(move || Ok(f2.clone())), 2, 340, fast());
        link.send(&[0xCF; 30], 9, 0.0).unwrap();
        assert!(wait_acks_for(&mut link, 1, Duration::from_millis(400)).is_empty());
        assert!(!link.status(0.0).up, "repeated failures: down");
        // Gateway recovers: the frame still goes out and is acked.
        fake.0.lock().unwrap().mo_status = 0;
        assert_eq!(wait_acks(&mut link, 1), vec![9]);
    }

    #[test]
    fn weak_signal_holds_frames() {
        let fake = FakeModem::new(1, 0);
        let f2 = fake.clone();
        let mut link = SbdLink::with_opener(Box::new(move || Ok(f2.clone())), 2, 340, fast());
        link.send(&[0xCF; 30], 5, 0.0).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(link.poll(0.0).is_empty());
        assert!(!link.status(0.0).up, "signal below minimum");
        assert!(fake.0.lock().unwrap().delivered.is_empty());
        fake.0.lock().unwrap().csq = 3;
        assert_eq!(wait_acks(&mut link, 1), vec![5]);
    }

    #[test]
    fn oversize_frame_refused_up_front() {
        let fake = FakeModem::new(4, 0);
        let mut link = SbdLink::with_opener(Box::new(move || Ok(fake.clone())), 2, 340, fast());
        let e = link.send(&[0u8; 300], 1, 0.0).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    }
}
