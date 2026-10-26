use crate::packs::PackManifest;

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
fn rejects_unsupported_format_and_out_of_bounds_chunk() {
    let mut value = manifest(serde_json::json!([file("a")]));
    value.format_version = 2;
    assert!(value.validate().is_err());
    let mut entry = file("a");
    entry["size"] = 1.into();
    entry["chunks"] = serde_json::json!([{"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size":1,"pack":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "offset":50,"pack_size":20}]);
    assert!(manifest(serde_json::json!([entry])).validate().is_err());
}
