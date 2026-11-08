//! cfbd - Connected Flight Box daemon.
//!
//! Single-threaded 1 Hz control loop: read inputs, update flight phase,
//! set radio enables, queue reports, hand queued messages to the best
//! allowed link. All decisions are in `cfb-core`; this file is wiring.
//!
//! Usage:
//!   cfbd --config /etc/cfb/cfb.toml            # live, inputs on stdin
//!   cfbd --config config/cfb.toml --simulate   # scripted demo flight
//!
//! Live input format (one line per second, from the I/O adapter services):
//!   t=<mono s> wow=<0|1|-> gs=<kt|-> eng=<0|1|-> v=<volts|-> maint=<0|1>
//!   lat=<deg|-> lon=<deg|-> alt=<ft|-> trk=<deg|-> vs=<fpm|->
//!   link.<wifi|cellular|leo_sat|iridium>=<up bps|down>

use cfb_core::frame::EPOCH_2020;
use cfb_core::phase::PhaseInputs;
use cfb_core::*;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default)]
struct Sample {
    phase: PhaseInputs,
    supply_v: Option<f32>,
    maintenance: bool,
    lat: Option<f64>,
    lon: Option<f64>,
    alt_ft: Option<i32>,
    track: Option<f32>,
    vs_fpm: Option<i32>,
    links: BTreeMap<LinkKind, LinkStatus>,
}

trait Inputs {
    fn next(&mut self) -> Option<Sample>;
}

// ------------------------------------------------------------ live inputs

struct LineInputs<R: BufRead> {
    r: R,
    links: BTreeMap<LinkKind, LinkStatus>,
}

fn opt<T: std::str::FromStr>(v: &str) -> Option<T> {
    if v == "-" { None } else { v.parse().ok() }
}

fn link_kind(s: &str) -> Option<LinkKind> {
    Some(match s {
        "wifi" => LinkKind::Wifi,
        "cellular" => LinkKind::Cellular,
        "leo_sat" => LinkKind::LeoSat,
        "iridium" => LinkKind::Iridium,
        _ => return None,
    })
}

impl<R: BufRead> Inputs for LineInputs<R> {
    fn next(&mut self) -> Option<Sample> {
        let mut line = String::new();
        if self.r.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let mut s = Sample::default();
        for kv in line.split_whitespace() {
            let Some((k, v)) = kv.split_once('=') else { continue };
            match k {
                "t" => s.phase.t = v.parse().unwrap_or(0.0),
                "wow" => s.phase.wow = opt::<u8>(v).map(|x| x != 0),
                "gs" => s.phase.ground_speed_kt = opt(v),
                "eng" => s.phase.engine_running = opt::<u8>(v).map(|x| x != 0),
                "v" => s.supply_v = opt(v),
                "maint" => s.maintenance = v == "1",
                "lat" => s.lat = opt(v),
                "lon" => s.lon = opt(v),
                "alt" => s.alt_ft = opt(v),
                "trk" => s.track = opt(v),
                "vs" => s.vs_fpm = opt(v),
                _ if k.starts_with("link.") => {
                    if let Some(kind) = link_kind(&k[5..]) {
                        let (up, bps) = match v.split_once(':') {
                            Some(("up", b)) => (true, b.parse().unwrap_or(0)),
                            _ => (false, 0),
                        };
                        self.links.insert(kind, LinkStatus { kind, up, est_bps: bps });
                    }
                }
                _ => {}
            }
        }
        s.links = self.links.clone();
        Some(s)
    }
}

// ------------------------------------------------------------ simulation

/// Scripted flight: parked, taxi, takeoff, 25 min cruise, landing, taxi in.
struct SimInputs {
    t: u32,
}

