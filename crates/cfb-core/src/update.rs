//! Field-loadable software update gate.
//!
//! An update is installed only when **all** of these hold (CFB-HLR-040):
//!
//! * on ground positively confirmed, engine confirmed off, not moving;
//! * maintenance mode selected (the maintenance discrete / key switch, or an
//!   authenticated technician session on the maintenance port);
//! * supply voltage above the minimum (ground power or healthy battery);
//! * the manifest carries a valid Ed25519 signature from a trusted key;
//! * the part number and version are listed in the signed Approved Software
//!   Configuration Index (ASCI) for this hardware part number;
//! * the image hash matches the manifest.
//!
//! The image itself is a RAUC bundle written to the inactive A/B slot; RAUC
//! checks its own X.509 signature as an independent second check. After the
//! reboot the new slot must pass the boot health check, otherwise the
//! bootloader falls back to the previous slot (see docs/05-update-process.md).

use crate::phase::FlightPhase;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    /// Software part number, e.g. "CFB-SW-0100".
    pub part_number: String,
    /// Semantic version "major.minor.patch".
    pub version: String,
    /// Lower-case hex SHA-256 of the RAUC bundle.
    pub image_sha256: String,
    pub image_size: u64,
    /// Hardware part numbers this software may be loaded on.
    pub compatible_hw: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsciEntry {
    pub part_number: String,
    pub version: String,
    pub hw_part_number: String,
    /// Allows loading this version over a newer one (approved rollback).
    #[serde(default)]
    pub allow_downgrade: bool,
}

/// Approved Software Configuration Index, issued by the design organisation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asci {
    pub issue: u32,
    pub entries: Vec<AsciEntry>,
}

#[derive(Debug, Clone, Copy)]
pub struct AircraftState {
    pub phase: FlightPhase,
    pub engine_running: Option<bool>,
    pub ground_speed_kt: Option<f32>,
    pub maintenance_mode: bool,
    pub supply_v: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    NotOnGround,
    EngineNotConfirmedOff,
    Moving,
    NotInMaintenanceMode,
    SupplyVoltageLow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateError {
    Conditions(Vec<Blocker>),
    BadManifest(String),
    BadSignature,
    UntrustedKey,
    NotInAsci,
    HardwareIncompatible,
    Downgrade,
    ImageHashMismatch,
    ImageSizeMismatch,
}

#[derive(Debug, Clone)]
pub struct UpdateGate {
    pub trusted_keys: Vec<VerifyingKey>,
    pub hw_part_number: String,
    pub installed_version: String,
    pub min_supply_v: f32,
}

impl UpdateGate {
    /// Aircraft-state preconditions. Returns every blocker, not just the first,
    /// so the maintenance page can show the technician what to fix.
    pub fn check_conditions(&self, s: &AircraftState) -> Result<(), UpdateError> {
        let mut b = Vec::new();
        if !s.phase.is_confirmed_ground() {
            b.push(Blocker::NotOnGround);
        }
        if s.engine_running != Some(false) {
            b.push(Blocker::EngineNotConfirmedOff);
        }
        if matches!(s.ground_speed_kt, Some(gs) if gs > 2.0) {
            b.push(Blocker::Moving);
        }
        if !s.maintenance_mode {
            b.push(Blocker::NotInMaintenanceMode);
        }
        if !matches!(s.supply_v, Some(v) if v >= self.min_supply_v) {
            b.push(Blocker::SupplyVoltageLow);
        }
        if b.is_empty() { Ok(()) } else { Err(UpdateError::Conditions(b)) }
    }

    /// Detached Ed25519 signature check against the trusted keys.
    pub fn verify_detached(&self, data: &[u8], sig: &[u8]) -> Result<(), UpdateError> {
        self.verify_sig(data, sig)
    }

    /// Verify and parse the ASCI.
    pub fn verify_asci(&self, asci_toml: &[u8], asci_sig: &[u8]) -> Result<Asci, UpdateError> {
        self.verify_sig(asci_toml, asci_sig)?;
        parse_toml(asci_toml)
    }

