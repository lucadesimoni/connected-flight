# Ground service

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

`cfb-ground` (crate `crates/cfb-ground`):

| Part | What it does |
|---|---|
| UDP receiver (in the WireGuard tunnels, `10.66.0.1:7400`) | Acks every frame; stores each once (SQLite); box and link from the tunnel address `10.66.<link>.<box>` |
| Store (`/var/lib/cfb-ground/cfb.db`) | `frames` (all messages, dedup on box+seq+time+type), `positions` (indexed track), `duplicates` (resend counter); schema version for migrations |
| Web view (`127.0.0.1:8090`) | Fleet overview, per-box map (Leaflet + OpenStreetMap tiles) and events; JSON API `/api/boxes`, `/api/track`, `/api/events`; view token |
| SBD webhook (`POST /sbd?token=…`) | Iridium deliveries from the service provider (form fields `imei`, `data` hex, …); box id `imei-<IMEI>`; provider retries are deduplicated |
| OTA package server (`cfb-ota-http`) | Serves `/srv/cfb` inside the tunnel |
| Tools | `--decode-sbd <hex>`, `--encode-sbd "<text>"` (MT payload for the provider API) |

Set up with `scripts/make-wg.sh` and `ground/install-ground.sh` (creates the
view and webhook tokens). Publish the web view and webhook only through a
TLS reverse proxy (Caddy, nginx). Test plan: [docs/10](../docs/10-bench-and-portable-test.md).

Map tiles: the OpenStreetMap public tile server is fine for development;
for a product use a commercial tile provider (OSM tile usage policy).

Still to do for a full product backend: user accounts and roles,
retention policy, alerting (e.g. hard-landing e-mail), file-chunk
reassembly for flight data files, backups.
