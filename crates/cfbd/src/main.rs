//! cfbd - Connected Flight Box daemon.
//!
//! Single-threaded 1 Hz control loop:
//!   raw inputs (ARINC 429, own GNSS, own IMU, discretes, link state)
//!   -> decoders -> reference selection -> flight phase
//!   -> radio enables, reports, advisories -> queue -> best permitted link.
//! All decisions are in `cfb-core`; this file is wiring.
//!
//! Usage:
//!   cfbd --config /etc/cfb/cfb.toml                    # live, inputs on stdin
//!   cfbd --config config/cfb.toml --simulate           # scripted demo flight
//!   cfbd --config config/cfb.toml --simulate --hard-landing

mod input;
mod sim;

use cfb_core::arinc429::{Decoder, Word};
use cfb_core::frame::EPOCH_2020;
use cfb_core::gnss::NmeaParser;
use cfb_core::imu::{EngineVibDetector, ImuProcessor};
use cfb_core::reference;
use cfb_core::*;
use input::{Inputs, LineInputs};
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

fn main() {
    let mut cfg_path = PathBuf::from("/etc/cfb/cfb.toml");
    let (mut simulate, mut hard_landing) = (false, false);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = args.next().map(PathBuf::from).unwrap_or(cfg_path),
            "--simulate" => simulate = true,
            "--hard-landing" => hard_landing = true,
            "-h" | "--help" => {
                println!("cfbd [--config PATH] [--simulate [--hard-landing]]");
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
            has_arinc: m.arinc429.is_some(),
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

        // ---- drain the queue onto links
        out.queue.expire(t as u64);
        let status: Vec<LinkStatus> = s.links.values().copied().filter(|l| m.has_link(l.kind)).collect();
        while let Some(msg) = out.queue.peek(|p| lm.can_carry(phase, p, &status)) {
            let Some(kind) = lm.select(phase, msg.priority, msg.payload.len(), &status) else { break };
            let (id, len) = (msg.id, msg.payload.len());
            // Live mode hands the frame to the link adapter here; delivery is
            // acknowledged end-to-end by the ground service (MsgType::Ack).
            // The simulation acks immediately.
            lm.record_tx(kind, len);
            let e = sent.entry(kind).or_default();
            e.0 += 1;
            e.1 += len as u64;
            out.queue.ack(id);
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

        // ---- update readiness, shown on the maintenance page
        if s.maintenance && phase.is_confirmed_ground() && (t as u64).is_multiple_of(30) {
            let gate = UpdateGate {
                trusted_keys: cfg.trusted_verifying_keys().unwrap_or_default(),
                hw_part_number: cfg.update.hw_part_number.clone(),
                installed_version: env!("CARGO_PKG_VERSION").into(),
                min_supply_v: cfg.update.min_supply_v,
            };
            let st = update::AircraftState {
                phase,
                engine_running: r.engine_running,
                ground_speed_kt: r.gs_kt,
                maintenance_mode: s.maintenance,
                supply_v: s.supply_v,
            };
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
    println!("still queued: {} messages, {} bytes; evicted: {}", out.queue.len(), out.queue.bytes(), out.queue.evicted);
}
