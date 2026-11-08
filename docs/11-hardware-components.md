# 11 – Hardware Components: Portable Unit and Integrated Unit

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

Two builds, one software:

* **A – Portable (standalone):** off-the-shelf parts, no aircraft
  connection, carried in the cabin. Purpose: real flight and airfield data,
  link performance, edge-app development. Months, not years.
* **B – Integrated (installed):** custom carrier board on the Compute
  Module 5, aircraft power, isolated aircraft inputs, hardware RF interlock,
  qualified enclosure and antennas. Purpose: the certified product.

Part numbers are **examples** that fit the design; check current
availability, datasheets and temperature ratings before ordering. Prices are
rough orders of magnitude for planning, not quotes.

---

## A. Portable unit

### A.1 Components

| Function | Recommended part (example) | Why | Software status |
|---|---|---|---|
| Compute | **Raspberry Pi 5, 4 GB** + Active Cooler | Same software as the bench | ✅ |
| Storage | **NVMe SSD 128–256 GB** on the official **M.2 HAT+** (or an industrial-grade microSD) | SD cards wear out and dislike vibration | ✅ |
| Real-time clock | Official RTC battery for the Pi 5 | Correct time before GNSS fix | ✅ (OS) |
| GNSS | **u-blox M9 / M10 receiver on USB** (e.g. NEO-M9N / MAX-M10S boards), or **ZED-F9P** for dual-frequency | Fast fix, multi-constellation, shows up as `/dev/ttyACM0` | ✅ `cfb-io` |
| GNSS antenna | Active patch antenna with cable, mounted on the glare shield (window view), or a suction mount | A cockpit roof blocks a lot; the antenna position matters more than the receiver | – |
| Accelerometer (IMU) | **ST ISM330DHCX** (industrial, ±16 g) breakout on I²C/SPI | Engine-on from vibration, touchdown Nz, turbulence | ✅ IIO reader (`cfb-drivers::iio`), overlay `os/dt/` |
| Barometer (optional) | **Bosch BMP390** or MS5611 breakout | Cabin pressure altitude (≈ static in unpressurised aircraft) | ✅ altitude source when no aircraft data (`reference::apply_baro`) |
| 4G modem | **Quectel EG25-G** (LTE Cat 4, global bands) in a **USB enclosure**, QMI/MBIM mode | Controllable by ModemManager (radio on/off works); good band coverage in Europe | ✅ |
| 5G alternative | Quectel RM520N-GL in a USB 3 enclosure | Faster uploads, but 2–3× the power and heat | ✅ |
| 4G antennas | 2 × LTE antennas (main + diversity), on short cables | Diversity matters at the edge of airfield coverage | – |
| Wi-Fi | Pi 5 onboard; optionally a **USB Wi-Fi adapter with an external antenna** (mainline-supported chipset, e.g. MediaTek mt76) | The aircraft's metal fuselage and hangar shield the small onboard antenna | ✅ |
| **Physical radio switch** | Switched USB hub (per-port power switch) or a toggle in the modem's USB power line; a second contact to a GPIO tells the software | Hardware "radio off" for the PIC, while there is no WoW interlock. **Strongly recommended** | ✅ `rfsw` input; links behind it inhibited, unknown = OFF |
| Status indicator | 3 LEDs (green/amber/red) via `gpio-led` overlays | The pilot must be able to see what the box is doing | ✅ `cfb-core::status` + `cfb-drivers::led` |
| Satcom (option) | **Iridium 9603 SBD** module (e.g. RockBLOCK 9603) with a small patch antenna | Position reports in flight, global, ~a few W peak | ✅ SBD driver (`cfb-drivers::link_sbd`); ground decode `cfb-ground --decode-sbd` |
| ~~Starlink Mini~~ | – | **Not for in-flight use:** Starlink requires an aviation terminal with an approved installation. On the ground it works as a "Wi-Fi" source | – |
| Power, option 1 | **12/24 V → 5.1 V / 5 A USB-C PD converter** from the aircraft's accessory socket (where one is fitted) | The Pi 5 only gives 1.6 A total to USB at 5 A input; with 3 A it limits USB to 600 mA, too little for modem + GNSS | ✅ |
| Power, option 2 | USB-C PD power bank that supplies **5 V / 5 A**, or 5 V / 3 A with `usb_max_current_enable=1` and a checked power budget | Independent of the aircraft | ✅ |
| Clean shutdown | Supercapacitor UPS HAT (no extra lithium cells) with a power-fail output to a GPIO | Power loss → queue snapshot and clean shutdown | ✅ `pfail` input → snapshot, power-off after 5 s |
| Enclosure | Small aluminium case with heatsink fins, strap / Velcro mount; keep away from the magnetic compass | The cockpit in sun gets well above 50 °C; the Pi throttles | – |

