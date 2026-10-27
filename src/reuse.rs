//! Install files whose bytes are already on disk under another path, and prune files the
//! manifest no longer lists. Asset paths carry the CASC build key, so a new client build
//! renames files whose bytes did not change: on 2026-10-27, 282,664 of 292,888 files under
//! the new key were already installed under the old key, and the old 260,535 files stayed.

use crate::{FileEntry, cache::FileHashCache};
use std::{collections::HashSet, path::Path};

/// Places every needed file whose content a current receipt already holds, moving it when
/// its old path is no longer listed and copying it otherwise. Returns what must still be fetched.
pub async fn reuse_local_content(
    game_dir: &Path,
    cache: &mut FileHashCache,
    needed: Vec<FileEntry>,
    listed: &HashSet<&str>,
) -> Result<Vec<FileEntry>, String> {
    let mut sources = cache.paths_by_content();
    let mut remaining = Vec::new();
    for entry in needed {
        let source = sources
            .get_mut(&(entry.sha256.clone(), entry.size))
            .and_then(|paths| take_current_source(game_dir, cache, paths, &entry.path));
        match source {
            Some(source) => {
                let moved = !listed.contains(source.as_str());
                place(game_dir, &source, &entry.path, moved).await?;
                if moved {
                    cache.forget(&source);
                } else if let Some(paths) = sources.get_mut(&(entry.sha256.clone(), entry.size)) {
                    paths.push(source);
                }
                cache.record_verified(game_dir, &entry)?;
            }
            None => remaining.push(entry),
        }
    }
    Ok(remaining)
}

fn take_current_source(
    game_dir: &Path,
    cache: &FileHashCache,
    paths: &mut Vec<String>,
    target: &str,
) -> Option<String> {
    let index = paths
        .iter()
        .position(|path| path != target && cache.is_current(game_dir, path))?;
    Some(paths.swap_remove(index))
}

async fn place(game_dir: &Path, source: &str, target: &str, moved: bool) -> Result<(), String> {
    let from = game_dir.join(source);
    let to = game_dir.join(target);
    if let Some(parent) = to.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    if moved {
        return tokio::fs::rename(&from, &to)
            .await
            .map_err(|e| format!("Move {} to {}: {e}", from.display(), to.display()));
    }
    // Copy beside the target, then rename, so an interrupted copy never looks installed.
    let partial = to.with_extension("reuse-partial");
    tokio::fs::copy(&from, &partial)
        .await
        .map_err(|e| format!("Copy {} to {}: {e}", from.display(), partial.display()))?;
    tokio::fs::rename(&partial, &to)
        .await
        .map_err(|e| format!("Install {}: {e}", to.display()))
}

/// Deletes files the launcher installed (they have receipts) that the manifest no longer
/// lists, then their emptied directories. Files without receipts are never touched.
pub fn prune_unlisted(
    game_dir: &Path,
    cache: &mut FileHashCache,
    listed: &HashSet<&str>,
) -> Result<usize, String> {
    let mut removed = 0;
    for path in cache.paths() {
        if listed.contains(path.as_str()) {
            continue;
        }
        let file = game_dir.join(&path);
        match std::fs::remove_file(&file) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Remove {}: {error}", file.display())),
        }
        cache.forget(&path);
        let mut parent = file.parent();
        while let Some(directory) = parent.filter(|directory| *directory != game_dir) {
            if std::fs::remove_dir(directory).is_err() {
                break;
            }
            parent = directory.parent();
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn entry(path: &str, bytes: &[u8]) -> FileEntry {
        FileEntry {
            path: path.into(),
            sha256: hex::encode(Sha256::digest(bytes)),
            size: bytes.len() as u64,
        }
    }

    fn temp_root(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("game-launcher-reuse-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn install(dir: &Path, cache: &mut FileHashCache, path: &str, bytes: &[u8]) {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
        cache.record_verified(dir, &entry(path, bytes)).unwrap();
    }

    #[test]
    fn build_key_rename_moves_unlisted_copies_listed_and_fetches_only_new_bytes() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let dir = temp_root("rename");
            let root = dir.as_path();
            let mut cache = FileHashCache::default();
            let old = "data/products/wow/dcfc/textures/120191.blp";
            let shared = "data/models/484859.skin";
            install(root, &mut cache, old, b"unchanged texture");
            install(root, &mut cache, shared, b"skin");
            let renamed = entry(
                "data/products/wow/88d5/textures/120191.blp",
                b"unchanged texture",
            );
            let copied = entry("data/products/wow/88d5/models/484859.skin", b"skin");
            let fresh = entry(
                "data/products/wow/88d5/textures/9.blp",
                b"new in this build",
            );
            let listed: HashSet<&str> = [
                shared,
                renamed.path.as_str(),
                copied.path.as_str(),
                fresh.path.as_str(),
            ]
            .into();

            let remaining = reuse_local_content(
                root,
                &mut cache,
                vec![renamed.clone(), copied.clone(), fresh.clone()],
                &listed,
            )
            .await
            .unwrap();

            assert_eq!(
                remaining.iter().map(|e| &e.path).collect::<Vec<_>>(),
                [&fresh.path]
            );
            assert_eq!(
                std::fs::read(root.join(&renamed.path)).unwrap(),
                b"unchanged texture"
            );
            assert!(
                !root.join(old).exists(),
                "an unlisted source is moved, not duplicated"
            );
            assert_eq!(std::fs::read(root.join(&copied.path)).unwrap(), b"skin");
            assert_eq!(std::fs::read(root.join(shared)).unwrap(), b"skin");
            assert!(cache.is_current(root, &renamed.path) && cache.is_current(root, &copied.path));
        });
    }

    #[test]
    fn changed_source_is_not_reused() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let dir = temp_root("changed");
            let root = dir.as_path();
            let mut cache = FileHashCache::default();
            install(root, &mut cache, "data/old.blp", b"recorded bytes");
            // Same length, different bytes, later mtime: the receipt no longer describes the file.
            std::thread::sleep(std::time::Duration::from_millis(20));
            std::fs::write(root.join("data/old.blp"), b"edited! bytes!").unwrap();
            let wanted = entry("data/new.blp", b"recorded bytes");
            let remaining =
                reuse_local_content(root, &mut cache, vec![wanted.clone()], &HashSet::new())
                    .await
                    .unwrap();
            assert_eq!(remaining.len(), 1);
            assert!(!root.join("data/new.blp").exists());
        });
    }

    #[test]
    fn prune_deletes_only_unlisted_receipts_and_their_empty_directories() {
        let dir = temp_root("prune");
        let root = dir.as_path();
        let mut cache = FileHashCache::default();
        install(
            root,
            &mut cache,
            "data/products/wow/dcfc/models/1.m2",
            b"old",
        );
        install(
            root,
            &mut cache,
            "data/products/wow/88d5/models/1.m2",
            b"new",
        );
        std::fs::write(root.join("data/products/wow/user-notes.txt"), b"not ours").unwrap();
        let listed: HashSet<&str> = ["data/products/wow/88d5/models/1.m2"].into();

        assert_eq!(prune_unlisted(root, &mut cache, &listed).unwrap(), 1);

        assert!(!root.join("data/products/wow/dcfc").exists());
        assert!(root.join("data/products/wow/88d5/models/1.m2").exists());
        assert!(root.join("data/products/wow/user-notes.txt").exists());
        assert_eq!(cache.paths(), ["data/products/wow/88d5/models/1.m2"]);
    }
}
