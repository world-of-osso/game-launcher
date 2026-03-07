# World of Osso — Game Launcher

Desktop launcher that keeps game files in sync with the server. Built with Dioxus (Rust).

## Features

- Downloads and verifies game files via SHA256 manifest
- HMAC-authenticated API requests
- Launcher self-update mechanism
- Conditional manifest fetching (ETag/Last-Modified) to avoid redundant downloads
- Battle.net-inspired dark UI

## Building

```bash
cargo run          # dev mode
cargo build --release
```

## How it works

1. On launch, fetches the manifest from the file server (`/api/manifest`)
2. Compares local file SHA256 hashes against the manifest
3. Downloads any missing or outdated files
4. Launches the game engine binary

Game files are stored in `~/.local/share/WorldOfOsso/` (Linux) or the platform equivalent.

## Related

- [file-server](../file-server) — Serves the manifest API and game files
