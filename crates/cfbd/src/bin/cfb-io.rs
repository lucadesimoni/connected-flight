// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-io - minimal I/O adapter for the bench / portable unit.
//!
//! Writes one line per second in the cfbd input format to stdout:
//!
//!   cfb-io --gnss /dev/ttyACM0 --discretes /run/cfb/discretes \
//!          --hw base,gnss,wifi,cellular_4g \
//!          [--gpio "wow=gpioget -c gpiochip0 17"] [--invert wow] | cfbd ...
//!
//! * GNSS: reads NMEA lines from the device (USB receivers appear as
//!   /dev/ttyACM*; set the baud rate of a UART receiver with `stty` first).
//!   Only RMC and GGA are forwarded. If the device disappears it is reopened.
//! * Discretes: `wow`, `eng`, `maint`, `v` from a small text file (e.g.
//!   `wow=1 eng=0 maint=0 v=13.6`), edited by hand on the bench or written by
//!   a maintenance page. Missing file = all unknown (`-`).
//! * GPIO: optional command per discrete, run once per second (libgpiod
//!   `gpioget` v1 prints `0`/`1`, v2 prints `"17"=active|inactive`). GPIO
//!   values override the file. `--invert` for active-low opto inputs.
//!   Also from the environment (for systemd): `CFB_GPIO="wow=gpioget -c
//!   gpiochip0 17;eng=gpioget -c gpiochip0 27"`, `CFB_INVERT="wow"`.
//! * Links: not reported here; with `[transport]` configured, cfbd measures
//!   link health itself.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const KEYS: [&str; 4] = ["wow", "eng", "maint", "v"];

/// Parse a discretes file: only known keys, values `0|1|-` (or a number for v).
fn parse_discretes(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for kv in text.split_whitespace() {
        if let Some((k, v)) = kv.split_once('=') {
            let ok = match k {
                "v" => v == "-" || v.parse::<f32>().is_ok(),
                "wow" | "eng" | "maint" => matches!(v, "0" | "1" | "-"),
                _ => false,
            };
            if ok {
                m.insert(k.to_string(), v.to_string());
            }
        }
    }
    m
}

/// Interpret gpioget output (v1: "1"/"0", v2: "\"17\"=active"/"=inactive").
fn parse_gpio(out: &str) -> Option<bool> {
    let o = out.trim();
    if o.ends_with("=inactive") {
        Some(false)
    } else if o.ends_with("=active") {
        Some(true)
    } else {
        match o {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }
}

/// `KEY=CMD;KEY=CMD`, only known discrete keys.
fn parse_gpio_env(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|e| e.split_once('='))
        .map(|(k, c)| (k.trim().to_string(), c.trim().to_string()))
        .filter(|(k, c)| KEYS.contains(&k.as_str()) && k != "v" && !c.is_empty())
        .collect()
}

fn is_wanted_nmea(l: &str) -> bool {
    l.starts_with('$') && l.len() > 6 && (&l[3..6] == "RMC" || &l[3..6] == "GGA")
}

