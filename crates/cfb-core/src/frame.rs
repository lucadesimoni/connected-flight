// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Compact binary framing for paid-per-byte links (satcom).
//!
//! A position report as JSON is ~150 bytes; as a frame here it is 30 bytes.
//! On Iridium Certus / SBD, where you pay per byte, that is what keeps
//! in-flight tracking cheap.
//!
//! Frame layout (little endian):
//!
//! ```text
//! 0      1            2..4    4..8        8..        ..        last 4
//! 0xCF | ver<<4|type | seq u16 | time u32 | len LEB128 | payload | CRC-32C
//! ```
//!
//! `time` is seconds since 2020-01-01T00:00:00Z (fits until 2156).

/// Seconds between the Unix epoch and 2020-01-01.
pub const EPOCH_2020: u64 = 1_577_836_800;
pub const MAGIC: u8 = 0xCF;
pub const VERSION: u8 = 1;
/// Upper bound on payload size, so a corrupt length can't make us allocate.
pub const MAX_PAYLOAD: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MsgType {
    Position = 1,
    Oooi = 2,
    EngineTrend = 3,
    Text = 4,
    Ack = 5,
    Health = 6,
    /// Chunk of a large file (flight data, logs). Ground links only.
    FileChunk = 7,
    /// Output of an edge app (exceedance, trend, summary).
    Edge = 8,
}

impl MsgType {
    fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            1 => Self::Position,
            2 => Self::Oooi,
            3 => Self::EngineTrend,
            4 => Self::Text,
            5 => Self::Ack,
            6 => Self::Health,
            7 => Self::FileChunk,
            8 => Self::Edge,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub msg_type: MsgType,
    pub seq: u16,
    /// Seconds since 2020-01-01Z.
    pub time: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    TooShort,
    BadMagic,
    BadVersion(u8),
    BadType(u8),
    BadLength,
    BadCrc,
}

impl Frame {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(8 + 3 + self.payload.len() + 4);
        out.push(MAGIC);
        out.push((VERSION << 4) | (self.msg_type as u8 & 0x0F));
        out.extend_from_slice(&self.seq.to_le_bytes());
        out.extend_from_slice(&self.time.to_le_bytes());
        write_leb128(&mut out, self.payload.len() as u64);
        out.extend_from_slice(&self.payload);
        let crc = crc32c(&out);
        out.extend_from_slice(&crc.to_le_bytes());
        out
    }

    pub fn decode(buf: &[u8]) -> Result<Frame, FrameError> {
        if buf.len() < 13 {
            return Err(FrameError::TooShort);
        }
        if buf[0] != MAGIC {
            return Err(FrameError::BadMagic);
        }
        let (body, crc) = buf.split_at(buf.len() - 4);
        if crc32c(body) != u32::from_le_bytes(crc.try_into().unwrap()) {
            return Err(FrameError::BadCrc);
        }
        let ver = body[1] >> 4;
        if ver != VERSION {
            return Err(FrameError::BadVersion(ver));
        }
        let t = body[1] & 0x0F;
        let msg_type = MsgType::from_u8(t).ok_or(FrameError::BadType(t))?;
        let seq = u16::from_le_bytes([body[2], body[3]]);
        let time = u32::from_le_bytes(body[4..8].try_into().unwrap());
        let (len, n) = read_leb128(&body[8..]).ok_or(FrameError::BadLength)?;
        let start = 8 + n;
        if len as usize > MAX_PAYLOAD || start + len as usize != body.len() {
            return Err(FrameError::BadLength);
        }
        Ok(Frame { msg_type, seq, time, payload: body[start..].to_vec() })
    }
}

/// Position report, 17 bytes on the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionReport {
    pub lat_deg: f64,
    pub lon_deg: f64,
    /// Pressure or GNSS altitude in feet. Encoded with 10 ft resolution, -1000..654 340 ft.
    pub alt_ft: i32,
    pub gs_kt: u16,
    /// True track, degrees. Encoded with 0.01 deg resolution.
    pub track_deg: f32,
    /// Vertical speed in ft/min. Encoded with 10 fpm resolution.
    pub vs_fpm: i32,
    pub airborne: bool,
    pub emergency: bool,
}

impl PositionReport {
    pub const LEN: usize = 17;