impl Inputs for SimInputs {
    fn next(&mut self) -> Option<Sample> {
        let t = self.t;
        self.t += 1;
        let (gs, wow, eng): (f32, bool, bool) = match t {
            0..=59 => (0.0, true, false),                          // parked
            60..=179 => (if t < 90 { 0.0 } else { 15.0 }, true, true), // start, taxi
            180..=199 => (15.0 + (t - 180) as f32 * 4.0, t < 196, true), // takeoff roll
            200..=1700 => (140.0, false, true),                    // airborne
            1701..=1720 => (70.0 - (t - 1701) as f32 * 2.5, t > 1703, true), // landing
            1721..=1800 => (12.0, true, true),                     // taxi in
            1801..=1900 => (0.0, true, false),                     // shutdown
            _ => return None,
        };
        let airborne_part = (200..=1700).contains(&t);
        let mut links = BTreeMap::new();
        // Cellular coverage on the airfield only; Wi-Fi at the home base only.
        let at_base = !(120..=1820).contains(&t);
        links.insert(LinkKind::Wifi, LinkStatus { kind: LinkKind::Wifi, up: at_base, est_bps: 50_000_000 });
        links.insert(LinkKind::Cellular, LinkStatus { kind: LinkKind::Cellular, up: !airborne_part, est_bps: 20_000_000 });
        links.insert(LinkKind::Iridium, LinkStatus { kind: LinkKind::Iridium, up: true, est_bps: 22_000 });
        let d = (t.saturating_sub(200).min(1500)) as f64;
        Some(Sample {
            phase: PhaseInputs { t: t as f64, wow: Some(wow), ground_speed_kt: Some(gs), engine_running: Some(eng) },
            supply_v: Some(if eng { 28.2 } else { 24.6 }),
            maintenance: t > 1850,
            lat: Some(47.0 + d * 0.0005),
            lon: Some(8.3 + d * 0.0003),
            alt_ft: Some(if airborne_part { 8500 } else { 1400 }),
            track: Some(31.0),
            vs_fpm: Some(0),
            links,
        })
    }
}

// ------------------------------------------------------------ systemd

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

// ------------------------------------------------------------ main loop

