//! ARINC 429 receive-only decoding.
//!
//! The CFB only *listens* on ARINC 429 buses. The receiver module has no
//! transmitter fitted (see docs/03-hardware.md), so nothing here can put a
//! word on an aircraft bus.
//!
//! Word convention used throughout: a `u32` where bit 1 of the ARINC word is
//! the least significant bit. Bits 1–8 are the label, sent MSB first on the
//! wire, so in this `u32` the label's most significant bit is bit 1. Most
//! receiver ICs can be set to deliver the label either way round; use
//! [`Word::from_raw`] with `label_reversed` to match the chip setting.
//!
//! ```text
//!  32 | 31 30 | 29 | 28 .................. 11 | 10 9 | 8 ...... 1
//!   P |  SSM  |sign|        data              | SDI  |   label
//! ```
//!
//! Each installation lists the labels it decodes in the config
//! (`[[arinc429.labels]]`), taken from the ICD of the source LRU. The
//! defaults below are common GAMA/ARINC 429 assignments and must be checked
//! against the actual source equipment.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Word(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ssm {
    /// BNR 11 / BCD 00 or 11: data valid.
    Normal,
    NoComputedData,
    FunctionalTest,
    FailureWarning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Two's complement binary, sign at bit 29.
    Bnr,
    /// Binary coded decimal, bits 11–29, sign in SSM.
    Bcd,
    /// Discrete bits 11–29 as an unsigned integer.
    Discrete,
}

impl Word {
    /// Build from what the receiver delivers. `label_reversed = true` when the
    /// chip has already put the label in natural order (MSB at bit 8).
    pub fn from_raw(raw: u32, label_reversed: bool) -> Word {
        if label_reversed {
            Word((raw & !0xFF) | (raw & 0xFF).reverse_bits() >> 24)
        } else {
            Word(raw)
        }
    }

    /// Label as an octal-readable number, e.g. 0o310.
    pub fn label(self) -> u16 {
        ((self.0 & 0xFF) as u8).reverse_bits() as u16
    }

    pub fn sdi(self) -> u8 {
        ((self.0 >> 8) & 0b11) as u8
    }

    /// Odd parity over all 32 bits.
    pub fn parity_ok(self) -> bool {
        self.0.count_ones() % 2 == 1
    }

    fn ssm_bits(self) -> u8 {
        ((self.0 >> 29) & 0b11) as u8
    }

    pub fn ssm(self, fmt: Format) -> Ssm {
        match (fmt, self.ssm_bits()) {
            (Format::Bnr, 0b11) => Ssm::Normal,
            (Format::Bnr, 0b01) => Ssm::NoComputedData,
            (Format::Bnr, 0b10) => Ssm::FunctionalTest,
            (Format::Bnr, _) => Ssm::FailureWarning,
            (_, 0b00) | (_, 0b11) => Ssm::Normal,
            (_, 0b01) => Ssm::NoComputedData,
            (_, _) => Ssm::FunctionalTest,
        }
    }

    /// BNR value. `range` is the full-scale value (bit 28 = range / 2).
    /// `sig_bits` up to 18 uses bits 11–28; 19–20 extends into bits 9–10
    /// (labels without SDI, e.g. high-resolution latitude/longitude).
    /// Bits below the significant ones (and the SDI) are ignored.
    pub fn bnr(self, range: f64, sig_bits: u8) -> f64 {
        let sig_bits = sig_bits.clamp(1, 20) as u32;
        // bits 9..=29 as a 21-bit two's complement number, sign at bit 29
        let mut v = ((self.0 >> 8) & 0x1F_FFFF) as i32;
        if v & 0x10_0000 != 0 {
            v -= 0x20_0000;
        }
        let pad = 20 - sig_bits;
        let v = (v >> pad) << pad;
        v as f64 * range / (1u32 << 20) as f64
    }

