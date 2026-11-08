# 03 – Hardware

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

> Concrete parts lists for the **portable unit** and the **integrated
> (certifiable) unit**, with power budgets, costs and development sequence:
> see [11 – Hardware components](11-hardware-components.md).

## Compute: Raspberry Pi 5 vs Compute Module 5

| | Raspberry Pi 5 (board) | **Compute Module 5 on custom carrier** (production) |
|---|---|---|
| SoC | BCM2712, 4× Cortex-A76 2.4 GHz | same |
| Storage | microSD (wear and vibration weak point) | **on-module eMMC** (8–64 GB) |
| Connectors | consumer (USB-C, micro-HDMI) | your choice: locking aviation connectors |
| Mechanical | board + HAT stack, harder to qualify for vibration | single carrier PCB, potted/staked parts |
| Supply chain | long-term availability stated by Raspberry Pi | long-term availability stated by Raspberry Pi |
| Use | **prototypes, ground lab, flight-test rig** | **certified installation** |

Both run the same software image. Keep the Pi 5 for development and use the
CM5 for the certified unit. Check temperature ratings in the current
datasheets against the DO-160 category you choose; derate the CPU clock
(`arm_freq`) if needed. The software needs well under one core.

## Modular architecture

One base unit, plug-in modules on board-to-board connectors inside the same
enclosure. Every module has an ID EEPROM (I²C, read at boot). The installed
configuration (`[modules]` in `cfb.toml`) says what must be fitted; the
software checks both and disables anything not present
(`cfb-core::modules`).

| Module | Contents | Fitted | Interface to CM5 |
|---|---|---|---|
| **Base** | CM5, power supply + hold-up, opto discretes (WoW, oil press., maint. key, PWR_FAIL), Wi-Fi (CM5 onboard, external antenna), Ethernet maint. port, HW watchdog | always | – |
| **Reference sensor** | multi-band GNSS receiver (u-blox M10/F9-class) + industrial 6-axis IMU (ISM330DHCX / BMI088-class), shared 1PPS | standard | UART + PPS, SPI |
| **ARINC 429 RX** | 2 or 4 receive channels (HI-3593-class), no transmitter fitted, 12.5 / 100 kbit/s | option | SPI |
| **Cellular** | M.2 key-B LTE Cat-4 (4G) **or** 5G NR module, eSIM/SIM, 2× MIMO antennas | standard | USB 3 |
| **Satcom** | Iridium Certus 100 / SBD transceiver, and/or Ethernet port for an approved LEO terminal | option | UART/USB, Ethernet |

Typical builds:

| Build | Modules | Use |
|---|---|---|
| CFB-G (ground) | base + reference sensor + 4G | Flight school / club: track and data uploaded after landing, no ARINC |
| CFB-A | base + reference + ARINC + 5G | Glass-cockpit aircraft, data from avionics, ground upload |
| CFB-S | base + reference + ARINC + 5G + Iridium | Commercial / ops: live tracking in flight |
| CFB-S+ | CFB-S + LEO Ethernet | Where an approved LEO terminal is installed |

## Block diagram (base + modules)

```
 14/28 V ─┬─ CB 3A ─▶ [reverse-pol. + TVS] ─▶ [EMI filter] ─▶ [9–36 V → 5 V/5 A buck] ─▶ CM5
          │                                        │
          │                                 [supercap hold-up, ~5 s]──▶ PWR_FAIL# GPIO
          │
 WoW ─────┴─▶ [opto] ─┬─▶ GPIO (software reads phase)
                      └─▶ AND ─▶ modem W_DISABLE# ◀── GPIO "cell_enable"
                                (hardware: modem can only transmit when WoW=ground
                                 AND software agrees)
 Oil-press sw ─▶ [opto] ─▶ GPIO      (optional: IMU vibration used if not wired)
 Maint key sw ─▶ [opto] ─▶ GPIO
 ┌ ARINC 429 module ──────────────────────────────────────────────────────────┐
 │ bus A/B ─▶ [receiver, RX only, high-impedance] ─▶ SPI  (× 2 or 4 channels)  │
 └────────────────────────────────────────────────────────────────────────────┘
 ┌ Reference sensor module ──────────────────────────────┐
 │ GNSS antenna ─▶ [GNSS receiver] ─▶ UART + 1PPS        │
 │ [6-axis IMU, 100 Hz+] ─▶ SPI  (hard-mounted, axes on label)
 └───────────────────────────────────────────────────────┘
 ┌ Cellular module ─────────────┐   ┌ Satcom module ─────────────────────┐
 │ 4G or 5G M.2 key-B ─▶ USB 3  │   │ Iridium Certus/SBD ─▶ UART/USB     │
 │ W_DISABLE# ◀── interlock     │   │ LEO terminal ─▶ Ethernet (option)   │
 └──────────────────────────────┘   └────────────────────────────────────┘
 RS-232/422 ──▶ [isolated RX-only transceiver] ─▶ UART   (engine monitor, no TX line wired)
 CAN ─────────▶ [isolated transceiver, TX pin tied recessive] ─▶ SPI CAN controller
 Ethernet ────▶ maintenance port (keyed connector)
 Hardware watchdog (external, e.g. TPS3431-class) ─▶ RESET
```

### ARINC 429 installation rules

* **Receive only.** The module has no ARINC transmitter; it cannot put a word
  on any aircraft bus.
* **Bus loading.** ARINC 429 allows up to 20 receivers per transmitter.
  Check how many receivers are already on each bus you tap, and the source
  LRU's installation manual for its maximum load.
