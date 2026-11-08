// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Status outputs. The LEDs are Linux LED-class devices (for example from
//! the Raspberry Pi `gpio-led` overlay: `/sys/class/leds/cfb-green`),
//! so the GPIO numbers live in the device tree, not in the software.

use std::io;
use std::path::PathBuf;

/// Anything that can show on/off (LED, buzzer, display segment).
pub trait Indicator {
    fn set(&mut self, on: bool) -> io::Result<()>;
}

/// LED-class device; writes `brightness` only when the level changes.
pub struct SysfsLed {
    path: PathBuf,
    last: Option<bool>,
}

impl SysfsLed {
    pub fn new(dir: &str) -> Self {
        Self { path: PathBuf::from(dir).join("brightness"), last: None }
    }
}

impl Indicator for SysfsLed {
    fn set(&mut self, on: bool) -> io::Result<()> {
        if self.last == Some(on) {
            return Ok(());
        }
        std::fs::write(&self.path, if on { "1" } else { "0" })?;
        self.last = Some(on);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_on_change() {
        let dir = std::env::temp_dir().join(format!("cfb-led-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut l = SysfsLed::new(dir.to_str().unwrap());
        l.set(true).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap(), "1");
        std::fs::write(dir.join("brightness"), "x").unwrap();
        l.set(true).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap(), "x", "no rewrite");
        l.set(false).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("brightness")).unwrap(), "0");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
