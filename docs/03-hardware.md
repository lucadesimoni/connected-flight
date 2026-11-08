# 03 – Hardware

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

## Block diagram (carrier board)

```
 14/28 V ─┬─ CB 3A ─▶ [reverse-pol. + TVS] ─▶ [EMI filter] ─▶ [9–36 V → 5 V/5 A buck] ─▶ CM5
          │                                        │
          │                                 [supercap hold-up, ~5 s]──▶ PWR_FAIL# GPIO
          │
 WoW ─────┴─▶ [opto] ─┬─▶ GPIO (software reads phase)
                      └─▶ AND ─▶ modem W_DISABLE# ◀── GPIO "cell_enable"
                                (hardware: modem can only transmit when WoW=ground
                                 AND software agrees)
 Oil-press sw ─▶ [opto] ─▶ GPIO
 Maint key sw ─▶ [opto] ─▶ GPIO
 ARINC 429 ───▶ [HI-3593-class receiver, RX only] ─▶ SPI
 RS-232/422 ──▶ [isolated RX-only transceiver]   ─▶ UART      (no TX line wired)
 CAN ─────────▶ [isolated transceiver, TX pin tied recessive] ─▶ SPI CAN controller
 GNSS ────────▶ internal module + external antenna (or aircraft GNSS via RS-232)
 USB 3 ───────▶ LTE/5G M.2 key-B modem (USB mode)
 USB / UART ──▶ Iridium Certus / SBD transceiver
 SDIO/PCIe ───▶ Wi-Fi 2.4/5 GHz (onboard CM5 Wi-Fi, external antenna)
 Ethernet ────▶ maintenance port (keyed connector)
 Hardware watchdog (external, e.g. TPS3431-class) ─▶ RESET
```

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
| Carrier, isolators, GNSS | 0.5 W | 1 W |
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
| LTE/5G | Quectel EM-series / RM-series M.2 key B, USB mode | W_DISABLE# wired to interlock |
| Satcom | Iridium Certus 100 transceiver module, or Iridium 9603 SBD for tracking only | SBD: lowest cost, 340 byte messages |
| ARINC 429 RX | Holt HI-3593-class receiver | 2 RX channels, SPI |
| Isolated UART RX | ISO7721 + RS-232 receiver | RX only |
| CAN | MCP2518FD + isolated transceiver | listen-only mode + TXD tied |
| DC/DC | 9–36 V in, 5 V 5 A out, automotive/aero grade | DO-160 Sec. 16 |
| Watchdog | TPS3431 or similar | windowed |
