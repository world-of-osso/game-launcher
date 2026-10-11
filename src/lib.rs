//! Player file transfer shared by the desktop UI and CLI.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size: u64,
}

pub async fn download_file(
    client: &reqwest::Client,
    url: &str,
    game_dir: &Path,
    entry: &FileEntry,
) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("Download {}: {error}", entry.path))?;
    let destination = game_dir.join(&entry.path);
    let mut name = destination.as_os_str().to_os_string();
    name.push(".part");
    let partial = PathBuf::from(name);
    let result = write_verified_response(response, &partial, entry).await;
    if let Err(error) = result {
        if let Err(cleanup) = tokio::fs::remove_file(&partial).await
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            return Err(format!("{error}; remove {}: {cleanup}", partial.display()));
        }
        return Err(error);
    }
    tokio::fs::rename(&partial, &destination)
        .await
        .map_err(|error| format!("Install {}: {error}", destination.display()))
}

async fn write_verified_response(
    mut response: reqwest::Response,
    partial: &Path,
    entry: &FileEntry,
) -> Result<(), String> {
    let mut file = create_partial_file(partial).await?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|error| format!("Read {}: {error}", entry.path))?
    {
        size += bytes.len() as u64;
        if size > entry.size {
            return Err(format!(
                "Size mismatch for {}: exceeds {} bytes",
                entry.path, entry.size
            ));
        }
        hasher.update(&bytes);
        file.write_all(&bytes)
            .await
            .map_err(|error| format!("Write {}: {error}", partial.display()))?;
    }
    file.flush()
        .await
        .map_err(|error| format!("Flush {}: {error}", partial.display()))?;
    verify_file_receipt(entry, size, &hex::encode(hasher.finalize()))
}

async fn create_partial_file(path: &Path) -> Result<tokio::fs::File, String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("Create {}: {error}", parent.display()))?;
    }
    tokio::fs::File::create(path)
        .await
        .map_err(|error| format!("Create {}: {error}", path.display()))
}

fn verify_file_receipt(entry: &FileEntry, size: u64, actual: &str) -> Result<(), String> {
    if size != entry.size {
        return Err(format!(
            "Size mismatch for {}: expected {}, got {size}",
            entry.path, entry.size
        ));
    }
    if actual != entry.sha256 {
        return Err(format!(
            "SHA256 mismatch for {}: expected {}, got {actual}",
            entry.path, entry.sha256
        ));
    }
    Ok(())
}

pub async fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut bytes = vec![0; HASH_BUFFER_BYTES];
    let mut hasher = Sha256::new();
    loop {
        let count = file.read(&mut bytes).await?;
        if count == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&bytes[..count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn response_server(bytes: &'static [u8]) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            socket.write_all(bytes).unwrap();
        });
        (format!("http://{address}/texture.blp"), handle)
    }

    #[test]
    fn wrong_download_hash_never_replaces_the_installed_file() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let directory =
                std::env::temp_dir().join(format!("launcher-integrity-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join("texture.blp"), b"previous").unwrap();
            let entry = FileEntry {
                path: "texture.blp".into(),
                sha256: hex::encode(Sha256::digest(b"expected")),
                size: 8,
            };
            let (url, server) = response_server(b"tampered");
            let result = download_file(&reqwest::Client::new(), &url, &directory, &entry).await;
            server.join().unwrap();
            assert!(result.is_err(), "corrupt downloaded bytes accepted");
            assert_eq!(
                std::fs::read(directory.join("texture.blp")).unwrap(),
                b"previous"
            );
            std::fs::remove_dir_all(directory).unwrap();
        });
    }
}
