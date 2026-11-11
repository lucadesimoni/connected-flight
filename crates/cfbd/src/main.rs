// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfbd - Connected Flight Box daemon.
//!
//! Single-threaded 1 Hz control loop:
//!   raw inputs (ARINC 429, own GNSS, own IMU, discretes, link state)
//!   -> decoders -> reference selection -> flight phase
//!   -> radio enables, reports, advisories -> queue -> best permitted link.
//!   -> edge apps (sandboxed rule packages) -> queue
//!   -> ground transport (UDP in WireGuard per bearer, acked) / OTA updates.
//! All decisions are in `cfb-core`; this file is wiring.
//!
//! Usage:
//!   cfbd --config /etc/cfb/cfb.toml                    # live, inputs on stdin
//!   cfbd --config config/cfb.toml --simulate           # scripted demo flight
//!   cfbd --config config/cfb.toml --simulate --hard-landing
//!   cfbd --config config/cfb-pilatus-example.toml --simulate --apps config/apps

mod input;
mod ota_exec;
mod sim;
mod status_out;

use cfb_core::arinc429::{Decoder, Ssm, Word};
use cfb_core::edge::{AppPackage, DataView, EdgeRuntime, RuntimeEvent};
use cfb_core::ota::{Ota, OtaCmd, OtaCtx, OtaState};
use cfb_core::frame::EPOCH_2020;
use cfb_core::gnss::NmeaParser;
use cfb_core::imu::{EngineVibDetector, ImuProcessor};
use cfb_core::reference;
use cfb_core::status::{self, Fault, Lights, StatusInputs};
use cfb_drivers::led::{Indicator, SysfsLed};
use cfb_drivers::link::{LinkDriver, Rx};
use cfb_drivers::link_sbd::SbdLink;
use cfb_drivers::link_udp::UdpBearer;
use std::collections::BTreeSet;
use cfb_core::*;
use input::{Inputs, LineInputs};
use std::path::Path;
use std::collections::BTreeMap;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Minimal sd_notify, no libsystemd dependency.
fn sd_notify(msg: &str) {
    if let Ok(path) = std::env::var("NOTIFY_SOCKET") {
        if let Ok(sock) = UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), path);
        }
    }
}

fn now_2020() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(EPOCH_2020) as u32)
        .unwrap_or(0)
}

struct Out {
    queue: StoreForwardQueue,
    seq: u16,
}

impl Out {
    fn send(&mut self, ty: MsgType, prio: Priority, payload: Vec<u8>, t: f64, ttl: u64) {
        let f = Frame { msg_type: ty, seq: self.seq, time: now_2020(), payload };
        self.seq = self.seq.wrapping_add(1);
        self.queue.push(prio, f.encode(), t as u64, ttl);
    }
    /// Ground/maintenance message: text, logged locally and queued.
    fn event(&mut self, t: f64, ty: MsgType, prio: Priority, text: String) {
        println!("[{t:>6.0}s] {text}");
        self.send(ty, prio, text.into_bytes(), t, 0);
    }
}

/// Load edge app packages. Live: each must be signed and listed in the
/// signed ASCI. Simulation: signatures are not checked (development only).
fn load_apps(cfg: &Config, gate: &UpdateGate, simulate: bool, dirs: &[String]) -> EdgeRuntime {
    let mut rt = EdgeRuntime::new(cfg.edge.limits);
    let asci = if simulate {
        None
    } else {
        let Some(a) = &cfg.edge.asci else { return rt };
        match (std::fs::read(a), std::fs::read(format!("{a}.sig"))) {
            (Ok(b), Ok(sig)) => match gate.verify_asci(&b, &sig) {
                Ok(x) => Some(x),
                Err(e) => {
                    eprintln!("edge: ASCI rejected ({e:?}); no apps loaded");
                    return rt;
                }
            },
            _ => {
                eprintln!("edge: ASCI {a} missing; no apps loaded");
                return rt;
            }
        }
    };
    if simulate {
        println!("edge: SIMULATION - app signatures not checked");
    }
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        let mut files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "toml")).collect();
        files.sort();
        for f in files {
            let Ok(bytes) = std::fs::read(&f) else { continue };
            let pkg = match &asci {
                None => AppPackage::parse(&bytes),
                Some(a) => match std::fs::read(format!("{}.sig", f.display())) {
                    Ok(sig) => AppPackage::load_verified(gate, a, &bytes, &sig),
                    Err(_) => Err("no signature".into()),
                },
            };
            match pkg {
                Ok(p) => {
                    println!("edge: loaded {} {} ({})", p.app.id, p.app.version, p.app.part_number);
                    rt.add(Box::new(p.into_app()));
                }
                Err(e) => eprintln!("edge: skipped {}: {e}", f.display()),
            }
        }
    }
    rt
}

