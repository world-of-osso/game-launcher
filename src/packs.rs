//! Versioned content-addressed player packs.
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackManifest {
    pub version: u32,
    pub format_version: u32,
    pub chunking: Chunking,
    pub files: Vec<PackFile>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chunking {
    pub version: u32,
    pub min: u64,
    pub average: u64,
    pub max: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub chunks: Vec<PackChunk>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PackChunk {
    pub sha256: String,
    pub size: u64,
    pub pack: String,
    pub offset: u64,
    pub pack_size: u64,
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_path(path: &str) -> Result<String, String> {
    let mut normalized = Vec::new();
    for part in path.split('/') {
        let name = part.to_lowercase();
        let stem = name.split('.').next().unwrap_or("");
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "\\:*?\"<>|".contains(c))
            || matches!(stem, "con" | "prn" | "aux" | "nul" | "clock$")
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!("Unsafe install path: {path}"));
        }
        normalized.push(name);
    }
    if normalized.first().is_some_and(|part| {
        part == ".pack-staging" || part == ".file_hashes.json" || part == ".manifest_cache.json"
    }) {
        return Err(format!("Reserved install path: {path}"));
    }
    Ok(normalized.join("/"))
}

impl PackManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 2
            || self.format_version != 1
            || self.chunking.version != 1
            || (self.chunking.min, self.chunking.average, self.chunking.max)
                != (262_144, 1_048_576, 4_194_304)
        {
            return Err("Unsupported pack or chunk format".into());
        }
        let mut paths = HashSet::new();
        let mut pack_sizes = HashMap::new();
        for file in &self.files {
            let path = validate_path(&file.path)?;
            if !paths.insert(path) || !valid_hash(&file.sha256) {
                return Err(format!("Duplicate path or invalid digest: {}", file.path));
            }
            let mut size = 0u64;
            for chunk in &file.chunks {
                if !valid_hash(&chunk.sha256)
                    || !valid_hash(&chunk.pack)
                    || chunk.size > self.chunking.max
                    || chunk.pack_size > 64 * 1024 * 1024 + 8
                    || chunk.offset < 12
                    || chunk
                        .offset
                        .checked_add(chunk.size)
                        .is_none_or(|end| end > chunk.pack_size)
                {
                    return Err(format!("Invalid chunk for {}", file.path));
                }
                if pack_sizes
                    .insert(&chunk.pack, chunk.pack_size)
                    .is_some_and(|old| old != chunk.pack_size)
                {
                    return Err(format!("Inconsistent pack size: {}", chunk.pack));
                }
                size = size.checked_add(chunk.size).ok_or("File size overflow")?;
            }
            if size != file.size {
                return Err(format!("Chunk size mismatch: {}", file.path));
            }
        }
        for path in &paths {
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if paths.contains(prefix) {
                    return Err(format!("File used as directory: {prefix}"));
                }
                parent = prefix;
            }
        }
        Ok(())
    }
}

/// Download referenced immutable packs, then reconstruct verified files.
pub async fn sync_packs(
    client: &reqwest::Client,
    base_url: &str,
    game_dir: &std::path::Path,
    manifest: &PackManifest,
    needed: &[String],
    cold: bool,
    mut completed: impl FnMut(&crate::FileEntry, usize) -> Result<(), String>,
) -> Result<(), String> {
    use futures_util::stream::{self, StreamExt, TryStreamExt};
    manifest.validate()?;
    let requested: HashSet<&str> = needed.iter().map(String::as_str).collect();
    let files: Vec<_> = manifest
        .files
        .iter()
        .filter(|file| requested.contains(file.path.as_str()))
        .collect();
    if files.len() != requested.len() {
        return Err("Requested file is absent from pack manifest".into());
    }
    reject_symlinks(game_dir, ".pack-staging")?;
    for file in &files {
        reject_symlinks(game_dir, &file.path)?;
    }
    let mut packs: HashMap<String, Vec<PackChunk>> = HashMap::new();
    for file in &files {
        for chunk in &file.chunks {
            packs
                .entry(chunk.pack.clone())
                .or_default()
                .push(chunk.clone());
        }
    }
    let mut space: Vec<crate::FileEntry> = files
        .iter()
        .map(|file| crate::FileEntry {
            path: file.path.clone(),
            sha256: file.sha256.clone(),
            size: file.size,
        })
        .collect();
    if cold {
        space.extend(packs.iter().map(|(hash, chunks)| crate::FileEntry {
            path: format!(".pack-staging/{hash}"),
            sha256: hash.clone(),
            size: chunks[0].pack_size,
        }));
    } else {
        let mut seen = HashSet::new();
        for chunks in packs.values() {
            for chunk in chunks {
                if seen.insert(&chunk.sha256) {
                    space.push(crate::FileEntry {
                        path: format!(".pack-staging/chunks/{}-{}", chunk.pack, chunk.sha256),
                        sha256: chunk.sha256.clone(),
                        size: chunk.size,
                    });
                }
            }
        }
    }
    crate::disk::prepare_download_space(game_dir, &space).await?;
    let staging = game_dir.join(".pack-staging");
    tokio::fs::create_dir_all(&staging)
        .await
        .map_err(|e| format!("Create staging directory: {e}"))?;
    stream::iter(packs.into_iter().map(|(hash, chunks)| {
        let path = staging.join(&hash);
        let chunk_dir = staging.join("chunks");
        let url = format!("{base_url}/packs/{hash}");
        async move {
            if cold {
                fetch_pack(client, &url, &path, &hash, chunks[0].pack_size).await
            } else {
                fetch_chunks(client, &url, &path, &chunk_dir, &hash, &chunks).await
            }
        }
    }))
    .buffer_unordered(crate::MAX_PARALLEL_DOWNLOADS)
    .try_collect::<Vec<_>>()
    .await?;
    for (index, file) in files.iter().enumerate() {
        install_file(game_dir, &staging, file).await?;
        completed(
            &crate::FileEntry {
                path: file.path.clone(),
                sha256: file.sha256.clone(),
                size: file.size,
            },
            index + 1,
        )?;
    }
    Ok(())
}

