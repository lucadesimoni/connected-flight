#!/usr/bin/env bash
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# Load test (docs/17): worst-case bus + edge + storm + flood on one core,
# then end-to-end UDP with a ground outage. Run on the target (Pi 5 / CM5)
# after `cargo build --release`:   scripts/loadtest.sh [work-dir]
# Pass criteria are checked; exit 1 on any failure.
set -euo pipefail
cd "$(dirname "$0")/.."
W=${1:-$(mktemp -d)}
B=target/release
FAILS=0
check() { if eval "$2"; then echo "  PASS $1"; else echo "  FAIL $1"; FAILS=$((FAILS+1)); fi; }
PIN=""; command -v taskset >/dev/null && PIN="taskset -c 1"

echo "== platform: $(tr -d "\0" 2>/dev/null < /proc/device-tree/model || grep -m1 'model name' /proc/cpuinfo | cut -d: -f2)"

echo "== A+B: 1 h at the bus maximum, 128 rules, storm, flood (as fast as possible, one core)"
$B/cfb-loadgen setup --dir "$W/ab" --labels 64 --channels 4 --rules 128 2>/dev/null
T0=$(date +%s.%N)
$B/cfb-loadgen run --config "$W/ab/loadtest.toml" --seconds 3600 --words-per-sec 11000 --imu-hz 100 \
  --storm 1200:30 --flood 2400:60 2>/dev/null | $PIN $B/cfbd --config "$W/ab/loadtest.toml" > "$W/ab.out" 2>&1
T1=$(date +%s.%N)
grep -E "^tick work|^peak memory|^ARINC|^input lines" "$W/ab.out"
echo "  wall time $(python3 -c "print(round($T1-$T0,1))") s for 3600 simulated s"
P99=$(grep '^tick work' "$W/ab.out" | sed -E 's/.*p99 ([0-9]+) us.*/\1/')
MAX=$(grep '^tick work' "$W/ab.out" | sed -E 's/.*max ([0-9]+) us.*/\1/')
check "p99 tick work < 100 ms (10 % of the 1 s budget): ${P99} us" "[ $P99 -lt 100000 ]"
check "max tick work < 500 ms: ${MAX} us" "[ $MAX -lt 500000 ]"
check "storm: 128 exceedance starts delivered" "[ \$(grep -c 'EXC .* START' $W/ab.out) -eq 128 ]"
check "storm: 128 exceedance ends delivered" "[ \$(grep -c 'EXC .* END' $W/ab.out) -eq 128 ]"
check "no ARINC parity errors from cut lines" "grep -q '0 parity errors' $W/ab.out"
check "no GNSS fault during the flood" "! awk -F'[][s]' '{t=\$2+0} t>=2400 && t<=2470' $W/ab.out | grep -q Gnss"

echo "== C: end to end over UDP, ground outage during a storm (50x time compression)"
$B/cfb-loadgen setup --dir "$W/c" --labels 64 --channels 4 --rules 128 --transport 127.0.0.1:7600 2>/dev/null
G="$B/cfb-ground --listen 127.0.0.1:7600 --db $W/c/ground.db"
$G > /dev/null 2>&1 & GP=$!
sleep 0.5
( $B/cfb-loadgen run --config "$W/c/loadtest.toml" --seconds 900 --words-per-sec 11000 --pace-ms 20 \
    --ground-only --storm 250:30 2>/dev/null | $B/cfbd --config "$W/c/loadtest.toml" > "$W/c.out" 2>&1 ) & BP=$!
sleep 4; kill $GP; sleep 4
$G > /dev/null 2>&1 & GP=$!
wait $BP; sleep 1; kill $GP
R=$(python3 - "$W/c/ground.db" <<'PY'
import sqlite3, sys
db = sqlite3.connect(sys.argv[1]); q = lambda s: db.execute(s).fetchone()[0]
print(q("select count(*) from frames where body like '%EXC % START%'"), q("select count(*) from frames where body like '%EXC % END%'"), q("select count(*) from positions"))
PY
)
read -r S E P <<< "$R"
echo "  ground DB: EXC START $S, END $E, positions $P"
check "all 128 storm starts reached the ground after the outage" "[ $S -eq 128 ]"
check "all 128 storm ends reached the ground" "[ $E -eq 128 ]"
check "queue empty at the end" "grep -q 'still queued: 0 messages' $W/c.out"

echo "== result: $FAILS failure(s); work dir $W"
[ $FAILS -eq 0 ]
