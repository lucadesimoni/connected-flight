# Ground server

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

Minimal ground side for the bench / portable phase:

* WireGuard hub `wg0` at 10.66.0.1 (UDP 51820 public, nothing else);
* `cfb-ground`: receives CFB frames, acks them, appends them to
  `/var/lib/cfb-ground/rx.log` (tab-separated: received unix time, sender,
  frame time, seq, type, decoded body);
* `cfb-ota-http`: serves `/srv/cfb` inside the tunnel for OTA downloads.

Set up with `scripts/make-wg.sh` and `ground/install-ground.sh`. The test
plan is in [docs/10](../docs/10-bench-and-portable-test.md).

This is a reference service, not a production backend: no database, no
deduplication, no user access, no monitoring.