    /// BCD value: 5 digits in bits 11–29 (top digit 3 bits), sign from SSM.
    /// `scale` multiplies the integer result (e.g. 0.1 for one decimal).
    pub fn bcd(self, scale: f64) -> Option<f64> {
        let d = (self.0 >> 10) & 0x7FFFF;
        let digits = [(d >> 16) & 0x7, (d >> 12) & 0xF, (d >> 8) & 0xF, (d >> 4) & 0xF, d & 0xF];
        let mut n = 0u32;
        for x in digits {
            if x > 9 {
                return None;
            }
            n = n * 10 + x;
        }
        let sign = if self.ssm_bits() == 0b11 { -1.0 } else { 1.0 };
        Some(sign * n as f64 * scale)
    }

    // ---- encoding, used by the simulator and tests ---------------------

    pub fn encode_bnr(label: u16, sdi: u8, value: f64, range: f64, sig_bits: u8, ssm: Ssm) -> Word {
        let sig_bits = sig_bits.clamp(1, 20) as u32;
        let lsb = range / (1u32 << sig_bits) as f64;
        let v = ((value / lsb).round() as i64).clamp(-(1i64 << sig_bits), (1i64 << sig_bits) - 1);
        let field21 = ((v << (20 - sig_bits)) as u32) & 0x1F_FFFF;
        let ssm_bits = match ssm {
            Ssm::Normal => 0b11,
            Ssm::NoComputedData => 0b01,
            Ssm::FunctionalTest => 0b10,
            Ssm::FailureWarning => 0b00,
        };
        let sdi = if sig_bits <= 18 { (sdi & 0b11) as u32 } else { 0 };
        Self::with_parity(((label as u8).reverse_bits() as u32) | (field21 << 8) | (sdi << 8) | (ssm_bits << 29))
    }

    fn with_parity(mut w: u32) -> Word {
        w &= 0x7FFF_FFFF;
        if w.count_ones().is_multiple_of(2) {
            w |= 1 << 31;
        }
        Word(w)
    }

    #[cfg(test)]
    fn assemble(label: u16, sdi: u8, field19: u32, ssm_bits: u32) -> Word {
        Self::with_parity(
            ((label as u8).reverse_bits() as u32)
                | (((sdi & 0b11) as u32) << 8)
                | ((field19 & 0x7FFFF) << 10)
                | (ssm_bits << 29),
        )
    }
}

/// One label the installation decodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelDef {
    /// Bus channel on the receiver module (0-based). `None` = any channel.
    #[serde(default)]
    pub channel: Option<u8>,
    /// Label in octal, as written in ICDs: "310".
    pub label: String,
    /// Only accept this SDI. `None` = any.
    #[serde(default)]
    pub sdi: Option<u8>,
    /// Parameter name. Well-known names used by the reference selection:
    /// latitude, longitude, ground_speed, track, pressure_altitude,
    /// altitude_rate. Anything else is just recorded (engine trend etc.).
    pub param: String,
    pub format: Format,
    /// BNR full-scale range; BCD scale factor.
    #[serde(default = "one")]
    pub range: f64,
    #[serde(default = "eighteen")]
    pub sig_bits: u8,
}

fn one() -> f64 {
    1.0
}
fn eighteen() -> u8 {
    18
}

