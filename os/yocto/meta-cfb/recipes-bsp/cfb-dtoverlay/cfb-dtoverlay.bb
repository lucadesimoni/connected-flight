# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
SUMMARY = "CFB sensor device-tree overlay (os/dt/cfb-portable-overlay.dts) for the boot partition"
LICENSE = "CLOSED"
inherit deploy
DEPENDS = "dtc-native"
FILESEXTRAPATHS:prepend := "${THISDIR}/../../../../dt:"
SRC_URI = "file://cfb-portable-overlay.dts"
S = "${WORKDIR}"
do_compile() {
    dtc -@ -I dts -O dtb -o ${B}/cfb-portable.dtbo ${WORKDIR}/cfb-portable-overlay.dts
}
do_deploy() {
    install -m 0644 ${B}/cfb-portable.dtbo ${DEPLOYDIR}/cfb-portable.dtbo
}
addtask deploy after do_compile before do_build
