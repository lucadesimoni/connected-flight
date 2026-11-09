// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Storage (SQLite). One file, WAL mode, schema version in `user_version`
//! so later releases can migrate (docs/12).
//!
//! Deduplication: a frame is identified by (box, seq, frame time, type).
//! A resend after a lost ack has the same identity and is counted, not
//! stored again.

use cfb_core::frame::{Frame, MsgType, EPOCH_2020};
use cfb_core::PositionReport;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

pub const SCHEMA_VERSION: i64 = 1;

pub struct Store {
    db: Connection,
}

/// What the receiver knows about where a frame came from.
#[derive(Debug, Clone)]
pub struct Origin {
    pub box_id: String,
    pub bearer: String,
    pub rx_unix: i64,
}

pub fn describe(f: &Frame) -> String {
    match f.msg_type {
        MsgType::Position => PositionReport::decode(&f.payload)
            .map(|p| {
                format!(
                    "{:.5},{:.5} {}ft {}kt trk {:.0} vs {} air={}",
                    p.lat_deg, p.lon_deg, p.alt_ft, p.gs_kt, p.track_deg, p.vs_fpm, p.airborne
                )
            })
            .unwrap_or_else(|| "bad position".into()),
        // File chunks: size only (reassembly is a later feature).
        MsgType::FileChunk => format!("{} bytes", f.payload.len()),
        _ => String::from_utf8_lossy(&f.payload).replace(['\t', '\n', '\r'], " "),
    }
}

impl Store {
    pub fn open(path: &str) -> rusqlite::Result<Self> {
        let db = Connection::open(path)?;
        if path != ":memory:" {
            db.pragma_update(None, "journal_mode", "WAL")?;
        }
        db.pragma_update(None, "synchronous", "NORMAL")?;
        let v: i64 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v > SCHEMA_VERSION {
            return Err(rusqlite::Error::InvalidParameterName(format!(
                "database schema {v} is newer than this cfb-ground ({SCHEMA_VERSION})"
            )));
        }
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS frames(
                id INTEGER PRIMARY KEY,
                box TEXT NOT NULL, bearer TEXT NOT NULL,
                rx_unix INTEGER NOT NULL, frame_unix INTEGER NOT NULL,
                seq INTEGER NOT NULL, msg_type TEXT NOT NULL, body TEXT NOT NULL,
                UNIQUE(box, seq, frame_unix, msg_type));
             CREATE TABLE IF NOT EXISTS positions(
                frame_id INTEGER PRIMARY KEY REFERENCES frames(id),
                box TEXT NOT NULL, frame_unix INTEGER NOT NULL,
                lat REAL NOT NULL, lon REAL NOT NULL, alt_ft INTEGER, gs_kt INTEGER,
                track REAL, vs_fpm INTEGER, airborne INTEGER);
             CREATE TABLE IF NOT EXISTS duplicates(box TEXT PRIMARY KEY, n INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS idx_frames_box_time ON frames(box, frame_unix);
             CREATE INDEX IF NOT EXISTS idx_pos_box_time ON positions(box, frame_unix);",
        )?;
        db.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(Self { db })
    }

