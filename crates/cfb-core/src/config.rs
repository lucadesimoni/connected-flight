//! Box configuration (`/etc/cfb/cfb.toml`).
//!
//! The configuration is part of the installed configuration: aircraft-specific
//! values (registration, which radios are fitted, link budgets) live here,
//! not in the software, so one software part number covers every CS-23 type.
//! The file sits on the read-only root filesystem and changes only through a
//! signed configuration update.

use crate::arinc429::{default_labels, LabelDef};
use crate::imu::{ImuConfig, Mounting};
use crate::link::{LinkKind, LinkPolicy};
use crate::modules::Modules;
use crate::phase::PhaseConfig;
use crate::reference::ReferenceConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub aircraft: Aircraft,
    /// Which hardware modules are fitted.
    pub modules: Modules,
    #[serde(default)]
    pub arinc429: ArincConfig,
    #[serde(default)]
    pub imu: ImuConfig,
    #[serde(default)]
    pub reference: ReferenceConfig,
    #[serde(default)]
    pub phase: PhaseConfig,
    pub links: BTreeMap<LinkKind, LinkPolicy>,
    pub queue: QueueConfig,
    pub reporting: Reporting,
    pub update: UpdateConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aircraft {
    pub registration: String,
    pub icao24: String,
    pub aircraft_type: String,
    /// Electrical system nominal voltage: 14 or 28.
    pub bus_voltage: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArincConfig {
    /// Receiver delivers the label already bit-reversed (natural order).
    #[serde(default)]
    pub label_reversed: bool,
    /// Labels to decode, from the source LRU's ICD.
    #[serde(default = "default_labels")]
    pub labels: Vec<LabelDef>,
}

impl Default for ArincConfig {
    fn default() -> Self {
        Self { label_reversed: false, labels: default_labels() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    pub max_bytes: usize,
    pub snapshot_path: String,
    pub snapshot_interval_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reporting {
    /// Position report interval in flight, seconds.
    pub position_interval_air_s: u64,
    /// Position report interval on ground, seconds (0 = off).
    pub position_interval_ground_s: u64,
    /// Lifetime of a queued position report; older ones are useless.
    pub position_ttl_s: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    pub hw_part_number: String,
    pub min_supply_v: f32,
    /// Hex-encoded Ed25519 public keys of the design organisation.
    pub trusted_keys: Vec<String>,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self, ConfigError> {
        let c: Config = toml::from_str(s).map_err(|e| ConfigError(e.to_string()))?;
        c.validate()?;
        Ok(c)
    }

    /// Reject configurations that would break a safety rule, so a bad file
    /// can't be loaded at all (CFB-HLR-050).
    pub fn validate(&self) -> Result<(), ConfigError> {
        for k in [LinkKind::Cellular, LinkKind::Wifi] {
            if let Some(p) = self.links.get(&k) {
                if !p.ground_only {
                    return Err(ConfigError(format!("{k:?} must be ground_only")));
                }
            }
        }
        if !(self.aircraft.bus_voltage == 14 || self.aircraft.bus_voltage == 28) {
            return Err(ConfigError("bus_voltage must be 14 or 28".into()));
        }
        if self.phase.ground_speed_kt >= self.phase.airborne_speed_kt {
            return Err(ConfigError("phase.ground_speed_kt must be below airborne_speed_kt".into()));
        }
        self.trusted_verifying_keys()?;
        self.validate_modules()?;
        if self.reporting.position_interval_air_s == 0 {
            return Err(ConfigError("position_interval_air_s must be > 0".into()));
        }
        Ok(())
    }
}

impl Config {
    /// CFB-HLR-071: every enabled link and every configured input must have
    /// its hardware module fitted.
    fn validate_modules(&self) -> Result<(), ConfigError> {
        self.modules.validate().map_err(ConfigError)?;
        for (k, p) in &self.links {
            if p.enabled && !self.modules.has_link(*k) {
                return Err(ConfigError(format!("link {k:?} enabled but its module is not fitted")));
            }
        }
        if let Some(a) = &self.modules.arinc429 {
            crate::arinc429::Decoder::new(self.arinc429.labels.clone()).map_err(ConfigError)?;
            for l in &self.arinc429.labels {
                if l.channel.is_some_and(|c| c >= a.channels) {
                    return Err(ConfigError(format!("ARINC label {} on channel beyond module", l.label)));
                }
            }
        }
        if self.modules.imu {
            Mounting::parse(&self.imu.axes).map_err(ConfigError)?;
        }
        Ok(())
    }

    /// Parse `update.trusted_keys`. An invalid key is a configuration error,
    /// never silently skipped.
    pub fn trusted_verifying_keys(&self) -> Result<Vec<ed25519_dalek::VerifyingKey>, ConfigError> {
        self.update
            .trusted_keys
            .iter()
            .map(|h| {
                let h = h.trim();
                let bytes: Option<Vec<u8>> = (h.len() == 64)
                    .then(|| (0..64).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect())
                    .flatten();
                bytes
                    .and_then(|b| <[u8; 32]>::try_from(b).ok())
                    .and_then(|b| ed25519_dalek::VerifyingKey::from_bytes(&b).ok())
                    .ok_or_else(|| ConfigError(format!("invalid trusted key {h}")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../config/cfb.toml");
    const MINIMAL: &str = include_str!("../../../config/cfb-minimal.toml");

    #[test]
    fn minimal_config_parses() {
        let c = Config::from_toml(MINIMAL).unwrap();
        assert!(c.modules.arinc429.is_none());
        assert!(c.modules.satcom.is_empty());
    }

    #[test]
    fn enabled_link_without_module_rejected() {
        let bad = MINIMAL.replacen("[links.cellular]\nenabled = true", "[links.iridium]\nenabled = true", 1);
        assert_ne!(bad, MINIMAL);
        let err = Config::from_toml(&bad).unwrap_err().0;
        assert!(err.contains("not fitted"), "{err}");
    }

    #[test]
    fn example_config_parses() {
        let c = Config::from_toml(EXAMPLE).unwrap();
        assert!(c.links.contains_key(&LinkKind::Iridium));
        assert!(c.links[&LinkKind::Cellular].ground_only);
    }

    #[test]
    fn cellular_in_flight_is_rejected() {
        let bad = EXAMPLE.replacen(
            "[links.cellular]\nenabled = true\nground_only = true",
            "[links.cellular]\nenabled = true\nground_only = false",
            1,
        );
        assert_ne!(bad, EXAMPLE, "test must actually modify the config");
        assert!(Config::from_toml(&bad).is_err());
    }
}