    /// Is this part number + version approved for this hardware?
    pub fn approved(&self, asci: &Asci, part_number: &str, version: &str) -> bool {
        asci.entries.iter().any(|e| {
            e.part_number == part_number && e.version == version && e.hw_part_number == self.hw_part_number
        })
    }

    fn verify_sig(&self, data: &[u8], sig: &[u8]) -> Result<(), UpdateError> {
        let sig = Signature::from_slice(sig).map_err(|_| UpdateError::BadSignature)?;
        if self.trusted_keys.is_empty() {
            return Err(UpdateError::UntrustedKey);
        }
        if self.trusted_keys.iter().any(|k| k.verify(data, &sig).is_ok()) {
            Ok(())
        } else {
            Err(UpdateError::BadSignature)
        }
    }

    /// Verify the signed ASCI and the signed manifest. Signatures are
    /// detached and cover the exact file bytes, so there is no
    /// canonicalisation step to get wrong.
    pub fn verify(
        &self,
        asci_toml: &[u8],
        asci_sig: &[u8],
        manifest_toml: &[u8],
        manifest_sig: &[u8],
    ) -> Result<UpdateManifest, UpdateError> {
        self.verify_sig(asci_toml, asci_sig)?;
        self.verify_sig(manifest_toml, manifest_sig)?;
        let asci: Asci = parse_toml(asci_toml)?;
        let m: UpdateManifest = parse_toml(manifest_toml)?;

        if !m.compatible_hw.iter().any(|h| h == &self.hw_part_number) {
            return Err(UpdateError::HardwareIncompatible);
        }
        let entry = asci
            .entries
            .iter()
            .find(|e| {
                e.part_number == m.part_number
                    && e.version == m.version
                    && e.hw_part_number == self.hw_part_number
            })
            .ok_or(UpdateError::NotInAsci)?;
        let new_v = parse_semver(&m.version).ok_or_else(|| UpdateError::BadManifest("version".into()))?;
        let cur_v = parse_semver(&self.installed_version).unwrap_or((0, 0, 0));
        if new_v < cur_v && !entry.allow_downgrade {
            return Err(UpdateError::Downgrade);
        }
        if m.image_sha256.len() != 64 || !m.image_sha256.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(UpdateError::BadManifest("image_sha256".into()));
        }
        Ok(m)
    }

    /// Hash the image in a streaming way (the bundle may be hundreds of MB;
    /// we never hold it in RAM).
    pub fn verify_image(m: &UpdateManifest, mut image: impl std::io::Read) -> Result<(), UpdateError> {
        let mut h = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let n = image.read(&mut buf).map_err(|e| UpdateError::BadManifest(e.to_string()))?;
            if n == 0 {
                break;
            }
            total += n as u64;
            h.update(&buf[..n]);
        }
        if total != m.image_size {
            return Err(UpdateError::ImageSizeMismatch);
        }
        let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if hex != m.image_sha256.to_ascii_lowercase() {
            return Err(UpdateError::ImageHashMismatch);
        }
        Ok(())
    }
}

fn parse_toml<T: for<'de> Deserialize<'de>>(b: &[u8]) -> Result<T, UpdateError> {
    let s = std::str::from_utf8(b).map_err(|e| UpdateError::BadManifest(e.to_string()))?;
    toml::from_str(s).map_err(|e| UpdateError::BadManifest(e.to_string()))
}

