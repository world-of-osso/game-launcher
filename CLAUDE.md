# Game Launcher

Dioxus desktop app that downloads and launches game files via a manifest API.

## Related Projects

- Server: `../file-server` — serves the manifest API (`/api/manifest`) and game files

## Architecture

- `src/main.rs` — single-file app: UI components, manifest fetch, file sync, HMAC auth
- `assets/style.css` — Battle.net-style dark UI
- Manifest is cached locally (`.manifest_cache.json`) with ETag/Last-Modified conditional requests

## Build

```bash
cargo run          # dev
cargo build --release
```
