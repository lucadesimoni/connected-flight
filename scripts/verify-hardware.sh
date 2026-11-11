#!/usr/bin/env bash
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# cfb-verify-hardware: checks every component of the portable kit (docs/14)
# against exactly what the CFB software expects. Run on the Pi as root,
# before the first use and after every hardware change:
#
#   sudo cfb-verify-hardware                 # all checks, radios untouched
#   sudo cfb-verify-hardware --radio-test    # also switch 4G / Wi-Fi off and on
#
# Exit status: 0 = all PASS (WARN allowed), 1 = at least one FAIL.
# Every FAIL line says what to check. Nothing here changes the configuration.
set -u

ENV_FILE=${CFB_ENV:-/etc/cfb/io.env}
RADIO_TEST=0
[ "${1:-}" = "--radio-test" ] && RADIO_TEST=1
# shellcheck disable=SC1090
[ -f "$ENV_FILE" ] && { set -a; . "$ENV_FILE"; set +a; }
CFB_GNSS=${CFB_GNSS:-/dev/ttyAMA0}
CFB_GNSS_BAUD=${CFB_GNSS_BAUD:-38400}
CFB_HW=${CFB_HW:-base,gnss,wifi,cellular_4g}
CFB_IMU_IIO=${CFB_IMU_IIO:-}
CFB_BARO_IIO=${CFB_BARO_IIO:-}
CFB_GPIO=${CFB_GPIO:-}

PASS=0; WARN=0; FAIL=0
pass() { PASS=$((PASS+1)); printf '  \033[32mPASS\033[0m %s\n' "$*"; }
warn() { WARN=$((WARN+1)); printf '  \033[33mWARN\033[0m %s\n' "$*"; }
fail() { FAIL=$((FAIL+1)); printf '  \033[31mFAIL\033[0m %s\n' "$*"; }
section() { printf '\n== %s\n' "$*"; }
has() { case ",$CFB_HW," in *",$1,"*) return 0;; *) return 1;; esac; }
have() { command -v "$1" >/dev/null 2>&1; }

[ "$(id -u)" = 0 ] || echo "note: not root; some checks (radio, serial ports, sysfs writes) may fail"

# ---------------------------------------------------------------- platform
section "Platform"
MODEL=$(tr -d '\0' < /proc/device-tree/model 2>/dev/null || echo unknown)
case "$MODEL" in
  *"Raspberry Pi 5"*|*"Compute Module 5"*) pass "board: $MODEL";;
  *) fail "board: '$MODEL' (expected Raspberry Pi 5 or Compute Module 5)";;
esac
KREL=$(uname -r)
KMAJ=$(echo "$KREL" | cut -d. -f1); KMIN=$(echo "$KREL" | cut -d. -f2)
if [ "$KMAJ" -gt 6 ] || { [ "$KMAJ" -eq 6 ] && [ "$KMIN" -ge 12 ]; }; then
  pass "kernel $KREL (>= 6.12: BMP390 and ISM330DHCX supported)"
else
  fail "kernel $KREL too old: BMP390 support needs >= 6.7, tested baseline 6.12 / 6.18 (update Raspberry Pi OS)"
fi
if have gpioget && gpioget --version 2>/dev/null | grep -qE 'v?2\.'; then
  pass "libgpiod 2.x ($(gpioget --version | head -1))"
else
  warn "libgpiod 2.x not found: the GPIO lines in io.env use 2.x syntax (Raspberry Pi OS Trixie has it)"
fi

# ---------------------------------------------------------------- power
section "Power"
if have vcgencmd; then
  T=$(vcgencmd get_throttled | cut -d= -f2)
  if [ "$T" = "0x0" ]; then pass "no undervoltage / throttling since boot ($T)"
  else fail "get_throttled=$T: undervoltage or throttling occurred (bit 0/16 = undervoltage): supply or cable too weak"; fi
  U=$(vcgencmd get_config usb_max_current_enable | cut -d= -f2)
  if [ "${U:-0}" != "0" ]; then pass "USB current limit 1.6 A (usb_max_current_enable=$U)"
  else fail "USB limited to 600 mA: modem + Iridium need usb_max_current_enable=1 (os/dt/config.txt.snippet) with a 5 V / 5 A supply"; fi
else
  fail "vcgencmd missing (raspi-utils): cannot check power"
fi

