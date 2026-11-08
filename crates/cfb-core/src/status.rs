// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! What the status lights show (CFB-HLR-100). Pure logic; the driver
//! (`cfb-drivers::led`) only switches the outputs.
//!
//! | Light | Solid | Blinking | Off |
//! |---|---|---|---|
//! | **Green**: radios allowed | on ground, radios allowed, a link is up | on ground, radios allowed, no link yet | radios not allowed |
//! | **Amber**: radios forced off | airborne, or the radio switch is OFF | phase unknown (no GNSS / inputs disagree) | radios allowed |
//! | **Red**: fault | a fault is active (see `Fault`) | – | no fault |
//!
//! Green and amber are never on together, so the pilot can tell at a glance
//! whether the box may transmit on its ground radios.

use crate::phase::FlightPhase;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fault {
    /// A radio on/off command failed: radio state not guaranteed.
    RfCommand,
    /// Fitted modules differ from the installed configuration.
    Module,
    /// GNSS fitted but no valid position for a while.
    Gnss,
    /// Queue above 90 % of its budget.
    QueueFull,
    /// An edge app was disabled.
    EdgeApp,
    /// Supply failing (power-fail input).
    Power,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    Off,
    On,
    Blink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lights {
    pub green: Light,
    pub amber: Light,
    pub red: Light,
}

#[derive(Debug, Clone)]
pub struct StatusInputs<'a> {
    pub phase: FlightPhase,
    /// Any ground radio currently permitted (after phase and radio switch).
    pub radios_allowed: bool,
    /// The physical radio switch is OFF.
    pub rf_switch_off: bool,
    /// At least one link is up.
    pub link_up: bool,
    pub faults: &'a BTreeSet<Fault>,
}

pub fn lights(s: &StatusInputs) -> Lights {
    let green = match (s.radios_allowed, s.link_up) {
        (true, true) => Light::On,
        (true, false) => Light::Blink,
        (false, _) => Light::Off,
    };
    let amber = if s.radios_allowed {
        Light::Off
    } else if s.phase == FlightPhase::Unknown && !s.rf_switch_off {
        Light::Blink
    } else {
        Light::On
    };
    let red = if s.faults.is_empty() { Light::Off } else { Light::On };
    Lights { green, amber, red }
}

impl Light {
    /// Output level at time `t` (blink at 1 Hz, 50 %).
    pub fn level(self, t: f64) -> bool {
        match self {
            Light::Off => false,
            Light::On => true,
            Light::Blink => (t as u64).is_multiple_of(2),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(phase: FlightPhase, allowed: bool, sw_off: bool, up: bool, f: &BTreeSet<Fault>) -> Lights {
        lights(&StatusInputs { phase, radios_allowed: allowed, rf_switch_off: sw_off, link_up: up, faults: f })
    }

    #[test]
    fn green_and_amber_never_both_on() {
        let none = BTreeSet::new();
        for phase in [FlightPhase::Unknown, FlightPhase::OnGround, FlightPhase::Airborne] {
            for allowed in [false, true] {
                for sw in [false, true] {
                    for up in [false, true] {
                        let l = st(phase, allowed, sw, up, &none);
                        assert!(l.green == Light::Off || l.amber == Light::Off, "{phase:?} {allowed} {sw} {up}");
                        assert!(l.green != Light::Off || l.amber != Light::Off, "one of them always lit");
                    }
                }
            }
        }
    }

    #[test]
    fn typical_states() {
        let none = BTreeSet::new();
        let l = st(FlightPhase::OnGround, true, false, true, &none);
        assert_eq!((l.green, l.amber, l.red), (Light::On, Light::Off, Light::Off));
        let l = st(FlightPhase::OnGround, true, false, false, &none);
        assert_eq!(l.green, Light::Blink);
        let l = st(FlightPhase::Airborne, false, false, true, &none);
        assert_eq!((l.green, l.amber), (Light::Off, Light::On));
        let l = st(FlightPhase::Unknown, false, false, false, &none);
        assert_eq!(l.amber, Light::Blink);
        let l = st(FlightPhase::OnGround, false, true, false, &none);
        assert_eq!(l.amber, Light::On, "switch off on ground: radios forced off");
        let f: BTreeSet<Fault> = [Fault::RfCommand].into();
        assert_eq!(st(FlightPhase::OnGround, true, false, true, &f).red, Light::On);
    }

    #[test]
    fn blink_levels() {
        assert!(Light::Blink.level(0.0) && !Light::Blink.level(1.0));
        assert!(Light::On.level(1.0) && !Light::Off.level(0.0));
    }
}