fn main() {
    let mut cfg_path = PathBuf::from("/etc/cfb/cfb.toml");
    let mut simulate = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = args.next().map(PathBuf::from).unwrap_or(cfg_path),
            "--simulate" => simulate = true,
            "-h" | "--help" => {
                println!("cfbd [--config PATH] [--simulate]");
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

    let mut src: Box<dyn Inputs> = if simulate {
        Box::new(SimInputs { t: 0 })
    } else {
        Box::new(LineInputs { r: std::io::stdin().lock(), links: BTreeMap::new() })
    };

    let snapshot = PathBuf::from(&cfg.queue.snapshot_path);
    let mut queue = if simulate {
        StoreForwardQueue::new(cfg.queue.max_bytes)
    } else {
        StoreForwardQueue::load_or_new(&snapshot, cfg.queue.max_bytes)
    };
    let mut det = PhaseDetector::new(cfg.phase);
    let mut lm = LinkManager::new(cfg.links.clone());
    let mut rf_enabled: BTreeMap<LinkKind, bool> = BTreeMap::new();
    let mut last_phase = det.phase();
    // Set at confirmed airborne, cleared at the next confirmed ground. The
    // landing roll passes through `Unknown` (WoW on, still fast), so a plain
    // Airborne -> OnGround edge check would miss it.
    let mut flown_this_leg = false;
    let mut last_pos_t = f64::NEG_INFINITY;
    let mut last_snapshot = Instant::now();
    let mut seq: u16 = 0;
    let mut sent: BTreeMap<LinkKind, (u32, u64)> = BTreeMap::new();

    println!("cfbd: {} ({}) started, sim={simulate}", cfg.aircraft.registration, cfg.aircraft.aircraft_type);
    sd_notify("READY=1");

    while let Some(s) = src.next() {
        let t = s.phase.t;
        let phase = det.update(s.phase);

        if phase != last_phase {
            println!("[{t:>6.0}s] phase {last_phase:?} -> {phase:?}");
            if phase == FlightPhase::Airborne && !flown_this_leg {
                lm.reset_flight_budgets();
                flown_this_leg = true;
            }
            if phase == FlightPhase::OnGround && flown_this_leg {
                flown_this_leg = false;
                // Post-flight: queue the flight data file for ground upload.
                // (Stand-in for the recorded engine/flight data.)
                for chunk in 0..8u8 {
                    let f = Frame { msg_type: MsgType::FileChunk, seq, time: now_2020(), payload: vec![chunk; 32 * 1024] };
                    seq = seq.wrapping_add(1);
                    queue.push(Priority::Bulk, f.encode(), t as u64, 0);
                }
                println!("[{t:>6.0}s] queued flight data file (8 x 32 KiB) for ground upload");
            }
            last_phase = phase;
        }

        // Radio enables (software layer of the RF inhibit).
        for kind in [LinkKind::Wifi, LinkKind::Cellular, LinkKind::LeoSat, LinkKind::Iridium] {
            let en = lm.tx_permitted(kind, phase);
            if rf_enabled.get(&kind) != Some(&en) {
                println!("[{t:>6.0}s] RF {kind:?}: {}", if en { "ENABLED" } else { "inhibited" });
                rf_enabled.insert(kind, en);
            }
        }

        // Position reports.
        let interval = if phase.is_confirmed_ground() {
            cfg.reporting.position_interval_ground_s
        } else {
            cfg.reporting.position_interval_air_s
        };
        if interval > 0 && t - last_pos_t >= interval as f64 {
            if let (Some(lat), Some(lon)) = (s.lat, s.lon) {
                let p = PositionReport {
                    lat_deg: lat,
                    lon_deg: lon,
                    alt_ft: s.alt_ft.unwrap_or(0),
                    gs_kt: s.phase.ground_speed_kt.unwrap_or(0.0).max(0.0) as u16,
                    track_deg: s.track.unwrap_or(0.0),
                    vs_fpm: s.vs_fpm.unwrap_or(0),
                    airborne: !phase.is_confirmed_ground(),
                    emergency: false,
                };
                let f = Frame { msg_type: MsgType::Position, seq, time: now_2020(), payload: p.encode().to_vec() };
                seq = seq.wrapping_add(1);
                queue.push(Priority::Critical, f.encode(), t as u64, cfg.reporting.position_ttl_s);
                last_pos_t = t;
            }
        }

        // Drain the queue onto links.
        queue.expire(t as u64);
        let status: Vec<LinkStatus> = s.links.values().copied().collect();
        while let Some(m) = queue.peek(|p| lm.can_carry(phase, p, &status)) {
            let Some(kind) = lm.select(phase, m.priority, m.payload.len(), &status) else { break };
            let (id, len) = (m.id, m.payload.len());
            // Live mode hands the frame to the link adapter here; delivery is
            // acknowledged end-to-end by the ground service (MsgType::Ack).
            // The simulation acks immediately.
            lm.record_tx(kind, len);
            let e = sent.entry(kind).or_default();
            e.0 += 1;
            e.1 += len as u64;
            queue.ack(id);
        }

        if !simulate {
            if last_snapshot.elapsed() >= Duration::from_secs(cfg.queue.snapshot_interval_s) {
                if let Err(e) = queue.save(&snapshot) {
                    eprintln!("cfbd: queue snapshot failed: {e}");
                }
                last_snapshot = Instant::now();
            }
            sd_notify("WATCHDOG=1");
        }

        // Update readiness, shown on the maintenance page.
        if s.maintenance && phase.is_confirmed_ground() && (t as u64).is_multiple_of(30) {
            let gate = UpdateGate {
                trusted_keys: cfg.trusted_verifying_keys().unwrap_or_default(),
                hw_part_number: cfg.update.hw_part_number.clone(),
                installed_version: env!("CARGO_PKG_VERSION").into(),
                min_supply_v: cfg.update.min_supply_v,
            };
            let st = update::AircraftState {
                phase,
                engine_running: s.phase.engine_running,
                ground_speed_kt: s.phase.ground_speed_kt,
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
        let _ = queue.save(&snapshot);
    }
    println!("--- traffic summary ---");
    for (k, (n, b)) in &sent {
        println!("{k:?}: {n} frames, {b} bytes");
    }
    println!("still queued: {} messages, {} bytes; evicted: {}", queue.len(), queue.bytes(), queue.evicted);
}