fn ota_state_name(s: &OtaState) -> String {
    match s {
        OtaState::Verified(m) | OtaState::Installing(m) | OtaState::PendingReboot(m) => {
            format!("{} {} {}", format!("{s:?}").split('(').next().unwrap_or(""), m.part_number, m.version)
        }
        OtaState::Failed { reason, .. } => format!("Failed: {reason}"),
        other => format!("{other:?}"),
    }
}

fn main() {
    let mut cfg_path = PathBuf::from("/etc/cfb/cfb.toml");
    let (mut simulate, mut hard_landing) = (false, false);
    let mut apps_override: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = args.next().map(PathBuf::from).unwrap_or(cfg_path),
            "--simulate" => simulate = true,
            "--hard-landing" => hard_landing = true,
            "--apps" => apps_override = args.next(),
            "-h" | "--help" => {
                println!("cfbd [--config PATH] [--apps DIR] [--simulate [--hard-landing]]");
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }

    let cfg = match std::fs::read_to_string(&cfg_path)
        .map_err(|e| e.to_string())
        .and_then(|s| Config::from_toml(&s).map_err(|e| e.to_string()))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cfbd: cannot load {}: {e}", cfg_path.display());
            std::process::exit(1);
        }
    };
    let m = &cfg.modules;

    let mut src: Box<dyn Inputs> = if simulate {
        Box::new(sim::SimInputs {
            t: 0,
            hard_landing,
            hw: m.expected_ids().into_iter().map(String::from).collect(),
            labels: if m.arinc429.is_some() { cfg.arinc429.labels.clone() } else { vec![] },
            seed: 42,
        })
    } else {
        Box::new(LineInputs { r: std::io::stdin().lock(), links: BTreeMap::new() })
    };

    // Modules: each decoder exists only if its hardware is fitted.
    let mut arinc = m.arinc429.as_ref().map(|_| Decoder::new(cfg.arinc429.labels.clone()).expect("validated"));
    let mut nmea = m.gnss.then(NmeaParser::default);
    let mut imu = m.imu.then(|| ImuProcessor::new(&cfg.imu).expect("validated"));
    let mut eng_vib = EngineVibDetector::new(&cfg.imu);

    let snapshot = PathBuf::from(&cfg.queue.snapshot_path);
    let queue = if simulate {
        StoreForwardQueue::new(cfg.queue.max_bytes)
    } else {
        StoreForwardQueue::load_or_new(&snapshot, cfg.queue.max_bytes)
    };
    let mut out = Out { queue, seq: 0 };
    let mut det = PhaseDetector::new(cfg.phase);
    let mut lm = LinkManager::new(cfg.links.clone());
    let gate = UpdateGate {
        trusted_keys: cfg.trusted_verifying_keys().unwrap_or_default(),
        hw_part_number: cfg.update.hw_part_number.clone(),
        installed_version: env!("CARGO_PKG_VERSION").into(),
        min_supply_v: cfg.update.min_supply_v,
    };

    // Edge apps.
    let app_dirs = apps_override.map(|d| vec![d]).unwrap_or_else(|| cfg.edge.app_dirs.clone());
    let mut edge = load_apps(&cfg, &gate, simulate, &app_dirs);

    // Link drivers (live mode only; the simulation acks immediately).
    // Each bearer is a `LinkDriver`; adding a new kind of link means adding
    // a driver in cfb-drivers and one registration here.
    let mut bearers: Vec<Box<dyn LinkDriver>> = Vec::new();
    if !simulate {
        if let Some(tr) = &cfg.transport {
            let ground = tr.ground.parse().expect("validated");
            for (k, b) in &tr.bearers {
                match UdpBearer::open(*k, b, ground) {
                    Ok(x) => bearers.push(Box::new(x)),
                    Err(e) => eprintln!("link: UDP bearer {k:?} ({}) not available: {e}", b.local),
                }
            }
        }
        if let Some(sbd) = &cfg.sbd {
            match SbdLink::open_serial(&sbd.device, sbd.min_signal, sbd.max_mo) {
                Ok(x) => bearers.push(Box::new(x)),
                Err(e) => eprintln!("link: Iridium SBD on {} not available: {e}", sbd.device),
            }
        }
    }
    for b in &bearers {
        println!("link: {}", b.describe());
    }

    // Status lights and fault tracking (CFB-HLR-100).
    let mut leds: Option<[SysfsLed; 3]> = match (&cfg.status_leds, simulate) {
        (Some(l), false) => Some([SysfsLed::new(&l.green), SysfsLed::new(&l.amber), SysfsLed::new(&l.red)]),
        _ => None,
    };
    let gnss_fault_after = cfg.status_leds.as_ref().map_or(30.0, |l| l.gnss_fault_after_s);
    let mut faults: BTreeSet<Fault> = BTreeSet::new();
    let mut last_lights: Option<Lights> = None;
    let mut last_pos_valid_t = 0.0;
    let switch_links: Vec<LinkKind> = cfg.rf_switch.as_ref().map(|s| s.links.clone()).unwrap_or_default();
    let mut last_switch_off: Option<bool> = None;
    let mut pfail_since: Option<f64> = None;
    let mut powered_off = false;
    let status_path = (!simulate && !cfg.maintenance.status_path.is_empty()).then(|| PathBuf::from(&cfg.maintenance.status_path));
    let mut status_err_logged = false;
    let retry_s = cfg.transport.as_ref().map_or(15, |t| t.retry_s);

    // OTA.
    let mut ota = Ota::new(cfg.ota.retry_s);
    let mut ota_exec = ota_exec::OtaExec::new(cfg.ota.clone(), simulate);
    let mut last_ota_state = String::new();
    let confirm_file = Path::new("/run/cfb/ota/confirm");

    let mut rf_enabled: BTreeMap<LinkKind, bool> = BTreeMap::new();
    let mut rf_last_cmd: BTreeMap<LinkKind, f64> = BTreeMap::new();
    let mut rf_children: Vec<(LinkKind, bool, std::process::Child)> = Vec::new();
    let mut last_phase = det.phase();
    // Set at confirmed airborne, cleared at the next confirmed ground. The
    // landing roll passes through `Unknown` (WoW on, still fast), so a plain
    // Airborne -> OnGround edge check would miss it.
    let mut flown_this_leg = false;
    let mut landing_nz_max: f32 = 0.0;
    let mut last_turb_t = f64::NEG_INFINITY;
    let mut last_ref = reference::Reference::default();
    let mut last_pos_t = f64::NEG_INFINITY;
    let mut last_snapshot = Instant::now();
    let mut sent: BTreeMap<LinkKind, (u32, u64)> = BTreeMap::new();

    println!(
        "cfbd: {} ({}) started, sim={simulate}, modules: {}",
        cfg.aircraft.registration,
        cfg.aircraft.aircraft_type,
        m.expected_ids().join(", ")
    );
    sd_notify("READY=1");

    // Per-tick work time (input reading excluded): the real-time budget is
    // 1 s per tick. Kept as a histogram of microseconds (bounded memory).
    let mut tick_hist = TickHistogram::default();
    let mut input_truncated: u64 = 0;
    let mut truncated_reported: u64 = 0;
    let mut last_trunc_report = f64::NEG_INFINITY;
    // Edge drops summed per (app, reason), reported at most once a minute.
    let mut edge_drops: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    let mut last_drop_report = f64::NEG_INFINITY;
    let mut last_tick_us: u64 = 0;
    while let Some(s) = src.next() {
        let tick_start = Instant::now();
        let t = s.t;
        if s.truncated {
            input_truncated += 1;
        }
        if input_truncated > truncated_reported && t - last_trunc_report >= 60.0 {
            out.event(t, MsgType::Health, Priority::Normal,
                format!("INPUT {} line(s) over the size limit, tails cut (bulk data lost)", input_truncated - truncated_reported));
            truncated_reported = input_truncated;
            last_trunc_report = t;
        }

        // ---- module check (CFB-HLR-070)
        if let Some(hw) = &s.hw {
            let det_ids: Vec<&str> = hw.iter().map(String::as_str).collect();
            for msg in m.check_detected(&det_ids) {
                faults.insert(Fault::Module);
                out.event(t, MsgType::Health, Priority::Normal, format!("MODULE {msg}"));
            }
        }

        // ---- decode raw inputs
        if let Some(d) = arinc.as_mut() {
            for (ch, raw) in &s.a429 {
                d.feed(*ch, Word::from_raw(*raw, cfg.arinc429.label_reversed), t);
            }
        }
        if let Some(p) = nmea.as_mut() {
            for line in &s.nmea {
                p.feed(line, t);
            }
        }
        let imu_sum = imu.as_mut().and_then(|p| p.process(&s.imu));
        let engine_vib = eng_vib.update(imu_sum.as_ref(), t);

        // ---- reference selection + cross-check
        let mut r = reference::select(
            &cfg.reference,
            t,
            arinc.as_ref(),
            nmea.as_ref().map(|p| p.fix),
            s.engine_discrete,
            engine_vib,
        );
        if m.baro {
            reference::apply_baro(&mut r, s.baro_hpa);
        }
        if r.pos_src != last_ref.pos_src || r.engine_src != last_ref.engine_src {
            out.event(t, MsgType::Health, Priority::Normal,
                format!("REF position={:?} engine={:?}", r.pos_src, r.engine_src));
        }
        if r.pos_disagree != last_ref.pos_disagree || r.gs_disagree != last_ref.gs_disagree {
            out.event(t, MsgType::Health, Priority::Normal,
                format!("REF cross-check pos_disagree={} gs_disagree={}", r.pos_disagree, r.gs_disagree));
        }

        // ---- flight phase
        let pressure_alt_ft = s.baro_hpa.filter(|_| m.baro).map(reference::pressure_altitude_ft);
        let phase = det.update(PhaseInputs {
            pressure_alt_ft,
            t,
            wow: s.wow,
            ground_speed_kt: r.gs_kt,
            engine_running: r.engine_running,
        });
        if phase != last_phase {
            println!("[{t:>6.0}s] phase {last_phase:?} -> {phase:?}");
            last_phase = phase;
        }
        if phase == FlightPhase::Airborne && !flown_this_leg {
            lm.reset_flight_budgets();
            edge.reset_flight();
            flown_this_leg = true;
            landing_nz_max = 0.0;
        }

        // ---- IMU advisories
        if let Some(sum) = imu_sum {
            if flown_this_leg && phase != FlightPhase::Airborne {
                landing_nz_max = landing_nz_max.max(sum.nz_max);
            }
            let dnz = (sum.nz_max - 1.0).max(1.0 - sum.nz_min);
            if phase == FlightPhase::Airborne && dnz > cfg.imu.turbulence_dnz && t - last_turb_t > 60.0 {
                last_turb_t = t;
                out.event(t, MsgType::Text, Priority::High, format!(
                    "TURBULENCE ADVISORY nz {:.2}/{:.2} at {:?} {}ft",
                    sum.nz_min, sum.nz_max, r.pos.map(|(a, b)| (a as f32, b as f32)), r.alt_ft.unwrap_or(0)));
            }
            // Parked: refresh the at-rest gravity reference.
            if let Some(p) = imu.as_mut() {
                if phase.is_confirmed_ground() && r.engine_running == Some(false) && r.gs_kt.unwrap_or(0.0) < 1.0 {
                    p.capture_rest();
                }
            }
        }

        // ---- edge apps (CFB-HLR-080..083)
        let mut dv = DataView { t, phase: Some(phase), values: BTreeMap::new() };
        if let Some((la, lo)) = r.pos {
            dv.set("lat", Some(la));
            dv.set("lon", Some(lo));
        }
        dv.set("gs", r.gs_kt.map(f64::from));
        dv.set("track", r.track_deg.map(f64::from));
        dv.set("alt", r.alt_ft.map(f64::from));
        dv.set("vs", r.vs_fpm.map(f64::from));
        dv.set("engine", r.engine_running.map(|e| if e { 1.0 } else { 0.0 }));
        dv.set("airborne", Some(if phase.is_confirmed_ground() { 0.0 } else { 1.0 }));
        dv.set("baro_hpa", s.baro_hpa);
        dv.set("baro_alt", s.baro_hpa.map(reference::pressure_altitude_ft));
        if let Some(sum) = imu_sum {
            dv.set("nz_max", Some(sum.nz_max as f64));
            dv.set("nz_min", Some(sum.nz_min as f64));
            dv.set("vib", Some(sum.vib_rms_g as f64));
            dv.set("long_accel", Some(sum.long_accel_g as f64));
        }
        if let Some(d) = &arinc {
            for (k, v) in d.values() {
                if v.ssm == Ssm::Normal && t - v.t <= cfg.reference.arinc_max_age_s {
                    dv.set(k, Some(v.value));
                }
            }
        }
        let (edge_out, edge_ev) = edge.tick(&dv);
        for o in edge_out {
            println!("[{t:>6.0}s] EDGE {}", String::from_utf8_lossy(&o.payload));
            out.send(MsgType::Edge, o.priority, o.payload, t, o.ttl_s);
        }
        for e in edge_ev {
            let txt = match e {
                RuntimeEvent::Disabled { app, reason } => {
                    faults.insert(Fault::EdgeApp);
                    format!("EDGE app {app} DISABLED: {reason}")
                }
                RuntimeEvent::Dropped { app, outputs, reason } => {
                    *edge_drops.entry((app, reason)).or_default() += outputs;
                    continue;
                }
            };
            out.event(t, MsgType::Health, Priority::Normal, txt);
        }
        if !edge_drops.is_empty() && t - last_drop_report >= 60.0 {
            for ((app, reason), n) in std::mem::take(&mut edge_drops) {
                out.event(t, MsgType::Health, Priority::Normal, format!("EDGE app {app}: {n} output(s) dropped ({reason})"));
            }
            last_drop_report = t;
        }

        if phase == FlightPhase::OnGround && flown_this_leg {
            flown_this_leg = false;
            if m.imu {
                let hard = landing_nz_max > cfg.imu.hard_landing_nz;
                out.event(t, MsgType::Text, if hard { Priority::High } else { Priority::Normal }, format!(
                    "{} touchdown nz_max {:.2}{}",
                    if hard { "HARD LANDING ADVISORY" } else { "LANDING" },
                    landing_nz_max,
                    if hard { " - inspect per AMM" } else { "" }));
            }
            // Post-flight: flight data file for ground upload (stand-in for
            // the recorded ARINC/engine data).
            for chunk in 0..8u8 {
                out.send(MsgType::FileChunk, Priority::Bulk, vec![chunk; 32 * 1024], t, 0);
            }
            println!("[{t:>6.0}s] queued flight data file (8 x 32 KiB) for ground upload");
        }

        // ---- physical radio switch: unknown counts as OFF (fail restrictive)
        let switch_off = cfg.rf_switch.is_some() && s.rf_switch != Some(true);
        if last_switch_off != Some(switch_off) && cfg.rf_switch.is_some() {
            out.event(t, MsgType::Health, Priority::Normal, format!("RF switch {}", if switch_off { "OFF" } else { "ON" }));
            last_switch_off = Some(switch_off);
        }
        let perm: BTreeMap<LinkKind, bool> = [LinkKind::Wifi, LinkKind::Cellular, LinkKind::LeoSat, LinkKind::Iridium]
            .into_iter()
            .map(|k| (k, lm.tx_permitted(k, phase) && !(switch_off && switch_links.contains(&k))))
            .collect();
        let permitted = |kind: LinkKind| perm.get(&kind).copied().unwrap_or(false);

        // ---- power fail: snapshot at once, clean shutdown if it persists
        if s.power_fail {
            let since = *pfail_since.get_or_insert_with(|| {
                faults.insert(Fault::Power);
                if !simulate {
                    let _ = out.queue.save(&snapshot);
                }
                t
            });
            if since == t {
                out.event(t, MsgType::Health, Priority::High, "POWER FAIL".into());
            }
            if !powered_off && t - since >= cfg.power.shutdown_after_s {
                powered_off = true;
                println!("[{t:>6.0}s] power fail for {:.0} s: final snapshot, shutting down", t - since);
                if !simulate {
                    let _ = out.queue.save(&snapshot);
                    let c = &cfg.power.off_cmd;
                    let _ = std::process::Command::new(&c[0]).args(&c[1..]).spawn();
                }
            }
        } else if pfail_since.take().is_some() {
            faults.remove(&Fault::Power);
            powered_off = false;
            out.event(t, MsgType::Health, Priority::Normal, "POWER RESTORED".into());
        }

        // ---- radio enables (software layer of the RF inhibit)
        for kind in [LinkKind::Wifi, LinkKind::Cellular, LinkKind::LeoSat, LinkKind::Iridium] {
            if !m.has_link(kind) {
                continue;
            }
            let en = permitted(kind);
            let changed = rf_enabled.get(&kind) != Some(&en);
            if changed {
                println!("[{t:>6.0}s] RF {kind:?}: {}", if en { "ENABLED" } else { "inhibited" });
                rf_enabled.insert(kind, en);
            }
            // CFB-HLR-024: switch the radio itself; re-assert "off".
            if let (false, Some(c)) = (simulate, cfg.rf.get(&kind)) {
                let due = !en && t - rf_last_cmd.get(&kind).copied().unwrap_or(f64::NEG_INFINITY) >= c.reassert_s as f64;
                if changed || due {
                    let argv = if en { &c.on } else { &c.off };
                    match std::process::Command::new(&argv[0])
                        .args(&argv[1..])
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .spawn()
                    {
                        Ok(ch) => rf_children.push((kind, en, ch)),
                        Err(e) => {
                            faults.insert(Fault::RfCommand);
                            out.event(t, MsgType::Health, Priority::Normal, format!("RF {kind:?} command failed to start: {e}"))
                        }
                    }
                    rf_last_cmd.insert(kind, t);
                }
            }
        }
        let mut still = Vec::new();
        for (kind, en, mut ch) in rf_children.drain(..) {
            match ch.try_wait() {
                Ok(Some(st)) if !st.success() => {
                    faults.insert(Fault::RfCommand);
                    out.event(t, MsgType::Health, Priority::Normal,
                        format!("RF {kind:?} {} command failed: {st}", if en { "on" } else { "off" }))
                }
                Ok(Some(_)) => {
                    faults.remove(&Fault::RfCommand);
                }
                Ok(None) => still.push((kind, en, ch)),
                Err(e) => eprintln!("RF {kind:?}: {e}"),
            }
        }
        rf_children = still;

        // ---- position reports
        let interval = if phase.is_confirmed_ground() {
            cfg.reporting.position_interval_ground_s
        } else {
            cfg.reporting.position_interval_air_s
        };
        if interval > 0 && t - last_pos_t >= interval as f64 {
            if let Some((lat, lon)) = r.pos {
                let p = PositionReport {
                    lat_deg: lat,
                    lon_deg: lon,
                    alt_ft: r.alt_ft.unwrap_or(0),
                    gs_kt: r.gs_kt.unwrap_or(0.0).max(0.0) as u16,
                    track_deg: r.track_deg.unwrap_or(0.0),
                    vs_fpm: r.vs_fpm.unwrap_or(0),
                    airborne: phase == FlightPhase::Airborne,
                    emergency: false,
                };
                out.send(MsgType::Position, Priority::Critical, p.encode().to_vec(), t, cfg.reporting.position_ttl_s);
                last_pos_t = t;
            }
        }
        last_ref = r;

        // ---- receive: acks and uplink messages
        for b in bearers.iter_mut() {
            for rx in b.poll(t) {
                match rx {
                    Rx::Acked(id) => {
                        out.queue.ack(id);
                    }
                    Rx::Uplink(f) => {
                        let txt = String::from_utf8_lossy(&f.payload).into_owned();
                        println!("[{t:>6.0}s] UPLINK via {:?} {:?}: {txt}", b.kind(), f.msg_type);
                        if f.msg_type == MsgType::Text && txt.starts_with("OTA AVAILABLE") {
                            ota.announce();
                        }
                    }
                }
            }
        }
        if simulate && cfg.ota.enabled && t == 700.0 {
            println!("[{t:>6.0}s] UPLINK OTA AVAILABLE (simulated ground announcement)");
            ota.announce();
        }

        // ---- link status: from the link drivers when they run here.
        // Links behind a switched-off radio switch count as down.
        let mut status: Vec<LinkStatus> = if bearers.is_empty() {
            s.links.values().copied().filter(|l| m.has_link(l.kind)).collect()
        } else {
            bearers.iter().map(|b| b.status(t)).collect()
        };
        for l in status.iter_mut() {
            if switch_off && switch_links.contains(&l.kind) {
                l.up = false;
            }
        }

        // ---- drain the queue onto links
        let now_u = t as u64;
        out.queue.expire(now_u);
        let mut budget: BTreeMap<LinkKind, usize> = bearers.iter().map(|b| (b.kind(), b.tick_budget())).collect();
        while let Some(msg) = out.queue.peek_ready(|p| lm.can_carry(phase, p, &status), now_u, retry_s) {
            let Some(kind) = lm.select(phase, msg.priority, msg.payload.len(), &status) else { break };
            let (id, len) = (msg.id, msg.payload.len());
            if let Some(b) = bearers.iter_mut().find(|b| b.kind() == kind) {
                let left = budget.get_mut(&kind).expect("bearer budget");
                if *left == 0 {
                    break; // this second's capacity is used; continue next tick
                }
                *left = left.saturating_sub(len);
                let frame = msg.payload.clone();
                if let Err(e) = b.send(&frame, id, t) {
                    if e.kind() == std::io::ErrorKind::InvalidInput {
                        // This link can't carry this message (e.g. too big for
                        // SBD): park it for retry_s so it can't block the queue.
                        out.queue.mark_sent(id, now_u);
                        continue;
                    }
                    if e.kind() != std::io::ErrorKind::WouldBlock {
                        eprintln!("link {kind:?}: {e}");
                    }
                    break;
                }
                out.queue.mark_sent(id, now_u); // removed when the ground acks
            } else {
                // Simulation (no transport): delivery assumed immediate.
                out.queue.ack(id);
            }
            lm.record_tx(kind, len);
            let e = sent.entry(kind).or_default();
            e.0 += 1;
            e.1 += len as u64;
        }
        for b in bearers.iter_mut() {
            if permitted(b.kind()) {
                b.maintain(t, out.seq);
                out.seq = out.seq.wrapping_add(1);
            }
        }

        // ---- faults and status lights
        if r.pos.is_some() || !m.gnss {
            last_pos_valid_t = t;
        }
        if t - last_pos_valid_t > gnss_fault_after {
            faults.insert(Fault::Gnss);
        } else {
            faults.remove(&Fault::Gnss);
        }
        if out.queue.bytes() * 10 > cfg.queue.max_bytes * 9 {
            faults.insert(Fault::QueueFull);
        } else {
            faults.remove(&Fault::QueueFull);
        }
        let radios_allowed = [LinkKind::Wifi, LinkKind::Cellular].iter().any(|k| m.has_link(*k) && permitted(*k));
        let lights = status::lights(&StatusInputs {
            phase,
            radios_allowed,
            rf_switch_off: switch_off,
            link_up: status.iter().any(|l| l.up),
            faults: &faults,
        });
        if last_lights != Some(lights) {
            println!("[{t:>6.0}s] LIGHTS green={:?} amber={:?} red={:?} faults={:?}", lights.green, lights.amber, lights.red, faults);
            last_lights = Some(lights);
        }
        if let Some(l) = leds.as_mut() {
            for (led, light) in l.iter_mut().zip([lights.green, lights.amber, lights.red]) {
                if let Err(e) = led.set(light.level(t)) {
                    eprintln!("led: {e}");
                }
            }
        }

        if !simulate {
            if last_snapshot.elapsed() >= Duration::from_secs(cfg.queue.snapshot_interval_s) {
                if let Err(e) = out.queue.save(&snapshot) {
                    eprintln!("cfbd: queue snapshot failed: {e}");
                }
                last_snapshot = Instant::now();
            }
            sd_notify("WATCHDOG=1");
        }

        // ---- OTA (CFB-HLR-041..044)
        let st = update::AircraftState {
            phase,
            engine_running: r.engine_running,
            ground_speed_kt: r.gs_kt,
            maintenance_mode: s.maintenance,
            supply_v: s.supply_v,
        };
        let gate_open = gate.check_conditions(&st).is_ok();
        if cfg.ota.enabled {
            let dl_link = if phase.is_confirmed_ground() {
                lm.select(phase, Priority::Bulk, 0, &status)
            } else {
                status
                    .iter()
                    .find(|l| l.up && cfg.ota.inflight_download_links.contains(&l.kind) && lm.tx_permitted(l.kind, phase))
                    .map(|l| l.kind)
            };
            let confirmed = if simulate { s.maintenance } else { confirm_file.exists() };
            if let Some(res) = ota_exec.poll(t) {
                ota.complete(t, res);
            }
            if !ota_exec.busy() || matches!(ota.state, OtaState::Downloading) {
                let ctx = OtaCtx { download_link_ok: dl_link.is_some(), gate_open, confirmed };
                if let Some(cmd) = ota.step(t, ctx) {
                    let bind_ip = dl_link
                        .and_then(|k| cfg.transport.as_ref()?.bearers.get(&k))
                        .and_then(|b| b.local.parse::<std::net::SocketAddr>().ok())
                        .map(|a| a.ip().to_string());
                    if cmd == OtaCmd::StartDownload {
                        println!("[{t:>6.0}s] OTA download over {:?}{}", dl_link.unwrap(),
                            if phase.is_confirmed_ground() { "" } else { " (in flight: download only, install on ground)" });
                    }
                    if cmd == OtaCmd::Install && !simulate {
                        let _ = std::fs::remove_file(confirm_file);
                    }
                    if cmd == OtaCmd::Reboot && simulate {
                        println!("[{t:>6.0}s] OTA reboot into new slot (simulated); mark-good after health check");
                    }
                    ota_exec.run(&cmd, t, &gate, bind_ip);
                }
            }
            let name = ota_state_name(&ota.state);
            if name != last_ota_state {
                out.event(t, MsgType::Health, Priority::Normal, format!("OTA {name}"));
                last_ota_state = name;
            }
        }

        // ---- status snapshot for the maintenance page
        if let Some(p) = &status_path {
            let snap = status_out::Snapshot {
                unix_time: SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                t,
                version: env!("CARGO_PKG_VERSION").into(),
                registration: cfg.aircraft.registration.clone(),
                modules: m.expected_ids().iter().map(|s| s.to_string()).collect(),
                phase: format!("{phase:?}"),
                lat: r.pos.map(|p| p.0),
                lon: r.pos.map(|p| p.1),
                pos_source: format!("{:?}", r.pos_src),
                alt_ft: r.alt_ft,
                alt_source: format!("{:?}", r.alt_src),
                gs_kt: r.gs_kt,
                engine_running: r.engine_running,
                engine_source: format!("{:?}", r.engine_src),
                lights: [("green", lights.green), ("amber", lights.amber), ("red", lights.red)]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), format!("{v:?}")))
                    .collect(),
                faults: faults.iter().map(|f| format!("{f:?}")).collect(),
                rf_switch_off: switch_off,
                power_fail: s.power_fail,
                maintenance_mode: s.maintenance,
                links: status
                    .iter()
                    .map(|l| status_out::LinkSnap {
                        kind: format!("{:?}", l.kind),
                        up: l.up,
                        permitted: permitted(l.kind),
                        detail: bearers.iter().find(|b| b.kind() == l.kind).map(|b| b.describe()).unwrap_or_default(),
                    })
                    .collect(),
                queue_messages: out.queue.len(),
                queue_bytes: out.queue.bytes(),
                queue_max_bytes: cfg.queue.max_bytes,
                queue_evicted: out.queue.evicted,
                ota_state: ota_state_name(&ota.state),
                update_gate: match gate.check_conditions(&st) {
                    Ok(()) => "OPEN".into(),
                    Err(e) => format!("blocked: {e:?}"),
                },
                edge_apps: edge.apps().into_iter().map(|(id, v, en)| format!("{id} {v} {}", if en { "running" } else { "DISABLED" })).collect(),
                tick_work_us_last: last_tick_us,
                input_truncated,
                edge_backlog: edge.backlog_len(),
                tick_work_us_max: tick_hist.max_us,
            };
            if let Err(e) = status_out::write(p, &snap) {
                if !status_err_logged {
                    eprintln!("cfbd: status snapshot {}: {e}", p.display());
                    status_err_logged = true;
                }
            }
        }

        // ---- update readiness, shown on the maintenance page
        if s.maintenance && phase.is_confirmed_ground() && (t as u64).is_multiple_of(30) {
            match gate.check_conditions(&st) {
                Ok(()) => println!("[{t:>6.0}s] update gate: OPEN (ready to load software)"),
                Err(e) => println!("[{t:>6.0}s] update gate: blocked {e:?}"),
            }
        }
        last_tick_us = tick_start.elapsed().as_micros() as u64;
        tick_hist.record(last_tick_us);
    }

    if !simulate {
        let _ = out.queue.save(&snapshot);
    }
    println!("--- summary ---");
    if let Some(d) = &arinc {
        println!("ARINC 429: {} words, {} parity errors, {} unknown labels",
            d.stats.words, d.stats.parity_errors, d.stats.unknown_labels);
    }
    if let Some(p) = &nmea {
        println!("GNSS: {} checksum errors", p.checksum_errors);
    }
    for (k, (n, b)) in &sent {
        println!("{k:?}: {n} frames, {b} bytes");
    }
    for b in &bearers {
        println!("link: {}", b.describe());
    }
    for (id, ver, en) in edge.apps() {
        println!("edge app {id} {ver}: {}", if en { "running" } else { "DISABLED" });
    }
    println!("still queued: {} messages, {} bytes; evicted: {}", out.queue.len(), out.queue.bytes(), out.queue.evicted);
    println!("input lines truncated: {input_truncated}; edge backlog at end: {}", edge.backlog_len());
    println!(
        "tick work: {} ticks, mean {} us, p50 {} us, p99 {} us, max {} us (budget 1 000 000 us per tick)",
        tick_hist.count,
        tick_hist.mean_us(),
        tick_hist.percentile_us(50.0),
        tick_hist.percentile_us(99.0),
        tick_hist.max_us
    );
    if let Some(kb) = peak_rss_kb() {
        println!("peak memory (VmHWM): {} KiB", kb);
    }
}

