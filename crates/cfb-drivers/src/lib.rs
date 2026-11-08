// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Connected Flight Box hardware I/O drivers.
//!
//! Layering (keep it this way for long-term maintenance):
//!
//! * `cfb-core`: decisions only, no I/O. This is the certified logic.
//! * `cfb-drivers` (this crate): talks to hardware and the network. Each
//!   driver sits behind a small trait or a plain struct, so a new modem,
//!   sensor or bearer is a new module here, not a change to the core.
//! * `cfbd` / `cfb-io`: wiring and configuration only.
//!
//! Extension points:
//!
//! | Need | Implement | Register in |
//! |---|---|---|
//! | New bearer (Certus IP, Starlink API, …) | [`link::LinkDriver`] | `cfbd` link set-up |
//! | New sensor | a reader yielding `cfb-io` line tokens | `cfb-io` |
//! | New output (LED, display, buzzer) | [`led::Indicator`] | `cfbd` status output |

#![forbid(unsafe_code)]

pub mod discretes;
pub mod iio;
pub mod led;
pub mod link;
pub mod link_sbd;
pub mod link_udp;
pub mod nmea_serial;
pub mod serial;
