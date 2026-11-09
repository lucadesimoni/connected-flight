#!/bin/sh
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
#
# DEVELOPMENT RAUC signing material for os/yocto (CA + one signing cert).
# Production: the CA and signing keys live on the release token / HSM of
# the design organisation, never in a build tree (docs/05, docs/cert/SCMP).
#   scripts/rauc-dev-keys.sh
set -eu
D=os/yocto/keys
mkdir -p "$D"; umask 077
[ -f "$D/ca.key.pem" ] && { echo "$D already has keys; refusing to overwrite"; exit 1; }
openssl req -x509 -newkey rsa:4096 -nodes -days 3650 -subj "/O=Outthinkx/CN=CFB development CA" \
  -keyout "$D/ca.key.pem" -out "$D/ca.cert.pem"
openssl req -newkey rsa:4096 -nodes -subj "/O=Outthinkx/CN=CFB development signing 1" \
  -keyout "$D/development-1.key.pem" -out "$D/development-1.csr.pem"
openssl x509 -req -days 1825 -in "$D/development-1.csr.pem" -CA "$D/ca.cert.pem" -CAkey "$D/ca.key.pem" \
  -CAcreateserial -out "$D/development-1.cert.pem"
echo "development keys in $D (git-ignored). Do NOT use for anything that flies."
