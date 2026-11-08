# Connected Flight Box (CFB)

**© 2026 Outthinkx by Luca De Simoni. All rights reserved.** Proprietary
software, **not open source**. The code is publicly visible for viewing
only; no licence to use, copy, modify or install it is granted without a
written agreement with Outthinkx. See
[LICENSE](LICENSE) and [NOTICE.md](NOTICE.md). Licensing: outthinkx@gmail.com

A low-power connectivity unit for **EASA CS-23 aeroplanes** (Level 1–4), built
on the **Raspberry Pi 5 / Compute Module 5**. It sends data **in flight over
satcom** and **on ground over LTE/5G or Wi-Fi**. It is designed to be
installed as a **minor change** with **DAL D field-loadable software**.

* **Modular:** a base unit (CM5, power, discretes, Wi-Fi) plus plug-in
  modules:
  * **ARINC 429** receiver (2 or 4 channels, receive only);
  * **reference sensors** (own GNSS and accelerometer);
  * **4G or 5G** cellular;
  * optional **satcom** (Iridium, LEO).

  One software build adapts to whichever modules are fitted.
* **Aircraft data plus own reference:** position, speed and altitude come
  from the aircraft over ARINC 429. When that source is lost, the box's own
  GNSS takes over, and while both work they are cross-checked. The
  accelerometer detects a running engine from vibration and flags hard
  landings and turbulence for maintenance.
* **Live in flight, the rest on ground:** Starlink/LEO and Iridium in flight,
  5G and Wi-Fi on ground. UDP runs inside one WireGuard tunnel per bearer,
  and the ground acks every frame end to end.
* **Edge apps:** signed, sandboxed rule packages run on board:
  exceedances, engine trend, flight summary. They send results instead of
  raw data.
* **Safety by hardware:** receive-only, isolated avionics interfaces, and a
  WoW-driven hardware interlock that holds the cellular modem off in flight.
  Because of that, software updates stay low-risk and cheap to approve.
* **Low data cost:** a 30-byte binary position report (JSON would be ~150 B),
  priority queues, and a per-flight satcom byte budget. Bulk data waits for a
  free ground link and uploads automatically after landing.
* **Small:** one ~600 KB Rust binary, no async runtime, <64 MB memory cap,
  ~4–6 W for the whole box.
* **Safe OTA updates:**
  * The ground announces a package; the box downloads it on ground (and,
    if configured, in flight over Starlink) and verifies it.
  * RAUC A/B slots, an Ed25519-signed manifest, and an Approved Software
    Configuration Index (ASCI).
  * It installs only when parked, engine off and confirmed by a technician.
    The reboot waits until it is safe, and rollback is automatic.
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
| [05 Update process](docs/05-update-process.md) | Release, signing, ASCI, OTA download/verify/install, A/B rollback, records |
| [06 Edge apps](docs/06-edge-apps.md) | In-box computing: containment argument, rule packages, release |
| [07 PC-12 / PC-24](docs/07-pc12-pc24-installation.md) | Certification basis, data access, Starlink/antennas, environment, sequence |
| [08 Avionics data access](docs/08-avionics-data-access.md) | Honeywell Apex/Epic maintenance port, ACMS/CMC, ARINC 429/717, QARs, Garmin logs, legal limits, competitors |
| [09 Ownership & commercialisation](docs/09-ownership-and-commercialisation.md) | IP protection, employment check, trademarks, business models, company setup |
| [10 Bench & portable test](docs/10-bench-and-portable-test.md) | Pi 5 + Wi-Fi + 4G + WireGuard setup, 16 tests, rules for carrying it as a portable device |
| [11 Hardware components](docs/11-hardware-components.md) | Parts for the portable unit and the integrated (certifiable) unit, power, cost, sequence |
| [12 Architecture & long-term support](docs/12-architecture-and-extension.md) | Layers, extension points, compatibility rules, quality gates, release/support model |

## Repository layout

```
crates/cfb-core   decisions, no I/O: arinc429, gnss, imu, reference, modules, phase, link, queue,
                  frame, edge, ota, update, status, sbd, datalog, config
crates/cfb-drivers  hardware/network I/O: LinkDriver trait (UDP/WireGuard, Iridium SBD),
                  IIO accelerometer/barometer, NMEA serial, discretes, LEDs
crates/cfbd       daemon (1 Hz loop), transport, OTA executor, simulator;
                  bins: cfb-release (signing), cfb-ground (reference ground service),
                        cfb-replay (edge apps over existing flight logs, on the ground),
                        cfb-a429-survey (bench check of an ARINC label table),
                        cfb-io (I/O adapter: USB GNSS, discretes file/GPIO)
os/net, scripts   WireGuard per bearer (make-wg.sh), NetworkManager profiles,
                  box installer (install-box.sh)
ground/           ground server: WireGuard hub, cfb-ground with log, OTA HTTP
config/cfb.toml          full build: ARINC + GNSS/IMU + Wi-Fi + 5G + Iridium
config/cfb-minimal.toml  ground build: GNSS/IMU + Wi-Fi + 4G, no ARINC, no satcom
config/cfb-pc12-example.toml  PC-12-class: ARINC + 5G + Starlink + Iridium, transport, OTA, edge
config/apps/             example edge app package (turboprop ops)
config/cfb-bench-pi5.toml  bench Pi 5: GNSS + Wi-Fi + 4G, WireGuard, RF inhibit
config/cfb-portable-pi5.toml  portable kit: + IMU, barometer, radio switch, LEDs, power-fail, Iridium SBD
os/rauc           RAUC A/B slot configuration
os/systemd        hardened service units, watchdog, mark-good
docs/             engineering and certification documents
```

