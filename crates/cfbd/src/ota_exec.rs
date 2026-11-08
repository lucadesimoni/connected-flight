// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Runs the commands of the OTA state machine (`cfb_core::ota`).
//!
//! * Download: `curl` per file, resumable (`-C -`), bound to the chosen
//!   bearer's tunnel address so it goes over the permitted link only.
//! * Verify: in a worker thread (hashing a large bundle must not stall the
//!   1 Hz loop): ASCI + manifest signatures, ASCI listing, bundle hash/size.
//! * Install: `rauc install` (writes the inactive slot; RAUC checks its own
//!   bundle signature as an independent second check).
//! * Reboot: `systemctl reboot`; `cfb-mark-good.service` confirms the slot.
//!
//! All long-running work is a child process or thread polled every tick.

use cfb_core::config::OtaConfig;
use cfb_core::ota::{OtaCmd, OtaResult};
use cfb_core::update::{UpdateGate, UpdateManifest};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::JoinHandle;

pub const FILES: [&str; 5] = ["asci.toml", "asci.toml.sig", "manifest.toml", "manifest.toml.sig", "bundle.raucb"];

enum Job {
    Proc { child: Child, kind: &'static str },
    Verify(JoinHandle<OtaResult>),
    /// Simulation: result delivered at a given time.
    Sim(OtaResult, f64),
}

pub struct OtaExec {
    cfg: OtaConfig,
    dir: PathBuf,
    job: Option<Job>,
    simulate: bool,
}

fn read(dir: &Path, f: &str) -> Result<Vec<u8>, String> {
    std::fs::read(dir.join(f)).map_err(|e| format!("{f}: {e}"))
}

/// Verify a downloaded package directory. Public for the release tests.
pub fn verify_dir(gate: &UpdateGate, dir: &Path) -> OtaResult {
    let r = (|| {
        let m = gate
            .verify(
                &read(dir, "asci.toml")?,
                &read(dir, "asci.toml.sig")?,
                &read(dir, "manifest.toml")?,
                &read(dir, "manifest.toml.sig")?,
            )
            .map_err(|e| format!("{e:?}"))?;
        let f = std::fs::File::open(dir.join("bundle.raucb")).map_err(|e| e.to_string())?;
        UpdateGate::verify_image(&m, std::io::BufReader::new(f)).map_err(|e| format!("{e:?}"))?;
        Ok::<UpdateManifest, String>(m)
    })();
    match r {
        Ok(m) => OtaResult::Verified(Ok(m)),
        Err(e) => OtaResult::Verified(Err(cfb_core::update::UpdateError::BadManifest(e))),
    }
}

impl OtaExec {
    pub fn new(cfg: OtaConfig, simulate: bool) -> Self {
        let dir = PathBuf::from(&cfg.download_dir);
        Self { cfg, dir, job: None, simulate }
    }

