# 12 – Architecture, Extension and Long-Term Support

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

The CFB has to live for 15–25 years on aircraft, across hardware revisions,
modem generations and new links. These rules keep it maintainable. Every
change is checked against them.

## 1. Layers

```
┌────────────────────────────────────────────────────────────────┐
│ cfbd, cfb-io, tools          wiring + configuration only       │  crates/cfbd
├────────────────────────────────────────────────────────────────┤
│ cfb-drivers                  hardware + network I/O            │  crates/cfb-drivers
│   link (trait) ─ link_udp ─ link_sbd                           │
│   iio ─ nmea_serial ─ discretes ─ led (trait) ─ serial         │
├────────────────────────────────────────────────────────────────┤
│ cfb-core                     decisions, no I/O (certified)     │  crates/cfb-core
│   phase ─ link policy ─ queue ─ frame ─ reference ─ arinc429   │
│   gnss ─ imu ─ edge ─ ota ─ update ─ status ─ sbd ─ config     │
└────────────────────────────────────────────────────────────────┘
```

| Rule | Why |
|---|---|
| `cfb-core` does no I/O (the queue snapshot is the only file access) and has `#![forbid(unsafe_code)]` | It is the DAL D software; pure functions can be verified by HLR tests |
| `cfb-drivers` does I/O but makes no flight decisions | Hardware churn (new modem, new sensor) never touches certified logic |
| `cfbd` contains no protocol or decision logic | It stays a readable wiring diagram |
| Safety functions are in **hardware** first, software second | The software can then change under a minor-change process |
| Every hardware module is optional and declared in `[modules]` | One software for every build (portable, ground, PC-12, PC-24) |

## 2. Extension points

| To add… | Implement | Where | Register |
|---|---|---|---|
| A **new bearer** (Iridium Certus IP, Starlink status API, Inmarsat, VDL…) | `cfb_drivers::link::LinkDriver` | new `cfb-drivers/src/link_<name>.rs` | link set-up block in `cfbd/main.rs` + a config section |
| A **new sensor** | a reader yielding a `cfb-io` line token | `cfb-drivers` (driver) + `cfb-io` (token) | token parse in `cfbd/input.rs`; consumer in core |
| A **new aircraft data source** (other ARINC labels, CAN, serial engine monitor) | decoder in `cfb-core` (pure) | `cfb-core/src/<source>.rs` | `reference::select` or the edge `DataView` |
| A **new output** (display, buzzer) | `cfb_drivers::led::Indicator` or a similar small trait | `cfb-drivers` | status block in `cfbd` |
| A **new edge rule type** | rule struct + state in `cfb-core::edge::RuleApp` | `cfb-core/src/edge.rs` | package format (`AppPackage`); HLR + tests |
| A **new message type** | `MsgType` value (4-bit field, 9–15 free) | `cfb-core/src/frame.rs` | ground decoder (`cfb-ground`) |
| A **new hardware module** | ID in `modules.rs` `expected_ids`, config field | `cfb-core/src/modules.rs` | ID EEPROM on the module |

The `LinkDriver` contract is the most important one:
* `send` never blocks;
* delivery counts only on `Rx::Acked`;
* a driver must tolerate the same message id twice;
* `status().up` means "a send now has a real chance".

The UDP and SBD drivers show both styles: inline sockets, and a worker
thread with a channel.

## 3. Compatibility rules

| Interface | Rule |
|---|---|
| **Air–ground frame** (`frame.rs`) | Version nibble = 1. Additive changes only (new message types, new trailing payload fields readers can ignore). An incompatible change → version 2, and the ground accepts both for the fleet's lifetime |
| **SBD batch format** | `[len u8][frame]…`; never change, add a new prefix byte if ever needed |
| **Config (`cfb.toml`)** | New fields need defaults (`#[serde(default)]`); never rename, deprecate first. `Config::validate` rejects unsafe combinations, so a bad file can't be loaded |
| **`cfb-io` line protocol** | New tokens only; cfbd ignores unknown tokens; `-` = unknown |
| **Edge app packages** | Fields added with defaults; package `version` + ASCI entry per release |
| **Queue snapshot** | Magic `CFQ1`; a new layout → `CFQ2`, and the old one is still read (or discarded safely: empty queue on mismatch) |
| **Toolchain** | Pinned in `rust-toolchain.toml`; upgrades are deliberate, reviewed commits (CI catches new lints) |

## 4. Quality gates (every change)

1. `cargo clippy --all-targets -- -D warnings`, `cargo test` and the
   simulations: CI on every push, must be green before merging to `main`.
2. Each new behaviour gets an HLR line in `docs/04` and a test named in it.
3. Hardware-facing code gets a fake (fake sysfs, fake modem, localhost
   sockets) so it is testable without the hardware.
4. Docs updated in the same commit (config examples parse in tests, so
   they can't drift).
5. Locally: `scripts/check.sh` runs the same gates as CI before a commit.
6. Evidence per push (CI job `evidence`, downloadable artifact):
   `scripts/evidence.py` builds the trace matrix from the HLR table in
   docs/04 and the test run (**fails** on a missing or failed test), and
   `scripts/sci.py` writes the Software Configuration Index (commit,
   toolchain, dependency checksums, binary and config hashes).

## 5. Release and support model (proposal)

| Item | Practice |
|---|---|
| Versions | SemVer for the software part number (`CFB-SW-xxxx`); hardware part numbers per board revision |
| Branches | `main` = development; `release/x.y` per certified baseline, only fixes, each fix also on `main` |
| Long-term support | Each certified baseline supported for the life of its installations, with security fixes for the OS layer (Yocto LTS / Debian LTS) and the ground service |
| Configuration control | Software Configuration Index per release (toolchain, dependencies with `Cargo.lock`, OS packages, hashes); ASCI lists what may be loaded |
| Dependencies | Few, permissive licences, `Cargo.lock` committed; review on upgrade; `cargo about` licence list per release |
| Obsolescence | Hardware: second sources, module design (swap the cellular module, keep the base); software: drivers isolate part changes |
| Field data | Health messages (link stats, faults, OTA states) from every unit feed reliability tracking |
