# CFB system image (Yocto + RAUC A/B)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../../LICENSE).</sub>

> **Status: untested starting point.** Written against Yocto Scarthgap
> (5.0 LTS), meta-raspberrypi, meta-rauc and meta-rauc-community, but not
> built yet (a build needs a Linux machine with ~100 GB disk and several
> hours). Expect a few iterations on the first build; the points to check
> are listed below.

## What it produces

| Output | Purpose |
|---|---|
| `cfb-image-raspberrypi5.wic.bz2` | Complete card / eMMC image: boot, rootfs A, rootfs B, data |
| `cfb-bundle-raspberrypi5.raucb` | Signed RAUC update bundle for the OTA path (`docs/05`) |

Partition layout (`meta-cfb/wic/cfb-dual.wks.in`) matches
`os/rauc/system.conf`: p1 boot, p2 rootfs A, p3 rootfs B, p4 `/data`.
U-Boot with boot counting switches slots; `cfb-mark-good.service` marks a
healthy boot good, otherwise the bootloader falls back.

## Build

```sh
pip install kas
# 1. binaries: from the CI artifact "cfb-aarch64" of the release commit
scripts/yocto-stage-binaries.sh ~/Downloads/cfb-aarch64
# 2. development signing keys (production keys stay on the release token)
scripts/rauc-dev-keys.sh
# 3. image and bundle
kas build os/yocto/kas-cfb.yml
kas build os/yocto/kas-cfb.yml --target cfb-bundle
# flash
bmaptool copy build/tmp/deploy/images/raspberrypi5/cfb-image-raspberrypi5.wic.bz2 /dev/sdX
```

The binaries are **not rebuilt** in Yocto: the image packages the exact CI
binaries (checksums enforced by `cfb-bin.bb`), so the hashes in the
release's SCI are the hashes on the aircraft.

## Layer contents (`meta-cfb`)

| Recipe | Content |
|---|---|
| `cfb-bin` | CI-built binaries, checksum-verified |
| `cfb-config` | systemd units, polkit/udev rules, tmpfiles, default config, `cfb` user, from `os/` and `config/` directly |
| `cfb-dtoverlay` | sensor overlay `os/dt/cfb-portable-overlay.dts` → boot partition |
| `cfb-image` | root filesystem (one slot) |
| `cfb-bundle` | RAUC bundle (verity format) |
| `rauc_%.bbappend` | our `system.conf`, compatible string per hardware part |
| `linux-raspberrypi_%.bbappend` | IIO drivers for ISM330DHCX / BMP390, GPIO LEDs |

## Check on the first build

1. Layer branches and the `meta-rauc-raspberrypi` U-Boot boot script:
   variable names (`RPI_USE_U_BOOT`, boot counting) may differ by layer
   revision; follow the layer README.
2. Pi 5 + U-Boot support in the pinned meta-raspberrypi revision.
3. Package names: `networkmanager`, `modemmanager`, `wireguard-tools`,
   `libgpiod-tools`, `polkit` (meta-oe / meta-networking).
4. `CFB_COMPATIBLE` per hardware part number (portable vs integrated unit).
5. Then pin every repository to a commit (`commit:` in `kas-cfb.yml`) and
   record the kas file in the release SCI.

## Next steps (security, docs/13)

* Read-only root filesystem with `/etc` state (NetworkManager, WireGuard
  keys, tokens) on `/data`.
* dm-verity for the root filesystem; Raspberry Pi secure boot with the
  customer key; signed boot chain.
* `check-crl=true` in `system.conf` with a production CA that publishes a CRL.
