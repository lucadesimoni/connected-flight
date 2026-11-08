// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Input from the `cfb-io` adapter service: one line per second on a FIFO.
//!
//! ```text
//! t=<mono s> wow=<0|1|-> eng=<0|1|-> v=<volts|-> maint=<0|1>
//! a429=<ch>:<hex>,<ch>:<hex>,...        ARINC 429 words received this second
//! nmea=<sentence>                        repeatable, own GNSS
//! imu=<ax>,<ay>,<az>;<ax>,<ay>,<az>;...  raw accelerometer samples (g, sensor axes)
//! link.<wifi|cellular|leo_sat|iridium>=<up:bps|down>
//! hw=<id>,<id>,...                       module IDs read from ID EEPROMs (at boot)
//! ```
//!
//! `eng` is the oil-pressure discrete if one is wired; send `-` otherwise and
//! the IMU vibration is used.

use cfb_core::{LinkKind, LinkStatus};
use std::collections::BTreeMap;
use std::io::BufRead;

#[derive(Debug, Clone, Default)]
pub struct Sample {
    pub t: f64,
    pub wow: Option<bool>,
    pub engine_discrete: Option<bool>,
    pub supply_v: Option<f32>,
    pub maintenance: bool,
    pub a429: Vec<(u8, u32)>,
    pub nmea: Vec<String>,
    pub imu: Vec<[f32; 3]>,
    pub links: BTreeMap<LinkKind, LinkStatus>,
    pub hw: Option<Vec<String>>,
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
            "a429" => {
                for w in v.split(',') {
                    if let Some((ch, hex)) = w.split_once(':') {
                        if let (Ok(ch), Ok(word)) = (ch.parse(), u32::from_str_radix(hex, 16)) {
                            s.a429.push((ch, word));
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
        let mut line = String::new();
        if self.r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        Some(parse_line(&line, &mut self.links))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