    /// Start a command. `bind_ip` is the tunnel address of the bearer to
    /// download over (None = default route).
    pub fn run(&mut self, cmd: &OtaCmd, t: f64, gate: &UpdateGate, bind_ip: Option<String>) {
        if self.simulate {
            self.job = match cmd {
                OtaCmd::StartDownload => Some(Job::Sim(OtaResult::DownloadOk, t + 20.0)),
                OtaCmd::Verify => Some(Job::Sim(
                    OtaResult::Verified(Ok(UpdateManifest {
                        part_number: "CFB-SW-0100".into(),
                        version: "0.2.0".into(),
                        image_sha256: "0".repeat(64),
                        image_size: 0,
                        compatible_hw: vec![gate.hw_part_number.clone()],
                    })),
                    t + 2.0,
                )),
                OtaCmd::Install => Some(Job::Sim(OtaResult::InstallOk, t + 30.0)),
                OtaCmd::AbortDownload | OtaCmd::Reboot => None,
            };
            return;
        }
        match cmd {
            OtaCmd::StartDownload => {
                let _ = std::fs::create_dir_all(&self.dir);
                // Small files are always fetched fresh; only the bundle resumes.
                for f in &FILES[..4] {
                    let _ = std::fs::remove_file(self.dir.join(f));
                }
                let mut script = String::from("set -e\n");
                for f in FILES {
                    let iface = bind_ip.as_ref().map(|ip| format!("--interface {ip} ")).unwrap_or_default();
                    script.push_str(&format!(
                        "curl -fsS --retry 2 --connect-timeout 20 -C - {iface}-o '{}/{f}' '{}/{f}'\n",
                        self.dir.display(),
                        self.cfg.base_url.trim_end_matches('/')
                    ));
                }
                self.spawn("download", Command::new("sh").arg("-c").arg(script));
            }
            OtaCmd::AbortDownload => {
                if let Some(Job::Proc { mut child, .. }) = self.job.take() {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
            OtaCmd::Verify => {
                let (g, d) = (gate.clone(), self.dir.clone());
                self.job = Some(Job::Verify(std::thread::spawn(move || verify_dir(&g, &d))));
            }
            OtaCmd::Install => {
                let bundle = self.dir.join("bundle.raucb");
                self.spawn("install", Command::new("rauc").arg("install").arg(bundle));
            }
            OtaCmd::Reboot => {
                let _ = Command::new("systemctl").arg("reboot").status();
            }
        }
    }

    fn spawn(&mut self, kind: &'static str, cmd: &mut Command) {
        match cmd.stdin(Stdio::null()).stdout(Stdio::null()).spawn() {
            Ok(child) => self.job = Some(Job::Proc { child, kind }),
            Err(e) => {
                let r = if kind == "install" {
                    OtaResult::InstallFailed(e.to_string())
                } else {
                    OtaResult::DownloadFailed(e.to_string())
                };
                self.job = Some(Job::Sim(r, 0.0));
            }
        }
    }

    /// Check the running job. Returns its result when it has finished.
    pub fn poll(&mut self, t: f64) -> Option<OtaResult> {
        match self.job.take()? {
            Job::Sim(r, at) if t >= at => Some(r),
            j @ Job::Sim(..) => {
                self.job = Some(j);
                None
            }
            Job::Verify(h) if h.is_finished() => {
                let r = h.join().unwrap_or(OtaResult::Verified(Err(
                    cfb_core::update::UpdateError::BadManifest("verifier crashed".into()),
                )));
                if matches!(r, OtaResult::Verified(Err(_))) {
                    // Bad package: delete it so nothing is resumed from it.
                    for f in FILES {
                        let _ = std::fs::remove_file(self.dir.join(f));
                    }
                }
                Some(r)
            }
            j @ Job::Verify(_) => {
                self.job = Some(j);
                None
            }
            Job::Proc { mut child, kind } => match child.try_wait() {
                Ok(Some(st)) => Some(match (kind, st.success()) {
                    ("install", true) => OtaResult::InstallOk,
                    ("install", false) => OtaResult::InstallFailed(format!("rauc {st}")),
                    (_, true) => OtaResult::DownloadOk,
                    (_, false) => OtaResult::DownloadFailed(format!("curl {st}")),
                }),
                Ok(None) => {
                    self.job = Some(Job::Proc { child, kind });
                    None
                }
                Err(e) => Some(OtaResult::DownloadFailed(e.to_string())),
            },
        }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use sha2::{Digest, Sha256};

    #[test]
    fn verify_dir_accepts_signed_package_and_rejects_tampered_bundle() {
        let dir = std::env::temp_dir().join(format!("cfb-ota-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let k = SigningKey::from_bytes(&[5u8; 32]);
        let bundle = b"fake rauc bundle".to_vec();
        let hex: String = Sha256::digest(&bundle).iter().map(|b| format!("{b:02x}")).collect();
        let manifest = format!(
            "part_number = \"CFB-SW-0100\"\nversion = \"0.2.0\"\nimage_sha256 = \"{hex}\"\nimage_size = {}\ncompatible_hw = [\"CFB-HW-100\"]\n",
            bundle.len()
        );
        let asci = "issue = 1\n[[entries]]\npart_number = \"CFB-SW-0100\"\nversion = \"0.2.0\"\nhw_part_number = \"CFB-HW-100\"\n";
        let w = |f: &str, b: &[u8]| std::fs::write(dir.join(f), b).unwrap();
        w("asci.toml", asci.as_bytes());
        w("asci.toml.sig", &k.sign(asci.as_bytes()).to_bytes());
        w("manifest.toml", manifest.as_bytes());
        w("manifest.toml.sig", &k.sign(manifest.as_bytes()).to_bytes());
        w("bundle.raucb", &bundle);
        let gate = UpdateGate {
            trusted_keys: vec![k.verifying_key()],
            hw_part_number: "CFB-HW-100".into(),
            installed_version: "0.1.0".into(),
            min_supply_v: 12.0,
        };
        assert!(matches!(verify_dir(&gate, &dir), OtaResult::Verified(Ok(_))));
        w("bundle.raucb", b"evil rauc bundle");
        assert!(matches!(verify_dir(&gate, &dir), OtaResult::Verified(Err(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
