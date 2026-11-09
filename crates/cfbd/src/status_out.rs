// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Status snapshot for the maintenance page (`cfb-maint`), written once a
//! second to a tmpfs file (atomic rename). cfbd itself has no network
//! listener: the page is a separate process that only reads this file.
//! Field names are an interface: add fields, never rename (docs/12).

use serde::Serialize;
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

#[derive(Debug, Serialize, Default)]
pub struct LinkSnap {
    pub kind: String,
    pub up: bool,
    pub permitted: bool,
    pub detail: String,
}

#[derive(Debug, Serialize, Default)]
pub struct Snapshot {
    pub unix_time: u64,
    pub t: f64,
    pub version: String,
    pub registration: String,
    pub modules: Vec<String>,
    pub phase: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub pos_source: String,
    pub alt_ft: Option<i32>,
    pub alt_source: String,
    pub gs_kt: Option<f32>,
    pub engine_running: Option<bool>,
    pub engine_source: String,
    pub lights: BTreeMap<String, String>,
    pub faults: Vec<String>,
    pub rf_switch_off: bool,
    pub power_fail: bool,
    pub maintenance_mode: bool,
    pub links: Vec<LinkSnap>,
    pub queue_messages: usize,
    pub queue_bytes: usize,
    pub queue_max_bytes: usize,
    pub queue_evicted: u64,
    pub ota_state: String,
    pub update_gate: String,
    pub edge_apps: Vec<String>,
}

/// Atomic write: temp file + rename, so the reader never sees half a file.
pub fn write(path: &Path, s: &Snapshot) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(s).map_err(io::Error::other)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_valid_json_atomically() {
        let dir = std::env::temp_dir().join(format!("cfb-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("status.json");
        let s = Snapshot { phase: "OnGround".into(), faults: vec!["Gnss".into()], ..Default::default() };
        write(&p, &s).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(v["phase"], "OnGround");
        assert_eq!(v["faults"][0], "Gnss");
        assert!(!dir.join("status.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
