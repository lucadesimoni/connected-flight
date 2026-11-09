#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# The quality gate (same as CI). Run before every commit; it stops at the
# first failure.   scripts/check.sh
set -eu
cd "$(dirname "$0")/.."
echo "== clippy"; cargo clippy --all-targets -q -- -D warnings
echo "== tests";  cargo test -q 2>&1 | tee /dev/stderr | grep -q "test result" || { echo "no tests ran"; exit 1; }
if cargo test -q 2>&1 | grep -qE "FAILED|panicked"; then echo "TEST FAILURE"; exit 1; fi
echo "== simulations"
cargo build --release -q
for c in "config/cfb.toml" "config/cfb.toml --hard-landing" "config/cfb-minimal.toml" "config/cfb-pc12-example.toml --apps config/apps"; do
  # shellcheck disable=SC2086
  ./target/release/cfbd --config $c --simulate > /dev/null
done
echo "== all checks passed"
