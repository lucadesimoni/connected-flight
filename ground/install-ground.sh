#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# Ground server (Debian/Ubuntu VPS). Run as root from the repository root,
# after `cargo build --release` and with wg0.conf from scripts/make-wg.sh.
#
#   sudo ground/install-ground.sh /path/to/server/wg0.conf
set -eu
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
WG=${1:?path to wg0.conf}
apt-get install -y wireguard-tools python3
install -m 600 "$WG" /etc/wireguard/wg0.conf
install -m 755 target/release/cfb-ground /usr/local/bin/
install -d -m 755 /srv/cfb
install -m 644 ground/cfb-ground.service ground/cfb-ota-http.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now wg-quick@wg0 cfb-ground cfb-ota-http
echo "Open UDP 51820 in the firewall (only that port needs to be public)."
echo "Log: /var/lib/cfb-ground/rx.log   Live: journalctl -fu cfb-ground"