    pub fn encode(&self) -> [u8; Self::LEN] {
        let mut b = [0u8; Self::LEN];
        let lat = (self.lat_deg * 1e7).round() as i32;
        let lon = (self.lon_deg * 1e7).round() as i32;
        let alt = ((self.alt_ft.clamp(-1000, 654_340) + 1000) / 10) as u16;
        let trk = ((self.track_deg.rem_euclid(360.0) * 100.0).round() as u32 % 36000) as u16;
        let vs = (self.vs_fpm / 10).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let flags = (self.airborne as u8) | ((self.emergency as u8) << 1);
        b[0..4].copy_from_slice(&lat.to_le_bytes());
        b[4..8].copy_from_slice(&lon.to_le_bytes());
        b[8..10].copy_from_slice(&alt.to_le_bytes());
        b[10..12].copy_from_slice(&self.gs_kt.to_le_bytes());
        b[12..14].copy_from_slice(&trk.to_le_bytes());
        b[14..16].copy_from_slice(&vs.to_le_bytes());
        b[16] = flags;
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != Self::LEN {
            return None;
        }
        let i32_at = |i: usize| i32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        Some(Self {
            lat_deg: i32_at(0) as f64 / 1e7,
            lon_deg: i32_at(4) as f64 / 1e7,
            alt_ft: u16_at(8) as i32 * 10 - 1000,
            gs_kt: u16_at(10),
            track_deg: u16_at(12) as f32 / 100.0,
            vs_fpm: i16::from_le_bytes([b[14], b[15]]) as i32 * 10,
            airborne: b[16] & 1 != 0,
            emergency: b[16] & 2 != 0,
        })
    }
}

/// Ack payload: the acknowledged frame sequence numbers, u16 LE each.
pub fn encode_ack(seqs: &[u16]) -> Vec<u8> {
    seqs.iter().flat_map(|s| s.to_le_bytes()).collect()
}

pub fn decode_ack(payload: &[u8]) -> Vec<u16> {
    payload.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

/// Sequence number of an encoded frame without a full decode.
pub fn frame_seq(bytes: &[u8]) -> Option<u16> {
    (bytes.len() >= 4 && bytes[0] == MAGIC).then(|| u16::from_le_bytes([bytes[2], bytes[3]]))
}

fn write_leb128(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn read_leb128(b: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, &byte) in b.iter().enumerate().take(10) {
        v |= ((byte & 0x7F) as u64) << (7 * i);
        if byte & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

/// CRC-32C (Castagnoli), reflected, as used by iSCSI/ext4. Table-driven,
/// table built at compile time.
pub fn crc32c(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 { 0x82F6_3B78 ^ (c >> 1) } else { c >> 1 };
                k += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    };
    let mut crc = !0u32;
    for &b in data {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_known_vector() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn frame_roundtrip() {
        let f = Frame { msg_type: MsgType::Text, seq: 513, time: 123_456, payload: b"OPS NORMAL".to_vec() };
        let enc = f.encode();
        assert_eq!(Frame::decode(&enc).unwrap(), f);
    }

    #[test]
    fn frame_rejects_corruption() {
        let f = Frame { msg_type: MsgType::Ack, seq: 1, time: 1, payload: vec![1, 2, 3] };
        let mut enc = f.encode();
        enc[9] ^= 1;
        assert_eq!(Frame::decode(&enc), Err(FrameError::BadCrc));
        assert_eq!(Frame::decode(&enc[..5]), Err(FrameError::TooShort));
    }

    #[test]
    fn ack_payload_roundtrip() {
        let p = encode_ack(&[1, 65535, 300]);
        assert_eq!(decode_ack(&p), vec![1, 65535, 300]);
        let f = Frame { msg_type: MsgType::Edge, seq: 777, time: 0, payload: vec![] };
        assert_eq!(frame_seq(&f.encode()), Some(777));
    }

    #[test]
    fn long_payload_uses_multibyte_length() {
        let f = Frame { msg_type: MsgType::FileChunk, seq: 9, time: 9, payload: vec![0xAA; 1000] };
        assert_eq!(Frame::decode(&f.encode()).unwrap(), f);
    }

    #[test]
    fn position_report_is_small_and_roundtrips() {
        let p = PositionReport {
            lat_deg: 47.000_123_4,
            lon_deg: 8.307_654_3,
            alt_ft: 9500,
            gs_kt: 142,
            track_deg: 271.37,
            vs_fpm: -500,
            airborne: true,
            emergency: false,
        };
        let f = Frame { msg_type: MsgType::Position, seq: 1, time: 1, payload: p.encode().to_vec() };
        let wire = f.encode();
        assert_eq!(wire.len(), 30);
        let back = PositionReport::decode(&Frame::decode(&wire).unwrap().payload).unwrap();
        assert!((back.lat_deg - p.lat_deg).abs() < 1e-7);
        assert!((back.lon_deg - p.lon_deg).abs() < 1e-7);
        assert_eq!(back.alt_ft, 9500);
        assert_eq!(back.gs_kt, 142);
        assert!((back.track_deg - 271.37).abs() < 0.01);
        assert_eq!(back.vs_fpm, -500);
        assert!(back.airborne && !back.emergency);
    }

    #[test]
    fn negative_coordinates_and_wraparound_track() {
        let p = PositionReport {
            lat_deg: -33.9,
            lon_deg: -70.7,
            alt_ft: -500,
            gs_kt: 0,
            track_deg: 360.0,
            vs_fpm: 0,
            airborne: false,
            emergency: true,
        };
        let b = PositionReport::decode(&p.encode()).unwrap();
        assert!((b.lat_deg + 33.9).abs() < 1e-7);
        assert_eq!(b.track_deg, 0.0);
        assert_eq!(b.alt_ft, -500);
        assert!(b.emergency);
    }
}
