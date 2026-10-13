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
- [ ] Windows supports its normal install directory and long nested asset paths without host registry/firewall changes. Godot launch from a project/install root exceeding its approximately 260-character CWD limit is not required; document that limitation.
- [ ] Explicit LAN source/install-directory overrides permit isolated tests; normal launches use the production source and user data directory. CLI `play` forwards following arguments unchanged to the player so private-server proof uses the normal downloaded entry; GUI Play supplies no arguments.
- [ ] Connect deadline is 10 seconds; read inactivity deadline is 30 seconds and resets after successful reads. No whole-asset deadline.

### Launcher updates

- [ ] Fetch a separate per-platform launcher manifest on startup and every five minutes while open; compare semantic versions per platform, never downgrade.
- [ ] Each `platforms[linux-x86_64|windows-x86_64]` entry contains `version`, HTTPS `url`, `sha256`, `size`, and legacy website `file`. Top-level `version` remains for existing website consumers; updater ignores it for platform selection.
- [ ] Verify launcher size and SHA256 before replacing anything. Linux installs by atomic rename; Windows renames current executable to `.old`, then installs new executable at the original path.
- [ ] Require a bounded `--self-check` version handshake before installation and a startup acknowledgement after relaunch. Restore old executable on failed relaunch; delete confirmed `.old` on next startup.
- [ ] Show launcher update errors with Retry without disabling the existing launcher's game download/play functionality.
- [ ] Recheck game manifest every five minutes while open; Retry triggers a fresh check, not just a label change.
- [ ] HTTPS plus manifest SHA256 is the integrity model. HTTP test sources are loopback-only. Publishing is lead-owned.

## How it works

- [Operation and install paths](../../README.md#how-it-works).

## Implementation inventory

- `src/lib.rs`: player transfer, hashing and regression tests.
- `src/cache.rs`: verified receipts and subsecond mutation detection.
- `src/disk.rs`: allocation-granularity-aware available-space preflight.
- `src/main.rs`: startup/periodic manifest and shared UI/CLI synchronization integration.
- `src/self_update.rs`: platform-version selection, verified executable staging, replacement, startup handshake/rollback and recheck/Retry wait boundary.
- `scripts/publish-launcher.py` (via `deploy.sh`): compatible per-platform manifest migration and executable-first/atomic-manifest publishing.

## Tests asserting this spec

- `src/lib.rs::tests`: real local HTTP response, filesystem integrity, retry and disk-refusal boundaries.
- `src/cache.rs::tests`: manifest-size and same-second rewrite invalidation.
- `src/self_update.rs::tests`: fake HTTP integrity/refusal, HTTPS/redirect policy, semantic versions (including prereleases/build metadata), periodic/Retry waits, transient manifest retry and Windows rename-aside.
- `tests/self_update_process.rs`: actual executable self-check followed by failed startup and rollback.
- `scripts/test_launcher_manifest.py`: legacy consumer fields/mixed platform versions; `scripts/prove-self-update.py`: compiled old→new launcher replacement/restart and optional native Windows GUI periodic game refresh.
- `src/scheduler_tests.rs`: overlapping downloads with a fixed concurrency bound.
- `src/runtime_tests.rs`: missing spawn error, production read timeout, Windows long install path transfer/hash/extensionless PE spawn.

## Known gaps (current cycle)

- [x] Native Windows transfer tests10/10 at cbdb680: bounded scheduler, retry, nanos metadata/completed resume, disk-space refusal, body read timeout, spawn error and long install path PE spawn. Evidence: engine data/diagnostics/windows-release-2026-10-11/launcher-runtime-green-2.log.
- [x] Native Windows normal-root LAN subset: actual launcher verification/download and entry reach Login→roster→authoritative InWorld; 9/9 terrain tiles loaded, no pending/failed tiles. DLL `99256a4f9` / frozen protocol `58e8ac9`, matching historical server master `0fd9d401`, private UDP54392; evidence in engine `data/diagnostics/windows-release-2026-10-11/final-frozen58-lan-proof.log` and `lan-release4-proof/`. Published player builds must use current master rather than the frozen proof artifacts.
- [ ] Full published download/start and all-map/offline asset certification. LAN subset acceptance does not prove these.

## Out of scope

- Launcher release/publishing: lead-owned.
- Byte-range partial-file resume: completed files must resume; incomplete files may restart.
