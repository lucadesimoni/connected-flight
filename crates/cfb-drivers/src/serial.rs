// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Serial port helpers without unsafe code: line settings via `stty`,
//! then plain file I/O. With `min 0 time N` a read returns after N tenths
//! of a second without data, which gives timeouts without poll(2).

use std::fs::{File, OpenOptions};
use std::io;
use std::process::Command;

/// Raw mode, no echo, `baud`, reads time out after `timeout_ds` × 0.1 s.
pub fn configure(dev: &str, baud: u32, timeout_ds: u8) -> io::Result<()> {
    let st = Command::new("stty")
        .args(["-F", dev, &baud.to_string(), "raw", "-echo", "-crtscts", "min", "0", "time", &timeout_ds.to_string()])
        .status()?;
    if st.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("stty {dev} failed: {st}")))
    }
}

pub fn open_rw(dev: &str) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(dev)
}
