use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn update_uses_packs_when_v2_manifest_exists() {
    let mut pack = b"OSSPACK\x01".to_vec();
    pack.extend_from_slice(&5u32.to_le_bytes());
    pack.extend_from_slice(b"hello");
    let pack_hash = hex::encode(Sha256::digest(&pack));
    let file_hash = hex::encode(Sha256::digest(b"hello"));
    let manifest = serde_json::json!({"version":2,"format_version":1,"chunking":{"version":1,"min":262144,"average":1048576,"max":4194304},"files":[{"path":"fixture.txt","size":5,"sha256":file_hash,"chunks":[{"sha256":file_hash,"size":5,"pack":pack_hash,"offset":12,"pack_size":pack.len()}]}]}).to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let mut paths = Vec::new();
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let n = socket.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap();
            paths.push(path.to_string());
            let (status, body): (&str, &[u8]) = if path == "/launcher.json" {
                ("404 Not Found", b"")
            } else if path == "/manifest-v2.json" {
                ("200 OK", manifest.as_bytes())
            } else if path.starts_with("/packs/") {
                ("200 OK", &pack)
            } else {
                ("404 Not Found", b"")
            };
            write!(
                socket,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(body).unwrap();
        }
        paths
    });
    let dir = std::env::temp_dir().join(format!(
        "pack-cli-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_game-launcher"))
        .arg("update")
        .env("GAME_LAUNCHER_BASE_URL", format!("http://{address}"))
        .env(
            "GAME_LAUNCHER_MANIFEST_URL",
            format!("http://{address}/launcher.json"),
        )
        .env("GAME_LAUNCHER_GAME_DIR", &dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read(dir.join("fixture.txt")).unwrap(), b"hello");
    let paths = server.join().unwrap();
    assert!(paths.contains(&"/manifest-v2.json".to_string()));
    std::fs::remove_dir_all(dir).unwrap();
}