# ---------------------------------------------------------------- kernel modules
section "Kernel drivers"
for m in st_lsm6dsx_i2c:imu bmp280_i2c:baro qmi_wwan:cellular cdc_mbim:cellular option:cellular ftdi_sio:iridium wireguard:base; do
  mod=${m%%:*}; need=${m##*:}
  if [ "$need" = base ] || has "$need" || { [ "$need" = cellular ] && { has cellular_4g || has cellular_5g; }; }; then
    if modinfo "$mod" >/dev/null 2>&1; then pass "module $mod available"
    else fail "module $mod missing (needed for $need)"; fi
  fi
done

# ---------------------------------------------------------------- sensors
IIO_ROOT=${CFB_IIO_ROOT:-/sys/bus/iio/devices}
iio_dev() { for d in "$IIO_ROOT"/iio:device*; do [ -f "$d/name" ] && grep -q "$1" "$d/name" && { echo "$d"; return 0; }; done; return 1; }
if has imu; then
  section "IMU (accelerometer)"
  if D=$(iio_dev "${CFB_IMU_IIO:-ism330dhcx}"); then
    pass "IIO device $(cat "$D/name") at $D"
    if [ -f "$D/in_accel_scale" ]; then
      G=$(python3 - "$D" <<'PY'
import sys, math, os
d = sys.argv[1]
s = float(open(os.path.join(d, "in_accel_scale")).read())
v = [float(open(os.path.join(d, f"in_accel_{a}_raw")).read()) * s / 9.80665 for a in "xyz"]
print(f"{math.sqrt(sum(x*x for x in v)):.3f} {v[0]:+.2f} {v[1]:+.2f} {v[2]:+.2f}")
PY
)
      MAG=${G%% *}
      if python3 -c "import sys; sys.exit(0 if 0.85 <= $MAG <= 1.15 else 1)"; then
        pass "at rest |a| = $MAG g (x y z: ${G#* }) - set [imu] axes from the mounting"
      else fail "at rest |a| = $MAG g, expected ~1 g: sensor moving, wrong scale or faulty board"; fi
    else fail "no in_accel_scale under $D"; fi
  else
    fail "no IIO device '${CFB_IMU_IIO:-ism330dhcx}': check wiring (I2C 3.3 V, SDA GPIO2, SCL GPIO3), dtoverlay=cfb-portable, address 0x6a (Adafruit) or imu_addr=0x6b (SparkFun)"
    have i2cdetect && echo "       i2cdetect -y 1:" && i2cdetect -y 1 2>/dev/null | sed 's/^/       /'
  fi
fi
if has baro; then
  section "Barometer"
  if D=$(iio_dev "${CFB_BARO_IIO:-bmp}"); then
    P=$(python3 -c "print(f'{float(open(\"$D/in_pressure_input\").read())*10:.1f}')" 2>/dev/null || echo "")
    if [ -n "$P" ] && python3 -c "import sys; sys.exit(0 if 300 <= $P <= 1100 else 1)"; then
      pass "$(cat "$D/name"): $P hPa"
    else fail "$(cat "$D/name"): pressure '$P' hPa implausible"; fi
  else
    fail "no IIO barometer '${CFB_BARO_IIO:-bmp}': wiring, overlay (compatible bosch,bmp380), address 0x77 (or baro_addr=0x76)"
  fi
fi

# ---------------------------------------------------------------- GNSS
if has gnss; then
  section "GNSS ($CFB_GNSS)"
  if [ -e "$CFB_GNSS" ]; then
    RES=$(python3 - "$CFB_GNSS" "$CFB_GNSS_BAUD" <<'PY'
import os, subprocess, sys, time
dev, want = sys.argv[1], sys.argv[2]
def valid(l):
    if not l.startswith("$") or "*" not in l: return False
    body, cs = l[1:].rsplit("*", 1)
    try: return int(cs[:2], 16) == eval("^".join(str(ord(c)) for c in body) or "0")
    except Exception: return False
for baud in [want] + [b for b in ("38400", "9600", "115200") if b != want]:
    subprocess.run(["stty", "-F", dev, baud, "raw", "-echo", "min", "0", "time", "10"], check=False)
    fd = os.open(dev, os.O_RDONLY | os.O_NOCTTY)
    buf, t0 = b"", time.time()
    while time.time() - t0 < 4:
        buf += os.read(fd, 4096) or b""
    os.close(fd)
    lines = [l.strip() for l in buf.decode("ascii", "replace").splitlines()]
    good = [l for l in lines if valid(l)]
    if good:
        rmc = [l for l in good if l[3:6] == "RMC"]
        fix = "fix" if any(l.split(",")[2:3] == ["A"] for l in rmc) else "nofix"
        print(f"OK {baud} {len(good)} {fix}")
        sys.exit(0)
print("NONE")
PY
)
    case "$RES" in
      OK*) read -r _ G_BAUD G_N G_FIX <<< "$RES"
           if [ "$G_BAUD" = "$CFB_GNSS_BAUD" ]; then pass "valid NMEA at $G_BAUD baud ($G_N sentences in 4 s)"
           else fail "valid NMEA only at $G_BAUD baud: set CFB_GNSS_BAUD=$G_BAUD in $ENV_FILE"; fi
           if [ "$G_FIX" = "fix" ]; then pass "GNSS position fix"
           else warn "no position fix yet: antenna needs sky view (cold start can take minutes)"; fi;;
      *) fail "no valid NMEA on $CFB_GNSS at 38400/9600/115200: wiring TX->RX crossed? dtparam=uart0 set? receiver powered (3.3 V)?";;
    esac
  else
    fail "$CFB_GNSS does not exist: dtparam=uart0 in config.txt (UART GNSS) or check the USB device"
  fi
