// Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
// Proprietary software, not open source. See LICENSE.

//! Connected Flight Box (CFB) core logic.
//!
//! This crate contains every decision the box takes, written as pure,
//! deterministic, I/O-free functions so they can be verified against the
//! high-level requirements (see `docs/04-software-architecture.md`).
//!
//! Requirement tags (`CFB-HLR-xxx`) in doc comments trace each function to a
//! high-level requirement, as needed for an ED-12C / DO-178C Level D
//! development.
//!
//! The safety argument does **not** rely on this software: RF inhibit and
//! avionics isolation are implemented in hardware (see `docs/03-hardware.md`).
//! This software adds a second, independent layer on top.

#![forbid(unsafe_code)]

pub mod arinc429;
pub mod config;
pub mod datalog;
pub mod edge;
pub mod frame;
pub mod gnss;
pub mod imu;
pub mod link;
pub mod modules;
pub mod ota;
pub mod phase;
pub mod queue;
pub mod reference;
pub mod sbd;
pub mod status;
pub mod update;

pub use config::Config;
pub use frame::{Frame, MsgType, PositionReport};
pub use link::{LinkKind, LinkManager, LinkStatus};
pub use phase::{FlightPhase, PhaseDetector, PhaseInputs};
pub use queue::{Message, Priority, StoreForwardQueue};
pub use update::{UpdateGate, UpdateManifest};
