// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Link policy: which bearer may carry which message, when.
//!
//! Rules, in order:
//! 1. Ground-only links (cellular, Wi-Fi client) may transmit only when on
//!    ground is positively confirmed (CFB-HLR-020). This is the software
//!    layer; the hardware RF-inhibit interlock (WoW discrete wired to the
//!    modem's W_DISABLE# line) holds the cellular modem off on its own.
//! 2. A link carries only priorities at or above its `min_priority`
//!    (CFB-HLR-021). By default bulk data never goes over satcom.
//! 3. Metered links have a byte budget per flight (CFB-HLR-022), so a stuck
//!    retransmission can't run up the bill.
//! 4. Among the remaining links, pick the cheapest per MB, then the fastest
//!    (CFB-HLR-023).

use crate::phase::FlightPhase;
use crate::queue::Priority;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// Wi-Fi client to hangar / FBO / gatelink AP.
    Wifi,
    /// LTE / 5G modem.
    Cellular,
    /// LEO broadband satcom (where an approved terminal is installed).
    LeoSat,
    /// Iridium Certus: narrowband, global, works in all phases.
    Iridium,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkPolicy {
    pub enabled: bool,
    /// May transmit only with on-ground confirmed.
    pub ground_only: bool,
    /// Cost in currency units per megabyte, used for ranking only.
    pub cost_per_mb: f64,
    /// Lowest priority this link will carry.
    pub min_priority: Priority,
    /// Bytes allowed per flight; `None` = unmetered.
    pub flight_budget_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkStatus {
    pub kind: LinkKind,
    /// Link is registered / associated and can pass traffic now.
    pub up: bool,
    /// Current estimated throughput, bits per second.
    pub est_bps: u32,
}

#[derive(Debug)]
pub struct LinkManager {
    policies: BTreeMap<LinkKind, LinkPolicy>,
    used_this_flight: BTreeMap<LinkKind, u64>,
}

impl LinkManager {
    pub fn new(policies: BTreeMap<LinkKind, LinkPolicy>) -> Self {
        Self { policies, used_this_flight: BTreeMap::new() }
    }

    /// Default policy for a typical CS-23 install.
    pub fn default_policies() -> BTreeMap<LinkKind, LinkPolicy> {
        let mut m = BTreeMap::new();
        m.insert(LinkKind::Wifi, LinkPolicy {
            enabled: true, ground_only: true, cost_per_mb: 0.0,
            min_priority: Priority::Bulk, flight_budget_bytes: None,
        });
        m.insert(LinkKind::Cellular, LinkPolicy {
            enabled: true, ground_only: true, cost_per_mb: 0.01,
            min_priority: Priority::Bulk, flight_budget_bytes: None,
        });
        m.insert(LinkKind::LeoSat, LinkPolicy {
            enabled: false, ground_only: false, cost_per_mb: 2.0,
            min_priority: Priority::Normal, flight_budget_bytes: Some(200 * 1024 * 1024),
        });
        m.insert(LinkKind::Iridium, LinkPolicy {
            enabled: true, ground_only: false, cost_per_mb: 600.0,
            min_priority: Priority::High, flight_budget_bytes: Some(64 * 1024),
        });
        m
    }

    pub fn policy(&self, k: LinkKind) -> Option<&LinkPolicy> {
        self.policies.get(&k)
    }

    /// May this link's transmitter be on in this phase? (CFB-HLR-020)
    /// The daemon uses this to drive the software RF-enable for each radio.
    pub fn tx_permitted(&self, k: LinkKind, phase: FlightPhase) -> bool {
        match self.policies.get(&k) {
            Some(p) if p.enabled => !p.ground_only || phase.is_confirmed_ground(),
            _ => false,
        }
    }

    fn budget_left(&self, k: LinkKind) -> u64 {
        match self.policies.get(&k).and_then(|p| p.flight_budget_bytes) {
            Some(b) => b.saturating_sub(*self.used_this_flight.get(&k).unwrap_or(&0)),
            None => u64::MAX,
        }
    }

    /// Choose the link for a message of `priority` and `size` bytes.
    pub fn select(
        &self,
        phase: FlightPhase,
        priority: Priority,
        size: usize,
        status: &[LinkStatus],
    ) -> Option<LinkKind> {
        status
            .iter()
            .filter(|s| s.up && self.tx_permitted(s.kind, phase))
            .filter(|s| priority >= self.policies[&s.kind].min_priority)
            .filter(|s| self.budget_left(s.kind) >= size as u64)
            .min_by(|a, b| {
                let ca = self.policies[&a.kind].cost_per_mb;
                let cb = self.policies[&b.kind].cost_per_mb;
                ca.total_cmp(&cb).then(b.est_bps.cmp(&a.est_bps))
            })
            .map(|s| s.kind)
    }

    /// Highest-priority message class any link can carry right now. The
    /// daemon uses it to filter the queue before peeking.
    pub fn can_carry(&self, phase: FlightPhase, priority: Priority, status: &[LinkStatus]) -> bool {
        self.select(phase, priority, 0, status).is_some()
    }

    pub fn record_tx(&mut self, k: LinkKind, bytes: usize) {
        *self.used_this_flight.entry(k).or_default() += bytes as u64;
    }

    pub fn used(&self, k: LinkKind) -> u64 {
        *self.used_this_flight.get(&k).unwrap_or(&0)
    }

    /// Call at each new departure.
    pub fn reset_flight_budgets(&mut self) {
        self.used_this_flight.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(kind: LinkKind, bps: u32) -> LinkStatus {
        LinkStatus { kind, up: true, est_bps: bps }
    }

    fn all_up() -> Vec<LinkStatus> {
        vec![
            up(LinkKind::Wifi, 50_000_000),
            up(LinkKind::Cellular, 20_000_000),
            up(LinkKind::Iridium, 22_000),
        ]
    }

    #[test]
    fn cellular_and_wifi_never_transmit_unless_ground_confirmed() {
        let lm = LinkManager::new(LinkManager::default_policies());
        for ph in [FlightPhase::Airborne, FlightPhase::Unknown] {
            assert!(!lm.tx_permitted(LinkKind::Cellular, ph));
            assert!(!lm.tx_permitted(LinkKind::Wifi, ph));
            assert!(lm.tx_permitted(LinkKind::Iridium, ph));
        }
        assert!(lm.tx_permitted(LinkKind::Cellular, FlightPhase::OnGround));
    }

    #[test]
    fn ground_prefers_cheapest() {
        let lm = LinkManager::new(LinkManager::default_policies());
        assert_eq!(
            lm.select(FlightPhase::OnGround, Priority::Critical, 30, &all_up()),
            Some(LinkKind::Wifi)
        );
        let no_wifi = &all_up()[1..];
        assert_eq!(
            lm.select(FlightPhase::OnGround, Priority::Bulk, 1_000_000, no_wifi),
            Some(LinkKind::Cellular)
        );
    }

    #[test]
    fn airborne_uses_satcom_for_critical_only() {
        let lm = LinkManager::new(LinkManager::default_policies());
        assert_eq!(
            lm.select(FlightPhase::Airborne, Priority::Critical, 30, &all_up()),
            Some(LinkKind::Iridium)
        );
        assert_eq!(lm.select(FlightPhase::Airborne, Priority::Normal, 30, &all_up()), None);
        assert_eq!(lm.select(FlightPhase::Airborne, Priority::Bulk, 30, &all_up()), None);
    }

    #[test]
    fn flight_budget_caps_satcom() {
        let mut lm = LinkManager::new(LinkManager::default_policies());
        lm.record_tx(LinkKind::Iridium, 64 * 1024 - 10);
        assert_eq!(lm.select(FlightPhase::Airborne, Priority::Critical, 30, &all_up()), None);
        assert_eq!(
            lm.select(FlightPhase::Airborne, Priority::Critical, 10, &all_up()),
            Some(LinkKind::Iridium)
        );
        lm.reset_flight_budgets();
        assert_eq!(
            lm.select(FlightPhase::Airborne, Priority::Critical, 30, &all_up()),
            Some(LinkKind::Iridium)
        );
    }

    #[test]
    fn disabled_or_down_links_are_skipped() {
        let mut p = LinkManager::default_policies();
        p.get_mut(&LinkKind::Wifi).unwrap().enabled = false;
        let lm = LinkManager::new(p);
        let st = vec![
            up(LinkKind::Wifi, 1),
            LinkStatus { kind: LinkKind::Cellular, up: false, est_bps: 0 },
            up(LinkKind::Iridium, 22_000),
        ];
        assert_eq!(
            lm.select(FlightPhase::OnGround, Priority::Critical, 30, &st),
            Some(LinkKind::Iridium)
        );
    }

    #[test]
    fn leo_preferred_over_iridium_in_flight_when_enabled() {
        let mut p = LinkManager::default_policies();
        p.get_mut(&LinkKind::LeoSat).unwrap().enabled = true;
        let lm = LinkManager::new(p);
        let mut st = all_up();
        st.push(up(LinkKind::LeoSat, 10_000_000));
        assert_eq!(
            lm.select(FlightPhase::Airborne, Priority::Normal, 5000, &st),
            Some(LinkKind::LeoSat)
        );
    }
}