fi

# ---------------------------------------------------------------- cellular
if has cellular_4g || has cellular_5g; then
  section "Cellular modem"
  if have mmcli && M=$(mmcli -L 2>/dev/null | grep -oE '/Modem/[0-9]+' | head -1) && [ -n "$M" ]; then
    INFO=$(mmcli -m "${M##*/}" -K 2>/dev/null)
    get() { echo "$INFO" | grep -E "^$1 " | head -1 | sed 's/^[^:]*: *//'; }
    pass "modem: $(get 'modem.generic.manufacturer') $(get 'modem.generic.model') (fw $(get 'modem.generic.revision'))"
    PORTS=$(get 'modem.generic.ports.value\[1\]')$(echo "$INFO" | grep 'modem.generic.ports' | tr '\n' ' ')
    if echo "$PORTS" | grep -qE 'cdc-wdm[0-9]+ \((qmi|mbim)\)'; then pass "control port via QMI/MBIM (radio switchable by ModemManager)"
    else fail "no QMI/MBIM control port (ports: $PORTS): modem in a router/ECM/RNDIS mode cannot be switched off reliably"; fi
    SIM=$(get 'modem.generic.sim')
    if [ -n "$SIM" ] && [ "$SIM" != "--" ]; then pass "SIM present"; else fail "no SIM detected"; fi
    pass "state: $(get 'modem.generic.state'), signal $(get 'modem.generic.signal-quality.value')%"
    if [ $RADIO_TEST = 1 ] && have nmcli; then
      nmcli radio wwan off; sleep 6
      PS=$(mmcli -m "${M##*/}" -K | grep 'modem.generic.power-state' | sed 's/^[^:]*: *//')
      ST=$(mmcli -m "${M##*/}" -K | grep 'modem.generic.state ' | sed 's/^[^:]*: *//')
      if [ "$PS" = "low" ] || [ "$ST" = "disabled" ]; then pass "radio OFF works (power-state=$PS, state=$ST)"
      else fail "radio OFF not effective (power-state=$PS, state=$ST): RF inhibit would not work"; fi
      nmcli radio wwan on; sleep 3
    else
      warn "radio on/off not tested (run with --radio-test, takes the modem offline briefly)"
    fi
  else
    fail "no modem found by ModemManager: USB connected? (lsusb), ModemManager running? adapter powered?"
  fi
fi

# ---------------------------------------------------------------- Wi-Fi
if has wifi; then
  section "Wi-Fi"
  if ip link show wlan0 >/dev/null 2>&1; then pass "wlan0 present"; else fail "no wlan0"; fi
  if [ $RADIO_TEST = 1 ] && have nmcli; then
    nmcli radio wifi off; sleep 2
    if [ "$(nmcli radio wifi)" = disabled ]; then pass "Wi-Fi radio OFF works"; else fail "Wi-Fi radio OFF not effective"; fi
    nmcli radio wifi on
  fi
fi

