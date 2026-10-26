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
fn cold_install_fetches_and_verifies_a_pack() {
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
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
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
    std::fs::remove_dir_all(dir).unwrap();
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
