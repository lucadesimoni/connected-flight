# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
SUMMARY = "Connected Flight Box binaries (prebuilt by CI, hashed in the SCI)"
DESCRIPTION = "The aarch64 release binaries from the CI build whose trace \
matrix and Software Configuration Index were produced. They are packaged, \
not rebuilt, so the tested binaries are the shipped binaries. \
scripts/yocto-stage-binaries.sh copies them here and writes cfb-bin.sha256."
LICENSE = "CLOSED"

SRC_URI = "file://cfbd file://cfb-io file://cfb-maint file://cfb-release file://cfb-bin.sha256"
S = "${WORKDIR}"

do_compile() {
    # Refuse binaries that don't match the staged checksums.
    cd ${WORKDIR} && sha256sum -c cfb-bin.sha256
}

do_install() {
    install -d ${D}${bindir}
    for b in cfbd cfb-io cfb-maint cfb-release; do
        install -m 0755 ${WORKDIR}/$b ${D}${bindir}/$b
    done
}

INSANE_SKIP:${PN} += "already-stripped ldflags"
RDEPENDS:${PN} += "glibc"
