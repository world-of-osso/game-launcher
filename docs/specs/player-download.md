# Player download

Desktop UI and CLI update the player from a SHA256 manifest. Transfer source: `src/lib.rs`; integration: `src/main.rs`. [Operation](../../README.md#how-it-works).

## What it must do

### Integrity and storage

- [ ] Stream downloads and local hash checks without buffering entire assets in memory.
- [ ] Verify declared byte count and SHA256 before replacing an installed file; rejected/incomplete responses must not replace it.
- [ ] Reuse cached hashes only when actual size and nanosecond modification time match; actual size must also match the manifest. Hash changed files and compare the result to the manifest.
- [ ] Resume after interruption without fetching already completed, unchanged files again.
- [ ] Refuse an update when available disk space cannot hold its required downloads.

### Scheduling and Windows

- [ ] Download concurrently with a fixed bound; retry transient GET failures with bounded backoff and jitter, respecting Retry-After.
- [ ] UI and CLI use the same transfer/verification path, and expose errors rather than entering Ready/Launching on failure.
- [ ] Windows supports its normal install directory and long paths without host registry/firewall changes.
- [ ] Explicit LAN source/install-directory overrides permit isolated tests; normal launches use the production source and user data directory.
- [ ] Connect deadline is 10 seconds; read inactivity deadline is 30 seconds and resets after successful reads. No whole-asset deadline.

## How it works

- [Operation and install paths](../../README.md#how-it-works).

## Implementation inventory

- `src/lib.rs`: player transfer, hashing and regression tests.
- `src/cache.rs`: verified receipts and subsecond mutation detection.
- `src/disk.rs`: allocation-granularity-aware available-space preflight.
- `src/main.rs`: manifest and shared UI/CLI synchronization integration.

## Tests asserting this spec

- `src/lib.rs::tests`: real local HTTP response, filesystem integrity, retry and disk-refusal boundaries.
- `src/cache.rs::tests`: manifest-size and same-second rewrite invalidation.
- `src/scheduler_tests.rs`: overlapping downloads with a fixed concurrency bound.
- `src/runtime_tests.rs`: missing spawn error, production read timeout, Windows long install path transfer/hash/extensionless PE spawn.

## Known gaps (current cycle)

- [ ] Bounded scheduler, retry, metadata/resume and disk-space tests.
- [ ] Native Windows subset-manifest LAN fixture and full published download/start.

## Out of scope

- Launcher release/publishing: lead-owned.
- Byte-range partial-file resume: completed files must resume; incomplete files may restart.
