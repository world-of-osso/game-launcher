use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
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

fn pack_manifest(content: &[u8]) -> (String, Vec<u8>) {
    let mut pack = b"OSSPACK\x01".to_vec();
    pack.extend_from_slice(&(content.len() as u32).to_le_bytes());
    pack.extend_from_slice(content);
    let pack_hash = hex::encode(Sha256::digest(&pack));
    let file_hash = hex::encode(Sha256::digest(content));
    let manifest = serde_json::json!({"version":2,"format_version":1,"chunking":{"version":1,"min":262144,"average":1048576,"max":4194304},"files":[{"path":"fixture.txt","size":content.len(),"sha256":file_hash,"chunks":[{"sha256":file_hash,"size":content.len(),"pack":pack_hash,"offset":12,"pack_size":pack.len()}]}]}).to_string();
    (manifest, pack)
}

/// Serves one manifest revision (ETag `etag`) and its pack until `done`; returns each
/// request path with its If-None-Match header.
fn serve_revision(
    listener: &TcpListener,
    done: &AtomicBool,
    etag: &str,
    content: &[u8],
) -> Vec<(String, Option<String>)> {
    let (manifest, pack) = pack_manifest(content);
    let mut seen = Vec::new();
    loop {
        let mut socket = match listener.accept() {
            Ok((socket, _)) => socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if done.load(Ordering::SeqCst) {
                    return seen;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            Err(error) => panic!("{error}"),
        };
        socket.set_nonblocking(false).unwrap();
        let mut buf = [0; 4096];
        let n = socket.read(&mut buf).unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).to_string();
        let path = request.split_whitespace().nth(1).unwrap().to_string();
        let if_none_match = request
            .lines()
            .find_map(|line| line.strip_prefix("if-none-match: "))
            .map(str::to_owned);
        let range = request
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes="))
            .and_then(|range| range.split_once('-'))
            .map(|(start, end)| {
                (
                    start.parse::<usize>().unwrap(),
                    end.parse::<usize>().unwrap(),
                )
            });
        let (status, body, extra): (&str, &[u8], String) = match path.as_str() {
            "/manifest-v2.json" if if_none_match.as_deref() == Some(etag) => {
                ("304 Not Modified", b"", String::new())
            }
            "/manifest-v2.json" => ("200 OK", manifest.as_bytes(), String::new()),
            p if p.starts_with("/packs/") => match range {
                Some((start, end)) => (
                    "206 Partial Content",
                    &pack[start..=end],
                    format!("Content-Range: bytes {start}-{end}/{}\r\n", pack.len()),
                ),
                None => ("200 OK", &pack, String::new()),
            },
            _ => ("404 Not Found", b"", String::new()),
        };
        write!(
            socket,
            "HTTP/1.1 {status}\r\nETag: {etag}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
        seen.push((path, if_none_match));
    }
}

#[test]
fn unchanged_manifest_costs_one_conditional_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let dir = std::env::temp_dir().join(format!(
        "pack-cli-etag-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let update = || {
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
    };
    let paths = |seen: &[(String, Option<String>)]| -> Vec<String> {
        seen.iter().map(|(path, _)| path.clone()).collect()
    };

    let revision = |etag: &str, content: &[u8]| {
        let done = AtomicBool::new(false);
        std::thread::scope(|scope| {
            let server = scope.spawn(|| serve_revision(&listener, &done, etag, content));
            update();
            done.store(true, Ordering::SeqCst);
            server.join().unwrap()
        })
    };
    {
        let seen = revision("\"rev1\"", b"hello");
        assert!(paths(&seen).iter().any(|path| path.starts_with("/packs/")));
        assert_eq!(std::fs::read(dir.join("fixture.txt")).unwrap(), b"hello");

        let seen = revision("\"rev1\"", b"hello");
        assert_eq!(paths(&seen), ["/launcher.json", "/manifest-v2.json"]);
        assert_eq!(seen[1].1.as_deref(), Some("\"rev1\""));

        let seen = revision("\"rev2\"", b"world");
        assert!(paths(&seen).iter().any(|path| path.starts_with("/packs/")));
        assert_eq!(std::fs::read(dir.join("fixture.txt")).unwrap(), b"world");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
