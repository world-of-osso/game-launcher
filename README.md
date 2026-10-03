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

## Related

- [file-server](../file-server) — generates the manifest
- `../game-engine/deploy.sh` — publishes the client bundle and manifest to R2