/// Default label set (common GAMA assignments). Verify against the ICD.
pub fn default_labels() -> Vec<LabelDef> {
    let bnr = |label: &str, param: &str, range: f64, sig_bits: u8| LabelDef {
        channel: None,
        label: label.into(),
        sdi: None,
        param: param.into(),
        format: Format::Bnr,
        range,
        sig_bits,
    };
    vec![
        bnr("310", "latitude", 180.0, 20),
        bnr("311", "longitude", 180.0, 20),
        bnr("312", "ground_speed", 4096.0, 15),
        bnr("313", "track", 180.0, 15),
        bnr("203", "pressure_altitude", 131_072.0, 17),
        bnr("212", "altitude_rate", 32_768.0, 11),
        bnr("206", "computed_airspeed", 1024.0, 14),
        bnr("210", "true_airspeed", 2048.0, 15),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamValue {
    pub value: f64,
    pub ssm: Ssm,
    /// Monotonic seconds of last update.
    pub t: f64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct BusStats {
    pub words: u64,
    pub parity_errors: u64,
    pub unknown_labels: u64,
}

/// Decodes words into named parameters and keeps the latest value of each.
#[derive(Debug)]
pub struct Decoder {
    defs: Vec<(u16, LabelDef)>,
    values: BTreeMap<String, ParamValue>,
    pub stats: BusStats,
}

impl Decoder {
    pub fn new(defs: Vec<LabelDef>) -> Result<Self, String> {
        let defs = defs
            .into_iter()
            .map(|d| {
                u16::from_str_radix(&d.label, 8)
                    .ok()
                    .filter(|l| *l <= 0o377)
                    .map(|l| (l, d.clone()))
                    .ok_or_else(|| format!("bad ARINC label '{}'", d.label))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { defs, values: BTreeMap::new(), stats: BusStats::default() })
    }

    pub fn feed(&mut self, channel: u8, w: Word, t: f64) {
        self.stats.words += 1;
        if !w.parity_ok() {
            self.stats.parity_errors += 1;
            return;
        }
        let label = w.label();
        let mut matched = false;
        for (l, d) in &self.defs {
            if *l != label
                || d.channel.is_some_and(|c| c != channel)
                || d.sdi.is_some_and(|s| s != w.sdi())
            {
                continue;
            }
            matched = true;
            let ssm = w.ssm(d.format);
            let value = match d.format {
                Format::Bnr => Some(w.bnr(d.range, d.sig_bits)),
                Format::Bcd => w.bcd(d.range),
                Format::Discrete => Some(((w.0 >> 10) & 0x7FFFF) as f64),
            };
            if let Some(value) = value {
                self.values.insert(d.param.clone(), ParamValue { value, ssm, t });
            }
        }
        if !matched {
            self.stats.unknown_labels += 1;
        }
    }

    /// Latest value if it is valid (SSM normal) and not older than `max_age`.
    pub fn get(&self, param: &str, now: f64, max_age: f64) -> Option<f64> {
        self.values
            .get(param)
            .filter(|v| v.ssm == Ssm::Normal && now - v.t <= max_age)
            .map(|v| v.value)
    }

    pub fn values(&self) -> &BTreeMap<String, ParamValue> {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_is_bit_reversed_octal() {
        // label 0o310 = 0b11_001_000, reversed in bits 1-8 = 0b0001_0011
        let w = Word::encode_bnr(0o310, 0, 0.0, 180.0, 18, Ssm::Normal);
        assert_eq!(w.0 & 0xFF, 0b0001_0011);
        assert_eq!(w.label(), 0o310);
        assert!(w.parity_ok());
    }

    #[test]
    fn label_reversed_input_is_normalised() {
        let w = Word::encode_bnr(0o203, 2, 1000.0, 131_072.0, 17, Ssm::Normal);
        let chip = (w.0 & !0xFF) | 0o203; // chip delivered label in natural order
        let n = Word::from_raw(chip, true);
        assert_eq!(n, w);
        assert_eq!(n.sdi(), 2);
    }

    #[test]
    fn bnr_roundtrip_positive_and_negative() {
        for (v, range, bits) in [
            (47.123, 180.0, 20),
            (-122.5, 180.0, 20),
            (8.31, 180.0, 18),
            (142.0, 4096.0, 15),
            (-1500.0, 32_768.0, 11),
            (9500.0, 131_072.0, 17),
        ] {
            let w = Word::encode_bnr(0o310, 0, v, range, bits, Ssm::Normal);
            let lsb = range / (1u32 << bits) as f64;
            assert!((w.bnr(range, bits) - v).abs() <= lsb / 2.0 + 1e-9, "{v}");
        }
    }

    #[test]
    fn known_bnr_vector() {
        // Bit 28 alone set = half range. Altitude 65536 ft at range 131072.
        let w = Word((1 << 27) | (0b11 << 29));
        assert_eq!(w.bnr(131_072.0, 17), 65_536.0);
        // Sign bit alone = -range.
        let w = Word((1 << 28) | (0b11 << 29));
        assert_eq!(w.bnr(131_072.0, 17), -131_072.0);
    }

    #[test]
    fn ssm_bnr_vs_bcd() {
        let w = Word::encode_bnr(0o312, 0, 10.0, 4096.0, 15, Ssm::NoComputedData);
        assert_eq!(w.ssm(Format::Bnr), Ssm::NoComputedData);
        let fw = Word::encode_bnr(0o312, 0, 10.0, 4096.0, 15, Ssm::FailureWarning);
        assert_eq!(fw.ssm(Format::Bnr), Ssm::FailureWarning);
        assert_eq!(fw.ssm(Format::Bcd), Ssm::Normal, "BCD 00 = plus");
    }

    #[test]
    fn bcd_decoding() {
        // digits 0 1 2 3 4 -> 1234, SSM 11 = minus
        let field = (1 << 12) | (2 << 8) | (3 << 4) | 4;
        let w = Word::assemble(0o100, 0, field, 0b11);
        assert_eq!(w.bcd(0.1), Some(-123.4));
        let bad = Word::assemble(0o100, 0, 0xA, 0b00);
        assert_eq!(bad.bcd(1.0), None);
    }

    #[test]
    fn decoder_filters_parity_ssm_and_age() {
        let mut d = Decoder::new(default_labels()).unwrap();
        let gs = Word::encode_bnr(0o312, 0, 120.0, 4096.0, 15, Ssm::Normal);
        d.feed(0, gs, 10.0);
        assert!((d.get("ground_speed", 10.5, 1.0).unwrap() - 120.0).abs() < 0.2);
        assert_eq!(d.get("ground_speed", 12.0, 1.0), None, "stale");

        let bad = Word(gs.0 ^ 1 << 15);
        d.feed(0, bad, 11.0);
        assert_eq!(d.stats.parity_errors, 1);

        let ncd = Word::encode_bnr(0o312, 0, 0.0, 4096.0, 15, Ssm::NoComputedData);
        d.feed(0, ncd, 13.0);
        assert_eq!(d.get("ground_speed", 13.0, 1.0), None, "NCD is not valid data");

        d.feed(0, Word::encode_bnr(0o377, 0, 0.0, 1.0, 1, Ssm::Normal), 13.0);
        assert_eq!(d.stats.unknown_labels, 1);
    }

    #[test]
    fn channel_and_sdi_filters() {
        let defs = vec![LabelDef {
            channel: Some(1),
            label: "203".into(),
            sdi: Some(1),
            param: "pressure_altitude".into(),
            format: Format::Bnr,
            range: 131_072.0,
            sig_bits: 17,
        }];
        let mut d = Decoder::new(defs).unwrap();
        let w_sdi1 = Word::encode_bnr(0o203, 1, 5000.0, 131_072.0, 17, Ssm::Normal);
        let w_sdi2 = Word::encode_bnr(0o203, 2, 7000.0, 131_072.0, 17, Ssm::Normal);
        d.feed(0, w_sdi1, 0.0);
        assert_eq!(d.get("pressure_altitude", 0.0, 1.0), None, "wrong channel");
        d.feed(1, w_sdi2, 0.0);
        assert_eq!(d.get("pressure_altitude", 0.0, 1.0), None, "wrong SDI");
        d.feed(1, w_sdi1, 0.0);
        assert_eq!(d.get("pressure_altitude", 0.0, 1.0), Some(5000.0));
    }

    #[test]
    fn bad_label_in_config_rejected() {
        let mut l = default_labels();
        l[0].label = "999".into();
        assert!(Decoder::new(l).is_err());
    }
}
