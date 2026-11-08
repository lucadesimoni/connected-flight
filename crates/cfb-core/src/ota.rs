// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Over-the-air update state machine.
//!
//! Pure logic: the daemon runs the commands (download with curl, RAUC
//! install, reboot) and reports results back. That keeps every rule here
//! testable.
//!
//! ```text
//!  Idle ──announce + download link ok──▶ Downloading ──ok──▶ Downloaded ──▶ Verifying
//!   ▲  ◀──link lost (resume later)──────────┘                                 │ ok
//!   │                                                                           ▼
//!   │                                                Verified ──gate open + confirm──▶ Installing
//!   │                                                                                     │ ok
//!   └──── after reboot: new version running ◀── Rebooting ◀──gate open── PendingReboot ◀─┘
//! ```
//!
//! * CFB-HLR-041: download only on a link permitted for downloads (ground
//!   links on ground; optionally unmetered in-flight links like LEO).
//! * CFB-HLR-042: install only when the update gate is open and a technician
//!   has confirmed.
//! * CFB-HLR-043: the reboot into the new slot waits until the gate is open
//!   again (e.g. engine started after install: no reboot).
//! * CFB-HLR-044: an interrupted download resumes; a failed verification
//!   discards the package and waits for a new announcement.

use crate::update::{UpdateError, UpdateManifest};

