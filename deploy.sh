#!/usr/bin/env bash
set -euo pipefail

VERSION=$(cargo metadata --no-deps --format-version=1 -q \
  | nu --stdin -c 'from json | get packages.0.version')

BINARY="game-launcher-${VERSION}-linux-x86_64"
REMOTE_DIR="/data/osso-website/downloads"

echo "Building game-launcher v${VERSION}..."
cargo build --release

cp target/release/game-launcher "$BINARY"
SHA256=$(sha256sum "$BINARY" | cut -d' ' -f1)

echo "Writing launcher.json..."
nu -c "{version: \"${VERSION}\", file: \"${BINARY}\", sha256: \"${SHA256}\"} | to json" > launcher.json

echo "Uploading to sakuin:${REMOTE_DIR}..."
ssh sakuin "mkdir -p ${REMOTE_DIR} && rm -f ${REMOTE_DIR}/game-launcher-*"
scp "$BINARY" launcher.json "sakuin:${REMOTE_DIR}/"

rm "$BINARY"

echo "Done. v${VERSION} deployed."
echo "  https://worldofosso.com/downloads/launcher.json"
echo "  https://worldofosso.com/downloads/${BINARY}"
