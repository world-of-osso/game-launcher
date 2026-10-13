#!/usr/bin/env bash
set -euo pipefail
# Keep the existing deployment entry; Python owns manifest merging and ordered publish.
exec python3 "$(dirname "$0")/scripts/publish-launcher.py" "$@"
