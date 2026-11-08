// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Own GNSS receiver (NMEA 0183): reference position, independent of the
//! aircraft.
//!
//! Used for three things:
//! * position/speed when the aircraft has no ARINC 429 navigation source or
//!   it is lost;
//! * cross-check of the aircraft source (`reference.rs`);
//! * time: sets the system clock at boot (no RTC battery in the box).
//!
//! Only RMC and GGA are needed. Talker IDs GP, GN, GL, GA, GB are all accepted.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GnssFix {
    pub valid: bool,
    pub lat_deg: f64,
    pub lon_deg: f64,
    pub gs_kt: f32,
    pub track_deg: f32,
    /// Altitude above MSL in feet (from GGA).
    pub alt_ft: Option<f32>,
    pub sats: u8,
    pub hdop: Option<f32>,
    /// UTC time of day in seconds (from RMC).
    pub utc_tod_s: Option<f64>,
    /// UTC date (day, month, 2-digit year) from RMC.
    pub date: Option<(u8, u8, u8)>,
    /// Monotonic time of last valid RMC.
    pub t: f64,
}

#[derive(Debug, Default)]
pub struct NmeaParser {
    pub fix: GnssFix,
    pub checksum_errors: u64,
}

fn checksum_ok(line: &str) -> Option<&str> {
    let line = line.trim();
    let body = line.strip_prefix('$')?;
    let (data, cs) = body.rsplit_once('*')?;
    let want = u8::from_str_radix(cs.get(..2)?, 16).ok()?;
    let got = data.bytes().fold(0u8, |a, b| a ^ b);
    (got == want).then_some(data)
}

/// ddmm.mmmm + hemisphere -> signed degrees.
fn coord(v: &str, hemi: &str, deg_digits: usize) -> Option<f64> {
    if v.len() < deg_digits + 2 {
        return None;
    }
    let d: f64 = v[..deg_digits].parse().ok()?;
    let m: f64 = v[deg_digits..].parse().ok()?;
    if m >= 60.0 {
        return None;
    }
    let x = d + m / 60.0;
    match hemi {
        "N" | "E" => Some(x),
        "S" | "W" => Some(-x),
        _ => None,
    }
}

