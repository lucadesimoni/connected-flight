# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
# Portable-kit sensors as modules (IIO): ISM330DHCX accelerometer, BMP390 barometer.
FILESEXTRAPATHS:prepend := "${THISDIR}/files:"
SRC_URI += "file://cfb-sensors.cfg"