* **Connection** by an approved splice or a spare connector pin on the existing
  bus harness, shielded twisted pair, shield grounded per the source LRU's
  installation manual. EWIS separation from power wiring.
* **Labels** come from the source LRU's ICD (GNSS navigator, ADC/ADAHRS,
  engine data unit) and go into `[[arinc429.labels]]`. Nothing is
  hard-coded; the defaults are a starting point only.
* Bus speed (12.5 or 100 kbit/s) is set per module in `[modules]`.

### Reference sensors (own GNSS + IMU)

* **Own GNSS** is the position source when there is no ARINC source, or
  when the ARINC source is lost. When both work, the box compares them and
  reports disagreements to the ground. It also sets the system time at boot.
  It needs its own antenna (top fuselage), or an approved splitter on an
  existing GNSS antenna. A splitter must not degrade the certified GNSS
  installation.
* **IMU**: hard-mounted to the structure via the box tray. The axis mapping
  goes on the installation drawing and in `[imu] axes`. Uses:
  * engine-running detection from vibration (when no oil-pressure discrete
    is wired);
  * takeoff and braking acceleration;
  * touchdown Nz (**hard-landing advisory**) and turbulence advisory, sent to
    the maintenance/ops ground service. Advisory only; AMM criteria and crew
    reports remain the authority.
* The at-rest gravity reference is captured automatically whenever the
  aircraft is parked with the engine off, so the ramp slope and small mounting
  tolerances cancel out.

### Why each item is there

* **Receive-only interfaces.** No transmit driver is fitted towards avionics.
  This is the "cannot corrupt avionics" argument; it does not need any
  software assurance. CAN uses a transceiver with TXD forced recessive.
* **Galvanic isolation** on every aircraft signal: no ground loops, and a CFB
  fault cannot pull an avionics line.
* **Hardware RF interlock.** The cellular modem's `W_DISABLE#` is driven by
  `WoW_ground AND cell_enable`. In flight the modem is off even if the
  software is wrong. Wi-Fi client mode is software-inhibited; the cabin AP
  (if fitted) is allowed in flight per the installation's EMI approval.
* **Hold-up supercap.** Engine cranking on 14/28 V systems drops the bus.
  The hold-up rides through short dropouts and, on longer loss, gives ~5 s
  for a final queue snapshot and a clean shutdown (`PWR_FAIL#`). No lithium
  battery in the box (avoids battery-installation requirements); the RTC
  uses a supercap or is set from GNSS at boot.
* **External watchdog** in addition to the SoC watchdog, fed by `cfbd` via
  systemd. A hang causes a reset; repeated failed boots trigger the A/B
  fallback.

## Power budget (estimate, to confirm by measurement)

| Item | Typical | Peak |
|---|---|---|
| CM5 (light load, CPU capped) | 2.5 W | 6 W |
| LTE/5G modem (ground only) | 1.5 W | 5 W |
| Iridium Certus transceiver | 1.5 W | 8 W (TX) |
| Wi-Fi | 0.5 W | 1.5 W |
| Base carrier, isolators | 0.4 W | 0.8 W |
| Reference sensor (GNSS + IMU) | 0.15 W | 0.3 W |
| ARINC 429 module (4 RX) | 0.2 W | 0.3 W |
| **Total** | **~4–6 W** | **~15 W** (cellular is never on together with in-flight satcom TX bursts in normal use) |

At 28 V that is ≈ 0.2 A typical, 0.6 A peak; at 14 V ≈ 0.4 A / 1.1 A.
Use a 3 A CB on the non-essential (or avionics) bus.

## Enclosure and installation

* Machined or folded aluminium enclosure, conduction-cooled to a tray; no
  fan. Thermal design point: full software load at the upper DO-160
  temperature of the chosen zone.
* Connectors: circular MIL-style (e.g. D38999-type) or locking D-sub; coax
  TNC/SMA with torque values in the ICA.
* Mount: tray qualified to CS 23.2270 / ASTM F3083 emergency load factors
  for the box mass (~600–900 g).
* Antennas: Iridium patch on top fuselage (sky view), cellular blade on
  belly or inside a composite fairing, GNSS shared or separate. Doublers per
  approved repair/standard practice or type-specific drawing.
* Compass safe distance placard from DO-160 Sec. 15 test.

## Example BOM (prototype)

| Part | Example | Note |
|---|---|---|
| Compute | Raspberry Pi 5 4 GB (proto) / CM5 4 GB 32 GB eMMC (prod) | 2 GB is enough for the software |
| Own GNSS | u-blox M10 (single band) or F9-class (multi-band) | UART + 1PPS, active antenna |
| IMU | ST ISM330DHCX or Bosch BMI088 (industrial, ±16 g) | SPI, 100–400 Hz |
| LTE/5G | Quectel EM-series / RM-series M.2 key B, USB mode | W_DISABLE# wired to interlock |
| Satcom | Iridium Certus 100 transceiver module, or Iridium 9603 SBD for tracking only | SBD: lowest cost, 340 byte messages |
| ARINC 429 RX | Holt HI-3593-class receiver(s) | 2 RX channels per IC, SPI |
| Isolated UART RX | ISO7721 + RS-232 receiver | RX only |
| CAN | MCP2518FD + isolated transceiver | listen-only mode + TXD tied |
| DC/DC | 9–36 V in, 5 V 5 A out, automotive/aero grade | DO-160 Sec. 16 |
| Watchdog | TPS3431 or similar | windowed |