#[derive(Debug, Clone, PartialEq)]
pub enum OtaState {
    Idle,
    Downloading,
    Downloaded,
    Verifying,
    Verified(UpdateManifest),
    Installing(UpdateManifest),
    PendingReboot(UpdateManifest),
    Rebooting,
    Failed { reason: String, retry_at: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OtaCmd {
    StartDownload,
    AbortDownload,
    Verify,
    Install,
    Reboot,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OtaResult {
    DownloadOk,
    DownloadFailed(String),
    Verified(Result<UpdateManifest, UpdateError>),
    InstallOk,
    InstallFailed(String),
}

#[derive(Debug, Clone, Copy)]
pub struct OtaCtx {
    /// A link permitted for downloads is up now.
    pub download_link_ok: bool,
    /// Update gate conditions (`UpdateGate::check_conditions`) hold.
    pub gate_open: bool,
    /// Technician confirmed the install on the maintenance page.
    pub confirmed: bool,
}

#[derive(Debug)]
pub struct Ota {
    pub state: OtaState,
    pub available: bool,
    pub retry_s: f64,
}

impl Ota {
    pub fn new(retry_s: f64) -> Self {
        Self { state: OtaState::Idle, available: false, retry_s }
    }

    /// The ground service says a new package is published.
    pub fn announce(&mut self) {
        self.available = true;
    }

    /// Next command to run, given the current context. Call once per tick.
    pub fn step(&mut self, t: f64, c: OtaCtx) -> Option<OtaCmd> {
        match &self.state {
            OtaState::Idle if self.available && c.download_link_ok => {
                self.state = OtaState::Downloading;
                Some(OtaCmd::StartDownload)
            }
            OtaState::Downloading if !c.download_link_ok => {
                self.state = OtaState::Idle; // available stays set: resume later
                Some(OtaCmd::AbortDownload)
            }
            OtaState::Downloaded => {
                self.state = OtaState::Verifying;
                Some(OtaCmd::Verify)
            }
            OtaState::Verified(m) if c.gate_open && c.confirmed => {
                self.state = OtaState::Installing(m.clone());
                Some(OtaCmd::Install)
            }
            OtaState::PendingReboot(_) if c.gate_open => {
                self.state = OtaState::Rebooting;
                Some(OtaCmd::Reboot)
            }
            OtaState::Failed { retry_at, .. } if t >= *retry_at => {
                self.state = OtaState::Idle;
                None
            }
            _ => None,
        }
    }

    pub fn complete(&mut self, t: f64, r: OtaResult) {
        let fail = |reason: String, retry_s: f64| OtaState::Failed { reason, retry_at: t + retry_s };
        self.state = match (std::mem::replace(&mut self.state, OtaState::Idle), r) {
            (OtaState::Downloading, OtaResult::DownloadOk) => OtaState::Downloaded,
            (OtaState::Downloading, OtaResult::DownloadFailed(e)) => fail(format!("download: {e}"), self.retry_s),
            (OtaState::Verifying, OtaResult::Verified(Ok(m))) => {
                self.available = false;
                OtaState::Verified(m)
            }
            (OtaState::Verifying, OtaResult::Verified(Err(e))) => {
                self.available = false; // don't fetch the same bad package again
                fail(format!("verification: {e:?}"), self.retry_s)
            }
            (OtaState::Installing(m), OtaResult::InstallOk) => OtaState::PendingReboot(m),
            (OtaState::Installing(_), OtaResult::InstallFailed(e)) => fail(format!("install: {e}"), self.retry_s),
            (s, _) => s, // stale result (e.g. after abort): ignore
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m() -> UpdateManifest {
        UpdateManifest {
            part_number: "CFB-SW-0100".into(),
            version: "1.3.0".into(),
            image_sha256: "0".repeat(64),
            image_size: 1,
            compatible_hw: vec!["CFB-HW-100".into()],
        }
    }

    const NOLINK: OtaCtx = OtaCtx { download_link_ok: false, gate_open: false, confirmed: false };
    const LINK: OtaCtx = OtaCtx { download_link_ok: true, gate_open: false, confirmed: false };
    const READY: OtaCtx = OtaCtx { download_link_ok: true, gate_open: true, confirmed: true };

    #[test]
    fn happy_path() {
        let mut o = Ota::new(300.0);
        assert_eq!(o.step(0.0, LINK), None, "nothing announced");
        o.announce();
        assert_eq!(o.step(1.0, NOLINK), None, "no download without link");
        assert_eq!(o.step(2.0, LINK), Some(OtaCmd::StartDownload));
        o.complete(3.0, OtaResult::DownloadOk);
        assert_eq!(o.step(3.0, LINK), Some(OtaCmd::Verify));
        o.complete(4.0, OtaResult::Verified(Ok(m())));
        assert_eq!(o.step(5.0, LINK), None, "gate closed");
        assert_eq!(o.step(6.0, OtaCtx { confirmed: false, ..READY }), None, "not confirmed");
        assert_eq!(o.step(7.0, READY), Some(OtaCmd::Install));
        o.complete(8.0, OtaResult::InstallOk);
        assert!(matches!(o.state, OtaState::PendingReboot(_)));
        assert_eq!(o.step(9.0, READY), Some(OtaCmd::Reboot));
    }

    #[test]
    fn reboot_deferred_until_gate_open_again() {
        let mut o = Ota::new(300.0);
        o.state = OtaState::Installing(m());
        o.complete(0.0, OtaResult::InstallOk);
        for t in 1..10 {
            assert_eq!(o.step(t as f64, LINK), None, "engine running: no reboot");
        }
        assert_eq!(o.step(10.0, READY), Some(OtaCmd::Reboot));
    }

    #[test]
    fn link_loss_aborts_and_resumes() {
        let mut o = Ota::new(300.0);
        o.announce();
        assert_eq!(o.step(0.0, LINK), Some(OtaCmd::StartDownload));
        assert_eq!(o.step(1.0, NOLINK), Some(OtaCmd::AbortDownload));
        o.complete(1.5, OtaResult::DownloadFailed("killed".into())); // stale result ignored
        assert_eq!(o.state, OtaState::Idle);
        assert_eq!(o.step(2.0, LINK), Some(OtaCmd::StartDownload), "resumes");
    }

    #[test]
    fn bad_package_not_retried_until_new_announcement() {
        let mut o = Ota::new(10.0);
        o.announce();
        o.step(0.0, LINK);
        o.complete(1.0, OtaResult::DownloadOk);
        o.step(1.0, LINK);
        o.complete(2.0, OtaResult::Verified(Err(UpdateError::BadSignature)));
        assert!(matches!(o.state, OtaState::Failed { .. }));
        assert_eq!(o.step(20.0, LINK), None);
        assert_eq!(o.step(21.0, LINK), None, "not re-downloaded");
        o.announce();
        assert_eq!(o.step(22.0, LINK), Some(OtaCmd::StartDownload));
    }

    #[test]
    fn download_failure_retries_after_delay() {
        let mut o = Ota::new(10.0);
        o.announce();
        o.step(0.0, LINK);
        o.complete(1.0, OtaResult::DownloadFailed("404".into()));
        assert_eq!(o.step(5.0, LINK), None);
        assert_eq!(o.step(11.0, LINK), None); // back to Idle
        assert_eq!(o.step(12.0, LINK), Some(OtaCmd::StartDownload));
    }
}
