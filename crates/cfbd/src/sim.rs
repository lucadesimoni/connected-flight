// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Scripted flight for `--simulate`. Produces the same raw inputs the real
//! `cfb-io` would: ARINC 429 words, NMEA sentences, accelerometer samples,
//! discretes and link state.
//!
//! Script: parked, engine start, taxi, takeoff, climb, cruise with an ITT
//! excursion (t = 600–612 s), an OTA announcement (t = 700 s), an ARINC
//! outage (t = 900–1000 s) and a turbulence encounter (t = 1200 s), descent,
//! landing, taxi in, shutdown, maintenance mode.
//!
//! ARINC words are generated for whatever labels the config defines, from
//! the parameter name, so the simulator follows the installed label table.

use crate::input::{Inputs, Sample};
use cfb_core::arinc429::{Format, LabelDef, Ssm, Word};
use cfb_core::gnss::make_rmc_gga;
use cfb_core::{LinkKind, LinkStatus};
use std::collections::BTreeMap;

pub struct SimInputs {
    pub t: u32,
    pub hard_landing: bool,
    pub hw: Vec<String>,
    /// Labels to generate (empty = no ARINC module).
    pub labels: Vec<LabelDef>,
    pub seed: u32,
}

/// Engine values for the script: (torque %, ITT °C, Ng %, fuel flow kg/h).
fn engine_params(t: u32) -> (f64, f64, f64, f64) {
    match t {
        0..=59 | 1801.. => (0.0, 20.0, 0.0, 0.0),
        180..=399 => (100.0, 760.0, 101.0, 320.0),
        600..=612 => (86.0, 812.0, 97.5, 265.0),
        400..=1499 => (85.0, 720.0, 97.0, 260.0),
        1500..=1703 => (35.0, 600.0, 85.0, 150.0),
        _ => (5.0, 550.0, 65.0, 70.0),
    }
}

impl SimInputs {
    fn noise(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

const TAKEOFF_ROLL: std::ops::RangeInclusive<u32> = 180..=199;
const AIRBORNE: std::ops::RangeInclusive<u32> = 200..=1703;
const TOUCHDOWN: u32 = 1704;

impl Inputs for SimInputs {
    fn next(&mut self) -> Option<Sample> {
        let t = self.t;
        self.t += 1;
        if t > 1900 {
            return None;
        }
        let engine = (60..=1800).contains(&t);
        let gs: f32 = match t {
            0..=89 => 0.0,
            90..=179 => 15.0,
            180..=199 => 15.0 + (t - 180) as f32 * 4.0,
            200..=1703 => 140.0,
            1704..=1720 => 70.0 - (t - 1704) as f32 * 3.5,
            1721..=1800 => 12.0,
            _ => 0.0,
        };
        let wow = !(196..=1703).contains(&t);
        let (alt, vs): (f64, f64) = match t {
            200..=399 => (1400.0 + (t - 200) as f64 * 35.5, 2130.0),
            400..=1499 => (8500.0, 0.0),
            1500..=1703 => (8500.0 - (t - 1500) as f64 * 34.8, -2090.0),
            _ => (1400.0, 0.0),
        };
        let d = t.saturating_sub(200).min(1500) as f64;
        let (lat, lon) = (47.0 + d * 0.0005, 8.3 + d * 0.0003);
        let trk = 31.0;

        // ARINC 429, for every configured BNR label the script knows.
        let (tq, itt, ng, ff) = engine_params(t);
        let mut a429 = Vec::new();
        if !(900..=1000).contains(&t) {
            for l in self.labels.iter().filter(|l| l.format == Format::Bnr) {
                let v = match l.param.as_str() {
                    "latitude" => lat,
                    "longitude" => lon,
                    "ground_speed" => gs as f64,
                    "track" => trk,
                    "pressure_altitude" => alt,
                    "altitude_rate" => vs,
                    "torque" => tq,
                    "itt" => itt,
                    "ng" => ng,
                    "fuel_flow" => ff,
                    "oat" => 15.0 - 2.0 * alt / 1000.0,
                    _ => continue,
                };
                let Ok(num) = u16::from_str_radix(&l.label, 8) else { continue };
                let w = Word::encode_bnr(num, l.sdi.unwrap_or(0), v, l.range, l.sig_bits, Ssm::Normal);
                a429.push((l.channel.unwrap_or(0), w.0));
            }
        }

        // Own GNSS: ~10 m from the aircraft source, geometric altitude.
        let nmea = make_rmc_gga(lat + 0.0001, lon, gs, trk as f32, alt as f32 + 200.0, 36_000 + t).to_vec();

        // Accelerometer, 100 Hz, sensor axes = aircraft axes here.
        let rolling = wow && gs > 5.0;
        let vib = match (engine, rolling, AIRBORNE.contains(&t)) {
            (false, _, _) => 0.003,
            (true, true, _) => 0.20,
            (true, _, true) => 0.06,
            _ => 0.08,
        };
        let long = if TAKEOFF_ROLL.contains(&t) {
            0.25
        } else if (TOUCHDOWN..=1720).contains(&t) {
            -0.18
        } else {
            0.0
        };
        let mut imu = Vec::with_capacity(100);
        for i in 0..100 {
            let mut nz = 1.0;
            if t == TOUCHDOWN && (30..34).contains(&i) {
                nz = if self.hard_landing { 2.3 } else { 1.45 };
            }
            if t == 1200 && (50..53).contains(&i) {
                nz = 2.15;
            }
            let s = [long + vib * self.noise(), vib * self.noise(), -nz + vib * self.noise()];
            imu.push(s);
        }

        let mut links = BTreeMap::new();
        let at_base = !(120..=1820).contains(&t);
        let mut put = |kind, up, est_bps| {
            links.insert(kind, LinkStatus { kind, up, est_bps });
        };
        put(LinkKind::Wifi, at_base, 50_000_000);
        put(LinkKind::Cellular, !AIRBORNE.contains(&t), 20_000_000);
        put(LinkKind::Iridium, true, 22_000);
        put(LinkKind::LeoSat, true, 50_000_000); // only used if fitted and enabled

        Some(Sample {
            t: t as f64,
            wow: Some(wow),
            engine_discrete: None, // no oil-pressure discrete: IMU decides
            supply_v: Some(if engine { 28.2 } else { 24.6 }),
            maintenance: t > 1850,
            a429,
            nmea,
            imu,
            links,
            hw: (t == 0).then(|| self.hw.clone()),
            ..Default::default()
        })
    }
}
