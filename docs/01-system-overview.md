# 01 – System Overview

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

## Purpose

The Connected Flight Box (CFB) is a small, low-power connectivity unit for
CS-23 aeroplanes (Level 1–4, piston and turboprop). It:

* sends **position reports and operational messages in flight** over
  satellite (Iridium Certus; LEO broadband where an approved terminal is
  installed);
* uploads **flight, engine and maintenance data on ground** over LTE/5G or
  Wi-Fi, automatically after landing;
* provides a **cabin Wi-Fi access point** for EFBs (weather, NOTAMs, text
  messaging), optional;
* receives **field-loadable software updates**, installed only when parked,
  engine off, in maintenance mode.

The hardware is **modular**: a base unit (compute, power, discretes, Wi-Fi)
plus plug-in modules for the reference sensors (own GNSS + accelerometer),
an ARINC 429 receiver, 4G or 5G cellular, and optional satcom.

It is **non-essential equipment**. No aircraft function depends on it, and it
cannot send data to any avionics.

## Context

```
                    ┌──────────── Satellite (Iridium / LEO) ───────────┐
                    │                                                   │
   ┌────────────────┴───┐  read-only, isolated   ┌─────────────────┐    │
   │  Avionics / EFIS   │──ARINC 429 (RX only)─▶ │  [ARINC module] │    │
   │  Engine monitor    │──RS-232 / CAN (listen)▶│      CFB        │──[satcom module, option]
   │  GNSS antenna(own) │──────────────────────▶ │ [ref: GNSS+IMU] │    │
   │  WoW / squat switch│──opto discrete───────▶ │                 │──LTE/5G (ground only)
   │  Oil-press switch  │──opto discrete───────▶ │                 │──Wi-Fi client (ground only)
   └────────────────────┘                        │                 │──Cabin Wi-Fi AP (EFB)
   Non-essential bus 14/28 V ──CB──────────────▶ │                 │──Maintenance port (keyed)
                                                 └─────────────────┘
                                                          │
                                         Ground service (operator / CAMO)
```

## Design principles

1. **Safety by hardware, features by software.** The things that matter for
   airworthiness (no output to avionics, cellular transmitter off in
   flight, electrical protection) are enforced in hardware. The software
   repeats them as a second layer but the safety argument does not depend on
   it. This keeps the software's assurance level low and updates cheap.
2. **Fail restrictive.** Missing or conflicting inputs mean "airborne":
   ground-only radios off, no updates.
3. **Pay per byte only when it matters.** In flight, only high-priority data
   goes over satcom, in a 30-byte binary frame, inside a per-flight byte
   budget. Everything else waits for a free ground link.
4. **One software part number for all CS-23 types.** Aircraft-specific values
   are in the signed installed configuration (`config/cfb.toml`), not in code.
5. **Small footprint.** Single 600 KB static Rust binary, < 64 MB RAM,
   ~3–6 W typical.

## Main functions

| Function | Where | Notes |
|---|---|---|
| ARINC 429 decoding | `cfb-core::arinc429` | Receive only, configurable label table, parity/SSM/age checks |
| Own GNSS | `cfb-core::gnss` | NMEA RMC/GGA, backup position, time source |
| Own accelerometer | `cfb-core::imu` | Engine vibration, accel, touchdown/turbulence Nz advisories |
| Reference selection | `cfb-core::reference` | Aircraft source first, own sensors as fallback and cross-check |
| Module management | `cfb-core::modules` | Configured vs detected modules; links only if fitted |
| Flight phase detection | `cfb-core::phase` | WoW primary, GNSS fallback, debounced, fail-to-airborne |
| Radio enable policy | `cfb-core::link` | Software layer; hardware interlock is primary |
| Link selection / budgets | `cfb-core::link` | Cheapest permitted link, per-flight satcom cap |
| Store-and-forward | `cfb-core::queue` | 4 priorities, byte-bounded, power-cut-safe snapshot |
| Compact framing | `cfb-core::frame` | 30-byte position report incl. CRC-32C |
| Update gate | `cfb-core::update` | Ed25519 + signed ASCI + aircraft state |
| A/B update + rollback | RAUC, `os/` | Boot counter, mark-good after health check |

## What the box is **not**

* Not a flight tracker under ICAO GADSS / Annex 6 autonomous distress
  tracking (it could be developed further for that, but that is a different
  certification basis).
* Not an ELT, not ADS-B Out, not a source of navigation data.
* Not a passenger connectivity system (a cabin AP for crew EFBs only).
