# 14 – Portable Kit: Verified Parts List, Wiring and Integration Path

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

This list replaces the example parts in `docs/11` §A for anyone building the
kit. Every part was checked against exactly what the software expects:
kernel driver present in the Raspberry Pi kernel, device-tree compatible
string, I²C address, serial settings, GPIO pins and power budget. The
evidence is in §5.

**How "it works" is assured.** No list can guarantee hardware it hasn't
touched. What it does here:

1. Every part on the list was checked against the software's requirements (§5).
2. Where a check failed, the software or config was changed. Three real
   mismatches were found and fixed: the barometer compatible string, a GPIO
   collision with the UPS, and the GNSS baud rate.
3. `cfb-verify-hardware` (`scripts/verify-hardware.sh`) checks every part
   on the assembled kit in about a minute: present, right driver, plausible
   readings, radios switchable. Every FAIL says what to fix. Run it before
   the first use and after any change.

## 1. Parts (portable kit, integration-ready)

Each part uses the **same chip** as the integrated unit (§4), so the move
to the custom carrier board changes the board, not the software or
configuration.

| # | Function | Part (buy this) | Qty | Why this one |
|---|---|---|---|---|
| 1 | Compute | **Raspberry Pi 5, 4 GB** + **Raspberry Pi Active Cooler** | 1 | Reference platform; ≤ 2 GB is used |
| 1a | Compute (integration preview) | **Raspberry Pi Compute Module 5** (4 GB RAM, 32 GB eMMC, wireless) + **CM5 IO Board** | – | Instead of 1: the exact compute module of the integrated unit; same 40-pin header and pin plan (the X1200 UPS is Pi 5 only: use a 5 V / 5 A supply) |
| 2 | Storage | **Industrial-grade microSD 32–64 GB** (e.g. SanDisk Industrial, Swissbit, Kingston Industrial) | 1 | Low write load by design (queue snapshot once a minute); industrial cards survive temperature and power cuts. NVMe (M.2 HAT+) optional |
| 3 | GNSS receiver | **SparkFun GPS Breakout NEO-M9N, SMA** (UART pins) | 1 | u-blox NEO-M9N, 4 constellations; **UART 38400 baud default**; on the Pi's hardware UART: no USB driver dependency |
| 4 | GNSS antenna | Active GNSS antenna 3–5 V, SMA, magnetic or adhesive mount, ≥ 3 m cable | 1 | Glare shield or window mount; sky view decides fix quality |
| 5 | Accelerometer | **Adafruit ISM330DHCX 6-DoF IMU (product 4502), STEMMA QT** | 1 | ST industrial IMU; **I²C 0x6A by default** = overlay default |
| 6 | Barometer | **Adafruit BMP390 (product 4816), STEMMA QT** | 1 | **I²C 0x77 by default** = overlay default |
| 7 | I²C cabling | 1 × STEMMA QT / Qwiic cable 100 mm (IMU ↔ baro) + 1 × STEMMA QT to female jumper (to Pi pins 1/3/5/6) | 1+1 | Daisy chain, 3.3 V |
| 8 | 4G modem | **Quectel EG25-G** (global LTE Cat 4, mini PCIe) | 1 | QMI + MBIM supported by ModemManager; radio switchable (RF inhibit); global bands |
| 9 | Modem adapter | Mini PCIe to USB adapter with nano/micro-SIM slot and 2 × U.FL | 1 | Short, direct USB connection to the Pi |
| 10 | 4G antennas | 2 × LTE antenna (main + diversity), U.FL→SMA pigtails | 2 | Diversity matters at the edge of airfield coverage |
| 11 | Iridium | **RockBLOCK 9603** (Iridium 9603N, built-in patch antenna) + **RockBLOCK USB adaptor** (FTDI) | 1 | 19200 8N1, standard 9603 AT set (the one `link_sbd` implements); max ~470 mA at 5 V; line rental + credits from the provider |
| 12 | UPS / power-fail | **Geekworm X1200** (Pi 5 UPS, 18650) + protected 18650 cells of a known maker as specified by Geekworm | 1 | **Power-loss signal on GPIO6** (low = supply lost) → `pfail`; fuel gauge at I²C 0x36 (no conflict) |
| 13 | Power supply | **Raspberry Pi 27 W USB-C power supply** (5.1 V / 5 A) | 1 | 5 A source: required for the 1.6 A USB budget (modem + Iridium) |
| 14 | Radio switch | 2-pole (DPDT) toggle switch + short USB-A extension (cut, VBUS through pole 1); pole 2 between GPIO23 (pin 16) and GND | 1 | Pole 1 removes the 4G modem's power, physically; pole 2 tells the software. Broken sense wire = OFF |
| 15 | Status lights | 3 mm LEDs green / amber / red + 3 × 330 Ω + panel holders | 3 | LED-class devices via `gpio-led` overlay |
| 16 | Enclosure | Aluminium enclosure with ventilation, cable glands/SMA bulkheads, strap/Velcro mount | 1 | Heat (cockpit sun), compass distance, secure stowage |