### A.2 Power budget (portable, estimated)

| Item | Typical | Peak |
|---|---|---|
| Pi 5 (light load) | 3 W | 8 W |
| NVMe | 0.5 W | 2 W |
| 4G (EG25-G) | 1 W | 4 W |
| GNSS + IMU + baro | 0.2 W | 0.3 W |
| Iridium 9603 (option) | 0.1 W | ~5 W bursts |
| **Total** | **~5 W** | **~15 W** |

A 20 000 mAh (≈ 74 Wh) power bank gives roughly 10–14 h at typical load.
For airline carriage, power banks up to 100 Wh are generally allowed in the
cabin. Private GA flights: secure it, and keep it cool.

### A.3 Cost (order of magnitude)

| Set | ≈ EUR/CHF |
|---|---|
| Pi 5 + cooler + NVMe + HAT + RTC battery | 150–200 |
| GNSS (M9/M10 USB) + antenna | 60–250 (F9P: +200) |
| IMU + barometer breakouts | 30–60 |
| 4G module + USB enclosure + antennas | 120–200 |
| USB Wi-Fi with antenna, switched hub, LEDs, case, cables | 80–150 |
| Power (PD converter or power bank, UPS HAT) | 60–120 |
| **Portable unit without satcom** | **≈ 500–950** |
| Iridium 9603 kit + antenna + airtime | + 250–350 + monthly |

### A.4 Software for the full portable kit: done

All items are implemented and tested (fakes for sensors and modem); see
`config/cfb-portable-pi5.toml`, `os/dt/`, `os/udev/` and tests T17–T24 in
`docs/10`. Still open: reading Iridium uplink (MT) messages, and using the
barometer as a phase cue (today it is an altitude source).

---

## B. Integrated unit (installed, certification target)

### B.1 Architecture

A base board with modules, as in `docs/03`: base (compute, power,
discretes, Wi-Fi) + reference sensor (GNSS + IMU) + ARINC 429 RX + cellular
\+ satcom option.

### B.2 Components

