# 10 – Bench Setup and Test Plan (Raspberry Pi 5, Wi-Fi + 4G)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

Goal: a real Pi 5 that connects to your own ground server over Wi-Fi and 4G
through WireGuard, delivers position and status messages with end-to-end
acks, switches its radios off whenever on-ground is not confirmed, and
survives link loss, server loss and power cuts. After that the same unit can
be carried as a portable device (section 5).

## 1. Hardware and accounts

| Item | Example | Note |
|---|---|---|
| Raspberry Pi 5 (4 GB) + 27 W USB-C PSU + cooler | – | Raspberry Pi OS **Bookworm 64-bit Lite** |
| microSD 32 GB (industrial preferred) or NVMe HAT | – | |
| GNSS receiver | u-blox NEO-M9N / MAX-M10S breakout on the UART (GPIO14/15, `/dev/ttyAMA0`, 38400 baud); exact parts in docs/14 | Antenna with sky view. UART, not USB: no USB-ACM driver dependency |
| USB 4G modem in **QMI/MBIM mode** | Quectel EG25-G / EM06 in a USB adapter, Sierra EM74xx | Avoid "HiLink" sticks (they act as a router; ModemManager can't control the radio). Data SIM, APN known |
| Ethernet cable | – | SSH over Ethernet: the Wi-Fi radio is switched off until the unit sees "on ground" |
| Small Linux VPS with a public IPv4 | any provider, Debian 12 | Only UDP 51820 open |

## 2. Install

### 2.1 Ground server (VPS)

```sh
# on a trusted machine with wireguard-tools:
scripts/make-wg.sh <VPS-public-IP> 2 ./wg-keys          # box number 2, bearers wifi + cell
# on the VPS (repo checked out, Rust installed, cargo build --release):
sudo ground/install-ground.sh ./wg-keys/server/wg0.conf
```

### 2.2 Box (Pi 5)

```sh
curl https://sh.rustup.rs -sSf | sh       # Rust, then: source ~/.cargo/env
git clone <repo> && cd connected-flight && cargo build --release
sudo install -m 600 ./wg-keys/box2/wg-wifi.conf ./wg-keys/box2/wg-cell.conf /etc/wireguard/
sudo scripts/install-box.sh
sudo os/net/setup-networkmanager.sh wifi "<SSID>" "<PSK>"
sudo os/net/setup-networkmanager.sh cell "<APN>"
sudo cfb-release keygen /root/cfb-release.key   # put the printed key into /etc/cfb/cfb.toml [update] trusted_keys
sudo nano /etc/cfb/io.env                         # GNSS device, module list
echo "wow=1 eng=0 maint=0 v=13.0" | sudo tee /run/cfb/discretes   # bench: "on ground"
sudo systemctl start cfbd && journalctl -fu cfbd
```

## 3. Tests

Record each result (pass/fail, notes, date, software commit).

| # | Test | Steps | Expected |
|---|---|---|---|
| T1 | Build & unit tests on target | `cargo test` on the Pi | All pass |
| T2 | GNSS input | `cfb-verify-hardware`, then `cfb-io --gnss /dev/ttyAMA0` alone, outdoors | Lines with `nmea=$G?RMC…` and `$G?GGA…` each second; valid fix within ~1 min (cold start may take longer) |
| T3 | Tunnels up | `sudo wg show` | Recent handshake on both `wg-wifi` and `wg-cell` |
| T4 | Bearer pinning | `ping -I 10.66.1.2 10.66.0.1` and `-I 10.66.2.2`; `sudo tcpdump -ni wlan0 udp port 51820` and `-ni wwan0` | Each ping works; wlan0 carries only the wg-wifi outer traffic, wwan0 only wg-cell |
| T5 | No leak when a bearer dies | Unplug the 4G modem; `ip route show table 102` | Only `unreachable default`; wg-cell traffic does **not** appear on wlan0/eth0 |
| T6 | Start-up inhibit | Restart cfbd with `wow=1`; watch log and `nmcli radio` | Wi-Fi + WWAN **off** at start; `phase Unknown -> OnGround` after ~20 s, then RF ENABLED, radios on |
| T7 | Delivery + ack | Wait 2 min; on the VPS `tail /var/lib/cfb-ground/rx.log` | Position + health lines arrive; box queue stays near empty |
| T8 | Link choice + failover | Turn off the Wi-Fi AP | Within `down_after_s` (45 s) traffic continues over cellular; AP back on → back to Wi-Fi (cheaper) |
| T9 | Ground server outage | `systemctl stop cfb-ground` for 5 min, then start | Messages queue on the box, resend every 15 s, all arrive after restart (duplicates possible: same seq + frame time) |
| T10 | Power cut | Pull power, boot again | Queue restored from `/data/cfb/queue.bin`; at most the last 60 s (snapshot interval) lost |
| T11 | **Flight inhibit** | `echo "wow=0 eng=1 maint=0 v=13.0" > /run/cfb/discretes` | Within ~5 s: phase Airborne, Wi-Fi + WWAN **off** (`nmcli radio`, `mmcli -m any`) |
| T12 | Inhibit re-assert | While "airborne", run `nmcli radio wwan on` by hand | Radio forced off again within 60 s |
| T13 | GNSS-only phase (portable) | `wow=-`; unplug GNSS | After 10 s phase Unknown → radios off. Plug back, stationary → on after 20 s |
| T14 | Watchdog | `sudo kill -STOP $(pidof cfbd)` | systemd restarts the service after ~10 s |
| T15 | OTA download + verify | Generate a test ASCI/manifest/bundle signed with your key into `/srv/cfb/CFB-HW-BENCH/`; run `cfb-ground --announce-ota` once | Box logs `OTA Downloading` → `Verified`; a tampered bundle → `Failed: verification`. *Install needs a RAUC A/B image; on stock Raspberry Pi OS stop at Verified.* |
| T16 | Data usage | `vnstat` or modem counters for 1 h idle on 4G | Note bytes/h (WireGuard keepalive 25 s + cfbd keepalive 30 s) |

### 3.1 Portable kit additions (config `config/cfb-portable-pi5.toml`)

Hardware set-up: wire the sensors and LEDs, append
`os/dt/config.txt.snippet` to `/boot/firmware/config.txt` (the installer
builds the overlay), reboot, check `cat /sys/bus/iio/devices/*/name`.
In `/etc/cfb/io.env`: `CFB_IMU_IIO=ism330dhcx`, `CFB_BARO_IIO=bmp`,
`CFB_HW=base,gnss,imu,baro,wifi,cellular_4g`, and the `CFB_GPIO` lines for
`rfsw` / `pfail`.

| # | Test | Steps | Expected |
|---|---|---|---|
| T17 | IMU live | `cfb-io --imu-iio ism330dhcx` alone | `imu=` token with ~100 samples per line; flat on a table: z ≈ −1 g (axes per mounting) |
| T18 | Engine from vibration | Unit on a running engine / vibrating surface (eng discrete `-`) | `REF … engine=OwnImu`; engine on after ~5 s, off ~10 s after stopping |
| T19 | Radio switch | Toggle the switch OFF / ON | `RF switch OFF`, 4G inhibited (`nmcli radio` / modem power), amber LED; ON → 4G back. Disconnect the switch sense wire → treated as OFF |
| T20 | Power fail | Pull the UPS input power | `POWER FAIL`, red LED, queue snapshot; after 5 s clean power-off; on restore before 5 s: `POWER RESTORED`, no shutdown |
| T21 | Barometer | Compare reported altitude with the airfield elevation and QNH | Within the QNH difference (altitude is pressure altitude, standard setting) |
| T22 | Status lights | Walk through: start-up, ground + link, switch OFF, GNSS unplugged (30 s) | Matches the table in `cfb-core/src/status.rs` |
| T23 | Iridium SBD | Modem with sky view, `[sbd]` configured; `wow=0` in the discretes file (airborne) | Position reports leave every interval via SBD (provider portal shows messages, ~2 frames per message); `cfb-ground --decode-sbd <hex>` decodes them |
| T24 | SBD bad conditions | Cover the antenna | Signal < 2: frames held, no credits used; uncovered: they go out |

### 3.2 Maintenance page

The installer creates a random token in `/etc/cfb/maint.token` (show it
with `sudo cat`); set the maintenance Ethernet address in
`/etc/cfb/maint.env` (laptop on the same subnet).

| # | Test | Steps | Expected |
|---|---|---|---|
| T25 | Status page | Open `http://<maint-address>:8080/` | Phase, position/altitude with sources, lights, faults, links, queue, OTA state, update gate; refreshes every 3 s |
| T26 | Maintenance mode | Button with a wrong token, then the right one | Wrong: "Wrong token" after 1 s, nothing changes. Right: maintenance mode ON within a second; update gate shows the remaining blockers |
| T27 | Install confirm | With a verified package waiting (T15) and the gate open | "Install confirmed"; OTA goes to Installing |
| T28 | Not reachable elsewhere | From Wi-Fi / 4G / tunnel addresses | Connection refused (bound to the maintenance address only) |

## 4. Known limits of the bench unit

* No hardware weight-on-wheels interlock: RF inhibit is software
  (NetworkManager), checked by T11–T13.
* Bench build without IMU: engine state comes from the discretes file (the
  portable kit config adds the IMU and barometer).
* No RAUC A/B image yet: OTA stops after verification.
* Not environmentally tested, no approval: **not an aircraft installation**.

## 5. Carrying it as a portable device (data collection)

Only after T6, T11, T12 and T13 pass. Not legal advice; check the rules
for your operation.

* **Private (non-commercial) flights:** the PIC decides whether a portable
  electronic device may be used and must make sure it does not interfere
  with aircraft systems (EASA Part-NCO). For commercial operations, the
  operator's PED procedures apply. Don't use it there without them.
* **Configuration:** `wow=-` in `/run/cfb/discretes` (no aircraft input).
  The phase then comes from GNSS: below 35 kt for 20 s = ground (radios
  may be on), above 50 kt for 5 s = airborne (radios off). Any GNSS loss =
  radios off.
* **Belt and braces:** until the hardware interlock exists, also put the
  4G modem in a state that cannot use the terrestrial network in flight.
  Either remove its USB plug before engine start, or use a SIM data profile
  you disable. Wi-Fi client use in the cabin follows normal PED rules.
* **Mounting/power:** secured so it cannot become a loose object; powered
  from a power bank you are allowed to carry, not from the aircraft. Keep it
  away from the magnetic compass.
* **First flights:** check the avionics for any interference: GNSS, VHF
  com/nav, autopilot. If there's any doubt, switch it off.

What you get: the in-flight GNSS track (queued in flight, uploaded after
landing), ground-link performance at real airfields (Wi-Fi/4G coverage,
upload times), and data for the edge apps (add an IMU module later).
