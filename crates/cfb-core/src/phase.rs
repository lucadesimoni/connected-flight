// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! Flight phase detection.
//!
//! The phase drives two things: which radios may transmit (cellular and Wi-Fi
//! client only on ground), and whether a software update may be installed
//! (only parked on ground, engine off).
//!
//! Design rule: when the inputs are missing or disagree, the detector falls
//! back to the *restrictive* state. `Unknown` counts as airborne everywhere
//! (CFB-HLR-010).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlightPhase {
    /// No valid inputs yet, or inputs lost. Treated as airborne.
    Unknown,
    /// Weight on wheels / below taxi speed for the confirm time.
    OnGround,
    /// Airborne confirmed.
    Airborne,
}

impl FlightPhase {
    /// True only when on ground is *positively* confirmed (CFB-HLR-010).
    pub fn is_confirmed_ground(self) -> bool {
        matches!(self, FlightPhase::OnGround)
    }
}

/// Raw inputs sampled once per cycle. Every field is optional because any
/// source can fail; the detector has to handle that.
#[derive(Debug, Clone, Copy, Default)]
pub struct PhaseInputs {
    /// Monotonic time in seconds.
    pub t: f64,
    /// Weight-on-wheels / squat switch, read through an opto-isolated
    /// discrete input. `Some(true)` = weight on wheels.
    pub wow: Option<bool>,
    /// GNSS ground speed in knots (only when the fix is valid).
    pub ground_speed_kt: Option<f32>,
    /// Engine running (oil pressure switch, RPM, or alternator voltage).
    pub engine_running: Option<bool>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PhaseConfig {
    /// Above this ground speed the aircraft counts as possibly airborne
    /// (used only when there is no WoW input).
    pub airborne_speed_kt: f32,
    /// Below this ground speed the aircraft counts as possibly on ground.
    pub ground_speed_kt: f32,
    /// How long a condition must hold before the phase changes, in seconds.
    pub airborne_confirm_s: f64,
    pub ground_confirm_s: f64,
    /// After this long with no valid input, revert to `Unknown`.
    pub input_timeout_s: f64,
}

impl Default for PhaseConfig {
    fn default() -> Self {
        Self {
            airborne_speed_kt: 50.0,
            ground_speed_kt: 35.0,
            airborne_confirm_s: 5.0,
            ground_confirm_s: 20.0,
            input_timeout_s: 10.0,
        }
    }
}

#[derive(Debug)]
pub struct PhaseDetector {
    cfg: PhaseConfig,
    phase: FlightPhase,
    /// The phase the inputs currently point to, and since when.
    candidate: Option<(FlightPhase, f64)>,
    last_valid_input_t: Option<f64>,
}

impl PhaseDetector {
    pub fn new(cfg: PhaseConfig) -> Self {
        Self { cfg, phase: FlightPhase::Unknown, candidate: None, last_valid_input_t: None }
    }

    pub fn phase(&self) -> FlightPhase {
        self.phase
    }

    /// Instantaneous indication from one sample, before debouncing.
    fn indicate(&self, i: &PhaseInputs) -> Option<FlightPhase> {
        // CFB-HLR-011: WoW is the primary source when present.
        if let Some(wow) = i.wow {
            // CFB-HLR-012: if WoW says ground but GNSS says we're fast, the
            // inputs disagree, so we take the restrictive result.
            if wow {
                if let Some(gs) = i.ground_speed_kt {
                    if gs > self.cfg.airborne_speed_kt {
                        return Some(FlightPhase::Unknown);
                    }
                }
                return Some(FlightPhase::OnGround);
            }
            return Some(FlightPhase::Airborne);
        }
        // CFB-HLR-013: fall back to GNSS ground speed, with a hysteresis band.
        match i.ground_speed_kt {
            Some(gs) if gs >= self.cfg.airborne_speed_kt => Some(FlightPhase::Airborne),
            Some(gs) if gs <= self.cfg.ground_speed_kt => Some(FlightPhase::OnGround),
            Some(_) => None, // inside the band: no new indication, keep state
            None => None,
        }
    }

