use crate::packs::{PackManifest, sync_packs};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{SystemTime, UNIX_EPOCH};

fn manifest(files: serde_json::Value) -> PackManifest {
    serde_json::from_value(serde_json::json!({
        "version": 2,
        "format_version": 1,
        "chunking": {"version": 1, "min": 262144, "average": 1048576, "max": 4194304},
        "files": files,
    }))
    .unwrap()
}

fn file(path: &str) -> serde_json::Value {
    serde_json::json!({"path":path,"size":0,"sha256":"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", "chunks": []})
}

#[test]
fn rejects_unsafe_paths_and_windows_collisions() {
    for path in [
        "../escape",
        "/absolute",
        "C:/evil",
        "foo/CON.txt",
        "foo/a.",
        "foo\\bar",
    ] {
        assert!(
            manifest(serde_json::json!([file(path)]))
                .validate()
                .is_err(),
            "{path}"
        );
    }
    assert!(
        manifest(serde_json::json!([file("Foo/a"), file("foo/A")]))
            .validate()
            .is_err()
    );
}

#[test]
fn cold_install_retries_transient_failure_and_verifies_a_pack() {
    let mut bytes = b"OSSPACK\x01".to_vec();
    bytes.extend_from_slice(&5u32.to_le_bytes());
    bytes.extend_from_slice(b"hello");
    let pack_hash = hex::encode(Sha256::digest(&bytes));
    let mut entry = file("dir/a");
    entry["size"] = 5.into();
    entry["sha256"] = hex::encode(Sha256::digest(b"hello")).into();
    entry["chunks"] = serde_json::json!([{"sha256":entry["sha256"], "size":5,"pack":pack_hash,"offset":12,"pack_size":bytes.len()}]);
    let manifest = manifest(serde_json::json!([entry]));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut failed, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        failed.read(&mut request).unwrap();
        failed.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nRetry-After: 0\r\nConnection: close\r\n\r\n").unwrap();
        let (mut socket, _) = listener.accept().unwrap();
        let count = socket.read(&mut request).unwrap();
        assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /packs/"));
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .unwrap();
        socket.write_all(&bytes).unwrap();
    });
    let dir = std::env::temp_dir().join(format!(
        "pack-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("dir")).unwrap();
    std::fs::write(dir.join("dir/a.part"), b"user data").unwrap();
    let chunk = &manifest.files[0].chunks[0];
    let chunks = dir.join(".pack-staging/chunks");
    std::fs::create_dir_all(&chunks).unwrap();
    std::fs::write(
        chunks.join(format!("{}-{}", chunk.pack, chunk.sha256)),
        b"wrong",
    )
    .unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(sync_packs(
            &crate::http_client().unwrap(),
            &format!("http://{address}"),
            &dir,
            &manifest,
            &["dir/a".into()],
            true,
            |_, _| Ok(()),
        ))
        .unwrap();
    server.join().unwrap();
    assert_eq!(std::fs::read(dir.join("dir/a")).unwrap(), b"hello");
    assert_eq!(std::fs::read(dir.join("dir/a.part")).unwrap(), b"user data");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn delta_fetches_ranges_and_reuses_verified_chunk_on_resume() {
    let mut bytes = b"OSSPACK\x01".to_vec();
    bytes.extend_from_slice(&5u32.to_le_bytes());
    bytes.extend_from_slice(b"hello");
    let pack_hash = hex::encode(Sha256::digest(&bytes));
    let mut entry = file("a");
    entry["size"] = 5.into();
    entry["sha256"] = hex::encode(Sha256::digest(b"hello")).into();
    entry["chunks"] = serde_json::json!([{"sha256":entry["sha256"], "size":5,"pack":pack_hash,"offset":12,"pack_size":bytes.len()}]);
    let manifest = manifest(serde_json::json!([entry]));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let count = socket.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]).to_lowercase();
            let range = request
                .lines()
                .find_map(|line| line.strip_prefix("range: bytes="))
                .unwrap();
            let (start, end) = range.split_once('-').unwrap();
            let start: usize = start.trim().parse().unwrap();
            let end: usize = end.trim().parse().unwrap();
            write!(socket, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n", end - start + 1, start, end, bytes.len()).unwrap();
            socket.write_all(&bytes[start..=end]).unwrap();
        }
    });
    let dir = std::env::temp_dir().join(format!(
        "pack-delta-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = crate::http_client().unwrap();
    runtime
        .block_on(sync_packs(
            &client,
            &format!("http://{address}"),
            &dir,
            &manifest,
            &["a".into()],
            false,
            |_, _| Ok(()),
        ))
        .unwrap();
    server.join().unwrap();
    assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"hello");
    std::fs::remove_file(dir.join("a")).unwrap();
    runtime
        .block_on(sync_packs(
            &client,
            &format!("http://{address}"),
            &dir,
            &manifest,
            &["a".into()],
            false,
            |_, _| Ok(()),
        ))
        .unwrap();
    assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"hello");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_incorrect_record_length_without_replacing_file() {
    let mut bytes = b"OSSPACK\x01".to_vec();
    bytes.extend_from_slice(&6u32.to_le_bytes());
    bytes.extend_from_slice(b"hello");
    let pack_hash = hex::encode(Sha256::digest(&bytes));
    let mut entry = file("a");
    entry["size"] = 5.into();
    entry["sha256"] = hex::encode(Sha256::digest(b"hello")).into();
    entry["chunks"] = serde_json::json!([{"sha256":entry["sha256"], "size":5,"pack":pack_hash,"offset":12,"pack_size":bytes.len()}]);
    let manifest = manifest(serde_json::json!([entry]));
    let dir = std::env::temp_dir().join(format!(
        "pack-bad-record-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join(".pack-staging")).unwrap();
    std::fs::write(dir.join(".pack-staging").join(pack_hash), &bytes).unwrap();
    std::fs::write(dir.join("a"), b"old").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(sync_packs(
        &crate::http_client().unwrap(),
        "http://127.0.0.1:9",
        &dir,
        &manifest,
        &["a".into()],
        true,
        |_, _| Ok(()),
    ));
    assert!(result.is_err());
    assert_eq!(std::fs::read(dir.join("a")).unwrap(), b"old");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_range_when_server_ignores_it() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        socket.read(&mut request).unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nOSSPACK\x01",
            )
            .unwrap();
    });
    let mut entry = file("a");
    entry["size"] = 1.into();
    entry["sha256"] = hex::encode(Sha256::digest(b"x")).into();
    entry["chunks"] = serde_json::json!([{"sha256":entry["sha256"], "size":1,"pack":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","offset":12,"pack_size":13}]);
    let manifest = manifest(serde_json::json!([entry]));
    let dir = std::env::temp_dir().join(format!(
        "pack-bad-range-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(sync_packs(
        &crate::http_client().unwrap(),
        &format!("http://{address}"),
        &dir,
        &manifest,
        &["a".into()],
        false,
        |_, _| Ok(()),
    ));
    server.join().unwrap();
    assert!(result.unwrap_err().contains("Invalid range response"));
    assert!(!dir.join("a").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_destination_directory_before_writing() {
    use std::os::unix::fs::symlink;
    let root = std::env::temp_dir().join(format!(
        "pack-link-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let install = root.join("install");
    let outside = root.join("outside");
    std::fs::create_dir_all(&install).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    symlink(&outside, install.join("link")).unwrap();
    let manifest = manifest(serde_json::json!([file("link/empty")]));
    let runtime = tokio::runtime::Runtime::new().unwrap();
    assert!(
        runtime
            .block_on(sync_packs(
                &crate::http_client().unwrap(),
                "http://127.0.0.1:9",
                &install,
                &manifest,
                &["link/empty".into()],
                false,
                |_, _| Ok(())
            ))
            .is_err()
    );
    assert!(!outside.join("empty").exists());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_unsupported_format_and_out_of_bounds_chunk() {
    let mut value = manifest(serde_json::json!([file("a")]));
    value.format_version = 2;
    assert!(value.validate().is_err());
    let mut entry = file("a");
    entry["size"] = 1.into();
    entry["chunks"] = serde_json::json!([{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size":1,"pack":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "offset":50,"pack_size":20}]);
    assert!(manifest(serde_json::json!([entry])).validate().is_err());
}
