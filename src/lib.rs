//! Player file transfer shared by the desktop UI and CLI.

use serde::{Deserialize, Serialize};
use std::path::Path;

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
    let bytes = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("Download error for {}: {e}", entry.path))?
        .bytes()
        .await
        .map_err(|e| format!("Read error: {e}"))?;
    let local_path = game_dir.join(&entry.path);
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("mkdir error: {e}"))?;
    }
    tokio::fs::write(&local_path, bytes)
        .await
        .map_err(|e| format!("Write error: {e}"))?;
    Ok(())
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

    #[tokio::test]
    async fn wrong_download_hash_never_replaces_the_installed_file() {
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
    }
}
