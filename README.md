# World of Osso — Game Launcher

Desktop launcher that keeps game files in sync with the public R2 bucket at `https://files.worldofosso.com`. Built with Dioxus (Rust).

## Features

- Downloads and verifies game files via SHA256 manifest
- Launcher self-update mechanism
- Conditional manifest fetching (ETag/Last-Modified) to avoid redundant downloads
- Battle.net-inspired dark UI

## Building

```bash
cargo run          # dev mode
cargo build --release
```

## How it works

1. On launch, fetches `https://files.worldofosso.com/manifest.json`
2. Compares local file SHA256 hashes against the manifest
3. Downloads any missing or outdated files from `https://files.worldofosso.com/<path>`
4. Launches the game engine binary

Game files are stored in `~/.local/share/WorldOfOsso/` (Linux) or the platform equivalent.

Transfers use four concurrent workers, 10-second connect and 30-second read-inactivity deadlines, and allocation-aware disk-space preflight (`fs2`). Every file streams to a temporary path; size and SHA256 must match before replacement. Verified hashes are reused only when actual size and nanosecond modification time match. Completed files remain installed after a failed batch and are skipped on resume; incomplete files restart. UI and CLI share synchronization and report download/spawn errors.

## Isolated LAN proof

Set `GAME_LAUNCHER_BASE_URL` to the owned manifest server and `GAME_LAUNCHER_GAME_DIR` to an isolated install directory; unset variables retain the production source and normal user-data location. Run `game-launcher update` to synchronize or `game-launcher play` to synchronize and start the real platform entry. Overrides do not alter host registry or firewall settings.

## Known self-update issue

The optional launcher-update schema currently has one SHA256 for every platform binary. Different platform executables need separate digests. Current file-server manifests omit launcher metadata, so this path is inactive in the present release flow; schema migration is deferred, not fixed by player-transfer verification.

## Related

- [file-server](../file-server) — generates the manifest
- `../game-engine/deploy.sh` — publishes the client bundle and manifest to R2
