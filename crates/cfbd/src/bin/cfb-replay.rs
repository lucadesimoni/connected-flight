// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-replay - run edge app packages over an existing avionics flight log
//! (e.g. a Garmin-style CSV data log from the SD card), on the ground.
//!
//!   cfb-replay --apps config/apps log_260108_100000_HBXXX.csv [more.csv ...]
//!
//! Same rule interpreter as on board, no aircraft installation needed.
//! Phase comes from ground speed (WoW is not in the log); engine running
//! from RPM, Ng or fuel flow.

use cfb_core::datalog::{parse_csv, GARMIN_DEFAULT_MAP};
use cfb_core::edge::{AppPackage, DataView, EdgeRuntime, Limits};
use cfb_core::phase::{PhaseConfig, PhaseDetector, PhaseInputs};
use std::collections::BTreeMap;

fn main() {
    let mut apps_dir = "config/apps".to_string();
    let mut logs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--apps" => apps_dir = args.next().unwrap_or(apps_dir),
            "-h" | "--help" => {
                println!("cfb-replay [--apps DIR] LOG.csv ...");
                return;
            }
            _ => logs.push(a),
        }
    }
    if logs.is_empty() {
        eprintln!("cfb-replay: no log files given");
        std::process::exit(2);
    }
    for path in logs {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        let log = match parse_csv(&text, GARMIN_DEFAULT_MAP) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("{path}: {e}");
                continue;
            }
        };
        println!("== {path}: {} records, mapped columns: {}", log.records.len(), log.mapped.join(", "));
        // Ground analysis: no flight byte budget (the data is already on the ground).
        let mut rt = EdgeRuntime::new(Limits { flight_bytes: u64::MAX, ..Limits::default() });
        if let Ok(rd) = std::fs::read_dir(&apps_dir) {
            let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
            files.sort();
            for f in files {
                match std::fs::read(&f).map_err(|e| e.to_string()).and_then(|b| AppPackage::parse(&b)) {
                    Ok(p) => rt.add(Box::new(p.into_app())),
                    Err(e) => eprintln!("skip {}: {e}", f.display()),
                }
            }
        }
        let mut det = PhaseDetector::new(PhaseConfig::default());
        for r in &log.records {
            let engine = ["rpm", "ng", "fuel_flow"].iter().find_map(|k| r.values.get(*k)).map(|v| *v > 1.0);
            let phase = det.update(PhaseInputs {
                t: r.t,
                wow: None,
                ground_speed_kt: r.values.get("gs").map(|v| *v as f32),
                engine_running: engine,
            });
            let mut dv = DataView { t: r.t, phase: Some(phase), values: BTreeMap::new() };
            for (k, v) in &r.values {
                dv.set(k, Some(*v));
            }
            dv.set("engine", engine.map(|e| if e { 1.0 } else { 0.0 }));
            if let Some(nz) = r.values.get("nz") {
                dv.set("nz_max", Some(*nz));
                dv.set("nz_min", Some(*nz));
            }
            let (outs, evs) = rt.tick(&dv);
            for o in outs {
                println!("[{:>6.0}s] {}", r.t, String::from_utf8_lossy(&o.payload));
            }
            for e in evs {
                println!("[{:>6.0}s] runtime: {e:?}", r.t);
            }
        }
    }
}