## Build and run

```sh
cargo test                                    # 118 HLR-traced tests
cargo run --release --bin cfbd -- --config config/cfb.toml --simulate
cargo run --release --bin cfbd -- --config config/cfb.toml --simulate --hard-landing
cargo run --release --bin cfbd -- --config config/cfb-minimal.toml --simulate
cargo run --release --bin cfbd -- --config config/cfb-pc12-example.toml --simulate --apps config/apps
```

The PC-12-class run shows the full chain:

```
[   520s] EDGE tprop-ops|TREND ENG_CRUISE n=121 torque=85.0,itt=720.0,ng=97.0,...
[   605s] EDGE tprop-ops|EXC ITT_HIGH START itt>800.0 val=812.0
[   613s] EDGE tprop-ops|EXC ITT_HIGH END dur=13s peak=812.0
[   700s] OTA download over LeoSat (in flight: download only, install on ground)
[   722s] OTA Verified CFB-SW-0100 0.2.0
[  1811s] EDGE tprop-ops|SUM block=29min flight=25min dist=60NM max_alt=8700ft ...
[  1851s] OTA Installing CFB-SW-0100 0.2.0           <- parked, engine off, confirmed
[  1881s] OTA reboot into new slot (simulated)
LeoSat: 69 frames, 2479 bytes
```

Bench test of the real transport (UDP, acks, uplink):

```sh
cfb-ground --listen 127.0.0.1:7400 --announce-ota
cfbd --config <config with [transport] pointing at 127.0.0.1:7400> < inputs
```

The simulator generates the raw inputs the real I/O service would deliver:
ARINC 429 words, NMEA sentences, 100 Hz accelerometer samples, discretes
and link state. The scripted leg covers parking, taxi, takeoff, cruise with
an ARINC outage and a turbulence encounter, landing, taxi-in, shutdown and
maintenance mode:

```
modules: base, arinc429, gnss, imu, wifi, cellular_5g, iridium
[    10s] REF position=Aircraft engine=OwnImu      <- engine detected from vibration
[   189s] phase OnGround -> Unknown                <- takeoff roll: cellular off
[   201s] phase Unknown -> Airborne
[   902s] REF position=OwnGnss engine=OwnImu      <- ARINC lost: own GNSS takes over
[  1001s] REF position=Aircraft engine=OwnImu
[  1200s] TURBULENCE ADVISORY nz 0.94/2.21 at Some((47.500076, 8.600063)) 8500ft
[  1730s] LANDING touchdown nz_max 1.44
[  1730s] queued flight data file (8 x 32 KiB) for ground upload
[  1860s] update gate: OPEN (ready to load software)
ARINC 429: 10800 words, 0 parity errors, 0 unknown labels
Cellular: 11 frames, 262401 bytes
Iridium: 28 frames, 893 bytes
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
| ARINC 429 decoder, own GNSS (NMEA), IMU processing, reference selection, module checks | Done, unit-tested |
| Daemon loop, simulation, systemd/watchdog, queue persistence | Done |
| Release signing tool | Done |
| `cfb-io` adapters (ARINC SPI driver, GNSS UART, IMU via IIO, gpiod, ModemManager, Certus AT driver, ID EEPROMs) | **To do** |
| Base board and module PCBs | **To do** |
| Air-ground transport (UDP per bearer, acks, retry, link health), reference ground service | Done, tested on localhost |
| Edge app runtime + rule packages (exceedance, trend, summary) | Done, unit-tested |
| OTA state machine + executor (curl, verify, rauc, deferred reboot) | Done; tested in simulation; real RAUC/curl on target to do |
| Bench networking: WireGuard per bearer, policy routing, NetworkManager, RF inhibit commands, cfb-io, installers | Done; chain tested on localhost; **on a real Pi 5: to test (docs/10)** |
| Portable kit software: IMU + barometer (IIO), radio switch, power-fail shutdown, status LEDs, Iridium SBD driver, device-tree/udev files | Done; tested with fake sensors/modem; **on hardware: docs/10 T17–T24** |
| Production ground service (database, dedup, users, SBD webhook), RAUC A/B image | **To do** |
| Starlink terminal installation approval per type | **To do** (outside the box) |
| Yocto/Buildroot image, secure boot, dm-verity | **To do** (RAUC + units provided) |
| Carrier board, enclosure, DO-160G tests | **To do** |
| PSAC / SDP / SVP / SCMP / SQAP, FHA sign-off, approval | **To do** with your DOA |
