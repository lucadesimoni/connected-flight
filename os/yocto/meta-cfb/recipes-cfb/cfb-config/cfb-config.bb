# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
SUMMARY = "Connected Flight Box OS integration: services, polkit, udev, tmpfiles, default config"
LICENSE = "CLOSED"
inherit systemd useradd

# Files come straight from the repository's os/ and config/ directories, so
# the image and the bench installer use the same sources.
FILESEXTRAPATHS:prepend := "${THISDIR}/../../../../systemd:${THISDIR}/../../../../polkit:${THISDIR}/../../../../udev:${THISDIR}/../../../../../config:"
SRC_URI = "file://cfbd.service file://cfb-maint.service file://cfb-mark-good.service \
           file://cfb.tmpfiles.conf file://discretes.default file://io.env file://maint.env \
           file://50-cfb.rules file://90-cfb.rules file://cfb-portable-pi5.toml"
S = "${WORKDIR}"

USERADD_PACKAGES = "${PN}"
USERADD_PARAM:${PN} = "--system --home /nonexistent --shell /sbin/nologin --groups dialout,gpio cfb"
GROUPADD_PARAM:${PN} = "--system gpio"

SYSTEMD_SERVICE:${PN} = "cfbd.service cfb-maint.service cfb-mark-good.service"
SYSTEMD_AUTO_ENABLE = "enable"

do_install() {
    install -d ${D}${systemd_system_unitdir} ${D}${sysconfdir}/cfb ${D}${sysconfdir}/tmpfiles.d \
               ${D}${sysconfdir}/polkit-1/rules.d ${D}${sysconfdir}/udev/rules.d
    for u in cfbd.service cfb-maint.service cfb-mark-good.service; do
        # Image installs binaries in /usr/bin (bench installer: /usr/local/bin).
        sed 's|/usr/local/bin/|${bindir}/|g' ${WORKDIR}/$u > ${D}${systemd_system_unitdir}/$u
    done
    install -m 0644 ${WORKDIR}/cfb.tmpfiles.conf ${D}${sysconfdir}/tmpfiles.d/cfb.conf
    install -m 0644 ${WORKDIR}/discretes.default ${WORKDIR}/io.env ${WORKDIR}/maint.env ${D}${sysconfdir}/cfb/
    install -m 0644 ${WORKDIR}/cfb-portable-pi5.toml ${D}${sysconfdir}/cfb/cfb.toml
    install -m 0644 ${WORKDIR}/50-cfb.rules ${D}${sysconfdir}/polkit-1/rules.d/
    install -m 0644 ${WORKDIR}/90-cfb.rules ${D}${sysconfdir}/udev/rules.d/
}

FILES:${PN} += "${systemd_system_unitdir} ${sysconfdir}"
RDEPENDS:${PN} += "cfb-bin networkmanager modemmanager wireguard-tools curl rauc libgpiod-tools polkit"
