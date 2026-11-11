#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# Install the CFB on a Raspberry Pi 5 (Raspberry Pi OS Bookworm, 64-bit) for
# bench / portable use. Run from the repository root as root, after
# `cargo build --release`. Idempotent; existing /etc/cfb files are kept.
#
#   sudo scripts/install-box.sh
set -eu
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
[ -x target/release/cfbd ] || { echo "build first: cargo build --release" >&2; exit 1; }

apt-get install -y network-manager modemmanager wireguard-tools gpiod curl device-tree-compiler

# Sensor overlay (portable kit); enable it in config.txt (os/dt/config.txt.snippet)
if [ -d /boot/firmware/overlays ]; then
  dtc -@ -I dts -O dtb -o /boot/firmware/overlays/cfb-portable.dtbo os/dt/cfb-portable-overlay.dts
fi

id cfb >/dev/null 2>&1 || useradd --system --home /nonexistent --shell /usr/sbin/nologin cfb
getent group gpio >/dev/null || groupadd --system gpio
usermod -aG dialout,gpio cfb

install -m 755 target/release/cfbd target/release/cfb-io target/release/cfb-release target/release/cfb-maint /usr/local/bin/
install -m 755 scripts/verify-hardware.sh /usr/local/bin/cfb-verify-hardware
install -d -m 755 /etc/cfb /etc/cfb/apps
[ -f /etc/cfb/cfb.toml ] || install -m 644 config/cfb-bench-pi5.toml /etc/cfb/cfb.toml
# io.env: bench default; portable kit: CFB_KIT=portable sudo -E scripts/install-box.sh
if [ ! -f /etc/cfb/io.env ]; then
  if [ "${CFB_KIT:-}" = portable ]; then
    install -m 644 os/systemd/io.env.portable-kit /etc/cfb/io.env
    install -m 644 config/cfb-portable-pi5.toml /etc/cfb/cfb.toml
  else
    install -m 644 os/systemd/io.env /etc/cfb/io.env
  fi
fi
[ -f /etc/cfb/discretes.default ] || install -m 644 os/systemd/discretes.default /etc/cfb/discretes.default
[ -f /etc/cfb/maint.env ] || install -m 644 os/systemd/maint.env /etc/cfb/maint.env
if [ ! -f /etc/cfb/maint.token ]; then
  # Random maintenance token, readable by the service only. Hand it to the
  # technicians through your maintenance documentation, not in the clear.
  head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n' > /etc/cfb/maint.token
  chown root:cfb /etc/cfb/maint.token && chmod 640 /etc/cfb/maint.token
fi
install -m 644 os/systemd/cfb.tmpfiles.conf /etc/tmpfiles.d/cfb.conf
install -m 644 os/systemd/cfbd.service os/systemd/cfb-mark-good.service os/systemd/cfb-maint.service /etc/systemd/system/
install -d -m 755 /etc/polkit-1/rules.d
install -m 644 os/polkit/50-cfb.rules /etc/polkit-1/rules.d/
install -m 644 os/udev/90-cfb.rules /etc/udev/rules.d/
udevadm control --reload || true
systemd-tmpfiles --create /etc/tmpfiles.d/cfb.conf
systemctl daemon-reload

for t in wifi cell; do
  if [ -f /etc/wireguard/wg-$t.conf ]; then
    chmod 600 /etc/wireguard/wg-$t.conf
    systemctl enable --now wg-quick@wg-$t
  else
    echo "note: /etc/wireguard/wg-$t.conf missing (scripts/make-wg.sh)"
  fi
done
systemctl enable cfbd.service cfb-maint.service
echo "installed. Next: os/net/setup-networkmanager.sh, edit /etc/cfb/io.env, then: systemctl start cfbd; journalctl -fu cfbd"
