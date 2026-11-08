//! Own accelerometer (IMU): reference motion data, independent of the
//! aircraft.
//!
//! Raw samples (~100 Hz, in g, sensor axes) are rotated to aircraft axes
//! (x forward, y right, z down) using the installation's mounting, then
//! reduced to one summary per second:
//!
//! * **vibration RMS** after removing gravity: engine-running reference
//!   when there is no oil-pressure discrete, and a "rolling on runway" cue;
//! * **longitudinal acceleration**: takeoff roll / braking reference;
//! * **normal load factor (Nz) min/max**: touchdown and turbulence events.
//!
//! Events reported from here are maintenance *advisories*. They do not
//! replace the crew report or the AMM hard-landing / turbulence inspection
//! criteria.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImuConfig {
    /// Aircraft axes in terms of sensor axes, e.g. ["+x", "-y", "-z"]
    /// means forward = +x_s, right = -y_s, down = -z_s.
    pub axes: [String; 3],
    pub sample_rate_hz: f32,
    /// Vibration RMS (g) above which the engine is considered running.
    pub engine_vib_g: f32,
    /// Seconds the vibration must stay above/below the threshold.
    pub engine_on_s: f64,
    pub engine_off_s: f64,
    /// Nz at touchdown above which a hard-landing advisory is raised.
    pub hard_landing_nz: f32,
    /// |Nz - 1| in flight above which a turbulence advisory is raised.
    pub turbulence_dnz: f32,
}

impl Default for ImuConfig {
    fn default() -> Self {
        Self {
            axes: ["+x".into(), "+y".into(), "+z".into()],
            sample_rate_hz: 100.0,
            engine_vib_g: 0.04,
            engine_on_s: 5.0,
            engine_off_s: 10.0,
            hard_landing_nz: 1.8,
            turbulence_dnz: 1.0,
        }
    }
}

/// Mapping from sensor axes to aircraft axes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mounting {
    /// For each aircraft axis: (sensor axis index, sign).
    map: [(usize, f32); 3],
}

impl Mounting {
    pub fn parse(axes: &[String; 3]) -> Result<Self, String> {
        let mut map = [(0, 1.0); 3];
        let mut used = [false; 3];
        for (i, a) in axes.iter().enumerate() {
            let (sign, rest) = match a.as_bytes().first() {
                Some(b'+') => (1.0, &a[1..]),
                Some(b'-') => (-1.0, &a[1..]),
                _ => (1.0, a.as_str()),
            };
            let idx = match rest {
                "x" => 0,
                "y" => 1,
                "z" => 2,
                _ => return Err(format!("bad IMU axis '{a}'")),
            };
            if used[idx] {
                return Err(format!("IMU axis '{a}' used twice"));
            }
            used[idx] = true;
            map[i] = (idx, sign);
        }
        Ok(Self { map })
    }

