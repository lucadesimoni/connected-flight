// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! cfb-io - I/O adapter for the bench / portable unit.
//!
//! Writes one line per second in the cfbd input format to stdout:
//!
//!   cfb-io --gnss /dev/ttyACM0 --discretes /run/cfb/discretes \
//!          --hw base,gnss,imu,baro,wifi,cellular_4g \
//!          --imu-iio ism330dhcx --baro-iio bmp | cfbd ...
//!
//! Sources (each optional, each a driver in `cfb-drivers`):
//! * GNSS: NMEA RMC/GGA from a serial/USB device (`nmea_serial`);
//! * IMU: accelerometer via Linux IIO at `--imu-rate` Hz (`iio::IioAccel`),
//!   sent as `imu=x,y,z;...` (g, sensor axes);
//! * barometer via Linux IIO (`iio::IioPressure`), sent as `baro=<hPa>`;
//! * discretes `wow eng maint v rfsw pfail` from a file and/or GPIO
//!   commands (`discretes`). Settings also from the environment, for
//!   systemd: `CFB_GPIO="wow=gpioget -c gpiochip0 17;rfsw=…"`,
//!   `CFB_INVERT="rfsw"`, `CFB_IMU_IIO`, `CFB_BARO_IIO`.
//!
//! Links are not reported here: cfbd's link drivers measure them.

use cfb_drivers::{discretes, iio, nmea_serial};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn main() {
    let mut gnss = None;
    let mut disc_file = "/run/cfb/discretes".to_string();
    let mut hw = None;
    let mut gpio: Vec<(String, String)> = Vec::new();
    let mut invert: Vec<String> = Vec::new();
    let mut imu_name = std::env::var("CFB_IMU_IIO").ok().filter(|s| !s.is_empty());
    let mut baro_name = std::env::var("CFB_BARO_IIO").ok().filter(|s| !s.is_empty());
    let mut imu_rate: u32 = 100;
    let mut iio_root = iio::SYSFS_IIO.to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--gnss" => gnss = args.next().filter(|s| !s.is_empty()),
            "--discretes" => disc_file = args.next().unwrap_or(disc_file),
            "--hw" => hw = args.next(),
            "--gpio" => gpio.extend(args.next().map(|s| discretes::parse_gpio_env(&s)).unwrap_or_default()),
            "--invert" => invert.extend(args.next()),
            "--imu-iio" => imu_name = args.next(),
            "--baro-iio" => baro_name = args.next(),
            "--imu-rate" => imu_rate = args.next().and_then(|s| s.parse().ok()).unwrap_or(imu_rate),
            "--iio-root" => iio_root = args.next().unwrap_or(iio_root),
            _ => {
                eprintln!("usage: cfb-io [--gnss DEV] [--discretes FILE] [--hw IDS] [--gpio KEY=CMD;...] [--invert KEY]... [--imu-iio NAME] [--imu-rate HZ] [--baro-iio NAME] [--iio-root DIR]");
                std::process::exit(2);
            }
        }
    }
    if let Ok(g) = std::env::var("CFB_GPIO") {
        gpio.extend(discretes::parse_gpio_env(&g));
    }
    if let Ok(i) = std::env::var("CFB_INVERT") {
        invert.extend(i.split_whitespace().map(String::from));
    }

    let nmea = Arc::new(Mutex::new(Vec::new()));
    if let Some(dev) = gnss {
        nmea_serial::spawn_reader(dev, nmea.clone());
    }
    let imu = Arc::new(Mutex::new(Vec::new()));
    if let Some(name) = &imu_name {
        match iio::find_device(Path::new(&iio_root), name).map(|d| iio::IioAccel::open(&d, imu_rate)) {
            Some(Ok(a)) => a.spawn(imu_rate, imu.clone()),
            Some(Err(e)) => eprintln!("cfb-io: IMU '{name}': {e}"),
            None => eprintln!("cfb-io: IMU '{name}' not found under {iio_root}"),
        }
    }
    let baro = baro_name.as_ref().and_then(|name| {
        let d = iio::find_device(Path::new(&iio_root), name);
        if d.is_none() {
            eprintln!("cfb-io: barometer '{name}' not found under {iio_root}");
        }
        d.map(|d| iio::IioPressure::open(&d))
    });

    let start = Instant::now();
    let mut stdout = std::io::stdout().lock();
    let mut first = true;
    for tick in 0u64.. {
        let d = discretes::read(&disc_file, &gpio, &invert);
        let mut line = format!("t={tick}");
        for k in discretes::KEYS {
            line.push_str(&format!(" {k}={}", d.get(k).map(String::as_str).unwrap_or("-")));
        }
        if let Some(b) = &baro {
            if let Ok(p) = b.read_hpa() {
                line.push_str(&format!(" baro={p:.2}"));
            }
        }
        for s in nmea.lock().unwrap().drain(..) {
            line.push_str(" nmea=");
            line.push_str(&s);
        }
        let samples: Vec<[f32; 3]> = imu.lock().unwrap().drain(..).collect();
        if !samples.is_empty() {
            let v: Vec<String> = samples.iter().map(|s| format!("{:.4},{:.4},{:.4}", s[0], s[1], s[2])).collect();
            line.push_str(" imu=");
            line.push_str(&v.join(";"));
        }
        if first {
            if let Some(h) = &hw {
                line.push_str(&format!(" hw={h}"));
            }
            first = false;
        }
        if writeln!(stdout, "{line}").and_then(|_| stdout.flush()).is_err() {
            return; // cfbd went away; systemd restarts the pipeline
        }
        let next = start + Duration::from_secs(tick + 1);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}