/// Peak resident memory of this process (Linux).
fn peak_rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()
}

/// Tick work-time histogram: 1 us buckets up to 1 ms, then 100 us buckets
/// up to 1 s, then one overflow bucket. Bounded memory for any run length.
#[derive(Default)]
struct TickHistogram {
    fine: Vec<u64>,
    coarse: Vec<u64>,
    over: u64,
    count: u64,
    sum_us: u64,
    max_us: u64,
}

impl TickHistogram {
    fn record(&mut self, us: u64) {
        if self.fine.is_empty() {
            self.fine = vec![0; 1000];
            self.coarse = vec![0; 10_000];
        }
        match us {
            0..=999 => self.fine[us as usize] += 1,
            1000..=999_999 => self.coarse[(us / 100) as usize] += 1,
            _ => self.over += 1,
        }
        self.count += 1;
        self.sum_us += us;
        self.max_us = self.max_us.max(us);
    }
    fn mean_us(&self) -> u64 {
        self.sum_us.checked_div(self.count).unwrap_or(0)
    }
    fn percentile_us(&self, p: f64) -> u64 {
        let target = ((self.count as f64) * p / 100.0).ceil() as u64;
        let mut acc = 0;
        for (i, n) in self.fine.iter().enumerate() {
            acc += n;
            if acc >= target {
                return i as u64;
            }
        }
        for (i, n) in self.coarse.iter().enumerate() {
            acc += n;
            if acc >= target {
                return i as u64 * 100;
            }
        }
        self.max_us
    }
}
