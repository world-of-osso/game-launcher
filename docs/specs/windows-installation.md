# Windows installation

Release packaging and portable first-run integration. [Operation](../../README.md#windows-installation).

## What it must do

- [ ] Release a Windows installer alongside the portable executable, without enabling launcher self-update.
- [ ] Install for the current user under `%LOCALAPPDATA%`, without elevation or system-wide installation.
- [ ] Create a Start Menu entry named **World of Osso**, using the app icon, and a per-user installed-app/uninstaller entry.
- [ ] Launch the actual launcher from the Start Menu entry. Uninstall removes launcher files, installer-owned shortcuts and registration, without deleting the downloaded game or its cache.
- [ ] On a portable GUI executable's first run, create `%APPDATA%\Microsoft\Windows\Start Menu\Programs\World of Osso.lnk` when no existing portable or installer Start Menu entry exists. Preserve an existing entry.

## Implementation inventory

- `Cargo.toml`: cargo-packager NSIS configuration, current-user mode and app identity. The supported top-level `preinstall-section` extension defines `un.onUninstSuccess`, removing only the launcher's runtime WebView2 profile before removing its empty install directory; the default packager uninstaller removes packaged files, not generated browser data.
- `.github/workflows/release.yml`: pinned cargo-packager tool and installer release artifact.
- `build.rs`, `assets/world-of-osso.ico`: Windows executable/installer icon. The ICO is derived from the existing launcher `hero-bg.png`, with 16–256 pixel variants; `winresource` embeds it into Windows executables.
- `src/windows_shortcut.rs`: portable GUI Start Menu integration using native UTF-16 `IShellLinkW`/`IPersistFile` through `windows`, with paired COM lifetime and explicit errors. Legacy scripting-host links fail on Unicode directory names.

## Tests asserting this spec

- `windows_shortcut::tests`: native Windows Shell link target/icon, Unicode/apostrophe paths, preservation of existing entries and no duplicate installer entry.
- Native desktop acceptance: install, Start search, actual shortcut launch and uninstall; captured PNG and registry/filesystem observations.

## Out of scope

- Publishing or tagging: lead-owned.
- System-wide installs, elevation and automatic launcher self-update.
- Deleting downloaded game data during launcher uninstall.
