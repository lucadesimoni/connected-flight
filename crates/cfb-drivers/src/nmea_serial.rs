// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! GNSS receiver on a serial/USB device: forwards RMC and GGA sentences.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Only the sentences the core uses; no spaces (line-protocol tokens).
pub fn is_wanted(l: &str) -> bool {
    l.is_ascii() && l.starts_with('$') && matches!(l.get(3..6), Some("RMC" | "GGA")) && !l.contains(' ')
}

/// Reader thread: reopens the device after unplug; keeps at most 20 lines
/// between drains.
pub fn spawn_reader(dev: String, buf: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || loop {
        match File::open(&dev) {
            Ok(f) => {
                eprintln!("gnss: {dev} open");
                for line in BufReader::new(f).lines() {
                    let Ok(l) = line else { break };
                    let l = l.trim().to_string();
                    if is_wanted(&l) {
                        let mut b = buf.lock().unwrap();
                        if b.len() < 20 {
                            b.push(l);
                        }
                    }
                }
                eprintln!("gnss: {dev} closed, reopening");
            }
            Err(e) => eprintln!("gnss: {dev}: {e}"),
        }
        std::thread::sleep(Duration::from_secs(1));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_rmc_gga_forwarded() {
        assert!(is_wanted("$GNRMC,1*00"));
        assert!(is_wanted("$GPGGA,1*00"));
        assert!(!is_wanted("$GPGSV,1*00"));
        assert!(!is_wanted("$GNRMC,a b*00"));
        assert!(!is_wanted("garbage"));
        assert!(!is_wanted("$G\u{e9}RMC,1*00"), "non-ASCII must not panic");
    }
}
