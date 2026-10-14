# World of Osso — Game Launcher

Desktop launcher that keeps game files in sync with the public R2 bucket at `https://files.worldofosso.com`. Built with Dioxus (Rust).

## Features

- Downloads and verifies game files via SHA256 manifest
- Automatic verified launcher updates with rollback/restart
- Rechecks launcher and game manifests every five minutes while open; Retry checks immediately
- Conditional manifest fetching (ETag/Last-Modified) to avoid redundant downloads
- Battle.net-inspired dark UI

## Building

```bash
cargo run          # dev mode
cargo build --release
```

## Windows installation

Use `game-launcher-<version>-windows-x86_64-setup.exe` for normal Windows installation. The cargo-packager NSIS installer runs without elevation, installs into `%LOCALAPPDATA%\World of Osso`, creates the **World of Osso** Start Menu entry with the app icon, and registers an uninstaller in the current user's installed apps. Uninstall removes the launcher, shortcuts and registration, not `%APPDATA%\WorldOfOsso` game downloads.

The portable `.exe` remains available. Its first GUI run creates a per-user Start Menu shortcut when no portable/installer entry exists; an existing entry is preserved. CLI update/play commands do not create shortcuts. See [Windows installation contract](docs/specs/windows-installation.md).

Build the release executable, then run `cargo packager --release --formats nsis --binaries-dir target/release --out-dir dist` with cargo-packager 0.11.8. CI uploads both portable and installer executables; publishing/tagging remains lead-owned. Windows-only dependencies: `winresource` embeds the app icon; `windows` exposes native UTF-16 Shell links without invoking the legacy scripting host. The [OpenAI-generated app icon](assets/README.md) uses a bronze/gold W crest, with a tracked transparent source PNG and seven ICO sizes.

## How it works

1. On startup, checks `https://worldofosso.com/downloads/launcher.json` for a newer launcher for this platform.
2. Downloads and verifies its size/SHA256, runs `--self-check`, replaces the executable and relaunches with the original arguments. Failed startup acknowledgement restores the old launcher; confirmed `.old` is deleted on the next check/start.
3. Fetches `https://files.worldofosso.com/manifest.json`, compares local hashes and downloads missing/outdated game files.
4. Rechecks both manifests every five minutes while open. Game/launcher errors have Retry; a launcher-update failure does not prevent downloading or playing the game.

Game files are stored in `~/.local/share/WorldOfOsso/` (Linux) or the platform equivalent.

Transfers use at most 32 concurrent workers, 10-second connect and 30-second read-inactivity deadlines, and allocation-aware disk-space preflight (`fs2`). Every file streams to a temporary path; size and SHA256 must match before replacement. Verified hashes are reused only when actual size and nanosecond modification time match. Completed files remain installed after a failed batch and are skipped on resume; incomplete files restart. UI and CLI share synchronization and report download/spawn errors.

## Isolated LAN proof

Set `GAME_LAUNCHER_BASE_URL` to the owned manifest server and `GAME_LAUNCHER_GAME_DIR` to an isolated install directory; unset variables retain the production source and normal user-data location. Run `game-launcher update` to synchronize or `game-launcher play` to synchronize and start the real platform entry. CLI arguments after `play` are forwarded unchanged to the player, e.g. `game-launcher play --server 192.168.2.149:54392 --screen login`; GUI Play retains the normal no-argument launch. Overrides do not alter host registry or firewall settings.

## Release limitations

Windows uses the normal `%APPDATA%/WorldOfOsso` install root. Long nested asset paths can transfer and verify, but Godot cannot start when the project/install root exceeds its Windows current-directory limit (approximately 260 characters). Do not use an overlong install root; no host registry changes or Godot CWD patch are provided.

Launchers older than 0.4.0 have self-update disabled and need one manual installation of 0.4.0. Later updates are automatic, including CLI startup; `game-launcher self-update` explicitly checks/retries. The executable's directory must be writable by the current user. Linux atomically renames the staged executable over the original path (hard-linked `.old` backup). Windows moves the running executable to `.old`, installs at its original path and restarts. A bounded 15-second version self-check and a CLI-startup/GUI-mount acknowledgement gate success. This proves process startup, not every future UI action. Failed replacement/relaunch leaves the old launcher operational and reports an error.

## Launcher publishing (lead-owned)

`./deploy.sh` builds/publishes the native portable executable. To publish a prebuilt platform artifact: `./deploy.sh --platform windows-x86_64 --artifact /path/to/game-launcher.exe`. Add `--dry-run` to stage the merged manifest and print exact upload commands without publishing. Publisher requires Python 3.11+, SSH/scp and a prebuilt artifact or Cargo. Upload order: executable first, then `launcher.json.next`, then atomic manifest rename; previously published executables remain available.

Each platform has its own version and integrity metadata (mixed release versions are allowed):

```json
{
  "version": "0.4.0",
  "platforms": {
    "linux-x86_64": {
      "version": "0.4.0",
      "url": "https://worldofosso.com/downloads/game-launcher-0.4.0-linux-x86_64",
      "sha256": "<64 lowercase hex characters>",
      "size": 123456,
      "file": "game-launcher-0.4.0-linux-x86_64"
    }
  }
}
```

`windows-x86_64` uses the same fields and an `.exe` filename. Retain top-level `version`, per-platform `file` and any unrelated existing fields for published manifest consumers. Migration adds legacy platform versions from the previous top-level version and sizes from the actual published files. The updater never uses one global digest or the global version for platform comparison. The website's separate `/launcher.json` GitHub-release mirror (`tag_name`/`assets`) is unchanged; `/downloads/launcher.json` is this updater manifest.

HTTPS plus manifest SHA256 is the integrity model, not independent code signing. `GAME_LAUNCHER_MANIFEST_URL` permits an isolated updater source; HTTP is allowed only on loopback (including redirects), never on a public/LAN HTTP host. `--self-check` prints the compiled version without opening the UI or networking.

### Self-update proof

`cargo test --no-default-features --lib self_update` covers version ordering, fake HTTP size/hash rejection and Windows rename-aside behavior. `cargo test --no-default-features --test self_update_process` exercises a successful binary self-check followed by failed real relaunch/rollback. `python3 scripts/test_launcher_manifest.py` checks publication compatibility. CLI-only Linux builds use `cargo build --no-default-features`; shipped desktop defaults are unchanged.

`python3 scripts/prove-self-update.py --proof-dir /owned/path` builds updater-enabled fixture versions 0.3.0 and the current version, serves a loopback manifest and proves atomic replacement, actual relaunch and backup cleanup. This fixture does not imply the historical disabled 0.3.0 can bootstrap itself. On native Windows add `--gui --periodic` from the logged-in interactive session: proves the real running-EXE rename, new App mount and changed game-file download after five minutes. Runner stops only its own launcher processes. Do not publish fixture binaries.

## Related

- [file-server](../file-server) — generates the manifest
- `../game-engine/deploy.sh` — publishes the client bundle and manifest to R2
