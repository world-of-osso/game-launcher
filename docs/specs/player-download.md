# Player download

Desktop UI and CLI update the player from a SHA256 manifest. Transfer source: `src/lib.rs`; integration: `src/main.rs`. [Operation](../../README.md#how-it-works).

## What it must do

### Integrity and storage

- [ ] Stream downloads and local hash checks without buffering entire assets in memory.
- [ ] Verify declared byte count and SHA256 before replacing an installed file; rejected/incomplete responses must not replace it.
- [ ] Reuse cached hashes only when actual size and modification time match; hash changed files and compare the result to the manifest.
- [ ] Resume after interruption without fetching already completed, unchanged files again.
- [ ] Refuse an update when available disk space cannot hold its required downloads.

### Scheduling and Windows

- [ ] Download concurrently with a fixed bound; retry transient GET failures with bounded backoff and jitter, respecting Retry-After.
- [ ] UI and CLI use the same transfer/verification path, and expose errors rather than entering Ready/Launching on failure.
- [ ] Windows supports its normal install directory and long paths without host registry/firewall changes.
- [ ] Explicit LAN source override permits isolated tests; normal launches use the production source.

## How it works

- [Operation and install paths](../../README.md#how-it-works).

## Implementation inventory

- `src/lib.rs`: player transfer, hashing and regression tests.
- `src/main.rs`: manifest, cache and UI/CLI integration.

## Tests asserting this spec

- `src/lib.rs::tests`: real local HTTP response and filesystem integrity boundary.

## Known gaps (current cycle)

- [ ] Bounded scheduler, retry, metadata/resume and disk-space tests.
- [ ] Native Windows subset-manifest LAN fixture and full published download/start.

## Out of scope

- Launcher release/publishing: lead-owned.
- Byte-range partial-file resume: completed files must resume; incomplete files may restart.
