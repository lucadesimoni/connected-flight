# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
SUMMARY = "Connected Flight Box root filesystem (one A/B slot)"
LICENSE = "CLOSED"
inherit core-image

IMAGE_INSTALL = "packagegroup-core-boot cfb-config cfb-bin rauc openssh-sshd \
                 kernel-module-st-lsm6dsx-i2c kernel-module-bmp280-i2c libubootenv-bin"
IMAGE_FEATURES += "ssh-server-openssh"
# Next steps (docs/13 open items): read-only-rootfs with /etc overlays for
# NetworkManager/WireGuard state on /data, dm-verity, secure boot.
IMAGE_ROOTFS_EXTRA_SPACE = "131072"
