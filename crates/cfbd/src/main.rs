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
mod transport;

use cfb_core::arinc429::{Decoder, Ssm, Word};
use cfb_core::edge::{AppPackage, DataView, EdgeRuntime, RuntimeEvent};
use cfb_core::ota::{Ota, OtaCmd, OtaCtx, OtaState};
use cfb_core::frame::EPOCH_2020;
use cfb_core::gnss::NmeaParser;
use cfb_core::imu::{EngineVibDetector, ImuProcessor};
use cfb_core::reference;
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

    // Ground transport: one UDP socket per IP bearer (live mode only).
    let mut bearers: Vec<transport::Bearer> = Vec::new();
    if let (false, Some(tr)) = (simulate, &cfg.transport) {
        let ground = tr.ground.parse().expect("validated");
        for (k, b) in &tr.bearers {
            match transport::Bearer::open(*k, b, ground) {
                Ok(x) => bearers.push(x),
                Err(e) => eprintln!("transport: bearer {k:?} ({}) not available: {e}", b.local),
            }
        }
    }
    let retry_s = cfg.transport.as_ref().map_or(15, |t| t.retry_s);

    // OTA.
    let mut ota = Ota::new(cfg.ota.retry_s);
    let mut ota_exec = ota_exec::OtaExec::new(cfg.ota.clone(), simulate);
    let mut last_ota_state = String::new();
    let confirm_file = Path::new("/run/cfb/ota/confirm");

    let mut rf_enabled: BTreeMap<LinkKind, bool> = BTreeMap::new();
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

    while let Some(s) = src.next() {
        let t = s.t;

        // ---- module check (CFB-HLR-070)
        if let Some(hw) = &s.hw {
            let det_ids: Vec<&str> = hw.iter().map(String::as_str).collect();
            for msg in m.check_detected(&det_ids) {
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
        let r = reference::select(
            &cfg.reference,
            t,
            arinc.as_ref(),
            nmea.as_ref().map(|p| p.fix),
            s.engine_discrete,
            engine_vib,
        );
        if r.pos_src != last_ref.pos_src || r.engine_src != last_ref.engine_src {
            out.event(t, MsgType::Health, Priority::Normal,
                format!("REF position={:?} engine={:?}", r.pos_src, r.engine_src));
        }
        if r.pos_disagree != last_ref.pos_disagree || r.gs_disagree != last_ref.gs_disagree {
            out.event(t, MsgType::Health, Priority::Normal,
                format!("REF cross-check pos_disagree={} gs_disagree={}", r.pos_disagree, r.gs_disagree));
        }

        // ---- flight phase
        let phase = det.update(PhaseInputs {
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
                RuntimeEvent::Disabled { app, reason } => format!("EDGE app {app} DISABLED: {reason}"),
                RuntimeEvent::Dropped { app, outputs, reason } => format!("EDGE app {app}: {outputs} output(s) dropped ({reason})"),
            };
            out.event(t, MsgType::Health, Priority::Normal, txt);
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

        // ---- radio enables (software layer of the RF inhibit)
        for kind in [LinkKind::Wifi, LinkKind::Cellular, LinkKind::LeoSat, LinkKind::Iridium] {
            if !m.has_link(kind) {
                continue;
            }
            let en = lm.tx_permitted(kind, phase);
            if rf_enabled.get(&kind) != Some(&en) {
                println!("[{t:>6.0}s] RF {kind:?}: {}", if en { "ENABLED" } else { "inhibited" });
                rf_enabled.insert(kind, en);
            }
        }

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
                    airborne: !phase.is_confirmed_ground(),
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
                    transport::Rx::Acked(id) => {
                        out.queue.ack(id);
                    }
                    transport::Rx::Uplink(f) => {
                        let txt = String::from_utf8_lossy(&f.payload).into_owned();
                        println!("[{t:>6.0}s] UPLINK via {:?} {:?}: {txt}", b.kind, f.msg_type);
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

        // ---- link status: from the bearers when the transport runs here
        let status: Vec<LinkStatus> = if bearers.is_empty() {
            s.links.values().copied().filter(|l| m.has_link(l.kind)).collect()
        } else {
            bearers.iter().map(|b| b.status(t)).collect()
        };

        // ---- drain the queue onto links
        let now_u = t as u64;
        out.queue.expire(now_u);
        let mut budget: BTreeMap<LinkKind, usize> = bearers.iter().map(|b| (b.kind, b.tick_budget())).collect();
        while let Some(msg) = out.queue.peek_ready(|p| lm.can_carry(phase, p, &status), now_u, retry_s) {
            let Some(kind) = lm.select(phase, msg.priority, msg.payload.len(), &status) else { break };
            let (id, len) = (msg.id, msg.payload.len());
            if let Some(b) = bearers.iter_mut().find(|b| b.kind == kind) {
                let left = budget.get_mut(&kind).expect("bearer budget");
                if *left == 0 {
                    break; // this second's capacity is used; continue next tick
                }
                *left = left.saturating_sub(len);
                let frame = msg.payload.clone();
                if let Err(e) = b.send(&frame, id, t) {
                    eprintln!("transport {kind:?}: {e}");
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
            if lm.tx_permitted(b.kind, phase) {
                b.maybe_keepalive(t, out.seq);
                out.seq = out.seq.wrapping_add(1);
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

        // ---- update readiness, shown on the maintenance page
        if s.maintenance && phase.is_confirmed_ground() && (t as u64).is_multiple_of(30) {
            match gate.check_conditions(&st) {
                Ok(()) => println!("[{t:>6.0}s] update gate: OPEN (ready to load software)"),
                Err(e) => println!("[{t:>6.0}s] update gate: blocked {e:?}"),
            }
        }
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
    for (id, ver, en) in edge.apps() {
        println!("edge app {id} {ver}: {}", if en { "running" } else { "DISABLED" });
    }
    println!("still queued: {} messages, {} bytes; evicted: {}", out.queue.len(), out.queue.bytes(), out.queue.evicted);
}
