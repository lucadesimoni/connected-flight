// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Input from the `cfb-io` adapter service: one line per second on a FIFO.
//!
//! ```text
//! t=<mono s> wow=<0|1|-> eng=<0|1|-> v=<volts|-> maint=<0|1>
//! rfsw=<0|1|->                           physical radio switch (0 = OFF)
//! pfail=<0|1|->                          power failing
//! baro=<hPa>                             own barometer
//! a429=<ch>:<hex>,<ch>:<hex>,...        ARINC 429 words received this second
//! nmea=<sentence>                        repeatable, own GNSS
//! imu=<ax>,<ay>,<az>;<ax>,<ay>,<az>;...  raw accelerometer samples (g, sensor axes)
//! link.<wifi|cellular|leo_sat|iridium>=<up:bps|down>
//! hw=<id>,<id>,...                       module IDs read from ID EEPROMs (at boot)
//! ```
//!
//! Token order rule (CFB-HLR-111): bulk tokens last, largest at the very
//! end: `... nmea=… imu=… a429=…`. A line longer than `MAX_LINE` is cut, and
//! only the tail is lost; GNSS and discretes always survive. At the physical
//! bus maximum (4 high-speed ARINC channels, ~11 100 words/s) a line is
//! ~125 KB, within `MAX_LINE` with a 2x margin.
//!
//! `eng` is the oil-pressure discrete if one is wired; send `-` otherwise and
//! the IMU vibration is used.

use cfb_core::{LinkKind, LinkStatus};
use std::collections::BTreeMap;
use std::io::{BufRead, Read};

/// Longest accepted input line (100 Hz IMU samples need ~3 KiB). A longer
/// line is cut and its remainder discarded, so a broken producer can't
/// exhaust memory (security review finding S2).
pub const MAX_LINE: u64 = 256 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub t: f64,
    pub wow: Option<bool>,
    pub engine_discrete: Option<bool>,
    pub supply_v: Option<f32>,
    pub maintenance: bool,
    pub rf_switch: Option<bool>,
    pub power_fail: bool,
    pub baro_hpa: Option<f64>,
    pub a429: Vec<(u8, u32)>,
    pub nmea: Vec<String>,
    pub imu: Vec<[f32; 3]>,
    pub links: BTreeMap<LinkKind, LinkStatus>,
    pub hw: Option<Vec<String>>,
    /// The line was longer than MAX_LINE and was cut (CFB-HLR-111).
    pub truncated: bool,
}

pub trait Inputs {
    fn next(&mut self) -> Option<Sample>;
}

fn opt<T: std::str::FromStr>(v: &str) -> Option<T> {
    if v == "-" { None } else { v.parse().ok() }
}

pub fn link_kind(s: &str) -> Option<LinkKind> {
    Some(match s {
        "wifi" => LinkKind::Wifi,
        "cellular" => LinkKind::Cellular,
        "leo_sat" => LinkKind::LeoSat,
        "iridium" => LinkKind::Iridium,
        _ => return None,
    })
}

pub struct LineInputs<R: BufRead> {
    pub r: R,
    pub links: BTreeMap<LinkKind, LinkStatus>,
}

pub fn parse_line(line: &str, links: &mut BTreeMap<LinkKind, LinkStatus>) -> Sample {
    let mut s = Sample::default();
    for kv in line.split_whitespace() {
        let Some((k, v)) = kv.split_once('=') else { continue };
        match k {
            "t" => s.t = v.parse().unwrap_or(0.0),
            "wow" => s.wow = opt::<u8>(v).map(|x| x != 0),
            "eng" => s.engine_discrete = opt::<u8>(v).map(|x| x != 0),
            "v" => s.supply_v = opt(v),
            "maint" => s.maintenance = v == "1",
            "rfsw" => s.rf_switch = opt::<u8>(v).map(|x| x != 0),
            "pfail" => s.power_fail = v == "1",
            "baro" => s.baro_hpa = opt(v),
            "a429" => {
                for w in v.split(',') {
                    // Exactly 8 hex digits: a word cut by line truncation
                    // must be dropped, not decoded as a shorter number.
                    if let Some((ch, hex)) = w.split_once(':') {
                        if hex.len() == 8 {
                            if let (Ok(ch), Ok(word)) = (ch.parse(), u32::from_str_radix(hex, 16)) {
                                s.a429.push((ch, word));
                            }
                        }
                    }
                }
            }
            "nmea" => s.nmea.push(v.to_string()),
            "imu" => {
                for smp in v.split(';') {
                    let a: Vec<f32> = smp.split(',').filter_map(|x| x.parse().ok()).collect();
                    if a.len() == 3 {
                        s.imu.push([a[0], a[1], a[2]]);
                    }
                }
            }
            "hw" => s.hw = Some(v.split(',').map(str::to_string).collect()),
            _ if k.starts_with("link.") => {
                if let Some(kind) = link_kind(&k[5..]) {
                    let (up, bps) = match v.split_once(':') {
                        Some(("up", b)) => (true, b.parse().unwrap_or(0)),
                        _ => (false, 0),
                    };
                    links.insert(kind, LinkStatus { kind, up, est_bps: bps });
                }
            }
            _ => {}
        }
    }
    s.links = links.clone();
    s
}

