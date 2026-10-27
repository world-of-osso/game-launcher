//! Verified local-file receipts shared by UI and CLI updates.

use crate::{FileEntry, hash_file};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct FileHashCache {
    files: HashMap<String, CachedFileHash>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct CachedFileHash {
    sha256: String,
    size: u64,
    #[serde(rename = "mtime")]
    mtime_nanos: u64,
}

fn mtime_nanos(meta: &std::fs::Metadata) -> Result<u64, String> {
    let modified = meta
        .modified()
        .map_err(|error| format!("Read modification time: {error}"))?;
    let elapsed = modified
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| format!("Modification time before epoch: {error}"))?;
    elapsed
        .as_nanos()
        .try_into()
        .map_err(|error| format!("Modification time out of range: {error}"))
}

impl FileHashCache {
    pub fn load(game_dir: &Path) -> Result<Self, String> {
        let path = game_dir.join(".file_hashes.json");
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|error| format!("Read {}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(format!("Read {}: {error}", path.display())),
        }
    }

    pub fn save(&self, game_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(game_dir)
            .map_err(|error| format!("Create {}: {error}", game_dir.display()))?;
        let json =
            serde_json::to_vec(self).map_err(|error| format!("Encode hash cache: {error}"))?;
        std::fs::write(game_dir.join(".file_hashes.json"), json)
            .map_err(|error| format!("Save hash cache in {}: {error}", game_dir.display()))
    }

    fn cache_receipt(
        &mut self,
        path: &str,
        digest: String,
        meta: &std::fs::Metadata,
    ) -> Result<(), String> {
        let receipt = CachedFileHash {
            sha256: digest,
            size: meta.len(),
            mtime_nanos: mtime_nanos(meta)?,
        };
        self.files.insert(path.to_owned(), receipt);
        Ok(())
    }

    /// Installed paths per (sha256, size), from receipts only.
    pub fn paths_by_content(&self) -> HashMap<(String, u64), Vec<String>> {
        let mut index: HashMap<(String, u64), Vec<String>> = HashMap::new();
        for (path, receipt) in &self.files {
            index
                .entry((receipt.sha256.clone(), receipt.size))
                .or_default()
                .push(path.clone());
        }
        index
    }

    /// True when `path` still has the size and mtime its receipt was recorded with.
    pub fn is_current(&self, game_dir: &Path, path: &str) -> bool {
        let Some(receipt) = self.files.get(path) else {
            return false;
        };
        std::fs::metadata(game_dir.join(path))
            .ok()
            .and_then(|meta| Some((meta.len(), mtime_nanos(&meta).ok()?)))
            == Some((receipt.size, receipt.mtime_nanos))
    }

    pub fn paths(&self) -> Vec<String> {
        self.files.keys().cloned().collect()
    }

    pub fn forget(&mut self, path: &str) {
        self.files.remove(path);
    }

    pub fn record_verified(&mut self, game_dir: &Path, entry: &FileEntry) -> Result<(), String> {
        let path = game_dir.join(&entry.path);
        let meta = std::fs::metadata(&path)
            .map_err(|error| format!("Stat {}: {error}", path.display()))?;
        self.cache_receipt(&entry.path, entry.sha256.clone(), &meta)
    }

    pub async fn needs_update(
        &mut self,
        game_dir: &Path,
        entry: &FileEntry,
    ) -> Result<bool, String> {
        let path = game_dir.join(&entry.path);
        let meta = match tokio::fs::metadata(&path).await {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(format!("Stat {}: {error}", path.display())),
        };
        if meta.len() != entry.size {
            return Ok(true);
        }
        let modified = mtime_nanos(&meta)?;
        if let Some(cached) = self.files.get(&entry.path)
            && cached.size == meta.len()
            && cached.mtime_nanos == modified
        {
            return Ok(cached.sha256 != entry.sha256);
        }
        let digest = hash_file(&path)
            .await
            .map_err(|error| format!("Hash {}: {error}", path.display()))?;
        let needed = digest != entry.sha256;
        self.cache_receipt(&entry.path, digest, &meta)?;
        Ok(needed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn same_second_rewrite_and_manifest_size_changes_invalidate_receipts() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let dir = std::env::temp_dir().join(format!("launcher-cache-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("asset.blp");
            std::fs::write(&path, b"expected").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_millis(100_100))
                .unwrap();
            let entry = FileEntry {
                path: "asset.blp".into(),
                sha256: hex::encode(Sha256::digest(b"expected")),
                size: 8,
            };
            let mut cache = FileHashCache::default();
            assert!(!cache.needs_update(&dir, &entry).await.unwrap());
            let wrong_size = FileEntry {
                size: 9,
                ..entry.clone()
            };
            assert!(
                cache.needs_update(&dir, &wrong_size).await.unwrap(),
                "manifest size must be checked even with a cached digest"
            );
            std::fs::write(&path, b"tampered").unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(UNIX_EPOCH + Duration::from_millis(100_200))
                .unwrap();
            assert!(
                cache.needs_update(&dir, &entry).await.unwrap(),
                "same-second rewrite must be rehashed"
            );
            std::fs::remove_dir_all(dir).unwrap();
        });
    }
}
