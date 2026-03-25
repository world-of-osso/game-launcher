#!/usr/bin/env bash
set -euo pipefail

VERSION=$(cargo metadata --no-deps --format-version=1 -q \
  | nu --stdin -c 'from json | get packages.0.version')

REMOTE_DIR="/data/osso-website/downloads"

# Detect platform
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)   PLATFORM="linux-x86_64";   EXT="" ;;
  Darwin-arm64)   PLATFORM="macos-aarch64";   EXT="" ;;
  Darwin-x86_64)  PLATFORM="macos-x86_64";    EXT="" ;;
  MINGW*|MSYS*|CYGWIN*)  PLATFORM="windows-x86_64";  EXT=".exe" ;;
  *) echo "Unsupported platform: $(uname -s)-$(uname -m)"; exit 1 ;;
esac

BINARY="game-launcher-${VERSION}-${PLATFORM}${EXT}"

echo "Building game-launcher v${VERSION} for ${PLATFORM}..."
cargo build --release

cp "target/release/game-launcher${EXT}" "$BINARY"
SHA256=$(sha256sum "$BINARY" | cut -d' ' -f1)

echo "Merging into launcher.json..."
# Fetch existing launcher.json from server, merge in this platform
EXISTING=$(ssh sakuin "cat ${REMOTE_DIR}/launcher.json 2>/dev/null" || echo '{}')
nu -c "
  let existing = ('${EXISTING}' | from json);
  let platforms = (if 'platforms' in (\$existing | columns) { \$existing.platforms } else { {} });
  let updated = (\$platforms | merge { ${PLATFORM}: { file: \"${BINARY}\", sha256: \"${SHA256}\" } });
  { version: \"${VERSION}\", platforms: \$updated } | to json
" > launcher.json

echo "Uploading to sakuin:${REMOTE_DIR}..."
ssh sakuin "mkdir -p ${REMOTE_DIR} && rm -f ${REMOTE_DIR}/game-launcher-*-${PLATFORM}*"
scp "$BINARY" launcher.json "sakuin:${REMOTE_DIR}/"

rm "$BINARY"

echo "Done. v${VERSION} (${PLATFORM}) deployed."
echo "  https://worldofosso.com/downloads/launcher.json"
echo "  https://worldofosso.com/downloads/${BINARY}"