| Function | Recommended part (example) | Key requirement |
|---|---|---|
| Compute | **Raspberry Pi Compute Module 5**, 4 GB RAM, 32 GB eMMC, with wireless | eMMC instead of SD; u.FL for an external Wi-Fi antenna; check the temperature grade against the DO-160 category; long-term availability per Raspberry Pi |
| Power input protection | TVS diodes (SMCJ / SMDJ series) + ideal-diode reverse-polarity controller (**TI LM74700-Q1** / ADI LTC4359) + surge stopper (**ADI LTC4380**) | DO-160G Sec. 16/17: 14 and 28 V systems, spikes, reverse polarity, engine-start dropouts |
| EMI filter | Common-mode choke + π filter at the input; filtered connector | DO-160G Sec. 21 Cat M emissions (the hardest test for a box with a fast SoC and radios) |
| DC/DC | 9–36 V → 5 V / 5 A synchronous buck (automotive grade, e.g. **TI LM5146-Q1** controller), or a rugged DC/DC module (RECOM / TDK-Lambda railway/military series) | Efficiency, wide input, frequency outside sensitive bands |
| Hold-up | Supercapacitor bank + backup controller (**ADI LTC3350**) | Ride-through of short dropouts; on longer power loss ~5 s for queue snapshot and shutdown; **no lithium battery** |
| Watchdog | External windowed watchdog (**TI TPS3431**) | Independent of the SoC |
| Discrete inputs (WoW, oil pressure, maintenance key, …) | Optocouplers (e.g. **Vishay VO617A** or Broadcom ACPL series), configurable for 28 V/open and ground/open | Galvanic isolation; can't load or drive aircraft lines |
| **Hardware RF interlock** | Logic AND (**74LVC1G08**): `WoW_ground AND cell_enable` → modem **W_DISABLE#** and **FULL_CARD_POWER_OFF#** | Cellular off in flight **without relying on software**. This is the core of the safety argument |
| ARINC 429 receiver | **Holt HI-3593** (2 RX channels per IC; the transmitter's external line driver is **not fitted**) behind a digital isolator (ADI ADuM family) + isolated DC/DC | Receive-only by construction; high-impedance; bus-load check per installation |
| RS-232/422 receive | Isolated receivers (TI ISO1410 for 422; RS-232 receiver + digital isolator); **TX not wired** | Engine monitors, legacy data outputs |
| CAN | **Microchip MCP2518FD** + isolated transceiver **TI ISO1042**, TXD forced recessive | Listen-only |
| GNSS | **u-blox NEO-F10N** (L1/L5 dual-band) or **ZED-F9P** | Multi-band, better integrity; 1PPS to the CM5 for time |
| GNSS antenna | Aviation GNSS antenna to **TSO-C190**/**C144**, top fuselage | Approved antenna avoids a separate antenna qualification |
| IMU | **ST ISM330DHCX** or **Murata SCH16T** (higher stability), hard-mounted on the base board | Axes on the label; mounting per installation drawing |
| Cellular | **Quectel EG25-G** (4G) or **RM520N-GL** (5G), M.2 key-B, USB mode | W_DISABLE# wired to the interlock; RED/CE + carrier approvals |
| Cellular antenna | Aircraft-rated LTE/5G blade antenna (several aviation antenna makers offer them) | Belly or composite fairing; intermodulation check with VHF/GNSS |
| Wi-Fi | CM5 onboard + external antenna, or an M.2 Wi-Fi 6 module | Ground use; cabin AP optional (PED-tolerance testing) |
| Satcom (option) | **Iridium Certus 9770** transceiver (Certus 100, IP) or **Iridium 9603** (SBD only) + aviation Iridium antenna | Global backup, works at all latitudes |
| LEO (option) | **Ethernet port** to an approved Starlink Aviation terminal (terminal + antenna approved separately) | The box only routes; the terminal carries its own approval |
| Ethernet | CM5 GbE + small managed switch IC (e.g. Microchip KSZ series) | Maintenance port + LEO terminal on separate VLANs |
| Module ID | I²C EEPROM per module (e.g. **24AA02E48**, includes a unique ID) | `modules.rs` checks configured vs. fitted |
| Connectors | **MIL-DTL-38999 Series III** circular for power/discretes/data; **TNC** for RF | Locking, sealed, standard in aviation |
| Enclosure | Machined aluminium, conductive gaskets, conduction cooling to a mounting tray; no fan | Emergency landing loads (CS 23.2270 / ASTM F3083); compass safe distance from DO-160 Sec. 15 |
| Cabling | EWIS per approved practice; shielded twisted pairs for ARINC/serial | Separation from power wiring |

### B.3 Parts policy for certification

* Temperature grade −40…+85 °C where available; AEC-Q100/Q200 (automotive)
  parts preferred for power and interface ICs.
* Approved Vendor List with traceable distributors (anti-counterfeit).
* Obsolescence plan: second source per critical part, lifetime-buy
  triggers; the CM5's availability statement is part of the argument.
* COTS electronic hardware argument per AMC 20-152A (DAL D), backed by the
  DO-160G qualification of the complete unit.

### B.4 Power budget (integrated)

As in `docs/03`: ~4–6 W typical, ~15 W peak at the box (Starlink terminal
powered separately). 28 V: ≈ 0.2 A typical; 14 V: ≈ 0.4 A. Use a 3 A CB on
the non-essential bus.

### B.5 Cost (order of magnitude)

| Item | ≈ EUR/CHF |
|---|---|
| Unit BOM incl. PCB, enclosure, connectors, small series (10–50 units) | 1 500–3 000 per unit |
| Aviation antennas (GNSS, cellular, Iridium) | 1 000–4 000 per aircraft |
| Starlink aviation terminal + installation | far higher; it's its own project |
| **One-off engineering (NRE):** carrier board design, layout, prototypes | 40 000–120 000 |
| DO-160G qualification campaign (test lab) | 50 000–150 000 |
| Certification (DOA partner, documents, flight test) per type / AML | strongly dependent on route and partner |

### B.6 Development sequence

1. **Portable unit (A)** with IMU, LEDs and the physical radio switch.
   Collect real data.
2. **Carrier board rev A** on the bench with the CM5: power stage, discretes,
   interlock, GNSS/IMU, modem. Pre-compliance EMI scan early, because
   emissions are the main risk.
3. Rev B with the ARINC module; bench test against a real avionics bench
   (with the OEM's ICD).
4. Enclosure and harness, DO-160G qualification.
5. Prototype installation under a Permit to Fly with the DOA partner, then
   the approval.