fn tod(v: &str) -> Option<f64> {
    if v.len() < 6 {
        return None;
    }
    let h: f64 = v[0..2].parse().ok()?;
    let m: f64 = v[2..4].parse().ok()?;
    let s: f64 = v[4..].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

impl NmeaParser {
    /// Feed one sentence. Returns true if it updated the fix.
    pub fn feed(&mut self, line: &str, t: f64) -> bool {
        let Some(data) = checksum_ok(line) else {
            if line.trim_start().starts_with('$') {
                self.checksum_errors += 1;
            }
            return false;
        };
        let f: Vec<&str> = data.split(',').collect();
        let kind = f[0].get(2..).unwrap_or("");
        match kind {
            // $GPRMC,hhmmss.ss,A,llll.ll,a,yyyyy.yy,a,x.x,x.x,ddmmyy,...
            "RMC" if f.len() >= 10 => {
                let valid = f[2] == "A";
                let lat = coord(f[3], f[4], 2);
                let lon = coord(f[5], f[6], 3);
                let fix = &mut self.fix;
                fix.utc_tod_s = tod(f[1]);
                fix.date = (f[9].len() == 6)
                    .then(|| Some((f[9][0..2].parse().ok()?, f[9][2..4].parse().ok()?, f[9][4..6].parse().ok()?)))
                    .flatten();
                match (valid, lat, lon) {
                    (true, Some(lat), Some(lon)) => {
                        fix.valid = true;
                        fix.lat_deg = lat;
                        fix.lon_deg = lon;
                        fix.gs_kt = f[7].parse().unwrap_or(0.0);
                        fix.track_deg = f[8].parse().unwrap_or(fix.track_deg);
                        fix.t = t;
                    }
                    _ => fix.valid = false,
                }
                true
            }
            // $GPGGA,hhmmss,llll.ll,a,yyyyy.yy,a,q,nn,hdop,alt,M,...
            "GGA" if f.len() >= 10 => {
                let q: u8 = f[6].parse().unwrap_or(0);
                self.fix.sats = f[7].parse().unwrap_or(0);
                self.fix.hdop = f[8].parse().ok();
                self.fix.alt_ft = if q > 0 { f[9].parse::<f32>().ok().map(|m| m * 3.28084) } else { None };
                if q == 0 {
                    self.fix.valid = false;
                }
                true
            }
            _ => false,
        }
    }

    /// Fix if valid and not older than `max_age` seconds.
    pub fn current(&self, now: f64, max_age: f64) -> Option<GnssFix> {
        (self.fix.valid && now - self.fix.t <= max_age).then_some(self.fix)
    }
}

/// Build an NMEA sentence with checksum (simulator and tests).
pub fn sentence(body: &str) -> String {
    let cs = body.bytes().fold(0u8, |a, b| a ^ b);
    format!("${body}*{cs:02X}")
}

/// RMC + GGA for a position (simulator and tests).
pub fn make_rmc_gga(lat: f64, lon: f64, gs_kt: f32, trk: f32, alt_ft: f32, tod_s: u32) -> [String; 2] {
    let fmt = |v: f64, deg_digits: usize| {
        let a = v.abs();
        let d = a.trunc();
        format!("{:0w$}{:07.4}", d as u32, (a - d) * 60.0, w = deg_digits)
    };
    let (ns, ew) = (if lat >= 0.0 { "N" } else { "S" }, if lon >= 0.0 { "E" } else { "W" });
    let hms = format!("{:02}{:02}{:02}.00", tod_s / 3600 % 24, tod_s / 60 % 60, tod_s % 60);
    [
        sentence(&format!(
            "GNRMC,{hms},A,{},{ns},{},{ew},{gs_kt:.1},{trk:.1},081126,,,A",
            fmt(lat, 2),
            fmt(lon, 3)
        )),
        sentence(&format!(
            "GNGGA,{hms},{},{ns},{},{ew},1,12,0.8,{:.1},M,47.0,M,,",
            fmt(lat, 2),
            fmt(lon, 3),
            alt_ft / 3.28084
        )),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reference_rmc() {
        // Classic NMEA example sentence.
        let mut p = NmeaParser::default();
        assert!(p.feed("$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6A", 1.0));
        let f = p.current(1.0, 2.0).unwrap();
        assert!((f.lat_deg - 48.1173).abs() < 1e-4);
        assert!((f.lon_deg - 11.516_666).abs() < 1e-4);
        assert_eq!(f.gs_kt, 22.4);
        assert_eq!(f.track_deg, 84.4);
        assert_eq!(f.utc_tod_s, Some(12.0 * 3600.0 + 35.0 * 60.0 + 19.0));
        assert_eq!(f.date, Some((23, 3, 94)));
    }

    #[test]
    fn parses_reference_gga() {
        let mut p = NmeaParser::default();
        assert!(p.feed("$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47", 0.0));
        assert_eq!(p.fix.sats, 8);
        assert_eq!(p.fix.hdop, Some(0.9));
        assert!((p.fix.alt_ft.unwrap() - 1789.4).abs() < 0.5);
    }

    #[test]
    fn rejects_bad_checksum_and_void_fix() {
        let mut p = NmeaParser::default();
        assert!(!p.feed("$GPRMC,123519,A,4807.038,N,01131.000,E,022.4,084.4,230394,003.1,W*6B", 0.0));
        assert_eq!(p.checksum_errors, 1);
        p.feed(&sentence("GPRMC,123519,V,,,,,,,230394,,"), 0.0);
        assert!(p.current(0.0, 1.0).is_none());
    }

    #[test]
    fn stale_fix_not_returned() {
        let mut p = NmeaParser::default();
        let [rmc, _] = make_rmc_gga(47.0, 8.0, 100.0, 90.0, 5000.0, 3600);
        p.feed(&rmc, 10.0);
        assert!(p.current(11.0, 2.0).is_some());
        assert!(p.current(13.0, 2.0).is_none());
    }

    #[test]
    fn generator_roundtrip_southern_western() {
        let mut p = NmeaParser::default();
        let [rmc, gga] = make_rmc_gga(-33.946_1, -70.785_2, 135.5, 271.0, 8500.0, 45_296);
        assert!(p.feed(&rmc, 0.0) && p.feed(&gga, 0.0));
        let f = p.current(0.0, 1.0).unwrap();
        assert!((f.lat_deg + 33.946_1).abs() < 1e-5);
        assert!((f.lon_deg + 70.785_2).abs() < 1e-5);
        assert_eq!(f.gs_kt, 135.5);
        assert!((f.alt_ft.unwrap() - 8500.0).abs() < 1.0);
        assert_eq!(f.utc_tod_s, Some(45_296.0));
    }
}