async fn fetch_pack(
    client: &reqwest::Client,
    url: &str,
    path: &std::path::Path,
    hash: &str,
    size: u64,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    if valid_pack(path, hash, size).await? {
        return Ok(());
    }
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Fetch pack {hash}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Fetch pack {hash}: {}", response.status()));
    }
    let temporary = path.with_extension("part");
    let mut output = tokio::fs::File::create(&temporary)
        .await
        .map_err(|e| format!("Create {}: {e}", temporary.display()))?;
    let mut count = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Read pack {hash}: {e}"))?
    {
        count = count
            .checked_add(chunk.len() as u64)
            .ok_or("Pack size overflow")?;
        if count > size {
            return Err(format!("Pack {hash} exceeds declared size"));
        }
        output
            .write_all(&chunk)
            .await
            .map_err(|e| format!("Write pack {hash}: {e}"))?;
    }
    output
        .flush()
        .await
        .map_err(|e| format!("Flush pack {hash}: {e}"))?;
    drop(output);
    if !valid_pack(&temporary, hash, size).await? {
        return Err(format!("Corrupt pack {hash}"));
    }
    tokio::fs::rename(&temporary, path)
        .await
        .map_err(|e| format!("Stage pack {hash}: {e}"))?;
    Ok(())
}

async fn valid_pack(path: &std::path::Path, hash: &str, size: u64) -> Result<bool, String> {
    use tokio::io::AsyncReadExt;
    let mut file = match tokio::fs::File::open(path).await {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(format!("Open {}: {error}", path.display())),
    };
    if file.metadata().await.map_err(|e| e.to_string())?.len() != size {
        return Ok(false);
    }
    let mut magic = [0; 8];
    if file.read_exact(&mut magic).await.is_err() || &magic != b"OSSPACK\x01" {
        return Ok(false);
    }
    Ok(crate::hash_file(path).await.map_err(|e| e.to_string())? == hash)
}

async fn install_file(
    game_dir: &std::path::Path,
    staging: &std::path::Path,
    entry: &PackFile,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
    let destination = game_dir.join(&entry.path);
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Create {}: {e}", parent.display()))?;
    }
    let (partial, mut output) = create_install_partial(&destination).await?;
    let mut file_hash = sha2::Sha256::new();
    for chunk in &entry.chunks {
        let chunk_path = staging
            .join("chunks")
            .join(format!("{}-{}", chunk.pack, chunk.sha256));
        let mut data = vec![0; chunk.size as usize];
        match tokio::fs::File::open(&chunk_path).await {
            Ok(mut cached) => cached
                .read_exact(&mut data)
                .await
                .map_err(|e| format!("Read cached chunk: {e}"))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut pack = tokio::fs::File::open(staging.join(&chunk.pack))
                    .await
                    .map_err(|e| format!("Open pack: {e}"))?;
                pack.seek(std::io::SeekFrom::Start(chunk.offset))
                    .await
                    .map_err(|e| format!("Seek pack: {e}"))?;
                pack.read_exact(&mut data)
                    .await
                    .map_err(|e| format!("Read chunk: {e}"))?
            }
            Err(error) => return Err(format!("Open cached chunk: {error}")),
        };
        if hex::encode(sha2::Sha256::digest(&data)) != chunk.sha256 {
            return Err(format!("Corrupt chunk for {}", entry.path));
        }
        file_hash.update(&data);
        output
            .write_all(&data)
            .await
            .map_err(|e| format!("Write {}: {e}", partial.display()))?;
    }
    output
        .flush()
        .await
        .map_err(|e| format!("Flush {}: {e}", partial.display()))?;
    drop(output);
    if hex::encode(file_hash.finalize()) != entry.sha256 {
        return Err(format!("File digest mismatch: {}", entry.path));
    }
    tokio::fs::rename(&partial, &destination)
        .await
        .map_err(|e| format!("Install {}: {e}", destination.display()))?;
    Ok(())
}

