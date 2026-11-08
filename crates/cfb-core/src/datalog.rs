// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Flight data logs written by the avionics itself, e.g. the CSV data log
//! that Garmin integrated flight decks (G1000 family) write to the SD card
//! once per second. The format is documented to operators in the pilot's
//! guides and widely used by analysis tools.
//!
//! This gives a data path with **no aircraft installation at all**: copy
//! the card (or, later, collect it automatically on ground) and run the
//! same edge apps on the ground (`cfb-replay`).
//!
//! The parser reads any comma-separated log: lines starting with `#`
//! (Garmin's airframe-info and units lines) are skipped, the first other line
//! is the column header, and columns are mapped to CFB parameter names by a
//! table. The default table follows common Garmin column names. **Check
//! every mapping and unit against a log from the actual installation.**
//! Column sets differ by aircraft and software version.

use std::collections::BTreeMap;

/// (log column name, CFB parameter, multiplier)
pub const GARMIN_DEFAULT_MAP: &[(&str, &str, f64)] = &[
    ("Latitude", "lat", 1.0),
    ("Longitude", "lon", 1.0),
    ("AltB", "alt", 1.0),
    ("AltMSL", "alt_msl", 1.0),
    ("GndSpd", "gs", 1.0),
    ("IAS", "ias", 1.0),
    ("TAS", "tas", 1.0),
    ("VSpd", "vs", 1.0),
    ("TRK", "track", 1.0),
    ("OAT", "oat", 1.0),
    ("NormAc", "nz", 1.0),
    ("Pitch", "pitch", 1.0),
    ("Roll", "roll", 1.0),
    ("E1 RPM", "rpm", 1.0),
    ("E1 FFlow", "fuel_flow", 1.0),
    ("E1 OilP", "oil_p", 1.0),
    ("E1 OilT", "oil_t", 1.0),
    ("E1 MAP", "map", 1.0),
    ("E1 Torq", "torque", 1.0),
    ("E1 ITT", "itt", 1.0),
    ("E1 NG", "ng", 1.0),
    ("E1 NP", "np", 1.0),
];

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Seconds since the first record.
    pub t: f64,
    pub values: BTreeMap<String, f64>,
}

#[derive(Debug, PartialEq)]
pub struct Log {
    pub columns: Vec<String>,
    /// Header columns that had a mapping.
    pub mapped: Vec<String>,
    pub records: Vec<Record>,
    pub skipped_lines: usize,
}

pub fn parse_csv(text: &str, map: &[(&str, &str, f64)]) -> Result<Log, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let header = lines.next().ok_or("no header line")?;
    let columns: Vec<String> = header.split(',').map(|c| c.trim().to_string()).collect();
    let idx: Vec<Option<(&str, f64)>> = columns
        .iter()
        .map(|c| map.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(c)).map(|(_, p, k)| (*p, *k)))
        .collect();
    let mapped = columns.iter().zip(&idx).filter(|(_, m)| m.is_some()).map(|(c, _)| c.clone()).collect();
    let mut records = Vec::new();
    let mut skipped = 0;
    for (n, line) in lines.enumerate() {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() < columns.len() / 2 {
            skipped += 1; // truncated line, e.g. power loss while writing
            continue;
        }
        let mut values = BTreeMap::new();
        for (v, m) in f.iter().zip(&idx) {
            if let (Some((p, k)), Ok(x)) = (m, v.parse::<f64>()) {
                if x.is_finite() {
                    values.insert(p.to_string(), x * k);
                }
            }
        }
        records.push(Record { t: n as f64, values });
    }
    Ok(Log { columns, mapped, records, skipped_lines: skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
#airframe_info, log_version=\"1.00\", airframe_name=\"Example\"
#yyy-mm-dd, hh:mm:ss,   deg,   deg,  ft, kt, fpm,   G
  Lcl Date, Lcl Time, Latitude, Longitude, AltB, GndSpd, VSpd, NormAc, E1 RPM, Unknown Col
2026-11-08, 10:00:00, 47.0000, 8.3000, 1400, 0.0, 0, 1.00, 0, x
2026-11-08, 10:00:01, 47.0001, 8.3001, 1400, 12.5, , 1.01, 2400, x
2026-11-08, 10:00:02, 47.0002
";

    #[test]
    fn parses_garmin_style_log() {
        let log = parse_csv(SAMPLE, GARMIN_DEFAULT_MAP).unwrap();
        assert_eq!(log.columns.len(), 10);
        assert_eq!(log.mapped.len(), 7);
        assert_eq!(log.records.len(), 2);
        assert_eq!(log.skipped_lines, 1, "truncated last line");
        let r = &log.records[1];
        assert_eq!(r.values["gs"], 12.5);
        assert_eq!(r.values["rpm"], 2400.0);
        assert!(!r.values.contains_key("vs"), "empty field = missing");
        assert_eq!(r.t, 1.0);
    }

    #[test]
    fn no_header_is_an_error() {
        assert!(parse_csv("#only comments\n", GARMIN_DEFAULT_MAP).is_err());
    }
}
