# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
# Our slot layout and compatible string instead of the layer default.
FILESEXTRAPATHS:prepend := "${THISDIR}/../../../../rauc:"
SRC_URI += "file://system.conf"
do_install:append() {
    install -m 0644 ${WORKDIR}/system.conf ${D}${sysconfdir}/rauc/system.conf
    sed -i 's|^compatible=.*|compatible=${CFB_COMPATIBLE}|; s|^statusfile=.*|statusfile=/data/rauc.status|' ${D}${sysconfdir}/rauc/system.conf
}
