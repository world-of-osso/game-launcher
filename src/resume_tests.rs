use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

#[test]
fn verified_files_survive_a_failed_batch_and_are_skipped_on_resume() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut replies = Vec::new();
            while replies.len() < 2 && Instant::now() < deadline {
                let Ok((mut socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                replies.push(thread::spawn(move || {
                    let mut bytes = [0; 4096];
                    let count = socket.read(&mut bytes).unwrap();
                    let bad = String::from_utf8_lossy(&bytes[..count]).contains("/bad.blp");
                    if bad {
                        thread::sleep(Duration::from_millis(200));
                    }
                    socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n",
                        )
                        .unwrap();
                    socket
                        .write_all(if bad { b"tampered" } else { b"expected" })
                        .unwrap();
                }));
            }
            assert_eq!(replies.len(), 2);
            for reply in replies {
                reply.join().unwrap();
            }
        });
        let dir = std::env::temp_dir().join(format!("launcher-resume-{}", std::process::id()));
        let entries: Vec<_> = ["good.blp", "bad.blp"]
            .into_iter()
            .map(|path| FileEntry {
                path: path.into(),
                sha256: hex::encode(Sha256::digest(b"expected")),
                size: 8,
            })
            .collect();
        let mut cache = cache::FileHashCache::default();
        let result = download_files(
            &http_client().unwrap(),
            &format!("http://{address}"),
            &dir,
            &entries,
            |entry, _| cache.record_verified(&dir, entry),
        )
        .await;
        server.join().unwrap();
        assert!(result.unwrap_err().contains("SHA256 mismatch"));
        cache.save(&dir).unwrap();
        assert_eq!(std::fs::read(dir.join("good.blp")).unwrap(), b"expected");
        assert!(!dir.join("bad.blp").exists());
        let mut cache = cache::FileHashCache::load(&dir).unwrap();
        let mut remaining = Vec::new();
        for entry in &entries {
            if cache.needs_update(&dir, entry).await.unwrap() {
                remaining.push(entry.clone());
            }
        }
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].path, "bad.blp");
        let (url, server) = crate::tests::response_server(b"expected");
        let base = url.strip_suffix("/texture.blp").unwrap();
        download_files(
            &http_client().unwrap(),
            base,
            &dir,
            &remaining,
            |entry, _| cache.record_verified(&dir, entry),
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(std::fs::read(dir.join("bad.blp")).unwrap(), b"expected");
        std::fs::remove_dir_all(dir).unwrap();
    });
}