fn gnss_reader(dev: String, buf: Arc<Mutex<Vec<String>>>) {
    loop {
        match File::open(&dev) {
            Ok(f) => {
                eprintln!("cfb-io: GNSS {dev} open");
                for line in BufReader::new(f).lines() {
                    let Ok(l) = line else { break };
                    let l = l.trim().to_string();
                    // No spaces inside a token of the line protocol.
                    if is_wanted_nmea(&l) && !l.contains(' ') {
                        let mut b = buf.lock().unwrap();
                        if b.len() < 20 {
                            b.push(l);
                        }
                    }
                }
                eprintln!("cfb-io: GNSS {dev} closed, reopening");
            }
            Err(e) => eprintln!("cfb-io: GNSS {dev}: {e}"),
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn main() {
    let mut gnss = None;
    let mut discretes = "/run/cfb/discretes".to_string();
    let mut hw = None;
    let mut gpio: Vec<(String, String)> = Vec::new();
    let mut invert: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--gnss" => gnss = args.next(),
            "--discretes" => discretes = args.next().unwrap_or(discretes),
            "--hw" => hw = args.next(),
            "--gpio" => {
                if let Some((k, cmd)) = args.next().and_then(|s| s.split_once('=').map(|(a, b)| (a.to_string(), b.to_string()))) {
                    gpio.push((k, cmd));
                }
            }
            "--invert" => invert.extend(args.next()),
            _ => {
                eprintln!("usage: cfb-io [--gnss DEV] [--discretes FILE] [--hw IDS] [--gpio KEY=CMD]... [--invert KEY]...");
                std::process::exit(2);
            }
        }
    }

    if let Ok(g) = std::env::var("CFB_GPIO") {
        gpio.extend(parse_gpio_env(&g));
    }
    if let Ok(i) = std::env::var("CFB_INVERT") {
        invert.extend(i.split_whitespace().map(String::from));
    }

    let nmea = Arc::new(Mutex::new(Vec::new()));
    if let Some(dev) = gnss {
        let b = nmea.clone();
        std::thread::spawn(move || gnss_reader(dev, b));
    }

    let start = Instant::now();
    let mut stdout = std::io::stdout().lock();
    let mut first = true;
    for tick in 0u64.. {
        let mut d = std::fs::read_to_string(&discretes).map(|s| parse_discretes(&s)).unwrap_or_default();
        for (k, cmd) in &gpio {
            let mut parts = cmd.split_whitespace();
            let Some(prog) = parts.next() else { continue };
            if let Ok(o) = Command::new(prog).args(parts).output() {
                if let Some(v) = parse_gpio(&String::from_utf8_lossy(&o.stdout)) {
                    let v = v != invert.contains(k);
                    d.insert(k.clone(), if v { "1" } else { "0" }.into());
                }
            }
        }
        let mut line = format!("t={tick}");
        for k in KEYS {
            line.push_str(&format!(" {k}={}", d.get(k).map(String::as_str).unwrap_or("-")));
        }
        for s in nmea.lock().unwrap().drain(..) {
            line.push_str(" nmea=");
            line.push_str(&s);
        }
        if first {
            if let Some(h) = &hw {
                line.push_str(&format!(" hw={h}"));
            }
            first = false;
        }
        if writeln!(stdout, "{line}").and_then(|_| stdout.flush()).is_err() {
            return; // cfbd went away; systemd restarts the pipeline
        }
        let next = start + Duration::from_secs(tick + 1);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discretes_file_filtered() {
        let d = parse_discretes("wow=1 eng=0 maint=x v=13.6 evil=1 nmea=$X");
        assert_eq!(d.get("wow").map(String::as_str), Some("1"));
        assert_eq!(d.get("eng").map(String::as_str), Some("0"));
        assert!(!d.contains_key("maint"), "bad value dropped");
        assert_eq!(d.get("v").map(String::as_str), Some("13.6"));
        assert_eq!(d.len(), 3, "unknown keys never forwarded");
    }

    #[test]
    fn gpioget_v1_and_v2_output() {
        assert_eq!(parse_gpio("1\n"), Some(true));
        assert_eq!(parse_gpio("0"), Some(false));
        assert_eq!(parse_gpio("\"17\"=active\n"), Some(true));
        assert_eq!(parse_gpio("\"17\"=inactive"), Some(false));
        assert_eq!(parse_gpio("error"), None);
    }

    #[test]
    fn gpio_from_environment() {
        let g = parse_gpio_env("wow=gpioget -c gpiochip0 17; eng=gpioget -c gpiochip0 27;bad=x;v=y");
        assert_eq!(g.len(), 2);
        assert_eq!(g[0], ("wow".into(), "gpioget -c gpiochip0 17".into()));
    }

    #[test]
    fn only_rmc_gga_forwarded() {
        assert!(is_wanted_nmea("$GNRMC,1*00"));
        assert!(is_wanted_nmea("$GPGGA,1*00"));
        assert!(!is_wanted_nmea("$GPGSV,1*00"));
        assert!(!is_wanted_nmea("garbage"));
    }
}
