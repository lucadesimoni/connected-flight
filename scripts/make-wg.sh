#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# Generate WireGuard configs for one box (one tunnel per bearer) and the
# matching peers for the ground server. Run on a trusted machine with
# wireguard-tools installed; keep OUTDIR private (it holds private keys).
#
#   scripts/make-wg.sh <ground-public-ip-or-host> <box-number 2..254> <outdir> [bearers]
#   bearers default: "wifi cell"   (also: leo iri)
#
# Address plan (tunnel inside, /32 per bearer and box):
#   ground 10.66.0.1   wifi 10.66.1.N   cell 10.66.2.N   iri 10.66.3.N   leo 10.66.4.N
# Outer packets of each tunnel carry a fwmark (0x101 wifi, 0x102 cell,
# 0x103 iri, 0x104 leo) and are routed by the bearer's own table (101..104),
# so each tunnel always leaves over its own bearer.
set -eu
GROUND=${1:?ground host}; N=${2:?box number}; OUT=${3:?outdir}; BEARERS=${4:-"wifi cell"}
case "$N" in ''|*[!0-9]*) echo "box number must be 2..254" >&2; exit 2;; esac
[ "$N" -ge 2 ] && [ "$N" -le 254 ] || { echo "box number must be 2..254" >&2; exit 2; }
umask 077
mkdir -p "$OUT/server" "$OUT/box$N"

if [ ! -f "$OUT/server/server.key" ]; then
  wg genkey > "$OUT/server/server.key"
fi
SPUB=$(wg pubkey < "$OUT/server/server.key")
if [ ! -f "$OUT/server/wg0.conf" ]; then
  cat > "$OUT/server/wg0.conf" <<CONF
[Interface]
PrivateKey = $(cat "$OUT/server/server.key")
Address = 10.66.0.1/16
ListenPort = 51820
CONF
fi

for b in $BEARERS; do
  case $b in
    wifi) net=1; mark=0x101; tbl=101; ka=25;;
    cell) net=2; mark=0x102; tbl=102; ka=25;;
    iri)  net=3; mark=0x103; tbl=103; ka=0;;   # metered: no keepalive
    leo)  net=4; mark=0x104; tbl=104; ka=25;;
    *) echo "unknown bearer $b" >&2; exit 2;;
  esac
  key=$(wg genkey); pub=$(printf '%s' "$key" | wg pubkey)
  ip=10.66.$net.$N; rt=$((200 + net))
  ka_line=""; [ "$ka" -gt 0 ] && ka_line="PersistentKeepalive = $ka"
  cat > "$OUT/box$N/wg-$b.conf" <<CONF
# CFB box $N, bearer $b. Install as /etc/wireguard/wg-$b.conf (mode 600).
[Interface]
PrivateKey = $key
Address = $ip/32
Table = off
FwMark = $mark
# Outer (encrypted) packets: fwmark -> bearer table $tbl (filled by NetworkManager).
# The unreachable fallback stops a dead bearer from leaking onto another one.
# Inner traffic from $ip -> table $rt -> this tunnel.
PostUp = ip route replace unreachable default metric 4294967295 table $tbl; ip rule add pref 10$net fwmark $mark lookup $tbl; ip rule add pref 11$net from $ip lookup $rt; ip route add 10.66.0.1/32 dev %i table $rt
PostDown = ip rule del pref 10$net fwmark $mark lookup $tbl; ip rule del pref 11$net from $ip lookup $rt; ip route flush table $rt

[Peer]
PublicKey = $SPUB
Endpoint = $GROUND:51820
AllowedIPs = 10.66.0.1/32
$ka_line
CONF
  cat >> "$OUT/server/wg0.conf" <<CONF

[Peer]
# box $N $b
PublicKey = $pub
AllowedIPs = $ip/32
CONF
  echo "box$N $b: $ip"
done
echo "server public key: $SPUB"
echo "box files:    $OUT/box$N/   -> /etc/wireguard/ on the box"
echo "server file:  $OUT/server/wg0.conf -> /etc/wireguard/wg0.conf on the ground server"
