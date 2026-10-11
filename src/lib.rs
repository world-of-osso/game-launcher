//! Player file transfer shared by the desktop UI and CLI.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub mod cache;
mod disk;
mod retry;

pub const MAX_PARALLEL_DOWNLOADS: usize = 4;

pub async fn download_files(
    client: &reqwest::Client,
    base_url: &str,
    game_dir: &Path,
    entries: &[FileEntry],
    mut completed: impl FnMut(&FileEntry, usize) -> Result<(), String>,
) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    disk::prepare_download_space(game_dir, entries).await?;
    let transfer = |entry: &FileEntry| {
        let client = client.clone();
        let url = format!("{base_url}/{}", entry.path);
        let game_dir = game_dir.to_path_buf();
        let entry = entry.clone();
        async move {
            download_file(&client, &url, &game_dir, &entry).await?;
            Ok::<_, String>(entry)
        }
    };
    let mut remaining = entries.iter();
    let mut tasks = tokio::task::JoinSet::new();
    for entry in remaining.by_ref().take(MAX_PARALLEL_DOWNLOADS) {
        tasks.spawn(transfer(entry));
    }
    let mut count = 0;
    while let Some(result) = tasks.join_next().await {
        let entry = result.map_err(|error| format!("Download task failed: {error}"))??;
        count += 1;
        completed(&entry, count)?;
        if let Some(entry) = remaining.next() {
            tasks.spawn(transfer(entry));
        }
    }
    Ok(())
}
use retry::{TransferFailure, retry_delay};

pub fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(concat!("WorldOfOsso-Launcher/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(30))
        .build()
}

#[cfg(unix)]
pub fn ensure_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let meta =
        std::fs::metadata(path).map_err(|error| format!("Stat {}: {error}", path.display()))?;
    if meta.permissions().mode() & 0o111 == 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Make {} executable: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn ensure_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub fn spawn_player(game_dir: &Path, entry_name: &str) -> Result<std::process::Child, String> {
    let directory = std::fs::canonicalize(game_dir)
        .map_err(|error| format!("Resolve install directory {}: {error}", game_dir.display()))?;
    let binary = directory.join(entry_name);
    ensure_executable(&binary)?;
    // CreateProcessW rejects a CWD beyond MAX_PATH. Windows player entries resolve
    // their installation from their executable, not from the process CWD.
    #[cfg(windows)]
    let working_dir = directory
        .ancestors()
        .last()
        .expect("canonical path is rooted");
    #[cfg(not(windows))]
    let working_dir = &directory;
    std::process::Command::new(&binary)
        .current_dir(working_dir)
        .spawn()
        .map_err(|error| format!("Start {}: {error}", binary.display()))
}

const HASH_BUFFER_BYTES: usize = 64 * 1024;
const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;

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
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        match download_once(client, url, game_dir, entry).await {
            Ok(()) => return Ok(()),
            Err(failure) => {
                let Some(server_delay) = failure.retry_after else {
                    return Err(failure.message);
                };
                if attempt + 1 == MAX_DOWNLOAD_ATTEMPTS {
                    return Err(failure.message);
                }
                let delay = retry_delay(attempt, server_delay)?;
                eprintln!(
                    "{}; retry {} in {:.2}s",
                    failure.message,
                    attempt + 1,
                    delay.as_secs_f64()
                );
                tokio::time::sleep(delay).await;
            }
        }
    }
    unreachable!("download attempt limit must be positive")
}

async fn download_once(
    client: &reqwest::Client,
    url: &str,
    game_dir: &Path,
    entry: &FileEntry,
) -> Result<(), TransferFailure> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| TransferFailure::network(&entry.path, error))?;
    if !response.status().is_success() {
        return Err(TransferFailure::status(&entry.path, &response));
    }
    let destination = game_dir.join(&entry.path);
    let partial = partial_path(&destination);
    let result = write_verified_response(response, &partial, entry).await;
    if let Err(error) = result {
        if let Err(cleanup) = tokio::fs::remove_file(&partial).await
            && cleanup.kind() != std::io::ErrorKind::NotFound
        {
            return Err(
                format!("{}; remove {}: {cleanup}", error.message, partial.display()).into(),
            );
        }
        return Err(error);
    }
    tokio::fs::rename(&partial, &destination)
        .await
        .map_err(|error| format!("Install {}: {error}", destination.display()).into())
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

