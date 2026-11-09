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
install -d -m 755 /etc/cfb-ground
for t in view webhook; do
  if [ ! -f /etc/cfb-ground/$t.token ]; then
    head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n' > /etc/cfb-ground/$t.token
    chmod 644 /etc/cfb-ground/$t.token   # DynamicUser must read it; the directory is root-owned
  fi
done
install -m 644 ground/cfb-ground.service ground/cfb-ota-http.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now wg-quick@wg0 cfb-ground cfb-ota-http
echo "Open UDP 51820 in the firewall (only that port needs to be public)."
echo "Web (localhost): http://127.0.0.1:8090/?token=\$(cat /etc/cfb-ground/view.token)"
echo "SBD webhook URL for the provider (via your TLS proxy): https://<host>/sbd?token=\$(cat /etc/cfb-ground/webhook.token)"
echo "Database: /var/lib/cfb-ground/cfb.db   Log: /var/lib/cfb-ground/rx.log   Live: journalctl -fu cfb-ground"
