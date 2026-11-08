# 05 – Field-Loadable Software Update Process

## Release (design organisation)

1. Change request → change classification (minor/major) under the DOA
   procedure. Typical CFB software changes are minor (see 02 §7).
2. Build the image from a tagged commit in a reproducible build; produce the
   **Software Configuration Index** (SCI: source tag, toolchain, package
   list, hashes).
3. Verification per SVP (Level D): `cargo test`, simulation, target tests,
   regression of all HLR tests.
4. Create the RAUC bundle, sign it with the release CA (X.509).
5. Write `manifest.toml` (part number, version, bundle SHA-256, size,
   compatible hardware P/N) and sign it:
   `cfb-release sign release.key manifest.toml` → `manifest.toml.sig`.
6. Issue a new **ASCI** (Approved Software Configuration Index) listing the
   approved P/N + version for each hardware P/N, sign it:
   `cfb-release sign release.key asci.toml`.
7. Publish the package: bundle + manifest + ASCI + signatures + release notes
   / Service Bulletin.

Keys: release keys stay on a hardware token in the DOA; the file-based
`cfb-release` is for development only.

## Delivery to the aircraft

* **Over the air** on ground (cellular / hangar Wi-Fi): the package is
  downloaded in the background into `/data/updates`. Downloading is allowed
  any time on ground; **installing** is not automatic.
* **Locally**: USB stick or laptop on the maintenance port.

## Installation (maintenance task per ICA)

The update gate (`cfb-core::update`) shows on the maintenance page and
allows install only if all conditions hold:

| Condition | Source |
|---|---|
| Phase = OnGround (confirmed) | WoW + GNSS |
| Engine confirmed off | oil pressure switch |
| Ground speed ≤ 2 kt | GNSS |
| Maintenance mode | key switch / technician login |
| Supply ≥ configured minimum | bus voltage (ground power recommended) |
| ASCI signature valid, entry for this HW P/N | Ed25519 |
| Manifest signature valid, P/N + version in ASCI, no unapproved downgrade | Ed25519 |
| Bundle hash and size match manifest | SHA-256, streamed |
| RAUC bundle signature valid, `compatible` matches | RAUC (independent) |

Then:

1. RAUC writes the inactive slot (A→B or B→A). The running system keeps
   working.
2. Reboot into the new slot with a boot counter (e.g. 3 attempts).
3. `cfb-mark-good.service` waits 60 s, checks `cfbd` is healthy, then
   `rauc status mark-good`.
4. If the new slot does not get marked good (crash, watchdog reset), the
   bootloader returns to the old slot automatically.
5. Technician checks the maintenance page shows the new P/N + version,
   performs the functional test from the ICA (phase indication, link status,
   test position report), and makes the logbook entry / release to service.

## Interruptions

| Event | Result |
|---|---|
| Power loss during download | Partial file discarded, re-download (hash check) |
| Power loss during slot write | Old slot still active and marked good; retry |
| New software fails to start | Watchdog/boot counter → old slot |
| Wrong package for HW | Rejected (manifest `compatible_hw`, RAUC `compatible`) |
| Engine started during install | Install continues (inactive slot only, no effect on running system); the reboot shall be deferred until the gate is open again (design requirement, not yet implemented) |

## Records

Each install writes an entry to `/data/cfb/update.log` (time, old and new
P/N/version, result) and reports it to the ground service, so the CAMO has
the installed configuration of every aircraft without asking.