async fn write_verified_response(
    mut response: reqwest::Response,
    partial: &Path,
    entry: &FileEntry,
) -> Result<(), TransferFailure> {
    let mut file = create_partial_file(partial).await?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    while let Some(bytes) = response
        .chunk()
        .await
        .map_err(|error| TransferFailure::network(&entry.path, error))?
    {
        size += bytes.len() as u64;
        if size > entry.size {
            return Err(format!(
                "Size mismatch for {}: exceeds {} bytes",
                entry.path, entry.size
            )
            .into());
        }
        hasher.update(&bytes);
        file.write_all(&bytes)
            .await
            .map_err(|error| format!("Write {}: {error}", partial.display()))?;
    }
    file.flush()
        .await
        .map_err(|error| format!("Flush {}: {error}", partial.display()))?;
    verify_file_receipt(entry, size, &hex::encode(hasher.finalize())).map_err(TransferFailure::from)
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
mod resume_tests;
#[cfg(test)]
mod runtime_tests;
#[cfg(test)]
mod scheduler_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    pub(super) fn response_server(bytes: &'static [u8]) -> (String, thread::JoinHandle<()>) {
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
    fn disk_preflight_refuses_an_impossible_update_before_http() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                while std::time::Instant::now() < deadline {
                    if let Ok((mut socket, _)) = listener.accept() {
                        socket.set_nonblocking(false).unwrap();
                        socket.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
                        let mut request = [0; 4096];
                        socket.read(&mut request).unwrap();
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nexpected").unwrap();
                        return true;
                    }
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                false
            });
            let directory = std::env::temp_dir().join(format!("launcher-disk-{}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let entry = FileEntry { path: "asset.blp".into(), sha256: hex::encode(Sha256::digest(b"expected")), size: 1 << 60 };
            let result = download_files(&reqwest::Client::new(), &format!("http://{address}"), &directory, &[entry], |_, _| Ok(())).await;
            let requested = server.join().unwrap();
            std::fs::remove_dir_all(directory).unwrap();
            assert!(result.unwrap_err().contains("disk space"));
            assert!(!requested, "preflight must precede all GETs");
        });
    }

    #[test]
    fn retry_after_is_respected_before_a_transient_get_is_retried() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let start = std::time::Instant::now();
                let mut arrivals = Vec::new();
                while start.elapsed() < std::time::Duration::from_secs(3) && arrivals.len() < 2 {
                    let Ok((mut socket, _)) = listener.accept() else {
                        thread::sleep(std::time::Duration::from_millis(5));
                        continue;
                    };
                    socket.set_nonblocking(false).unwrap();
                    socket.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
                    arrivals.push(start.elapsed());
                    let mut request = [0; 4096];
                    socket.read(&mut request).unwrap();
                    let response = if arrivals.len() == 1 {
                        b"HTTP/1.1 503 Unavailable\r\nRetry-After: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice()
                    } else {
                        b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nexpected".as_slice()
                    };
                    socket.write_all(response).unwrap();
                }
                arrivals
            });
            let directory = std::env::temp_dir().join(format!("launcher-retry-{}", std::process::id()));
            let entry = FileEntry { path: "texture.blp".into(), sha256: hex::encode(Sha256::digest(b"expected")), size: 8 };
            let result = download_file(&reqwest::Client::new(), &format!("http://{address}/texture.blp"), &directory, &entry).await;
            let arrivals = server.join().unwrap();
            assert!(result.is_ok(), "transient GET not retried: {result:?}");
            assert_eq!(arrivals.len(), 2);
            assert!(arrivals[1] - arrivals[0] >= std::time::Duration::from_secs(1));
            assert_eq!(std::fs::read(directory.join("texture.blp")).unwrap(), b"expected");
            std::fs::remove_dir_all(directory).unwrap();
        });
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
