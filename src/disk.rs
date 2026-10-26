//! Space for verified temporary files; installed files remain intact until replacement.

use crate::{FileEntry, partial_path};
use std::path::Path;

pub(crate) async fn prepare_download_space(
    game_dir: &Path,
    entries: &[FileEntry],
) -> Result<(), String> {
    tokio::fs::create_dir_all(game_dir)
        .await
        .map_err(|error| format!("Create {}: {error}", game_dir.display()))?;
    let directory = tokio::fs::canonicalize(game_dir)
        .await
        .map_err(|error| format!("Resolve {}: {error}", game_dir.display()))?;
    let stats = fs2::statvfs(&directory)
        .map_err(|error| format!("Check disk space at {}: {error}", directory.display()))?;
    let cluster = stats.allocation_granularity();
    if cluster == 0 {
        return Err("Cannot check disk space: filesystem allocation granularity is zero".into());
    }
    let total = total_allocations(entries, cluster)?;
    // Fresh full installs contain over a million paths: avoid a stat per path when
    // free space already covers every temporary file without partial-file credits.
    if stats.available_space() >= total {
        return Ok(());
    }
    let required = measure_remaining_space(&directory, entries, cluster, total).await?;
    if stats.available_space() < required {
        return Err(format!(
            "Insufficient disk space at {}: need {required} bytes, available {}",
            directory.display(),
            stats.available_space()
        ));
    }
    Ok(())
}

pub(crate) async fn prepare_pack_space(
    game_dir: &Path,
    entries: &[FileEntry],
) -> Result<(), String> {
    tokio::fs::create_dir_all(game_dir)
        .await
        .map_err(|error| format!("Create {}: {error}", game_dir.display()))?;
    let stats = fs2::statvfs(game_dir)
        .map_err(|error| format!("Check disk space at {}: {error}", game_dir.display()))?;
    let cluster = stats.allocation_granularity();
    if cluster == 0 {
        return Err("Cannot check disk space: filesystem allocation granularity is zero".into());
    }
    let required = total_allocations(entries, cluster)?;
    if stats.available_space() < required {
        return Err(format!(
            "Insufficient disk space at {}: need {required} bytes, available {}",
            game_dir.display(),
            stats.available_space()
        ));
    }
    Ok(())
}

fn total_allocations(entries: &[FileEntry], cluster: u64) -> Result<u64, String> {
    entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(allocated_bytes(entry.size, cluster)?)
            .ok_or_else(|| "Required disk space overflows u64".to_owned())
    })
}

async fn measure_remaining_space(
    directory: &Path,
    entries: &[FileEntry],
    cluster: u64,
    mut required: u64,
) -> Result<u64, String> {
    for entry in entries {
        let partial = partial_path(&directory.join(&entry.path));
        let existing = match tokio::fs::metadata(&partial).await {
            Ok(meta) => allocated_bytes(meta.len(), cluster)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(format!("Stat {}: {error}", partial.display())),
        };
        let allocation = allocated_bytes(entry.size, cluster)?;
        required -= existing.min(allocation);
    }
    Ok(required)
}

fn allocated_bytes(size: u64, cluster: u64) -> Result<u64, String> {
    size.div_ceil(cluster)
        .checked_mul(cluster)
        .ok_or_else(|| "Required disk space overflows u64".to_owned())
}