Software status of every row: supported by the code on `main`; run the
verify script to confirm on your build.

## 2. Wiring and pin plan (no conflicts)

| Header pin | GPIO | Connection |
|---|---|---|
| 1 | 3V3 | IMU + baro (STEMMA QT V+), GNSS 3V3 |
| 3 / 5 | GPIO2 / GPIO3 | I²C SDA / SCL → IMU, baro (X1200 fuel gauge 0x36 shares the bus) |
| 6, 9, 14, 20 | GND | sensors, GNSS, LEDs, switch |
| 8 | GPIO14 (TXD0) | → GNSS **RX** |
| 10 | GPIO15 (RXD0) | ← GNSS **TX** |
| 11 | GPIO17 | LED green (via 330 Ω) |
| 13 | GPIO27 | LED amber |
| 15 | GPIO22 | LED red |
| 16 | GPIO23 | radio switch pole 2 (to GND when ON; internal pull-up) |
| 31 | GPIO6 | X1200 power-loss (via the UPS's pogo pins) |
| 36 | GPIO16 | X1200 charge control (reserved, leave unconnected) |

`config.txt`: append `os/dt/config.txt.snippet` (I²C, UART0, sensor
overlay, LEDs, USB current). `io.env`: `os/systemd/io.env.portable-kit`
(installed with `CFB_KIT=portable sudo -E scripts/install-box.sh`).

## 3. Power budget (portable)

| Consumer | Typical | Peak | Feeds from |
|---|---|---|---|
| Pi 5 + cooler | 3–4 W | 8 W | 5 V rail |
| EG25-G 4G | 0.5–1 W | ~2 A bursts at 3.8 V inside the module | USB (≤ 1.6 A total) |
| RockBLOCK 9603 | 0.2 W idle | 470 mA at 5 V (limited) | USB |
| GNSS, IMU, baro, LEDs | 0.3 W | 0.5 W | 3.3 V rail |
| **Total** | **~5 W** | **~15 W** | X1200 → Pi |

USB: modem + Iridium stay within 1.6 A only with `usb_max_current_enable=1`.
The verify script fails if it isn't set, and fails on any undervoltage
event (`get_throttled`). Run it once with both modems transmitting.
Runtime on the X1200 with two typical 18650 cells: roughly 2–4 h (measure
it; cell capacity varies).

## 4. Integration path: same chips on the carrier board

| Function | Portable kit | Integrated unit (docs/11 §B) | Changes in software / config |
|---|---|---|---|
| Compute | Pi 5 (or CM5 + IO board) | CM5 on the carrier | none (same SoC, same OS image) |
| GNSS | NEO-M9N breakout on UART0 | NEO-M9N (or NEO-F10N) soldered, UART0 + 1PPS | none (NMEA on `ttyAMA0`) |
| IMU | ISM330DHCX breakout 0x6A | ISM330DHCX soldered, I²C 0x6A | none (same overlay) |
| Barometer | BMP390 breakout 0x77 | BMP390 soldered, 0x77 | none |
| 4G | EG25-G mini PCIe on USB adapter | EG25-G (mini PCIe or LGA) on the carrier, `W_DISABLE#` from the hardware WoW interlock | none (ModemManager / QMI) |
| Iridium | RockBLOCK 9603 (USB/FTDI) | Iridium 9603N module on a UART | `[sbd] device` path only |
| Power-fail | X1200 GPIO6 | Hold-up controller (LTC3350) PFO → GPIO6 | none (`pfail` on GPIO6) |
| Radio switch | DPDT toggle | Hardware WoW interlock + optional crew switch | `rfsw` source; interlock independent of software |
| LEDs | 3 mm LEDs, GPIO17/27/22 | Front-panel LEDs, same GPIOs | none |
| Aircraft inputs | – | ARINC 429 (HI-3593), discretes (opto) | `[arinc429]`, `wow`/`eng` discretes |

## 5. Compatibility evidence (checked November 2026)

| Claim | Evidence |
|---|---|
| Drivers in the Pi 5 kernel | `raspberrypi/linux` `rpi-6.18.y` `bcm2712_defconfig`: `CONFIG_IIO_ST_LSM6DSX=m`, `CONFIG_BMP280=m`, `CONFIG_IIO=m`, `CONFIG_LEDS_GPIO=y`, `CONFIG_USB_NET_QMI_WWAN=m`, `CONFIG_USB_NET_CDC_MBIM=m`, `CONFIG_USB_SERIAL_OPTION=m`, `CONFIG_USB_SERIAL_FTDI_SIO=m`, `CONFIG_WIREGUARD=m` |
| USB GNSS avoided | `CONFIG_USB_ACM` was **not found** in that defconfig (checked twice, 1 799 lines); u-blox USB receivers use `cdc_acm`, so the kit uses the UART |
| Raspberry Pi OS kernel | Trixie-based Raspberry Pi OS moved from 6.12 LTS to **6.18 LTS** (June 2026); BMP390 support was added to `bmp280` in 2023 |
| ISM330DHCX compatible | `st,ism330dhcx` in the `st_lsm6dsx` I²C match table and bindings |
| BMP390 compatible | Supported by `bmp280` as a member of the BMP380 family (chip ID 0x60, commit b19ac45bfe50); **no `bosch,bmp390` string** → overlay uses `bosch,bmp380` (was wrong before this check) |
| IMU address | Adafruit ISM330DHCX: 0x6A default; SparkFun: 0x6B default → overlay parameter `imu_addr` |
| GNSS baud | u-blox NEO-M9N and MAX-M10S: 38400 8N1 default (was 9600 in io.env before this check) |
| UART on Pi 5 | `dtparam=uart0` enables UART0 on GPIO14/15 as `/dev/ttyAMA0` |
| USB current | Pi 5 limits USB to 600 mA without a detected 5 A supply; 1.6 A with 5 A or `usb_max_current_enable=1` (Raspberry Pi white paper) |
| X1200 pins | GPIO6 power-loss detect (low = failed), GPIO16 charge control, fuel gauge 0x36 (Geekworm X1200 hardware page) → LEDs moved off GPIO6 |
| RockBLOCK | 19200 8N1, no auto-baud; ~470 mA max at 5 V (Ground Control / Rock7 docs) |

## 6. Remaining risks (found by the verify script, not by reading)

* Mechanical stacking: X1200 underneath, plus any HAT on top (keep the
  header pins reachable for sensors and LEDs).
* The mini-PCIe → USB adapter's power path during LTE transmit bursts:
  watch `get_throttled` and modem resets in the logs.
* Antenna placement in the actual aircraft (GNSS, 4G and Iridium all need
  sky / outside view; metal fuselages shield heavily).
* `cdc_acm` absence was read from the defconfig only; it doesn't matter for
  this kit (UART GNSS).