impl<R: BufRead> Inputs for LineInputs<R> {
    fn next(&mut self) -> Option<Sample> {
        let mut buf = Vec::new();
        let n = (&mut self.r).take(MAX_LINE).read_until(b'\n', &mut buf).ok()?;
        if n == 0 {
            return None; // end of input
        }
        let truncated = !buf.ends_with(b"\n") && n as u64 == MAX_LINE;
        if truncated {
            // Over-long line: discard the rest in bounded chunks.
            loop {
                let mut sink = Vec::new();
                match (&mut self.r).take(MAX_LINE).read_until(b'\n', &mut sink) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if sink.ends_with(b"\n") => break,
                    Ok(_) => {}
                }
            }
        }
        // Invalid UTF-8 must not stop the daemon: decode lossily.
        let mut sample = parse_line(&String::from_utf8_lossy(&buf), &mut self.links);
        sample.truncated = truncated;
        Some(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_and_bad_utf8_do_not_stop_input() {
        let mut data = Vec::new();
        data.extend_from_slice(b"t=1 wow=1\n");
        data.extend(std::iter::repeat_n(b'x', (MAX_LINE as usize) * 2 + 10));
        data.extend_from_slice(b"\nt=3 wow=\xff\xfe1\nt=4 wow=0\n");
        let mut li = LineInputs { r: std::io::Cursor::new(data), links: BTreeMap::new() };
        assert_eq!(li.next().unwrap().t, 1.0);
        let truncated = li.next().unwrap();
        assert_eq!(truncated.t, 0.0, "garbage line parsed as empty sample");
        let bad = li.next().unwrap();
        assert_eq!((bad.t, bad.wow), (3.0, None), "invalid UTF-8 value ignored");
        assert_eq!(li.next().unwrap().wow, Some(false));
        assert!(li.next().is_none());
    }

    #[test]
    fn truncated_line_keeps_early_tokens_and_drops_partial_words() {
        let mut line = String::from("t=7 wow=1 nmea=$GNRMC,1*00 imu=0,0,-1 a429=");
        while line.len() < MAX_LINE as usize + 1000 {
            line.push_str("0:e0000a41,");
        }
        line.push('\n');
        let mut li = LineInputs { r: std::io::Cursor::new(line.into_bytes()), links: BTreeMap::new() };
        let s = li.next().unwrap();
        assert!(s.truncated);
        assert_eq!((s.t, s.wow, s.nmea.len(), s.imu.len()), (7.0, Some(true), 1, 1), "early tokens survive");
        assert!(s.a429.iter().all(|(_, w)| *w == 0xe000_0a41), "no partial word decoded");
        let mut links = BTreeMap::new();
        assert!(parse_line("a429=0:e0000a41,1:e0000a,2:zz", &mut links).a429.len() == 1);
    }

    #[test]
    fn parses_all_tokens() {
        let mut links = BTreeMap::new();
        let s = parse_line(
            "t=12 wow=1 eng=- v=27.9 maint=0 a429=0:9c000013,1:e0000a41 \
             nmea=$GNRMC,1*00 nmea=$GNGGA,2*00 imu=0.01,0,-1;0.02,0,-1.01 \
             link.cellular=up:20000000 link.iridium=down hw=base,gnss,imu",
            &mut links,
        );
        assert_eq!(s.t, 12.0);
        assert_eq!(s.wow, Some(true));
        assert_eq!(s.engine_discrete, None);
        assert_eq!(s.a429, vec![(0, 0x9c00_0013), (1, 0xe000_0a41)]);
        assert_eq!(s.nmea.len(), 2);
        assert_eq!(s.imu.len(), 2);
        assert!(s.links[&LinkKind::Cellular].up);
        assert!(!s.links[&LinkKind::Iridium].up);
        assert_eq!(s.hw.unwrap().len(), 3);
    }
}