    pub fn apply(&self, s: [f32; 3]) -> [f32; 3] {
        [0, 1, 2].map(|i| s[self.map[i].0] * self.map[i].1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ImuSummary {
    pub samples: u32,
    /// RMS of the gravity-removed acceleration magnitude, g.
    pub vib_rms_g: f32,
    /// Mean longitudinal acceleration relative to the at-rest reference, g.
    pub long_accel_g: f32,
    /// Normal load factor extremes over the window (1.0 in level flight).
    pub nz_max: f32,
    pub nz_min: f32,
}

#[derive(Debug)]
pub struct ImuProcessor {
    mount: Mounting,
    /// Slow low-pass estimate of gravity + steady acceleration.
    g_lp: Option<[f32; 3]>,
    /// Gravity vector captured at rest (sets the zero for longitudinal accel).
    g_rest: Option<[f32; 3]>,
    alpha: f32,
}

impl ImuProcessor {
    pub fn new(cfg: &ImuConfig) -> Result<Self, String> {
        let tau_s = 1.0;
        Ok(Self {
            mount: Mounting::parse(&cfg.axes)?,
            g_lp: None,
            g_rest: None,
            alpha: (1.0 / (cfg.sample_rate_hz * tau_s)).min(1.0),
        })
    }

    /// Call when the aircraft is known to be parked (on ground, engine off,
    /// not moving): captures the at-rest gravity vector, which also absorbs
    /// the ramp slope and any small mounting misalignment.
    pub fn capture_rest(&mut self) {
        if let Some(g) = self.g_lp {
            self.g_rest = Some(g);
        }
    }

    pub fn has_rest_reference(&self) -> bool {
        self.g_rest.is_some()
    }

    /// Process one window (normally one second) of raw sensor samples.
    pub fn process(&mut self, raw: &[[f32; 3]]) -> Option<ImuSummary> {
        if raw.is_empty() {
            return None;
        }
        let mut sum_sq = 0.0f64;
        let mut sum_long = 0.0f64;
        let (mut nz_max, mut nz_min) = (f32::MIN, f32::MAX);
        for s in raw {
            let a = self.mount.apply(*s);
            let g = self.g_lp.get_or_insert(a);
            for i in 0..3 {
                g[i] += self.alpha * (a[i] - g[i]);
            }
            let d: [f32; 3] = [a[0] - g[0], a[1] - g[1], a[2] - g[2]];
            sum_sq += (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]) as f64;
            let rest_x = self.g_rest.map_or(0.0, |r| r[0]);
            sum_long += (a[0] - rest_x) as f64;
            // z axis points down: in level flight the sensor reads -1 g on
            // z_down (specific force is upwards), so Nz = -a_z.
            let nz = -a[2];
            nz_max = nz_max.max(nz);
            nz_min = nz_min.min(nz);
        }
        let n = raw.len() as f64;
        Some(ImuSummary {
            samples: raw.len() as u32,
            vib_rms_g: (sum_sq / n).sqrt() as f32,
            long_accel_g: (sum_long / n) as f32,
            nz_max,
            nz_min,
        })
    }
}

/// Engine-running detection from vibration, with on/off delays.
#[derive(Debug)]
pub struct EngineVibDetector {
    thr: f32,
    on_s: f64,
    off_s: f64,
    state: Option<bool>,
    since: Option<(bool, f64)>,
}

impl EngineVibDetector {
    pub fn new(cfg: &ImuConfig) -> Self {
        Self { thr: cfg.engine_vib_g, on_s: cfg.engine_on_s, off_s: cfg.engine_off_s, state: None, since: None }
    }

    pub fn update(&mut self, s: Option<&ImuSummary>, t: f64) -> Option<bool> {
        let Some(s) = s else {
            self.state = None;
            self.since = None;
            return None;
        };
        let ind = s.vib_rms_g >= self.thr;
        if self.state == Some(ind) {
            self.since = None;
            return self.state;
        }
        match self.since {
            Some((c, t0)) if c == ind => {
                if t - t0 >= if ind { self.on_s } else { self.off_s } {
                    self.state = Some(ind);
                    self.since = None;
                }
            }
            _ => self.since = Some((ind, t)),
        }
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random noise in [-1, 1).
    fn noise(seed: &mut u32) -> f32 {
        *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (*seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }

    fn window(n: usize, base: [f32; 3], vib: f32, seed: &mut u32) -> Vec<[f32; 3]> {
        (0..n).map(|_| [0, 1, 2].map(|i| base[i] + vib * noise(seed))).collect()
    }

    #[test]
    fn mounting_parse_and_apply() {
        let m = Mounting::parse(&["-y".into(), "+x".into(), "-z".into()]).unwrap();
        assert_eq!(m.apply([1.0, 2.0, 3.0]), [-2.0, 1.0, -3.0]);
        assert!(Mounting::parse(&["+x".into(), "+x".into(), "+z".into()]).is_err());
        assert!(Mounting::parse(&["+q".into(), "+y".into(), "+z".into()]).is_err());
    }

    #[test]
    fn level_at_rest_reads_nz_one_and_no_vibration() {
        let mut p = ImuProcessor::new(&ImuConfig::default()).unwrap();
        let s = p.process(&vec![[0.0, 0.0, -1.0]; 100]).unwrap();
        assert!(s.vib_rms_g < 1e-4);
        assert!((s.nz_max - 1.0).abs() < 1e-6 && (s.nz_min - 1.0).abs() < 1e-6);
    }

    #[test]
    fn takeoff_acceleration_relative_to_rest() {
        let mut p = ImuProcessor::new(&ImuConfig::default()).unwrap();
        // parked on a slight slope: 0.02 g forward component
        for _ in 0..5 {
            p.process(&vec![[0.02, 0.0, -1.0]; 100]);
        }
        p.capture_rest();
        let s = p.process(&vec![[0.27, 0.0, -1.0]; 100]).unwrap();
        assert!((s.long_accel_g - 0.25).abs() < 1e-3);
    }

    #[test]
    fn touchdown_spike_detected_in_nz_max() {
        let mut p = ImuProcessor::new(&ImuConfig::default()).unwrap();
        let mut w = vec![[0.0, 0.0, -1.0]; 100];
        w[50] = [0.0, 0.0, -2.1];
        let s = p.process(&w).unwrap();
        assert!((s.nz_max - 2.1).abs() < 1e-6);
    }

    #[test]
    fn engine_detection_with_delays() {
        let cfg = ImuConfig::default();
        let mut p = ImuProcessor::new(&cfg).unwrap();
        let mut e = EngineVibDetector::new(&cfg);
        let mut seed = 1;
        let mut t = 0.0;
        let mut step = |vib: f32, p: &mut ImuProcessor, e: &mut EngineVibDetector, seed: &mut u32| {
            let s = p.process(&window(100, [0.0, 0.0, -1.0], vib, seed));
            t += 1.0;
            e.update(s.as_ref(), t)
        };
        // quiet: off after the off delay
        for _ in 0..12 {
            step(0.0, &mut p, &mut e, &mut seed);
        }
        assert_eq!(step(0.0, &mut p, &mut e, &mut seed), Some(false));
        // engine start: on only after 5 s
        for _ in 0..4 {
            assert_eq!(step(0.15, &mut p, &mut e, &mut seed), Some(false));
        }
        for _ in 0..3 {
            step(0.15, &mut p, &mut e, &mut seed);
        }
        assert_eq!(step(0.15, &mut p, &mut e, &mut seed), Some(true));
        // one quiet second doesn't stop it
        assert_eq!(step(0.0, &mut p, &mut e, &mut seed), Some(true));
        assert_eq!(e.update(None, 100.0), None, "no IMU data -> unknown");
    }
}