fn parse_semver(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.split('.').map(|p| p.parse::<u32>());
    let r = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    if it.next().is_some() { None } else { Some(r) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn gate() -> UpdateGate {
        UpdateGate {
            trusted_keys: vec![key().verifying_key()],
            hw_part_number: "CFB-HW-100".into(),
            installed_version: "1.2.0".into(),
            min_supply_v: 12.0,
        }
    }

    fn parked() -> AircraftState {
        AircraftState {
            phase: FlightPhase::OnGround,
            engine_running: Some(false),
            ground_speed_kt: Some(0.0),
            maintenance_mode: true,
            supply_v: Some(13.8),
        }
    }

    fn image() -> Vec<u8> {
        b"rauc bundle bytes".to_vec()
    }

    fn manifest(version: &str) -> Vec<u8> {
        let hex: String = Sha256::digest(image()).iter().map(|b| format!("{b:02x}")).collect();
        format!(
            "part_number = \"CFB-SW-0100\"\nversion = \"{version}\"\nimage_sha256 = \"{hex}\"\nimage_size = {}\ncompatible_hw = [\"CFB-HW-100\"]\n",
            image().len()
        )
        .into_bytes()
    }

    fn asci() -> Vec<u8> {
        br#"issue = 3
[[entries]]
part_number = "CFB-SW-0100"
version = "1.3.0"
hw_part_number = "CFB-HW-100"
[[entries]]
part_number = "CFB-SW-0100"
version = "1.1.0"
hw_part_number = "CFB-HW-100"
allow_downgrade = true
[[entries]]
part_number = "CFB-SW-0100"
version = "1.0.0"
hw_part_number = "CFB-HW-100"
"#
        .to_vec()
    }

    fn sign(b: &[u8]) -> Vec<u8> {
        key().sign(b).to_bytes().to_vec()
    }

    #[test]
    fn parked_aircraft_in_maintenance_mode_may_update() {
        assert_eq!(gate().check_conditions(&parked()), Ok(()));
    }

    #[test]
    fn every_blocker_is_reported() {
        let s = AircraftState {
            phase: FlightPhase::Unknown,
            engine_running: None,
            ground_speed_kt: Some(10.0),
            maintenance_mode: false,
            supply_v: None,
        };
        let Err(UpdateError::Conditions(b)) = gate().check_conditions(&s) else { panic!() };
        assert_eq!(b.len(), 5);
    }

    #[test]
    fn airborne_blocks_update() {
        let mut s = parked();
        s.phase = FlightPhase::Airborne;
        assert_eq!(
            gate().check_conditions(&s),
            Err(UpdateError::Conditions(vec![Blocker::NotOnGround]))
        );
    }

    #[test]
    fn valid_signed_approved_update_verifies() {
        let (a, m) = (asci(), manifest("1.3.0"));
        let man = gate().verify(&a, &sign(&a), &m, &sign(&m)).unwrap();
        assert_eq!(man.version, "1.3.0");
        UpdateGate::verify_image(&man, &image()[..]).unwrap();
        assert_eq!(
            UpdateGate::verify_image(&man, &b"tampered bundle!!"[..]),
            Err(UpdateError::ImageHashMismatch)
        );
    }

    #[test]
    fn tampered_manifest_rejected() {
        let (a, m) = (asci(), manifest("1.3.0"));
        let sig = sign(&m);
        let mut m2 = m.clone();
        m2[0] = b'P';
        assert_eq!(gate().verify(&a, &sign(&a), &m2, &sig), Err(UpdateError::BadSignature));
    }

    #[test]
    fn foreign_key_rejected() {
        let (a, m) = (asci(), manifest("1.3.0"));
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let bad = other.sign(&m).to_bytes().to_vec();
        assert_eq!(gate().verify(&a, &sign(&a), &m, &bad), Err(UpdateError::BadSignature));
    }

    #[test]
    fn version_not_in_asci_rejected() {
        let (a, m) = (asci(), manifest("9.9.9"));
        assert_eq!(gate().verify(&a, &sign(&a), &m, &sign(&m)), Err(UpdateError::NotInAsci));
    }

    #[test]
    fn downgrade_only_when_asci_allows_it() {
        let a = asci();
        let m = manifest("1.0.0");
        assert_eq!(gate().verify(&a, &sign(&a), &m, &sign(&m)), Err(UpdateError::Downgrade));
        let m = manifest("1.1.0");
        assert!(gate().verify(&a, &sign(&a), &m, &sign(&m)).is_ok());
    }

    #[test]
    fn wrong_hardware_rejected() {
        let mut g = gate();
        g.hw_part_number = "CFB-HW-200".into();
        let (a, m) = (asci(), manifest("1.3.0"));
        assert_eq!(g.verify(&a, &sign(&a), &m, &sign(&m)), Err(UpdateError::HardwareIncompatible));
    }

    #[test]
    fn semver_parsing() {
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("1.2"), None);
        assert_eq!(parse_semver("1.2.3.4"), None);
        assert!(parse_semver("1.10.0") > parse_semver("1.9.9"));
    }
}
