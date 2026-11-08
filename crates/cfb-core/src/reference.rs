// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Reference selection and cross-check.
//!
//! Each quantity comes from the best source that is fitted and valid right
//! now:
//!
//! | Quantity | 1st | 2nd |
//! |---|---|---|
//! | position, ground speed, track | aircraft (ARINC 429) | own GNSS |
//! | altitude | aircraft pressure altitude | own GNSS altitude |
//! | vertical speed | aircraft altitude rate | – |
//! | engine running | oil-pressure discrete | own IMU vibration |
//!
//! The aircraft source comes first because it is the installed, approved
//! navigation source. When both are valid they are compared; a disagreement
//! raises a health message to the ground (CFB-HLR-061). Nothing is shown to
//! the crew and no aircraft function uses these values.

use crate::arinc429::Decoder;
use crate::gnss::GnssFix;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ReferenceConfig {
    /// ARINC parameters older than this are stale (labels typically refresh
    /// at 2–10 Hz).
    pub arinc_max_age_s: f64,
    pub gnss_max_age_s: f64,
    /// Position disagreement threshold, NM.
    pub pos_disagree_nm: f64,
    /// Ground speed disagreement threshold, kt.
    pub gs_disagree_kt: f32,
}

impl Default for ReferenceConfig {
    fn default() -> Self {
        Self { arinc_max_age_s: 2.0, gnss_max_age_s: 2.0, pos_disagree_nm: 0.5, gs_disagree_kt: 15.0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    #[default]
    None,
    /// Aircraft system over ARINC 429.
    Aircraft,
    /// The CFB's own GNSS receiver.
    OwnGnss,
    /// Oil-pressure (or similar) discrete.
    Discrete,
    /// The CFB's own accelerometer.
    OwnImu,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reference {
    pub pos: Option<(f64, f64)>,
    pub pos_src: Source,
    pub gs_kt: Option<f32>,
    pub track_deg: Option<f32>,
    pub alt_ft: Option<i32>,
    pub alt_src: Source,
    pub vs_fpm: Option<i32>,
    pub engine_running: Option<bool>,
    pub engine_src: Source,
    /// Both sources valid but far apart (CFB-HLR-061).
    pub pos_disagree: bool,
    pub gs_disagree: bool,
}

/// Approximate distance in NM (equirectangular; fine below a few hundred NM).
pub fn distance_nm(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dlat = (a.0 - b.0) * 60.0;
    let dlon = (a.1 - b.1) * 60.0 * ((a.0 + b.0) / 2.0).to_radians().cos();
    (dlat * dlat + dlon * dlon).sqrt()
}

/// CFB-HLR-060: pick each quantity from the best valid source.
pub fn select(
    cfg: &ReferenceConfig,
    now: f64,
    arinc: Option<&Decoder>,
    gnss: Option<GnssFix>,
    engine_discrete: Option<bool>,
    engine_vibration: Option<bool>,
) -> Reference {
    let a = |p: &str| arinc.and_then(|d| d.get(p, now, cfg.arinc_max_age_s));
    let g = gnss.filter(|f| f.valid && now - f.t <= cfg.gnss_max_age_s);

    let ac_pos = a("latitude").zip(a("longitude"));
    let own_pos = g.map(|f| (f.lat_deg, f.lon_deg));
    let ac_gs = a("ground_speed").map(|v| v as f32);
    let own_gs = g.map(|f| f.gs_kt);

    let mut r = Reference::default();
    if let Some(p) = ac_pos {
        r.pos = Some(p);
        r.pos_src = Source::Aircraft;
        r.gs_kt = ac_gs.or(own_gs);
        r.track_deg = a("track").map(|v| v.rem_euclid(360.0) as f32).or(g.map(|f| f.track_deg));
    } else if let Some(p) = own_pos {
        r.pos = Some(p);
        r.pos_src = Source::OwnGnss;
        r.gs_kt = own_gs;
        r.track_deg = g.map(|f| f.track_deg);
    } else {
        r.gs_kt = ac_gs;
    }

    if let Some(alt) = a("pressure_altitude") {
        r.alt_ft = Some(alt.round() as i32);
        r.alt_src = Source::Aircraft;
    } else if let Some(alt) = g.and_then(|f| f.alt_ft) {
        r.alt_ft = Some(alt.round() as i32);
        r.alt_src = Source::OwnGnss;
    }
    r.vs_fpm = a("altitude_rate").map(|v| v.round() as i32);

    if let (Some(x), Some(y)) = (ac_pos, own_pos) {
        r.pos_disagree = distance_nm(x, y) > cfg.pos_disagree_nm;
    }
    if let (Some(x), Some(y)) = (ac_gs, own_gs) {
        r.gs_disagree = (x - y).abs() > cfg.gs_disagree_kt;
    }

    (r.engine_running, r.engine_src) = match (engine_discrete, engine_vibration) {
        (Some(e), _) => (Some(e), Source::Discrete),
        (None, Some(e)) => (Some(e), Source::OwnImu),
        _ => (None, Source::None),
    };
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arinc429::{default_labels, Ssm, Word};

    fn arinc(lat: f64, lon: f64, gs: f64, t: f64) -> Decoder {
        let mut d = Decoder::new(default_labels()).unwrap();
        d.feed(0, Word::encode_bnr(0o310, 0, lat, 180.0, 20, Ssm::Normal), t);
        d.feed(0, Word::encode_bnr(0o311, 0, lon, 180.0, 20, Ssm::Normal), t);
        d.feed(0, Word::encode_bnr(0o312, 0, gs, 4096.0, 15, Ssm::Normal), t);
        d.feed(0, Word::encode_bnr(0o203, 0, 8500.0, 131_072.0, 17, Ssm::Normal), t);
        d
    }

    fn fix(lat: f64, lon: f64, gs: f32, t: f64) -> GnssFix {
        GnssFix { valid: true, lat_deg: lat, lon_deg: lon, gs_kt: gs, alt_ft: Some(8700.0), t, ..Default::default() }
    }

    #[test]
    fn aircraft_source_preferred_when_valid() {
        let d = arinc(47.0, 8.0, 140.0, 10.0);
        let r = select(&ReferenceConfig::default(), 10.0, Some(&d), Some(fix(47.0, 8.0, 141.0, 10.0)), None, None);
        assert_eq!(r.pos_src, Source::Aircraft);
        assert_eq!(r.alt_src, Source::Aircraft);
        assert_eq!(r.alt_ft, Some(8500));
        assert!(!r.pos_disagree && !r.gs_disagree);
    }

    #[test]
    fn falls_back_to_own_gnss_when_arinc_stale_or_absent() {
        let d = arinc(47.0, 8.0, 140.0, 0.0);
        let r = select(&ReferenceConfig::default(), 10.0, Some(&d), Some(fix(47.1, 8.1, 139.0, 10.0)), None, None);
        assert_eq!(r.pos_src, Source::OwnGnss);
        assert_eq!(r.pos, Some((47.1, 8.1)));
        assert_eq!(r.alt_src, Source::OwnGnss);
        let r = select(&ReferenceConfig::default(), 10.0, None, Some(fix(47.1, 8.1, 139.0, 10.0)), None, None);
        assert_eq!(r.pos_src, Source::OwnGnss);
    }

    #[test]
    fn disagreement_flagged() {
        let d = arinc(47.0, 8.0, 140.0, 10.0);
        // ~1.2 NM north, 30 kt different
        let r = select(&ReferenceConfig::default(), 10.0, Some(&d), Some(fix(47.02, 8.0, 110.0, 10.0)), None, None);
        assert!(r.pos_disagree && r.gs_disagree);
        assert_eq!(r.pos_src, Source::Aircraft, "still uses aircraft source");
    }

    #[test]
    fn no_sources_gives_none() {
        let r = select(&ReferenceConfig::default(), 0.0, None, None, None, None);
        assert_eq!(r.pos, None);
        assert_eq!(r.pos_src, Source::None);
        assert_eq!(r.engine_running, None);
    }

    #[test]
    fn engine_discrete_overrides_vibration() {
        let c = ReferenceConfig::default();
        let r = select(&c, 0.0, None, None, Some(false), Some(true));
        assert_eq!((r.engine_running, r.engine_src), (Some(false), Source::Discrete));
        let r = select(&c, 0.0, None, None, None, Some(true));
        assert_eq!((r.engine_running, r.engine_src), (Some(true), Source::OwnImu));
    }

    #[test]
    fn distance_sanity() {
        assert!((distance_nm((47.0, 8.0), (48.0, 8.0)) - 60.0).abs() < 1e-9);
        assert!((distance_nm((60.0, 0.0), (60.0, 1.0)) - 30.0).abs() < 0.01);
    }
}
