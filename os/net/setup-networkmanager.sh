#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# NetworkManager profiles for the CFB bearers (Raspberry Pi OS Bookworm).
# Each bearer's routes go into its own table (wifi 101, cellular 102) so the
# WireGuard tunnels can be pinned to it. System traffic (apt, NTP) uses
# main first (e.g. Ethernet on the bench), then Wi-Fi, then cellular.
#
#   sudo os/net/setup-networkmanager.sh wifi "<SSID>" "<PSK>"     # repeat per hangar/FBO network
#   sudo os/net/setup-networkmanager.sh cell "<APN>" [PIN]
set -eu
kind=${1:?wifi|cell}
case "$kind" in
  wifi)
    ssid=${2:?SSID}; psk=${3:?PSK}
    name="cfb-wifi-$(printf '%s' "$ssid" | tr -c 'A-Za-z0-9' '_')"
    nmcli connection delete "$name" >/dev/null 2>&1 || true
    nmcli connection add type wifi ifname wlan0 con-name "$name" ssid "$ssid" \
      wifi-sec.key-mgmt wpa-psk wifi-sec.psk "$psk" \
      ipv4.route-table 101 ipv4.routing-rules "priority 32800 table 101" \
      ipv6.method disabled connection.autoconnect yes
    echo "added $name (table 101)";;
  cell)
    apn=${2:?APN}; pin=${3:-}
    nmcli connection delete cfb-cell >/dev/null 2>&1 || true
    nmcli connection add type gsm ifname '*' con-name cfb-cell gsm.apn "$apn" \
      ${pin:+gsm.pin "$pin"} \
      ipv4.route-table 102 ipv4.routing-rules "priority 32801 table 102" \
      ipv6.method disabled connection.autoconnect yes
    echo "added cfb-cell (table 102)";;
  *) echo "usage: $0 wifi SSID PSK | cell APN [PIN]" >&2; exit 2;;
esac
