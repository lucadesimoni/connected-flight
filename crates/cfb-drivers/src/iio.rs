// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Linux IIO sensors through sysfs (no unsafe, no extra crates).
//!
//! * Accelerometer, e.g. ST ISM330DHCX (driver `st_lsm6dsx`, device name
//!   `ism330dhcx_accel`): `in_accel_{x,y,z}_raw` × `in_accel_scale`
//!   (m/s² per LSB), converted to g.
//! * Pressure, e.g. Bosch BMP390 (driver `bmp280`): `in_pressure_input`
//!   in kPa (IIO ABI), converted to hPa.
//!
//! Sysfs polling at 100 Hz costs a few hundred small reads per second,
//! which is fine on the Pi. A future buffered (/dev/iio) reader can replace
//! [`IioAccel::sample`] without touching anything else.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SYSFS_IIO: &str = "/sys/bus/iio/devices";
const G: f64 = 9.806_65;

/// First IIO device under `root` whose `name` contains `pattern`.
pub fn find_device(root: &Path, pattern: &str) -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    v.sort();
    v.into_iter().find(|d| fs::read_to_string(d.join("name")).is_ok_and(|n| n.trim().contains(pattern)))
}

fn read_f64(p: &Path) -> io::Result<f64> {
    fs::read_to_string(p)?.trim().parse().map_err(|_| io::Error::other(format!("{}: not a number", p.display())))
}

pub struct IioAccel {
    dir: PathBuf,
    scale: f64,
}

impl IioAccel {
    pub fn open(dir: &Path, rate_hz: u32) -> io::Result<Self> {
        // Best effort: the driver picks the nearest supported rate.
        let _ = fs::write(dir.join("in_accel_sampling_frequency"), rate_hz.to_string());
        let scale = read_f64(&dir.join("in_accel_scale"))?;
        Ok(Self { dir: dir.to_path_buf(), scale })
    }

    /// One sample in g, sensor axes.
    pub fn sample(&self) -> io::Result<[f32; 3]> {
        let mut out = [0f32; 3];
        for (i, a) in ["x", "y", "z"].iter().enumerate() {
            let raw = read_f64(&self.dir.join(format!("in_accel_{a}_raw")))?;
            out[i] = (raw * self.scale / G) as f32;
        }
        Ok(out)
    }

    /// Sampling thread at `rate_hz`; keeps at most 2 s of samples between drains.
    pub fn spawn(self, rate_hz: u32, buf: Arc<Mutex<Vec<[f32; 3]>>>) {
        let period = Duration::from_secs_f64(1.0 / rate_hz.max(1) as f64);
        let cap = 2 * rate_hz as usize;
        std::thread::spawn(move || {
            let mut next = Instant::now();
            let mut errors = 0u32;
            loop {
                match self.sample() {
                    Ok(s) => {
                        errors = 0;
                        let mut b = buf.lock().unwrap();
                        if b.len() < cap {
                            b.push(s);
                        }
                    }
                    Err(e) => {
                        errors += 1;
                        if errors == 1 || errors.is_multiple_of(1000) {
                            eprintln!("imu: {e}");
                        }
                    }
                }
                next += period;
                let now = Instant::now();
                if next > now {
                    std::thread::sleep(next - now);
                } else {
                    next = now; // overloaded: don't try to catch up
                }
            }
        });
    }
}

pub struct IioPressure {
    dir: PathBuf,
}

impl IioPressure {
    pub fn open(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf() }
    }

    /// Static pressure in hPa.
    pub fn read_hpa(&self) -> io::Result<f64> {
        Ok(read_f64(&self.dir.join("in_pressure_input"))? * 10.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_sysfs() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cfb-iio-{}-{:?}", std::process::id(), std::thread::current().id()));
        let acc = root.join("iio:device0");
        let prs = root.join("iio:device1");
        fs::create_dir_all(&acc).unwrap();
        fs::create_dir_all(&prs).unwrap();
        fs::write(acc.join("name"), "ism330dhcx_accel\n").unwrap();
        fs::write(acc.join("in_accel_scale"), "0.000598\n").unwrap(); // ±2 g range
        fs::write(acc.join("in_accel_x_raw"), "0\n").unwrap();
        fs::write(acc.join("in_accel_y_raw"), "164\n").unwrap();
        fs::write(acc.join("in_accel_z_raw"), "-16399\n").unwrap();
        fs::write(prs.join("name"), "bmp390\n").unwrap();
        fs::write(prs.join("in_pressure_input"), "101.325\n").unwrap();
        root
    }

    #[test]
    fn finds_devices_and_converts_units() {
        let root = fake_sysfs();
        let a = find_device(&root, "ism330dhcx").unwrap();
        let acc = IioAccel::open(&a, 104).unwrap();
        let s = acc.sample().unwrap();
        assert!(s[0].abs() < 1e-6);
        assert!((s[2] + 1.0).abs() < 0.01, "z ≈ -1 g at rest: {}", s[2]);
        let p = IioPressure::open(&find_device(&root, "bmp").unwrap());
        assert!((p.read_hpa().unwrap() - 1013.25).abs() < 1e-9);
        assert!(find_device(&root, "nothing").is_none());
        fs::remove_dir_all(&root).unwrap();
    }
}
