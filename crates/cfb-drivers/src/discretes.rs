// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Discrete inputs: from a text file (bench, maintenance page) and/or GPIO
//! read through a command (libgpiod `gpioget`).
//!
//! | Key | Meaning | Values |
//! |---|---|---|
//! | `wow` | weight on wheels | 1 ground, 0 air, - unknown |
//! | `eng` | engine running (oil pressure) | 1, 0, - |
//! | `maint` | maintenance mode | 1, 0 |
//! | `v` | supply voltage | volts or - |
//! | `rfsw` | physical radio switch | 1 radios may run, 0 switched OFF, - not fitted |
//! | `pfail` | power failing (UPS / hold-up signal) | 1, 0, - |

use std::collections::BTreeMap;
use std::process::Command;

pub const KEYS: [&str; 6] = ["wow", "eng", "maint", "v", "rfsw", "pfail"];

/// Parse a discretes file: only known keys and valid values.
pub fn parse_file(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for kv in text.split_whitespace() {
        if let Some((k, v)) = kv.split_once('=') {
            let ok = match k {
                "v" => v == "-" || v.parse::<f32>().is_ok(),
                "wow" | "eng" | "maint" | "rfsw" | "pfail" => matches!(v, "0" | "1" | "-"),
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
pub fn parse_gpio(out: &str) -> Option<bool> {
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

/// `KEY=CMD;KEY=CMD`, binary keys only.
pub fn parse_gpio_env(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|e| e.split_once('='))
        .map(|(k, c)| (k.trim().to_string(), c.trim().to_string()))
        .filter(|(k, c)| KEYS.contains(&k.as_str()) && k != "v" && !c.is_empty())
        .collect()
}

/// Read all discretes once: file first, GPIO commands override.
pub fn read(file: &str, gpio: &[(String, String)], invert: &[String]) -> BTreeMap<String, String> {
    let mut d = std::fs::read_to_string(file).map(|s| parse_file(&s)).unwrap_or_default();
    for (k, cmd) in gpio {
        let mut parts = cmd.split_whitespace();
        let Some(prog) = parts.next() else { continue };
        if let Ok(o) = Command::new(prog).args(parts).output() {
            if let Some(v) = parse_gpio(&String::from_utf8_lossy(&o.stdout)) {
                let v = v != invert.contains(k);
                d.insert(k.clone(), if v { "1" } else { "0" }.into());
            }
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_filtered() {
        let d = parse_file("wow=1 eng=0 maint=x v=13.6 rfsw=0 pfail=- evil=1 nmea=$X");
        assert_eq!(d.get("wow").map(String::as_str), Some("1"));
        assert_eq!(d.get("rfsw").map(String::as_str), Some("0"));
        assert!(!d.contains_key("maint"), "bad value dropped");
        assert_eq!(d.len(), 5, "unknown keys never forwarded");
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
    fn gpio_env() {
        let g = parse_gpio_env("wow=gpioget -c gpiochip0 17; pfail=gpioget -c gpiochip0 6;bad=x;v=y");
        assert_eq!(g.len(), 2);
        assert_eq!(g[1].0, "pfail");
    }

    #[test]
    fn portable_kit_gpio_setting_parses_as_intended() {
        // Exactly the value in os/systemd/io.env.portable-kit (libgpiod 2.x).
        let g = parse_gpio_env("rfsw=gpioget --bias=pull-up GPIO23;pfail=gpioget GPIO6");
        assert_eq!(g, vec![
            ("rfsw".to_string(), "gpioget --bias=pull-up GPIO23".to_string()),
            ("pfail".to_string(), "gpioget GPIO6".to_string()),
        ]);
        // gpioget 2.x output for the two states, after the configured inversion:
        // switch ON (pin pulled to GND) -> "inactive" -> rfsw=1 (radios may run);
        // wire broken (pull-up) -> "active" -> rfsw=0 (OFF, fail-safe).
        assert_eq!(parse_gpio("\"GPIO23\"=inactive").map(|v| !v), Some(true));
        assert_eq!(parse_gpio("\"GPIO23\"=active").map(|v| !v), Some(false));
    }

    #[test]
    fn gpio_overrides_file_with_inversion() {
        let dir = std::env::temp_dir().join(format!("cfb-disc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("d");
        std::fs::write(&f, "wow=1 rfsw=1").unwrap();
        let d = read(f.to_str().unwrap(), &[("rfsw".into(), "echo 1".into())], &["rfsw".into()]);
        assert_eq!(d.get("rfsw").map(String::as_str), Some("0"), "active-low switch");
        assert_eq!(d.get("wow").map(String::as_str), Some("1"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