    /// Feed one sample and return the debounced phase.
    pub fn update(&mut self, i: PhaseInputs) -> FlightPhase {
        let ind = self.indicate(&i);
        if ind.is_some() || i.ground_speed_kt.is_some() {
            self.last_valid_input_t = Some(i.t);
        }

        // CFB-HLR-014: input timeout -> Unknown.
        match self.last_valid_input_t {
            Some(t0) if i.t - t0 > self.cfg.input_timeout_s => {
                self.phase = FlightPhase::Unknown;
                self.candidate = None;
                return self.phase;
            }
            None => return self.phase,
            _ => {}
        }

        let Some(ind) = ind else {
            self.candidate = None;
            return self.phase;
        };

        // Moving to a more restrictive state happens immediately.
        if ind == FlightPhase::Unknown {
            self.phase = FlightPhase::Unknown;
            self.candidate = None;
            return self.phase;
        }
        if ind == self.phase {
            self.candidate = None;
            return self.phase;
        }

        let confirm = match ind {
            FlightPhase::Airborne => self.cfg.airborne_confirm_s,
            FlightPhase::OnGround => self.cfg.ground_confirm_s,
            FlightPhase::Unknown => 0.0,
        };
        match self.candidate {
            Some((c, since)) if c == ind => {
                if i.t - since >= confirm {
                    self.phase = ind;
                    self.candidate = None;
                }
            }
            _ => self.candidate = Some((ind, i.t)),
        }
        // CFB-HLR-015: leaving `Unknown` towards Airborne also needs the
        // confirm time, but while unconfirmed we stay in `Unknown`, which is
        // already treated as airborne.
        self.phase
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t: f64, wow: Option<bool>, gs: Option<f32>) -> PhaseInputs {
        PhaseInputs { t, wow, ground_speed_kt: gs, engine_running: None }
    }

    #[test]
    fn starts_unknown_without_inputs() {
        let mut d = PhaseDetector::new(PhaseConfig::default());
        assert_eq!(d.update(s(0.0, None, None)), FlightPhase::Unknown);
        assert!(!d.phase().is_confirmed_ground());
    }

    #[test]
    fn wow_ground_needs_confirm_time() {
        let mut d = PhaseDetector::new(PhaseConfig::default());
        for t in 0..19 {
            assert_eq!(d.update(s(t as f64, Some(true), Some(0.0))), FlightPhase::Unknown);
        }
        assert_eq!(d.update(s(20.0, Some(true), Some(0.0))), FlightPhase::OnGround);
    }

    #[test]
    fn takeoff_and_landing_with_gnss_only() {
        let mut d = PhaseDetector::new(PhaseConfig::default());
        for t in 0..=20 {
            d.update(s(t as f64, None, Some(5.0)));
        }
        assert_eq!(d.phase(), FlightPhase::OnGround);
        // Taxiing fast inside the hysteresis band stays on ground.
        for t in 21..40 {
            d.update(s(t as f64, None, Some(40.0)));
        }
        assert_eq!(d.phase(), FlightPhase::OnGround);
        for t in 40..=45 {
            d.update(s(t as f64, None, Some(90.0)));
        }
        assert_eq!(d.phase(), FlightPhase::Airborne);
        for t in 100..=120 {
            d.update(s(t as f64, None, Some(10.0)));
        }
        assert_eq!(d.phase(), FlightPhase::OnGround);
    }

    #[test]
    fn short_gust_does_not_flip_phase() {
        let mut d = PhaseDetector::new(PhaseConfig::default());
        for t in 0..=20 {
            d.update(s(t as f64, None, Some(0.0)));
        }
        d.update(s(21.0, None, Some(60.0)));
        d.update(s(22.0, None, Some(10.0)));
        assert_eq!(d.phase(), FlightPhase::OnGround);
    }

    #[test]
    fn wow_disagreeing_with_gnss_goes_unknown_immediately() {
        let mut d = PhaseDetector::new(PhaseConfig::default());
        for t in 0..=20 {
            d.update(s(t as f64, Some(true), Some(0.0)));
        }
        assert_eq!(d.phase(), FlightPhase::OnGround);
        assert_eq!(d.update(s(21.0, Some(true), Some(120.0))), FlightPhase::Unknown);
    }

    #[test]
    fn input_loss_times_out_to_unknown() {
        let mut d = PhaseDetector::new(PhaseConfig::default());
        for t in 0..=20 {
            d.update(s(t as f64, None, Some(0.0)));
        }
        assert_eq!(d.phase(), FlightPhase::OnGround);
        assert_eq!(d.update(s(35.0, None, None)), FlightPhase::Unknown);
    }
}
