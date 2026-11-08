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
| USB GNSS receiver | u-blox M8/M9/M10 USB (shows up as `/dev/ttyACM0`) | Antenna with sky view |
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
| T2 | GNSS input | `cfb-io --gnss /dev/ttyACM0` alone, outdoors | Lines with `nmea=$G?RMC…` and `$G?GGA…` each second; valid fix within ~1 min (cold start may take longer) |
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

## 4. Known limits of the bench unit

* No hardware weight-on-wheels interlock: RF inhibit is software
  (NetworkManager), checked by T11–T13.
* No IMU: engine state comes from the discretes file; no hard-landing or
  turbulence detection.
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
