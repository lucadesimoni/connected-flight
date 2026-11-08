// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Iridium Short Burst Data (SBD): the protocol rules, without I/O.
//!
//! An SBD mobile-originated (MO) message carries at most 340 bytes on
//! 9602/9603 transceivers and costs per message (and per 50-byte credit on
//! common plans). To use that well, several CFB frames are packed into one
//! message:
//!
//! ```text
//! [len u8][frame][len u8][frame]...      (each frame <= 255 bytes)
//! ```
//!
//! The ground side gets the message from the Iridium gateway / service
//! provider (e.g. as hex in an HTTP POST) and splits it with [`unpack`].
//! The I/O part (AT commands on the serial port) is in
//! `cfb-drivers::link_sbd`.

pub const MAX_MO: usize = 340;

/// SBD checksum: the low 16 bits of the byte sum, sent big-endian after the
/// payload of `AT+SBDWB`.
pub fn checksum(data: &[u8]) -> [u8; 2] {
    let sum = data.iter().fold(0u32, |a, b| a + *b as u32) as u16;
    sum.to_be_bytes()
}

/// Pack frames (with their message ids) into one MO payload, up to `max`
/// bytes. Returns the payload and the ids it contains. Frames longer than
/// 255 bytes can't be packed and are skipped (the caller must not route
/// them to SBD).
pub fn pack<'a>(frames: impl IntoIterator<Item = (u64, &'a [u8])>, max: usize) -> (Vec<u8>, Vec<u64>) {
    let mut out = Vec::new();
    let mut ids = Vec::new();
    for (id, f) in frames {
        if f.is_empty() || f.len() > 255 {
            continue;
        }
        if out.len() + 1 + f.len() > max {
            break;
        }
        out.push(f.len() as u8);
        out.extend_from_slice(f);
        ids.push(id);
    }
    (out, ids)
}

/// Split an MO payload back into frames (ground side).
pub fn unpack(payload: &[u8]) -> Result<Vec<&[u8]>, String> {
    let mut v = Vec::new();
    let mut p = 0;
    while p < payload.len() {
        let n = payload[p] as usize;
        if n == 0 || p + 1 + n > payload.len() {
            return Err(format!("bad length {n} at offset {p}"));
        }
        v.push(&payload[p + 1..p + 1 + n]);
        p += 1 + n;
    }
    Ok(v)
}

/// Result of `AT+SBDIX`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sbdix {
    pub mo_status: u8,
    pub momsn: u32,
    pub mt_status: u8,
    pub mtmsn: u32,
    pub mt_length: u32,
    pub mt_queued: u32,
}

impl Sbdix {
    /// MO status 0–4: the message reached the gateway.
    pub fn mo_ok(&self) -> bool {
        self.mo_status <= 4
    }
}

/// Parse `+SBDIX: <MO status>, <MOMSN>, <MT status>, <MTMSN>, <MT length>, <MT queued>`.
pub fn parse_sbdix(line: &str) -> Option<Sbdix> {
    let rest = line.trim().strip_prefix("+SBDIX:")?;
    let v: Vec<u32> = rest.split(',').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    if v.len() != 6 {
        return None;
    }
    Some(Sbdix {
        mo_status: u8::try_from(v[0]).ok()?,
        momsn: v[1],
        mt_status: u8::try_from(v[2]).ok()?,
        mtmsn: v[3],
        mt_length: v[4],
        mt_queued: v[5],
    })
}

/// Parse `+CSQ:<0..5>` (signal bars).
pub fn parse_csq(line: &str) -> Option<u8> {
    let n: u8 = line.trim().strip_prefix("+CSQ:")?.trim().parse().ok()?;
    (n <= 5).then_some(n)
}

/// Hex decode for ground tools (gateway deliveries are usually hex).
pub fn from_hex(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim();
    if !s.len().is_multiple_of(2) {
        return Err("odd length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_is_low_16_bits_big_endian() {
        assert_eq!(checksum(b"hello"), [0x02, 0x14]); // 104+101+108+108+111 = 532
        assert_eq!(checksum(&[0xFF; 300]), [0x2A, 0xD4]); // 300 * 255 = 76500 = 0x1_2AD4
    }

    #[test]
    fn pack_respects_limit_and_roundtrips() {
        let a = [1u8; 30];
        let b = [2u8; 200];
        let c = [3u8; 200];
        let big = [4u8; 300];
        let (p, ids) = pack([(1, &a[..]), (2, &big[..]), (3, &b[..]), (4, &c[..])], MAX_MO);
        assert_eq!(ids, vec![1, 3], "too-big frame skipped, last one doesn't fit");
        assert_eq!(p.len(), 1 + 30 + 1 + 200);
        assert!(p.len() <= MAX_MO);
        let f = unpack(&p).unwrap();
        assert_eq!(f, vec![&a[..], &b[..]]);
    }

    #[test]
    fn unpack_rejects_truncation() {
        assert!(unpack(&[5, 1, 2]).is_err());
        assert!(unpack(&[0]).is_err());
        assert_eq!(unpack(&[]).unwrap().len(), 0);
    }

    #[test]
    fn parse_modem_responses() {
        let r = parse_sbdix("+SBDIX: 0, 12, 0, 0, 0, 0").unwrap();
        assert!(r.mo_ok());
        assert_eq!(r.momsn, 12);
        let r = parse_sbdix("+SBDIX: 32, 13, 2, 0, 0, 0").unwrap();
        assert!(!r.mo_ok(), "32 = no network service");
        assert_eq!(parse_sbdix("+SBDIX: 1, 2"), None);
        assert_eq!(parse_csq("+CSQ:4"), Some(4));
        assert_eq!(parse_csq("+CSQ:9"), None);
        assert_eq!(from_hex("cf01ff").unwrap(), vec![0xcf, 0x01, 0xff]);
        assert!(from_hex("abc").is_err());
    }
}
