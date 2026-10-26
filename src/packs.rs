//! Versioned content-addressed player packs.
use serde::{Deserialize, Serialize};
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