fn reject_symlinks(root: &std::path::Path, relative: &str) -> Result<(), String> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("Symlink in install path: {}", path.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(format!("Stat {}: {error}", path.display())),
        }
    }
    Ok(())
}

async fn create_install_partial(
    destination: &std::path::Path,
) -> Result<(std::path::PathBuf, tokio::fs::File), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = destination.parent().ok_or("Destination has no parent")?;
    for _ in 0..100 {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".osso-pack-{}-{number}.part", std::process::id()));
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("Create {}: {error}", path.display())),
        }
    }
    Err("Cannot allocate a unique install staging file".into())
}

async fn fetch_chunks(
    client: &reqwest::Client,
    url: &str,
    pack_path: &std::path::Path,
    chunk_dir: &std::path::Path,
    pack_hash: &str,
    chunks: &[PackChunk],
) -> Result<(), String> {
    if valid_pack(pack_path, pack_hash, chunks[0].pack_size).await? {
        return Ok(());
    }
    tokio::fs::create_dir_all(chunk_dir)
        .await
        .map_err(|e| format!("Create chunk staging: {e}"))?;
    let mut unique = HashMap::new();
    for chunk in chunks {
        unique
            .entry(chunk.sha256.clone())
            .or_insert_with(|| chunk.clone());
    }
    let mut missing = Vec::new();
    for chunk in unique.into_values() {
        let path = chunk_dir.join(format!("{}-{}", chunk.pack, chunk.sha256));
        let valid = match tokio::fs::metadata(&path).await {
            Ok(meta) if meta.len() == chunk.size => {
                crate::hash_file(&path).await.map_err(|e| e.to_string())? == chunk.sha256
            }
            Ok(_) => false,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(format!("Stat {}: {e}", path.display())),
        };
        if !valid {
            missing.push(chunk);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let header = fetch_range(client, url, 0, 7, chunks[0].pack_size).await?;
    if header != b"OSSPACK\x01" {
        return Err(format!("Unsupported pack magic: {pack_hash}"));
    }
    missing.sort_by_key(|chunk| chunk.offset);
    let mut groups: Vec<Vec<PackChunk>> = Vec::new();
    for chunk in missing {
        if let Some(group) = groups.last_mut() {
            let last = group.last().unwrap();
            if chunk.offset <= last.offset + last.size + 65_536 {
                group.push(chunk);
                continue;
            }
        }
        groups.push(vec![chunk]);
    }
    for group in groups {
        let start = group[0].offset;
        let end = group
            .iter()
            .map(|chunk| chunk.offset + chunk.size)
            .max()
            .unwrap();
        let data = if end == start {
            Vec::new()
        } else {
            fetch_range(client, url, start, end - 1, group[0].pack_size).await?
        };
        for chunk in group {
            let begin = (chunk.offset - start) as usize;
            let body = &data[begin..begin + chunk.size as usize];
            if hex::encode(sha2::Sha256::digest(body)) != chunk.sha256 {
                return Err(format!("Corrupt chunk: {}", chunk.sha256));
            }
            let path = chunk_dir.join(format!("{}-{}", chunk.pack, chunk.sha256));
            let temporary = path.with_extension("part");
            tokio::fs::write(&temporary, body)
                .await
                .map_err(|e| format!("Write chunk: {e}"))?;
            tokio::fs::rename(&temporary, &path)
                .await
                .map_err(|e| format!("Stage chunk: {e}"))?;
        }
    }
    Ok(())
}

async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
    total: u64,
) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .send()
        .await
        .map_err(|e| format!("Fetch range: {e}"))?;
    let expected = format!("bytes {start}-{end}/{total}");
    if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
        || response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            != Some(expected.as_str())
    {
        return Err(format!("Invalid range response for {url}"));
    }
    let mut data = Vec::new();
    let length = (end - start + 1) as usize;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|e| format!("Read range: {e}"))?
    {
        if data.len() + bytes.len() > length {
            return Err("Range exceeds declared size".into());
        }
        data.extend_from_slice(&bytes);
    }
    if data.len() != length {
        return Err("Truncated range".into());
    }
    Ok(data)
}
