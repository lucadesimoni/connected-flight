# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
SUMMARY = "Connected Flight Box RAUC update bundle"
LICENSE = "CLOSED"
inherit bundle

RAUC_BUNDLE_COMPATIBLE = "${CFB_COMPATIBLE}"
RAUC_BUNDLE_FORMAT = "verity"
RAUC_BUNDLE_VERSION = "${CFB_SW_VERSION}"
RAUC_BUNDLE_DESCRIPTION = "CFB-SW ${CFB_SW_VERSION}"
RAUC_BUNDLE_SLOTS = "rootfs"
RAUC_SLOT_rootfs = "cfb-image"
RAUC_SLOT_rootfs[fstype] = "ext4"
CFB_SW_VERSION ?= "0.1.0"
