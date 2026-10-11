use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

#[test]
fn missing_player_spawn_returns_the_binary_path_and_error() {
    let dir = std::env::temp_dir().join(format!("launcher-spawn-error-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let error = spawn_player(&dir, "missing-player").unwrap_err();
    assert!(error.contains("missing-player"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn production_client_stops_a_stalled_response_body() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nx")
                .unwrap();
            thread::sleep(Duration::from_secs(32));
        });
        let mut response = http_client()
            .unwrap()
            .get(format!("http://{address}/stalled"))
            .send()
            .await
            .unwrap();
        let started = Instant::now();
        loop {
            match response.chunk().await {
                Ok(Some(_)) => continue,
                Err(error) => {
                    assert!(error.is_timeout(), "{error}");
                    assert!(started.elapsed() < Duration::from_secs(32));
                    break;
                }
                Ok(None) => panic!("stalled response completed without a read timeout"),
            }
        }
        server.join().unwrap();
    });
}

#[cfg(windows)]
#[test]
fn windows_long_install_path_can_receive_hash_and_spawn_an_extensionless_pe() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = std::env::temp_dir().join(format!("launcher-long-{}", std::process::id()));
        let mut directory = root.join("Player Unicode é space");
        for _ in 0..12 {
            directory = directory.join("long-installed-asset-directory");
        }
        assert!(directory.as_os_str().len() > 260);
        std::fs::create_dir_all(&directory).unwrap();
        let entry = FileEntry {
            path: "textures/asset.blp".into(),
            sha256: hex::encode(Sha256::digest(b"expected")),
            size: 8,
        };
        let (url, server) = crate::tests::response_server(b"expected");
        let base = url.strip_suffix("/texture.blp").unwrap();
        download_files(
            &http_client().unwrap(),
            base,
            &directory,
            &[entry.clone()],
            |_, _| Ok(()),
        )
        .await
        .unwrap();
        server.join().unwrap();
        assert_eq!(
            hash_file(&directory.join(&entry.path)).await.unwrap(),
            entry.sha256
        );
        let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
        std::fs::copy(
            system.join("System32/whoami.exe"),
            directory.join("game-engine-windows-x86_64"),
        )
        .unwrap();
        assert!(
            spawn_player(&directory, "game-engine-windows-x86_64")
                .unwrap()
                .wait()
                .unwrap()
                .success()
        );
        std::fs::remove_dir_all(root).unwrap();
    });
}
