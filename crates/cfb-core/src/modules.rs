// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Hardware module configuration.
//!
//! The CFB is a base unit plus plug-in modules (see docs/03-hardware.md):
//!
//! | Module | Fitted | Provides |
//! |---|---|---|
//! | Base: CM5, power, discretes, Wi-Fi | always | compute, WoW / engine / maintenance inputs, Wi-Fi |
//! | Reference sensor: GNSS + IMU | standard | own position, time, acceleration |
//! | ARINC 429 receiver (2 or 4 RX) | option | aircraft navigation / air data / engine labels |
//! | Cellular 4G or 5G | standard | ground link |
//! | Satcom: Iridium and/or LEO | option | in-flight link |
//!
//! Each module carries an ID EEPROM. `cfb-io` reads them at boot and reports
//! the list; [`Modules::check_detected`] compares it with the installed
//! configuration. A mismatch is a maintenance message, and the missing
//! function is disabled; the box keeps running with what it has
//! (CFB-HLR-070).

use crate::link::LinkKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArincSpeed {
    /// 12.5 kbit/s
    Low,
    /// 100 kbit/s
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArincModule {
    /// Receive channels fitted (2 or 4).
    pub channels: u8,
    pub speed: ArincSpeed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cellular {
    None,
    #[serde(rename = "4g")]
    Lte4g,
    #[serde(rename = "5g")]
    Nr5g,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Modules {
    #[serde(default)]
    pub arinc429: Option<ArincModule>,
    pub gnss: bool,
    pub imu: bool,
    pub wifi: bool,
    pub cellular: Cellular,
    /// Satcom bearers fitted: `iridium`, `leo_sat`. Empty = no satcom.
    #[serde(default)]
    pub satcom: Vec<LinkKind>,
}

impl Modules {
    /// Is the hardware for this link fitted?
    pub fn has_link(&self, k: LinkKind) -> bool {
        match k {
            LinkKind::Wifi => self.wifi,
            LinkKind::Cellular => self.cellular != Cellular::None,
            LinkKind::Iridium | LinkKind::LeoSat => self.satcom.contains(&k),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(a) = &self.arinc429 {
            if !(a.channels == 2 || a.channels == 4) {
                return Err("arinc429.channels must be 2 or 4".into());
            }
        }
        for s in &self.satcom {
            if !matches!(s, LinkKind::Iridium | LinkKind::LeoSat) {
                return Err(format!("{s:?} is not a satcom module"));
            }
        }
        Ok(())
    }

    /// Names as reported by the module ID EEPROMs.
    pub fn expected_ids(&self) -> Vec<&'static str> {
        let mut v = vec!["base"];
        if self.arinc429.is_some() {
            v.push("arinc429");
        }
        if self.gnss {
            v.push("gnss");
        }
        if self.imu {
            v.push("imu");
        }
        if self.wifi {
            v.push("wifi");
        }
        match self.cellular {
            Cellular::None => {}
            Cellular::Lte4g => v.push("cellular_4g"),
            Cellular::Nr5g => v.push("cellular_5g"),
        }
        for s in &self.satcom {
            v.push(if *s == LinkKind::Iridium { "iridium" } else { "leo_sat" });
        }
        v
    }

    /// Compare configured modules with detected ones. Returns
    /// human-readable mismatches (empty = all good). Extra modules that are
    /// detected but not configured are reported too, but never used.
    pub fn check_detected(&self, detected: &[&str]) -> Vec<String> {
        let exp = self.expected_ids();
        let mut out: Vec<String> = exp
            .iter()
            .filter(|e| !detected.contains(e))
            .map(|e| format!("module '{e}' configured but not detected"))
            .collect();
        out.extend(
            detected
                .iter()
                .filter(|d| !exp.contains(d))
                .map(|d| format!("module '{d}' detected but not in installed configuration (ignored)")),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Modules {
        Modules {
            arinc429: Some(ArincModule { channels: 2, speed: ArincSpeed::Low }),
            gnss: true,
            imu: true,
            wifi: true,
            cellular: Cellular::Nr5g,
            satcom: vec![LinkKind::Iridium],
        }
    }

    #[test]
    fn link_presence() {
        let m = full();
        assert!(m.has_link(LinkKind::Iridium));
        assert!(!m.has_link(LinkKind::LeoSat));
        assert!(m.has_link(LinkKind::Cellular));
    }

    #[test]
    fn detection_mismatch_reported() {
        let m = full();
        assert!(m.check_detected(&["base", "arinc429", "gnss", "imu", "wifi", "cellular_5g", "iridium"]).is_empty());
        let r = m.check_detected(&["base", "gnss", "imu", "wifi", "cellular_4g", "iridium"]);
        assert_eq!(r.len(), 3, "{r:?}"); // arinc missing, 5g missing, 4g extra
    }

    #[test]
    fn validation() {
        let mut m = full();
        m.arinc429.as_mut().unwrap().channels = 3;
        assert!(m.validate().is_err());
        let mut m = full();
        m.satcom.push(LinkKind::Wifi);
        assert!(m.validate().is_err());
    }
}
