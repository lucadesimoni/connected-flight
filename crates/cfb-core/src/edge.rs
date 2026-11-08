// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Edge computing: apps that run inside the box and turn raw data into a
//! few small results, so expensive links carry results instead of raw data.
//!
//! ## Containment (the certification argument)
//!
//! * An app sees a **read-only** [`DataView`] and can only return
//!   [`EdgeOutput`]s, which the runtime puts in the message queue. An app
//!   cannot reach the radios, the RF enables, the update gate or any
//!   avionics interface (CFB-HLR-080).
//! * Per app limits: outputs per tick, payload size, bytes per flight, and a
//!   CPU time budget per tick. An app that overruns its CPU budget
//!   repeatedly, or panics, is **disabled**; the box carries on (CFB-HLR-081/082).
//! * Outputs are for ground maintenance and operations only. Nothing goes
//!   to the crew.
//! * Apps are **declarative rule packages** (TOML), not code. A package is
//!   data with a part number and version, signed by the design organisation
//!   and listed in the ASCI (CFB-HLR-083). The rule interpreter itself is
//!   part of the verified airborne software.
//!
//! ## Rule types
//!
//! * `[[exceedance]]`: a parameter above/below a limit for a time, in a
//!   flight phase. Reports start (with the limit) and end (duration, peak).
//! * `[[trend]]`: when a set of stability conditions holds for a time
//!   (e.g. stable cruise), average a parameter set and report one snapshot,
//!   up to `max_per_flight` times per flight. Classic engine trend monitoring.
//! * `[summary]`: per-flight summary (block/flight time, distance, max
//!   altitude, max Nz, landings), sent at engine shutdown.

use crate::phase::FlightPhase;
use crate::queue::Priority;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

/// Read-only data an app sees each tick (1 Hz).
///
/// Standard keys: `lat lon gs track alt vs engine airborne`, IMU
/// `nz_max nz_min vib long_accel`, plus every valid, fresh ARINC 429
/// parameter by its configured name.
#[derive(Debug, Clone, Default)]
pub struct DataView {
    pub t: f64,
    pub phase: Option<FlightPhase>,
    pub values: BTreeMap<String, f64>,
}

