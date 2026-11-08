// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary and confidential. See LICENSE.

//! cfb-a429-survey - bench tool: which ARINC 429 labels are on the bus, at
//! what rate, with which SSM, compared with the installation's label table.
//!
//!   cfb-io ... | cfb-a429-survey --config /etc/cfb/cfb.toml
//!
//! Reads the cfb-io line format (`t=` and `a429=<ch>:<hex>,...` tokens).
//! Use it to verify an ICD-derived label table on the avionics test bench
//! (with the data owner's permission), not to guess undocumented data.

use cfb_core::arinc429::{Survey, Word};
use cfb_core::Config;
use std::io::BufRead;

fn main() {
    let mut cfg_path = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--config" {
            cfg_path = args.next();
        }
    }
    let cfg = cfg_path.map(|p| {
        let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{p}: {e}"));
        Config::from_toml(&s).unwrap_or_else(|e| panic!("{p}: {e}"))
    });
    let reversed = cfg.as_ref().is_some_and(|c| c.arinc429.label_reversed);
    let mut s = Survey::default();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        let mut t = 0.0;
        for kv in line.split_whitespace() {
            if let Some(v) = kv.strip_prefix("t=") {
                t = v.parse().unwrap_or(t);
            } else if let Some(v) = kv.strip_prefix("a429=") {
                for w in v.split(',') {
                    if let Some((ch, hex)) = w.split_once(':') {
                        if let (Ok(ch), Ok(raw)) = (ch.parse(), u32::from_str_radix(hex, 16)) {
                            s.feed(ch, Word::from_raw(raw, reversed), t);
                        }
                    }
                }
            }
        }
    }
    println!("ch label sdi   count  rate_Hz  ssm[00,01,10,11]      raw_min    raw_max");
    for ((ch, l, sdi), st) in &s.labels {
        println!("{ch:>2}   {l:03o}  {sdi}  {:>7}  {:>7.2}  {:?}  {:>9}  {:>9}", st.count, st.rate_hz(), st.ssm, st.min_raw, st.max_raw);
    }
    println!("parity errors: {}", s.parity_errors);
    if let Some(c) = cfg {
        let (missing, extra) = s.compare(&c.arinc429.labels);
        println!("configured but not seen: {missing:?}");
        println!("seen but not configured: {extra:?}");
    }
}
