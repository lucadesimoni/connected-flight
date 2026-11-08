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

apt-get install -y network-manager modemmanager wireguard-tools gpiod curl

id cfb >/dev/null 2>&1 || useradd --system --home /nonexistent --shell /usr/sbin/nologin cfb
getent group gpio >/dev/null || groupadd --system gpio
usermod -aG dialout,gpio cfb

install -m 755 target/release/cfbd target/release/cfb-io target/release/cfb-release /usr/local/bin/
install -d -m 755 /etc/cfb /etc/cfb/apps
[ -f /etc/cfb/cfb.toml ] || install -m 644 config/cfb-bench-pi5.toml /etc/cfb/cfb.toml
[ -f /etc/cfb/io.env ] || install -m 644 os/systemd/io.env /etc/cfb/io.env
[ -f /etc/cfb/discretes.default ] || install -m 644 os/systemd/discretes.default /etc/cfb/discretes.default
install -m 644 os/systemd/cfb.tmpfiles.conf /etc/tmpfiles.d/cfb.conf
install -m 644 os/systemd/cfbd.service os/systemd/cfb-mark-good.service /etc/systemd/system/
install -d -m 755 /etc/polkit-1/rules.d
install -m 644 os/polkit/50-cfb.rules /etc/polkit-1/rules.d/
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
systemctl enable cfbd.service
echo "installed. Next: os/net/setup-networkmanager.sh, edit /etc/cfb/io.env, then: systemctl start cfbd; journalctl -fu cfbd"