impl DataView {
    pub fn get(&self, k: &str) -> Option<f64> {
        self.values.get(k).copied()
    }
    pub fn set(&mut self, k: &str, v: Option<f64>) {
        if let Some(v) = v.filter(|v| v.is_finite()) {
            self.values.insert(k.to_string(), v);
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeOutput {
    pub priority: Priority,
    pub ttl_s: u64,
    pub payload: Vec<u8>,
}

pub trait EdgeApp {
    fn id(&self) -> &str;
    fn version(&self) -> &str;
    fn tick(&mut self, v: &DataView) -> Vec<EdgeOutput>;
}

// ------------------------------------------------------------ runtime

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Limits {
    pub max_outputs_per_tick: usize,
    pub max_payload: usize,
    pub flight_bytes: u64,
    pub tick_budget_us: u64,
    pub max_overruns: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_outputs_per_tick: 4, max_payload: 240, flight_bytes: 16 * 1024, tick_budget_us: 2000, max_overruns: 3 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    Disabled { app: String, reason: String },
    Dropped { app: String, outputs: usize, reason: &'static str },
}

struct Slot {
    app: Box<dyn EdgeApp>,
    enabled: bool,
    overruns: u32,
    bytes_this_flight: u64,
}

pub struct EdgeRuntime {
    slots: Vec<Slot>,
    pub limits: Limits,
}

impl EdgeRuntime {
    pub fn new(limits: Limits) -> Self {
        Self { slots: Vec::new(), limits }
    }

    pub fn add(&mut self, app: Box<dyn EdgeApp>) {
        self.slots.push(Slot { app, enabled: true, overruns: 0, bytes_this_flight: 0 });
    }

    pub fn apps(&self) -> Vec<(String, String, bool)> {
        self.slots.iter().map(|s| (s.app.id().to_string(), s.app.version().to_string(), s.enabled)).collect()
    }

    pub fn reset_flight(&mut self) {
        for s in &mut self.slots {
            s.bytes_this_flight = 0;
        }
    }

    /// Run every enabled app once. Returns (app id, output) pairs, with the
    /// app id prefixed to each payload, plus runtime events.
    pub fn tick(&mut self, v: &DataView) -> (Vec<EdgeOutput>, Vec<RuntimeEvent>) {
        let mut outs = Vec::new();
        let mut events = Vec::new();
        let lim = self.limits;
        for s in self.slots.iter_mut().filter(|s| s.enabled) {
            let id = s.app.id().to_string();
            let start = Instant::now();
            let r = catch_unwind(AssertUnwindSafe(|| s.app.tick(v)));
            let took = start.elapsed();
            let produced = match r {
                Ok(o) => o,
                Err(_) => {
                    s.enabled = false;
                    events.push(RuntimeEvent::Disabled { app: id, reason: "panicked".into() });
                    continue;
                }
            };
            if took > Duration::from_micros(lim.tick_budget_us) {
                s.overruns += 1;
                if s.overruns >= lim.max_overruns {
                    s.enabled = false;
                    events.push(RuntimeEvent::Disabled {
                        app: id,
                        reason: format!("CPU budget exceeded {} times (last {} us)", s.overruns, took.as_micros()),
                    });
                    continue;
                }
            } else {
                s.overruns = 0;
            }
            let n = produced.len();
            let mut kept = 0;
            for mut o in produced.into_iter().take(lim.max_outputs_per_tick) {
                let mut p = format!("{id}|").into_bytes();
                p.append(&mut o.payload);
                if p.len() > lim.max_payload {
                    events.push(RuntimeEvent::Dropped { app: id.clone(), outputs: 1, reason: "payload too large" });
                    continue;
                }
                if s.bytes_this_flight + p.len() as u64 > lim.flight_bytes {
                    events.push(RuntimeEvent::Dropped { app: id.clone(), outputs: 1, reason: "flight byte budget" });
                    continue;
                }
                s.bytes_this_flight += p.len() as u64;
                o.payload = p;
                outs.push(o);
                kept += 1;
            }
            if n > lim.max_outputs_per_tick {
                events.push(RuntimeEvent::Dropped { app: id, outputs: n - lim.max_outputs_per_tick, reason: "too many outputs" });
            }
            let _ = kept;
        }
        (outs, events)
    }
}

// ------------------------------------------------------------ rule package

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum When {
    #[default]
    Any,
    Airborne,
    Ground,
}

impl When {
    fn holds(self, p: Option<FlightPhase>) -> bool {
        match self {
            When::Any => true,
            When::Airborne => p == Some(FlightPhase::Airborne),
            When::Ground => p == Some(FlightPhase::OnGround),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppHeader {
    pub id: String,
    pub part_number: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceedanceRule {
    pub id: String,
    pub param: String,
    #[serde(default)]
    pub above: Option<f64>,
    #[serde(default)]
    pub below: Option<f64>,
    #[serde(default)]
    pub for_s: f64,
    #[serde(default)]
    pub when: When,
    #[serde(default = "prio_high")]
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StableCond {
    pub param: String,
    /// Parameter must stay within ± `band` of its value at the start.
    #[serde(default)]
    pub band: Option<f64>,
    /// Or its absolute value must stay below `abs_max`.
    #[serde(default)]
    pub abs_max: Option<f64>,
    #[serde(default)]
    pub min: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendRule {
    pub id: String,
    pub params: Vec<String>,
    pub stable: Vec<StableCond>,
    pub stable_s: f64,
    #[serde(default = "airborne")]
    pub when: When,
    #[serde(default = "one")]
    pub max_per_flight: u32,
    #[serde(default = "prio_normal")]
    pub priority: Priority,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SummaryRule {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPackage {
    pub app: AppHeader,
    #[serde(default)]
    pub exceedance: Vec<ExceedanceRule>,
    #[serde(default)]
    pub trend: Vec<TrendRule>,
    #[serde(default)]
    pub summary: SummaryRule,
}

fn prio_high() -> Priority {
    Priority::High
}
fn prio_normal() -> Priority {
    Priority::Normal
}
fn airborne() -> When {
    When::Airborne
}
fn one() -> u32 {
    1
}

impl AppPackage {
    pub fn parse(toml_bytes: &[u8]) -> Result<Self, String> {
        let s = std::str::from_utf8(toml_bytes).map_err(|e| e.to_string())?;
        let p: AppPackage = toml::from_str(s).map_err(|e| e.to_string())?;
        p.validate()?;
        Ok(p)
    }

    fn validate(&self) -> Result<(), String> {
        let ok_id = |s: &str| !s.is_empty() && s.len() <= 24 && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
        if !ok_id(&self.app.id) {
            return Err(format!("bad app id '{}'", self.app.id));
        }
        for e in &self.exceedance {
            if !ok_id(&e.id) || (e.above.is_none() && e.below.is_none()) {
                return Err(format!("exceedance '{}' needs an id and above/below", e.id));
            }
        }
        for t in &self.trend {
            if !ok_id(&t.id) || t.params.is_empty() || t.params.len() > 12 || t.stable.is_empty() {
                return Err(format!("trend '{}' needs an id, 1..12 params and stability conditions", t.id));
            }
        }
        Ok(())
    }

    /// Verify signature + ASCI listing, then parse (CFB-HLR-083).
    pub fn load_verified(
        gate: &crate::update::UpdateGate,
        asci: &crate::update::Asci,
        pkg: &[u8],
        sig: &[u8],
    ) -> Result<Self, String> {
        gate.verify_detached(pkg, sig).map_err(|e| format!("{e:?}"))?;
        let p = Self::parse(pkg)?;
        if !gate.approved(asci, &p.app.part_number, &p.app.version) {
            return Err(format!("{} {} not in ASCI", p.app.part_number, p.app.version));
        }
        Ok(p)
    }

    pub fn into_app(self) -> RuleApp {
        RuleApp {
            exc: self.exceedance.into_iter().map(|r| (r, ExcState::default())).collect(),
            trend: self.trend.into_iter().map(|r| (r, TrendState::default())).collect(),
            summary: self.summary.enabled.then(SummaryState::default),
            hdr: self.app,
            last_phase: None,
        }
    }
}

// ------------------------------------------------------------ rule app

#[derive(Debug, Default)]
struct ExcState {
    since: Option<f64>,
    active: bool,
    peak: f64,
}

#[derive(Debug, Default)]
struct TrendState {
    since: Option<f64>,
    refs: Vec<f64>,
    sums: Vec<f64>,
    n: u32,
    done_this_flight: u32,
}

#[derive(Debug, Default)]
struct SummaryState {
    engine_on_t: Option<f64>,
    flight_s: f64,
    dist_nm: f64,
    max_alt: f64,
    max_nz: f64,
    landings: u32,
    last_t: Option<f64>,
    engine: bool,
}

pub struct RuleApp {
    hdr: AppHeader,
    exc: Vec<(ExceedanceRule, ExcState)>,
    trend: Vec<(TrendRule, TrendState)>,
    summary: Option<SummaryState>,
    last_phase: Option<FlightPhase>,
}

fn f1(v: f64) -> String {
    format!("{v:.1}")
}

impl EdgeApp for RuleApp {
    fn id(&self) -> &str {
        &self.hdr.id
    }
    fn version(&self) -> &str {
        &self.hdr.version
    }

    fn tick(&mut self, v: &DataView) -> Vec<EdgeOutput> {
        let mut out = Vec::new();
        let new_flight = v.phase == Some(FlightPhase::Airborne) && self.last_phase != Some(FlightPhase::Airborne);

        // ---- exceedances
        for (r, st) in &mut self.exc {
            let val = v.get(&r.param);
            let cond = r.when.holds(v.phase)
                && val.is_some_and(|x| r.above.is_some_and(|a| x > a) || r.below.is_some_and(|b| x < b));
            if cond {
                let x = val.unwrap();
                let since = *st.since.get_or_insert(v.t);
                if !st.active {
                    st.peak = x;
                }
                st.peak = if r.above.is_some() { st.peak.max(x) } else { st.peak.min(x) };
                if !st.active && v.t - since >= r.for_s {
                    st.active = true;
                    let lim = r.above.map(|a| format!(">{}", f1(a))).or(r.below.map(|b| format!("<{}", f1(b)))).unwrap();
                    out.push(EdgeOutput {
                        priority: r.priority,
                        ttl_s: 0,
                        payload: format!("EXC {} START {}{} val={}", r.id, r.param, lim, f1(x)).into_bytes(),
                    });
                }
            } else {
                if st.active {
                    let dur = v.t - st.since.unwrap_or(v.t);
                    out.push(EdgeOutput {
                        priority: r.priority,
                        ttl_s: 0,
                        payload: format!("EXC {} END dur={:.0}s peak={}", r.id, dur, f1(st.peak)).into_bytes(),
                    });
                }
                *st = ExcState::default();
            }
        }

        // ---- trends
        for (r, st) in &mut self.trend {
            if new_flight {
                st.done_this_flight = 0;
            }
            let vals: Option<Vec<f64>> = r.params.iter().map(|p| v.get(p)).collect();
            let conds: Option<Vec<f64>> = r.stable.iter().map(|c| v.get(&c.param)).collect();
            let (Some(vals), Some(conds)) = (vals, conds) else {
                st.since = None;
                continue;
            };
            if !r.when.holds(v.phase) || st.done_this_flight >= r.max_per_flight {
                st.since = None;
                continue;
            }
            if st.since.is_none() {
                st.since = Some(v.t);
                st.refs = conds.clone();
                st.sums = vec![0.0; vals.len()];
                st.n = 0;
            }
            let stable = r.stable.iter().zip(&conds).zip(&st.refs).all(|((c, x), r0)| {
                c.band.is_none_or(|b| (x - r0).abs() <= b)
                    && c.abs_max.is_none_or(|m| x.abs() <= m)
                    && c.min.is_none_or(|m| *x >= m)
            });
            if !stable {
                st.since = None;
                continue;
            }
            for (s, x) in st.sums.iter_mut().zip(&vals) {
                *s += x;
            }
            st.n += 1;
            if v.t - st.since.unwrap() >= r.stable_s && st.n > 0 {
                let body: Vec<String> = r
                    .params
                    .iter()
                    .zip(&st.sums)
                    .map(|(p, s)| format!("{p}={}", f1(s / st.n as f64)))
                    .collect();
                out.push(EdgeOutput {
                    priority: r.priority,
                    ttl_s: 0,
                    payload: format!("TREND {} n={} {}", r.id, st.n, body.join(",")).into_bytes(),
                });
                st.done_this_flight += 1;
                st.since = None;
            }
        }

        // ---- flight summary
        // A touch-and-go counts as a landing; flight time counts airborne
        // time only, block time runs from engine start to engine stop.
        if let Some(s) = &mut self.summary {
            let engine = v.get("engine").is_some_and(|e| e > 0.5);
            let dt = s.last_t.map_or(0.0, |lt| (v.t - lt).clamp(0.0, 10.0));
            s.last_t = Some(v.t);
            if engine && !s.engine {
                *s = SummaryState { engine_on_t: Some(v.t), last_t: s.last_t, engine: true, ..Default::default() };
            }
            if v.phase == Some(FlightPhase::Airborne) {
                s.flight_s += dt;
            }
            if self.last_phase == Some(FlightPhase::Airborne) && v.phase != Some(FlightPhase::Airborne) {
                s.landings += 1;
            }
            s.dist_nm += v.get("gs").unwrap_or(0.0).max(0.0) * dt / 3600.0;
            s.max_alt = s.max_alt.max(v.get("alt").unwrap_or(0.0));
            s.max_nz = s.max_nz.max(v.get("nz_max").unwrap_or(0.0));
            if !engine && s.engine {
                if let Some(on) = s.engine_on_t {
                    out.push(EdgeOutput {
                        priority: Priority::Normal,
                        ttl_s: 0,
                        payload: format!(
                            "SUM block={:.0}min flight={:.0}min dist={:.0}NM max_alt={:.0}ft max_nz={:.2} landings={}",
                            (v.t - on) / 60.0,
                            s.flight_s / 60.0,
                            s.dist_nm,
                            s.max_alt,
                            s.max_nz,
                            s.landings
                        )
                        .into_bytes(),
                    });
                }
                s.engine = false;
            }
        }

        self.last_phase = v.phase;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(t: f64, phase: FlightPhase, kv: &[(&str, f64)]) -> DataView {
        let mut v = DataView { t, phase: Some(phase), values: BTreeMap::new() };
        for (k, x) in kv {
            v.set(k, Some(*x));
        }
        v
    }

    fn text(o: &EdgeOutput) -> String {
        String::from_utf8(o.payload.clone()).unwrap()
    }

    const PKG: &str = r#"
[app]
id = "ops"
part_number = "CFB-APP-0001"
version = "1.0.0"

[[exceedance]]
id = "ITT_HIGH"
param = "itt"
above = 800.0
for_s = 5
when = "any"

[[trend]]
id = "CRUISE"
params = ["itt", "torque"]
stable_s = 60
stable = [
  { param = "alt", band = 100.0 },
  { param = "vs", abs_max = 300.0 },
]

[summary]
enabled = true
"#;

    #[test]
    fn package_parses_and_validates() {
        let p = AppPackage::parse(PKG.as_bytes()).unwrap();
        assert_eq!(p.exceedance.len(), 1);
        assert_eq!(p.trend.len(), 1);
        assert!(AppPackage::parse(PKG.replace("id = \"ops\"", "id = \"bad id!\"").as_bytes()).is_err());
        assert!(AppPackage::parse(PKG.replace("above = 800.0", "").as_bytes()).is_err());
    }

    #[test]
    fn exceedance_start_and_end() {
        let mut a = AppPackage::parse(PKG.as_bytes()).unwrap().into_app();
        let mut outs = vec![];
        for t in 0..20 {
            let itt = if (5..15).contains(&t) { 805.0 + t as f64 } else { 700.0 };
            outs.extend(a.tick(&view(t as f64, FlightPhase::Airborne, &[("itt", itt)])));
        }
        let msgs: Vec<String> = outs.iter().map(text).filter(|m| m.starts_with("EXC")).collect();
        assert_eq!(msgs.len(), 2, "{msgs:?}");
        assert!(msgs[0].contains("START itt>800.0"));
        assert!(msgs[1].contains("END dur=10s peak=819.0"), "{}", msgs[1]);
    }

    #[test]
    fn short_exceedance_not_reported() {
        let mut a = AppPackage::parse(PKG.as_bytes()).unwrap().into_app();
        let mut n = 0;
        for t in 0..10 {
            let itt = if t == 3 || t == 4 { 900.0 } else { 700.0 };
            n += a.tick(&view(t as f64, FlightPhase::Airborne, &[("itt", itt)])).len();
        }
        assert_eq!(n, 0);
    }

    #[test]
    fn trend_needs_stability_and_reports_once_per_flight() {
        let mut a = AppPackage::parse(PKG.as_bytes()).unwrap().into_app();
        let mut trends = vec![];
        for t in 0..400 {
            // climbing until 100 s, then stable cruise
            let (alt, vs) = if t < 100 { (t as f64 * 50.0, 3000.0) } else { (5000.0, 0.0) };
            let v = view(t as f64, FlightPhase::Airborne, &[("itt", 720.0), ("torque", 85.0), ("alt", alt), ("vs", vs)]);
            trends.extend(a.tick(&v).iter().map(text).filter(|m| m.starts_with("TREND")));
        }
        assert_eq!(trends.len(), 1, "{trends:?}");
        assert!(trends[0].contains("itt=720.0,torque=85.0"));
    }

    #[test]
    fn summary_at_engine_stop() {
        let mut a = AppPackage::parse(PKG.as_bytes()).unwrap().into_app();
        let mut sums = vec![];
        for t in 0..=1000 {
            let (phase, eng, gs, alt) = match t {
                0..=9 => (FlightPhase::OnGround, 0.0, 0.0, 0.0),
                10..=99 => (FlightPhase::OnGround, 1.0, 10.0, 0.0),
                100..=699 => (FlightPhase::Airborne, 1.0, 120.0, 6000.0),
                700..=899 => (FlightPhase::OnGround, 1.0, 10.0, 0.0),
                _ => (FlightPhase::OnGround, 0.0, 0.0, 0.0),
            };
            // 20 NM cruise + 0.8 NM taxi
            let v = view(t as f64, phase, &[("engine", eng), ("gs", gs), ("alt", alt), ("nz_max", 1.3)]);
            sums.extend(a.tick(&v).iter().map(text).filter(|m| m.starts_with("SUM")));
        }
        assert_eq!(sums.len(), 1, "{sums:?}");
        assert!(sums[0].contains("block=15min flight=10min dist=21NM max_alt=6000ft"), "{}", sums[0]);
        assert!(sums[0].contains("landings=1"));
    }

    struct Spammer;
    impl EdgeApp for Spammer {
        fn id(&self) -> &str { "spam" }
        fn version(&self) -> &str { "0" }
        fn tick(&mut self, _: &DataView) -> Vec<EdgeOutput> {
            (0..10).map(|_| EdgeOutput { priority: Priority::Critical, ttl_s: 0, payload: vec![b'x'; 100] }).collect()
        }
    }

    struct Panicker;
    impl EdgeApp for Panicker {
        fn id(&self) -> &str { "boom" }
        fn version(&self) -> &str { "0" }
        fn tick(&mut self, _: &DataView) -> Vec<EdgeOutput> { panic!("app bug") }
    }

    struct Slow;
    impl EdgeApp for Slow {
        fn id(&self) -> &str { "slow" }
        fn version(&self) -> &str { "0" }
        fn tick(&mut self, _: &DataView) -> Vec<EdgeOutput> {
            std::thread::sleep(Duration::from_millis(5));
            vec![]
        }
    }

    #[test]
    fn runtime_enforces_output_and_byte_limits() {
        let mut rt = EdgeRuntime::new(Limits { flight_bytes: 500, ..Limits::default() });
        rt.add(Box::new(Spammer));
        let (o, ev) = rt.tick(&DataView::default());
        assert_eq!(o.len(), 4, "max outputs per tick");
        assert!(o[0].payload.starts_with(b"spam|"));
        assert!(ev.iter().any(|e| matches!(e, RuntimeEvent::Dropped { reason: "too many outputs", .. })));
        let (o, ev) = rt.tick(&DataView::default());
        assert_eq!(o.len(), 0, "flight budget used up: 4 x 105 = 420 of 500");
        assert!(ev.iter().any(|e| matches!(e, RuntimeEvent::Dropped { reason: "flight byte budget", .. })));
        rt.reset_flight();
        assert_eq!(rt.tick(&DataView::default()).0.len(), 4);
    }

    #[test]
    fn panicking_app_is_disabled_others_continue() {
        let mut rt = EdgeRuntime::new(Limits::default());
        rt.add(Box::new(Panicker));
        rt.add(Box::new(AppPackage::parse(PKG.as_bytes()).unwrap().into_app()));
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let (_, ev) = rt.tick(&DataView::default());
        std::panic::set_hook(prev);
        assert_eq!(ev, vec![RuntimeEvent::Disabled { app: "boom".into(), reason: "panicked".into() }]);
        let apps = rt.apps();
        assert!(!apps[0].2 && apps[1].2);
        assert_eq!(rt.tick(&DataView::default()).1, vec![], "disabled app not run again");
    }

    #[test]
    fn cpu_overrun_disables_after_limit() {
        let mut rt = EdgeRuntime::new(Limits { tick_budget_us: 1000, max_overruns: 3, ..Limits::default() });
        rt.add(Box::new(Slow));
        assert!(rt.tick(&DataView::default()).1.is_empty());
        assert!(rt.tick(&DataView::default()).1.is_empty());
        let (_, ev) = rt.tick(&DataView::default());
        assert!(matches!(&ev[0], RuntimeEvent::Disabled { app, .. } if app == "slow"));
    }

    #[test]
    fn signed_package_must_be_in_asci() {
        use crate::update::{Asci, AsciEntry, UpdateGate};
        use ed25519_dalek::{Signer, SigningKey};
        let k = SigningKey::from_bytes(&[3u8; 32]);
        let gate = UpdateGate {
            trusted_keys: vec![k.verifying_key()],
            hw_part_number: "CFB-HW-100".into(),
            installed_version: "1.0.0".into(),
            min_supply_v: 12.0,
        };
        let asci = Asci {
            issue: 1,
            entries: vec![AsciEntry {
                part_number: "CFB-APP-0001".into(),
                version: "1.0.0".into(),
                hw_part_number: "CFB-HW-100".into(),
                allow_downgrade: false,
            }],
        };
        let sig = k.sign(PKG.as_bytes()).to_bytes();
        assert!(AppPackage::load_verified(&gate, &asci, PKG.as_bytes(), &sig).is_ok());
        let other = PKG.replace("1.0.0", "1.0.1");
        let sig2 = k.sign(other.as_bytes()).to_bytes();
        assert!(AppPackage::load_verified(&gate, &asci, other.as_bytes(), &sig2).unwrap_err().contains("not in ASCI"));
        assert!(AppPackage::load_verified(&gate, &asci, other.as_bytes(), &sig).is_err(), "signature mismatch");
    }
}
