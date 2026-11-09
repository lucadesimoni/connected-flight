#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# Copy the aarch64 release binaries (CI artifact "cfb-aarch64" of the commit
# being released) into the cfb-bin recipe, with their checksums. The image
# then contains exactly the binaries listed in that commit's SCI.
#   scripts/yocto-stage-binaries.sh <dir-with-binaries>
set -eu
SRC=${1:?directory with cfbd cfb-io cfb-maint cfb-release (aarch64)}
DST=os/yocto/meta-cfb/recipes-cfb/cfb-bin/files
mkdir -p "$DST"
for b in cfbd cfb-io cfb-maint cfb-release; do
  file "$SRC/$b" 2>/dev/null | grep -q aarch64 || { echo "$SRC/$b is missing or not aarch64" >&2; exit 1; }
  install -m 0755 "$SRC/$b" "$DST/$b"
done
( cd "$DST" && sha256sum cfbd cfb-io cfb-maint cfb-release > cfb-bin.sha256 && cat cfb-bin.sha256 )
echo "compare these hashes with the SCI (target/evidence/sci.md) of the release commit"