# ---------------------------------------------------------------- Iridium
if has iridium; then
  section "Iridium SBD modem"
  DEV=$(grep -A3 '^\[sbd\]' /etc/cfb/cfb.toml 2>/dev/null | grep -oE '"/dev/[^"]+"' | tr -d '"' | head -1)
  DEV=${DEV:-/dev/iridium}
  if [ -e "$DEV" ]; then
    R=$(python3 - "$DEV" <<'PY'
import os, subprocess, sys, time
dev = sys.argv[1]
subprocess.run(["stty", "-F", dev, "19200", "raw", "-echo", "-crtscts", "min", "0", "time", "5"], check=False)
fd = os.open(dev, os.O_RDWR | os.O_NOCTTY)
def cmd(c, wait):
    os.write(fd, (c + "\r").encode()); out, t0 = b"", time.time()
    while time.time() - t0 < wait:
        out += os.read(fd, 512) or b""
        if b"OK" in out or b"ERROR" in out: break
    return out.decode("ascii", "replace")
a = cmd("AT", 3)
c = cmd("AT+CSQ", 60) if "OK" in a else ""
os.close(fd)
csq = c.split("+CSQ:")[1].split()[0] if "+CSQ:" in c else "?"
print(("OK" if "OK" in a else "NOAT") + " " + csq)
PY
)
    case "$R" in
      OK*) pass "AT handshake at 19200 baud"; CSQ=${R#OK }
           if [ "$CSQ" != "?" ] && [ "$CSQ" -ge 2 ] 2>/dev/null; then pass "signal $CSQ/5"
           else warn "signal ${CSQ}/5: antenna needs open sky (indoors 0 is normal)"; fi;;
      *) fail "$DEV does not answer AT at 19200 8N1: adaptor/cable, modem power (needs ~470 mA), udev symlink";;
    esac
  else
    fail "$DEV missing: plug in the RockBLOCK USB adaptor; udev rule os/udev/90-cfb.rules makes /dev/iridium"
  fi
fi

# ---------------------------------------------------------------- GPIO inputs
if [ -n "$CFB_GPIO" ]; then
  section "Discrete inputs (GPIO)"
  IFS=';' read -ra ENTRIES <<< "$CFB_GPIO"
  for e in "${ENTRIES[@]}"; do
    k=${e%%=*}; c=${e#*=}
    # shellcheck disable=SC2086
    if OUT=$($c 2>&1); then pass "$k: '$c' -> $OUT"; else fail "$k: '$c' failed: $OUT"; fi
  done
  echo "       toggle the radio switch and re-run: rfsw must change; unplug the UPS input: pfail must change"
fi

# ---------------------------------------------------------------- LEDs
section "Status LEDs"
for l in green amber red; do
  B=/sys/class/leds/cfb-$l/brightness
  if [ -e "$B" ]; then
    if [ -w "$B" ] || [ "$(id -u)" = 0 ]; then
      echo 1 > "$B" 2>/dev/null; sleep 0.4; echo 0 > "$B" 2>/dev/null
      pass "cfb-$l present (flashed once - did it light?)"
    else fail "cfb-$l not writable (udev rule os/udev/90-cfb.rules)"; fi
  else warn "cfb-$l missing (gpio-led overlays in config.txt; optional on the bench)"; fi
done

# ---------------------------------------------------------------- network + services
section "Tunnels and services"
for t in wg-wifi wg-cell; do
  if ip link show "$t" >/dev/null 2>&1; then
    HS=$(wg show "$t" latest-handshakes 2>/dev/null | awk '{print $2}' | head -1)
    if [ -n "$HS" ] && [ "$HS" != 0 ] && [ $(( $(date +%s) - HS )) -lt 300 ]; then pass "$t: handshake $(( $(date +%s) - HS )) s ago"
    else warn "$t: no recent handshake (bearer down or ground server unreachable)"; fi
  else warn "$t not configured (scripts/make-wg.sh)"; fi
done
for s in cfbd cfb-maint; do
  if systemctl is-active --quiet "$s" 2>/dev/null; then pass "$s running"; else warn "$s not running"; fi
done
if [ -f /run/cfb/status.json ]; then
  AGE=$(( $(date +%s) - $(stat -c %Y /run/cfb/status.json) ))
  [ $AGE -lt 5 ] && pass "status snapshot fresh (${AGE} s)" || warn "status snapshot ${AGE} s old"
fi

# ---------------------------------------------------------------- summary
printf '\n== Result: %d PASS, %d WARN, %d FAIL\n' $PASS $WARN $FAIL
[ $FAIL -eq 0 ]