    /// Store a frame. Returns true if new, false if it was a duplicate.
    pub fn insert(&self, o: &Origin, f: &Frame) -> rusqlite::Result<bool> {
        let frame_unix = f.time as i64 + EPOCH_2020 as i64;
        let n = self.db.execute(
            "INSERT OR IGNORE INTO frames(box, bearer, rx_unix, frame_unix, seq, msg_type, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![o.box_id, o.bearer, o.rx_unix, frame_unix, f.seq, format!("{:?}", f.msg_type), describe(f)],
        )?;
        if n == 0 {
            self.db.execute(
                "INSERT INTO duplicates(box, n) VALUES (?1, 1) ON CONFLICT(box) DO UPDATE SET n = n + 1",
                params![o.box_id],
            )?;
            return Ok(false);
        }
        if f.msg_type == MsgType::Position {
            if let Some(p) = PositionReport::decode(&f.payload) {
                self.db.execute(
                    "INSERT INTO positions(frame_id, box, frame_unix, lat, lon, alt_ft, gs_kt, track, vs_fpm, airborne)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        self.db.last_insert_rowid(),
                        o.box_id,
                        frame_unix,
                        p.lat_deg,
                        p.lon_deg,
                        p.alt_ft,
                        p.gs_kt,
                        p.track_deg as f64,
                        p.vs_fpm,
                        p.airborne
                    ],
                )?;
            }
        }
        Ok(true)
    }

    /// One row per box: last contact, counts, last position and last health.
    pub fn boxes(&self) -> rusqlite::Result<Vec<Value>> {
        let mut st = self.db.prepare(
            "SELECT box, MAX(rx_unix), COUNT(*), GROUP_CONCAT(DISTINCT bearer) FROM frames GROUP BY box ORDER BY box",
        )?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (b, last_rx, n, bearers) = row?;
            let pos = self
                .db
                .query_row(
                    "SELECT lat, lon, alt_ft, gs_kt, airborne, frame_unix FROM positions WHERE box=?1 ORDER BY frame_unix DESC, frame_id DESC LIMIT 1",
                    params![b],
                    |r| {
                        Ok(json!({"lat": r.get::<_, f64>(0)?, "lon": r.get::<_, f64>(1)?, "alt_ft": r.get::<_, i64>(2)?,
                                  "gs_kt": r.get::<_, i64>(3)?, "airborne": r.get::<_, bool>(4)?, "time": r.get::<_, i64>(5)?}))
                    },
                )
                .optional()?;
            let health = self
                .db
                .query_row(
                    "SELECT body FROM frames WHERE box=?1 AND msg_type='Health' ORDER BY frame_unix DESC, id DESC LIMIT 1",
                    params![b],
                    |r| r.get::<_, String>(0),
                )
                .optional()?;
            let dups: i64 = self
                .db
                .query_row("SELECT n FROM duplicates WHERE box=?1", params![b], |r| r.get(0))
                .optional()?
                .unwrap_or(0);
            out.push(json!({"box": b, "last_rx": last_rx, "frames": n, "duplicates": dups, "bearers": bearers,
                            "last_position": pos, "last_health": health}));
        }
        Ok(out)
    }

    /// Positions of a box, oldest first, newest `limit`.
    pub fn track(&self, box_id: &str, limit: i64) -> rusqlite::Result<Vec<Value>> {
        let mut st = self.db.prepare(
            "SELECT lat, lon, alt_ft, gs_kt, airborne, frame_unix FROM
               (SELECT * FROM positions WHERE box=?1 ORDER BY frame_unix DESC, frame_id DESC LIMIT ?2)
             ORDER BY frame_unix, frame_id",
        )?;
        let rows = st.query_map(params![box_id, limit], |r| {
            Ok(json!({"lat": r.get::<_, f64>(0)?, "lon": r.get::<_, f64>(1)?, "alt_ft": r.get::<_, i64>(2)?,
                      "gs_kt": r.get::<_, i64>(3)?, "airborne": r.get::<_, bool>(4)?, "time": r.get::<_, i64>(5)?}))
        })?;
        rows.collect()
    }

    /// Non-position frames of a box, newest first.
    pub fn events(&self, box_id: &str, limit: i64) -> rusqlite::Result<Vec<Value>> {
        let mut st = self.db.prepare(
            "SELECT frame_unix, rx_unix, bearer, seq, msg_type, body FROM frames
             WHERE box=?1 AND msg_type != 'Position' ORDER BY frame_unix DESC, id DESC LIMIT ?2",
        )?;
        let rows = st.query_map(params![box_id, limit], |r| {
            Ok(json!({"time": r.get::<_, i64>(0)?, "rx": r.get::<_, i64>(1)?, "bearer": r.get::<_, String>(2)?,
                      "seq": r.get::<_, i64>(3)?, "type": r.get::<_, String>(4)?, "body": r.get::<_, String>(5)?}))
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos_frame(seq: u16, time: u32, lat: f64) -> Frame {
        let p = PositionReport { lat_deg: lat, lon_deg: 8.3, alt_ft: 5000, gs_kt: 120, track_deg: 90.0, vs_fpm: 0, airborne: true, emergency: false };
        Frame { msg_type: MsgType::Position, seq, time, payload: p.encode().to_vec() }
    }

    fn o(b: &str) -> Origin {
        Origin { box_id: b.into(), bearer: "cellular".into(), rx_unix: 1_800_000_000 }
    }

    #[test]
    fn dedup_and_queries() {
        let s = Store::open(":memory:").unwrap();
        assert!(s.insert(&o("box-2"), &pos_frame(1, 100, 47.0)).unwrap());
        assert!(!s.insert(&o("box-2"), &pos_frame(1, 100, 47.0)).unwrap(), "resend = duplicate");
        assert!(s.insert(&o("box-2"), &pos_frame(2, 130, 47.1)).unwrap());
        assert!(s.insert(&o("box-3"), &pos_frame(1, 100, 46.0)).unwrap(), "same seq, other box");
        let h = Frame { msg_type: MsgType::Health, seq: 3, time: 140, payload: b"RF switch OFF".to_vec() };
        assert!(s.insert(&o("box-2"), &h).unwrap());

        let boxes = s.boxes().unwrap();
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0]["box"], "box-2");
        assert_eq!(boxes[0]["frames"], 3);
        assert_eq!(boxes[0]["duplicates"], 1);
        assert!((boxes[0]["last_position"]["lat"].as_f64().unwrap() - 47.1).abs() < 1e-6);
        assert_eq!(boxes[0]["last_health"], "RF switch OFF");

        let t = s.track("box-2", 10).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t[0]["time"].as_i64() < t[1]["time"].as_i64(), "oldest first");
        assert_eq!(s.track("box-2", 1).unwrap().len(), 1);
        let e = s.events("box-2", 10).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0]["type"], "Health");
    }

    #[test]
    fn schema_version_set_and_reopen() {
        let dir = std::env::temp_dir().join(format!("cfb-store-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.db");
        let ps = p.to_str().unwrap();
        {
            let s = Store::open(ps).unwrap();
            s.insert(&o("box-2"), &pos_frame(1, 1, 47.0)).unwrap();
        }
        let s = Store::open(ps).unwrap();
        assert_eq!(s.track("box-2", 10).unwrap().len(), 1, "data survives reopen");
        let v: i64 = s.db.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
