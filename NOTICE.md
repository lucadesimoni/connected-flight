# Notices

**Connected Flight Box (CFB)**: Copyright © 2026 Luca De Simoni, trading as
Outthinkx. All rights reserved. Proprietary software, not open source; the
source may be publicly visible for viewing only. See [LICENSE](LICENSE).

## Third-party trademarks

Pilatus, PC-12, PC-24, Honeywell, Primus Apex, Primus Epic, Garmin, G1000,
Pratt & Whitney Canada, PT6, FAST, Williams, Iridium, Certus, Starlink,
Aspen Avionics, Astronautics, AeroSync, Raspberry Pi, RAUC, WireGuard and
other names are trademarks of their respective owners. They are used only
to describe compatibility or context. Outthinkx is not affiliated with,
endorsed by or sponsored by any of them.

## Third-party data

No proprietary interface control documents (ICDs), label tables or data
formats of any avionics or aircraft manufacturer are included in this
repository. Label numbers marked ILLUSTRATIVE in the example configurations
are placeholders. Real label tables are installation data, used only under
the data owner's licence.

## Third-party software

The software uses open-source Rust crates under permissive licences
(MIT, Apache-2.0, BSD-3-Clause, Unicode-3.0, Unlicense). Their copyright
notices and licence texts must accompany every binary distribution.
Generate the full list for a release with:

```sh
cargo install cargo-about && cargo about generate about.hbs > THIRD_PARTY_LICENSES.html
```

Direct dependencies: `serde`, `toml`, `ed25519-dalek`, `sha2` (and their
transitive dependencies). The OS image adds further components (Linux,
RAUC, WireGuard, systemd, curl) with their own licences, some of them
GPL/LGPL. Those are separate programs and stay under their own licences.
The image's source-offer obligations must be met per release.
