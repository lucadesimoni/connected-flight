# Connected Flight Box (CFB)

A low-power connectivity unit for **EASA CS-23 aeroplanes** (Level 1–4), built
on the **Raspberry Pi 5 / Compute Module 5**. It sends data **in flight over
satcom** and **on ground over LTE/5G or Wi-Fi**. It is designed to be
installed as a **minor change** with **DAL D field-loadable software**.

* **Safety by hardware:** receive-only, isolated avionics interfaces, and a
  WoW-driven hardware interlock that holds the cellular modem off in flight.
  Because of that, software updates stay low-risk and cheap to approve.
* **Low data cost:** a 30-byte binary position report (JSON would be ~150 B),
  priority queues, and a per-flight satcom byte budget. Bulk data waits for a
  free ground link and uploads automatically after landing.
* **Small:** one ~600 KB Rust binary, no async runtime, <64 MB memory cap,
  ~4–6 W for the whole box.
* **Safe updates:** RAUC A/B slots, Ed25519-signed manifest and an Approved
  Software Configuration Index (ASCI). Installs only when parked, engine
  off, and in maintenance mode, with automatic rollback.
* **One software for all CS-23 types:** the aircraft-specific values are in a
  signed config file.

> **Status:** engineering prototype. The core logic is tested and the
> certification strategy is documented. Still to do: hardware qualification
> and the approval itself. See *Status* below.

## Documentation

| Doc | Content |
|---|---|
| [01 System overview](docs/01-system-overview.md) | Purpose, context, design principles |
| [02 Certification strategy](docs/02-certification-basis.md) | Approval route, CS-23 compliance matrix, FHA, DAL D, DO-160G, security |
| [03 Hardware](docs/03-hardware.md) | Pi 5 vs CM5, carrier block diagram, RF interlock, power budget, BOM |
| [04 Software architecture](docs/04-software-architecture.md) | Stack, HLR table with traceability to tests, phase logic, data efficiency |
| [05 Update process](docs/05-update-process.md) | Release, signing, ASCI, install gate, A/B rollback, records |

## Repository layout

```
crates/cfb-core   decision logic (phase, link policy, queue, framing, update gate, config)
crates/cfbd       daemon (1 Hz loop) + cfb-release signing tool
config/cfb.toml   example installed configuration
os/rauc           RAUC A/B slot configuration
os/systemd        hardened service units, watchdog, mark-good
docs/             engineering and certification documents
```

## Build and run

```sh
cargo test                                    # 37 HLR-traced tests
cargo run --release --bin cfbd -- --config config/cfb.toml --simulate
```

The simulation flies a scripted leg: parked, taxi, takeoff, 25 min cruise,
landing, taxi-in, shutdown. The output shows the phase changes, the radios
being enabled and inhibited, and the traffic per link:

```
[    20s] phase Unknown -> OnGround
[    20s] RF Cellular: ENABLED
[   189s] phase OnGround -> Unknown          <- takeoff roll: radios off
[   201s] phase Unknown -> Airborne
[  1729s] phase Unknown -> OnGround
[  1729s] queued flight data file (8 x 32 KiB) for ground upload
[  1860s] update gate: OPEN (ready to load software)
Cellular: 8 frames, 262264 bytes
Iridium: 27 frames, 810 bytes
```

To cross-compile for the Pi (aarch64), use
`cargo build --release --target aarch64-unknown-linux-gnu` with a cross
linker, or build natively on the Pi. CI also builds natively on arm64.

Release signing (development keys only; production keys belong on a token):

```sh
cfb-release keygen release.key            # prints the public key for cfb.toml
cfb-release sign   release.key manifest.toml
cfb-release verify <pubkey-hex> manifest.toml
```

## Status

| Area | State |
|---|---|
| Core logic (phase, links, queue, framing, update gate) | Done, unit-tested |
| Daemon loop, simulation, systemd/watchdog, queue persistence | Done |
| Release signing tool | Done |
| `cfb-io` adapters (gpsd, gpiod, ModemManager, Certus AT driver) | **To do** |
| Link adapters + WireGuard ground service + end-to-end acks | **To do** (simulation acks immediately) |
| Yocto/Buildroot image, secure boot, dm-verity | **To do** (RAUC + units provided) |
| Carrier board, enclosure, DO-160G tests | **To do** |
| PSAC / SDP / SVP / SCMP / SQAP, FHA sign-off, approval | **To do** with your DOA |
