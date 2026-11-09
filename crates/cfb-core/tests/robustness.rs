// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Robustness tests (CFB-HLR-110): every parser of data from outside the
//! box must reject garbage without panicking. Deterministic pseudo-random
//! inputs (no extra dependencies, same result on every run), plus mutations
//! of valid inputs, which reach deeper into the parsers than pure noise.

use cfb_core::arinc429::{default_labels, Decoder, Survey, Word};
use cfb_core::config::Config;
use cfb_core::datalog::{parse_csv, GARMIN_DEFAULT_MAP};
use cfb_core::edge::AppPackage;
use cfb_core::frame::{decode_ack, frame_seq, Frame, MsgType, PositionReport};
use cfb_core::gnss::{make_rmc_gga, NmeaParser};
use cfb_core::queue::StoreForwardQueue;
use cfb_core::sbd;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn bytes(&mut self, max: usize) -> Vec<u8> {
        let n = (self.next() as usize) % (max + 1);
        (0..n).map(|_| self.next() as u8).collect()
    }
    /// Flip, insert or delete a few bytes of a valid input.
    fn mutate(&mut self, v: &[u8]) -> Vec<u8> {
        let mut m = v.to_vec();
        for _ in 0..1 + self.next() % 4 {
            let i = if m.is_empty() { 0 } else { (self.next() as usize) % m.len() };
            match self.next() % 3 {
                0 if !m.is_empty() => m[i] ^= 1 << (self.next() % 8),
                1 => m.insert(i, self.next() as u8),
                _ if !m.is_empty() => {
                    m.remove(i);
                }
                _ => {}
            }
        }
        m
    }
    /// Text with some multi-byte UTF-8 mixed in (slicing at byte offsets
    /// panics on non-char boundaries: a classic parser bug).
    fn text(&mut self, max: usize) -> String {
        const ALPH: &[&str] = &["$", ",", "*", ".", "-", "0", "1", "9", "A", "N", "E", "G", "RMC", "GGA", "é", "€", "\u{1F6E9}", " ", "=", ";", ":", "\n"];
        let n = (self.next() as usize) % (max + 1);
        (0..n).map(|_| ALPH[(self.next() as usize) % ALPH.len()]).collect()
    }
}

const N: usize = 20_000;

#[test]
fn frame_and_position_decoders_never_panic() {
    let mut r = Rng(1);
    let valid = Frame { msg_type: MsgType::Position, seq: 7, time: 9, payload: vec![1; 17] }.encode();
    for _ in 0..N {
        let b = if r.next().is_multiple_of(2) { r.bytes(64) } else { r.mutate(&valid) };
        let _ = Frame::decode(&b);
        let _ = PositionReport::decode(&b);
        let _ = decode_ack(&b);
        let _ = frame_seq(&b);
    }
}

#[test]
fn sbd_parsers_never_panic() {
    let mut r = Rng(2);
    let (valid, _) = sbd::pack([(1, &[0xCFu8; 30][..]), (2, &[1u8; 5][..])], 340);
    for _ in 0..N {
        let b = if r.next().is_multiple_of(2) { r.bytes(400) } else { r.mutate(&valid) };
        let _ = sbd::unpack(&b);
        let t = r.text(60);
        let _ = sbd::parse_sbdix(&t);
        let _ = sbd::parse_csq(&t);
        let _ = sbd::from_hex(&t);
        let _ = sbd::parse_sbdix(&format!("+SBDIX:{t}"));
    }
}

#[test]
fn nmea_parser_never_panics() {
    let mut r = Rng(3);
    let [rmc, gga] = make_rmc_gga(47.1, 8.3, 120.0, 90.0, 5000.0, 3600);
    let mut p = NmeaParser::default();
    for i in 0..N {
        let line = match i % 3 {
            0 => r.text(90),
            1 => String::from_utf8_lossy(&r.mutate(rmc.as_bytes())).into_owned(),
            _ => String::from_utf8_lossy(&r.mutate(gga.as_bytes())).into_owned(),
        };
        p.feed(&line, i as f64);
        // Also with a recomputed checksum, so the field parsers are reached.
        if let Some(body) = line.strip_prefix('$').and_then(|l| l.split('*').next()) {
            p.feed(&cfb_core::gnss::sentence(body), i as f64);
        }
    }
}

#[test]
fn arinc_decoder_and_survey_never_panic() {
    let mut r = Rng(4);
    let mut d = Decoder::new(default_labels()).unwrap();
    let mut s = Survey::default();
    for i in 0..N {
        let w = Word(r.next() as u32);
        d.feed((r.next() % 4) as u8, w, i as f64);
        s.feed((r.next() % 4) as u8, w, i as f64);
        let _ = w.bnr(180.0, (r.next() % 25) as u8);
        let _ = w.bcd(0.1);
    }
    let _ = s.compare(&default_labels());
}

#[test]
fn queue_snapshot_loader_never_panics() {
    let mut r = Rng(5);
    let mut q = StoreForwardQueue::new(10_000);
    q.push(cfb_core::Priority::High, vec![1, 2, 3], 0, 0);
    let valid = q.to_bytes();
    for _ in 0..N {
        let b = if r.next().is_multiple_of(2) { r.bytes(200) } else { r.mutate(&valid) };
        let _ = StoreForwardQueue::from_bytes(&b, 10_000);
    }
}

#[test]
fn text_formats_never_panic() {
    let mut r = Rng(6);
    let cfg = include_str!("../../../config/cfb-portable-pi5.toml");
    let app = include_str!("../../../config/apps/turboprop-ops.toml");
    let csv = "#x\n  Lcl Date, Latitude, Longitude, GndSpd\n2026-11-08, 47.0, 8.3, 12.5\n";
    for i in 0..2_000u32 {
        // Config and app packages are signed, but must still never crash
        // the parser; mutate valid files.
        let c = String::from_utf8_lossy(&r.mutate(cfg.as_bytes())).into_owned();
        let _ = Config::from_toml(&c);
        let _ = AppPackage::parse(&r.mutate(app.as_bytes()));
        let t = if i.is_multiple_of(2) { r.text(200) } else { String::from_utf8_lossy(&r.mutate(csv.as_bytes())).into_owned() };
        let _ = parse_csv(&t, GARMIN_DEFAULT_MAP);
    }
}
