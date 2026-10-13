//! Verified launcher replacement and restart.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LauncherManifest {
    pub version: String,
    pub platforms: BTreeMap<String, LauncherRelease>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LauncherRelease {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    /// Retained for website download-link consumers.
    pub file: String,
}
pub const MANIFEST_URL: &str = "https://worldofosso.com/downloads/launcher.json";
pub const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

/// The UI uses the same wait boundary for both manifests and explicit Retry.
pub async fn wait_for_recheck<S>(requests: &mut S, interval: std::time::Duration) -> bool
where
    S: futures_util::Stream<Item = ()> + Unpin,
{
    use futures_util::StreamExt;
    tokio::select! {
        _ = tokio::time::sleep(interval) => true,
        request = requests.next() => request.is_some(),
    }
}
const START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub fn newer_version(candidate: &str, installed: &str) -> Result<bool, String> {
    let parse = |value| {
        semver::Version::parse(value).map_err(|e| format!("Invalid launcher version {value}: {e}"))
    };
    Ok(parse(candidate)?.cmp_precedence(&parse(installed)?) == std::cmp::Ordering::Greater)
}

fn sibling(executable: &Path, suffix: &str) -> PathBuf {
    let mut path = executable.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn validate_url(url: &reqwest::Url) -> Result<(), String> {
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        return Ok(());
    }
    Err(format!(
        "Launcher updates require HTTPS (HTTP allowed only on loopback): {url}"
    ))
}

pub fn update_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .read_timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("Too many redirects");
            }
            match validate_url(attempt.url()) {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(e),
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn stage_release(
    client: &reqwest::Client,
    release: &LauncherRelease,
    executable: &Path,
) -> Result<PathBuf, String> {
    validate_url(&reqwest::Url::parse(&release.url).map_err(|e| e.to_string())?)?;
    if release.size == 0 || release.sha256.len() != 64 || hex::decode(&release.sha256).is_err() {
        return Err("Invalid launcher size or SHA256".into());
    }
    let staged = sibling(executable, ".new");
    let directory = executable
        .parent()
        .ok_or("Launcher has no parent directory")?;
    let entry = crate::FileEntry {
        path: staged
            .file_name()
            .ok_or("Launcher has no filename")?
            .to_string_lossy()
            .into_owned(),
        sha256: release.sha256.clone(),
        size: release.size,
    };
    crate::disk::prepare_download_space(directory, std::slice::from_ref(&entry)).await?;
    // Remove only our previous stage; on Windows rename cannot replace an existing file.
    remove_if_present(&staged)?;
    crate::download_file(client, &release.url, directory, &entry).await?;
    crate::ensure_executable(&staged)?;
    Ok(staged)
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Remove {}: {e}", path.display())),
    }
}

pub fn install_staged(executable: &Path, staged: &Path, windows: bool) -> Result<(), String> {
    let backup = sibling(executable, ".old");
    if backup.exists() {
        return Err(format!(
            "Previous launcher backup still exists: {}",
            backup.display()
        ));
    }
    if windows {
        std::fs::rename(executable, &backup)
    } else {
        std::fs::hard_link(executable, &backup)
    }
    .map_err(|e| format!("Back up launcher: {e}"))?;
    if let Err(e) = std::fs::rename(staged, executable) {
        rollback(executable, windows)?;
        return Err(format!("Install launcher: {e}"));
    }
    Ok(())
}

pub fn rollback(executable: &Path, windows: bool) -> Result<(), String> {
    if windows {
        remove_if_present(executable)?;
    }
    std::fs::rename(sibling(executable, ".old"), executable)
        .map_err(|e| format!("Restore previous launcher: {e}"))
}

/// Self-check is a bounded real process handshake, not just a successful spawn.
async fn check_binary(staged: &Path, version: &str) -> Result<(), String> {
    let mut command = tokio::process::Command::new(staged);
    command.arg("--self-check").kill_on_drop(true);
    let output = tokio::time::timeout(START_TIMEOUT, command.output())
        .await
        .map_err(|_| "New launcher self-check timed out".to_string())?
        .map_err(|e| format!("Start new launcher self-check: {e}"))?;
    if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != version {
        return Err("New launcher failed version/startup self-check".into());
    }
    Ok(())
}

/// Child acknowledges only after CLI startup or the GUI App component mounts.
pub fn acknowledge_start(version: &str) -> Result<(), String> {
    if let Some(path) = std::env::var_os("GAME_LAUNCHER_START_ACK") {
        std::fs::write(path, version).map_err(|e| format!("Acknowledge launcher startup: {e}"))?;
    }
    Ok(())
}

pub fn cleanup_previous(executable: &Path, version: &str) -> Result<(), String> {
    // The updater parent is still running from .old during the child handshake.
    static FIRST_CHECK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
    let first_check = FIRST_CHECK.swap(false, std::sync::atomic::Ordering::Relaxed);
    if first_check && std::env::var_os("GAME_LAUNCHER_START_ACK").is_some() {
        return Ok(());
    }
    let ready = sibling(executable, ".ready");
    if sibling(executable, ".old").exists() {
        let confirmed = std::fs::read_to_string(&ready)
            .map_err(|e| format!("Read launcher startup receipt: {e}"))?;
        if confirmed != version {
            return Err("Previous launcher update has no matching startup receipt".into());
        }
        remove_if_present(&sibling(executable, ".old"))?;
    }
    remove_if_present(&ready)
}

async fn wait_for_start(
    child: &mut tokio::process::Child,
    ready: &Path,
    version: &str,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if std::fs::read_to_string(ready).is_ok_and(|value| value == version) {
            return Ok(());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Err(format!(
                "New launcher exited before startup handshake: {status}"
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("New launcher startup handshake timed out".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

pub async fn replace_and_restart(
    executable: &Path,
    staged: &Path,
    version: &str,
    arguments: &[std::ffi::OsString],
) -> Result<(), String> {
    check_binary(staged, version).await?;
    let ready = sibling(executable, ".ready");
    remove_if_present(&ready)?;
    install_staged(executable, staged, cfg!(windows))?;
    let result = async {
        let mut child = tokio::process::Command::new(executable)
            .args(arguments)
            .env("GAME_LAUNCHER_START_ACK", &ready)
            .spawn()
            .map_err(|e| format!("Relaunch launcher: {e}"))?;
        if let Err(error) = wait_for_start(&mut child, &ready, version).await {
            if child.try_wait().map_err(|e| e.to_string())?.is_none() {
                child
                    .kill()
                    .await
                    .map_err(|e| format!("Stop failed launcher: {e}"))?;
            }
            return Err(error);
        }
        // Ownership passes to the restarted launcher only after acknowledgement.
        Ok(())
    }
    .await;
    if let Err(error) = result {
        rollback(executable, cfg!(windows))?;
        remove_if_present(&ready)?;
        return Err(error);
    }
    Ok(())
}

async fn fetch_manifest_once(
    client: &reqwest::Client,
    url: &str,
) -> Result<LauncherManifest, crate::retry::TransferFailure> {
    use crate::retry::TransferFailure;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| TransferFailure::network(url, e))?;
    if !response.status().is_success() {
        return Err(TransferFailure::status(url, &response));
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| TransferFailure::network(url, e))?;
    serde_json::from_slice(&body)
        .map_err(|e| format!("Parse per-platform launcher manifest: {e}").into())
}

async fn fetch_launcher_manifest(
    client: &reqwest::Client,
    url: &str,
) -> Result<LauncherManifest, String> {
    for attempt in 0..crate::MAX_DOWNLOAD_ATTEMPTS {
        match fetch_manifest_once(client, url).await {
            Ok(manifest) => return Ok(manifest),
            Err(failure) => {
                let Some(delay) = failure.retry_after else {
                    return Err(failure.message);
                };
                if attempt + 1 == crate::MAX_DOWNLOAD_ATTEMPTS {
                    return Err(failure.message);
                }
                tokio::time::sleep(crate::retry::retry_delay(attempt, delay)?).await;
            }
        }
    }
    unreachable!("download attempt limit must be positive")
}

pub async fn check_for_update(
    executable: &Path,
    installed: &str,
    platform: &str,
    manifest_url: &str,
    arguments: &[std::ffi::OsString],
) -> Result<bool, String> {
    validate_url(&reqwest::Url::parse(manifest_url).map_err(|e| e.to_string())?)?;
    let client = update_client()?;
    let manifest = fetch_launcher_manifest(&client, manifest_url).await?;
    let release = manifest
        .platforms
        .get(platform)
        .ok_or_else(|| format!("No launcher release for {platform}"))?;
    if !newer_version(&release.version, installed)? {
        return Ok(false);
    }
    let staged = stage_release(&client, release, executable).await?;
    let result = replace_and_restart(executable, &staged, &release.version, arguments).await;
    if result.is_err() {
        remove_if_present(&staged)?;
    }
    result.map(|()| true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "launcher-self-update-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn serve(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/binary", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            socket.read(&mut request).unwrap();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        });
        (url, handle)
    }
    #[test]
    fn periodic_and_retry_waits_recheck_a_changed_http_manifest() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/manifest.json", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                for body in [
                    b"first".as_slice(),
                    b"second".as_slice(),
                    b"retry".as_slice(),
                ] {
                    let (mut socket, _) = listener.accept().unwrap();
                    let mut request = [0; 4096];
                    socket.read(&mut request).unwrap();
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .unwrap();
                    socket.write_all(body).unwrap();
                }
            });
            let client = update_client().unwrap();
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "first"
            );
            let mut requests = futures_util::stream::pending();
            assert!(wait_for_recheck(&mut requests, std::time::Duration::from_millis(20)).await);
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "second"
            );
            let mut requests = futures_util::stream::iter([()]);
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    wait_for_recheck(&mut requests, RECHECK_INTERVAL)
                )
                .await
                .unwrap()
            );
            assert_eq!(
                client.get(&url).send().await.unwrap().text().await.unwrap(),
                "retry"
            );
            server.join().unwrap();
        });
    }

    #[test]
    fn manifest_retries_transient_http_failure() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/launcher.json", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                for status in ["503 Service Unavailable", "200 OK"] {
                    let (mut socket, _) = listener.accept().unwrap();
                    let mut request = [0; 4096];
                    socket.read(&mut request).unwrap();
                    let body = serde_json::json!({"version": "0.4.0", "platforms": {"linux-x86_64": {"version": "0.4.0", "file": "launcher", "url": "https://example.com/launcher", "sha256": "a".repeat(64), "size": 10}}}).to_string();
                    write!(socket, "HTTP/1.1 {status}\r\nRetry-After: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
            });
            assert!(!check_for_update(Path::new("unused"), "0.4.0", "linux-x86_64", &url, &[]).await.unwrap());
            server.join().unwrap();
        });
    }

    #[test]
    fn manifest_uses_platform_version_not_global_version() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let body = serde_json::json!({"version": "99.0.0", "platforms": {"linux-x86_64": {"version": "0.3.0", "file": "launcher", "url": "https://example.com/launcher", "sha256": "a".repeat(64), "size": 10}}});
            let (url, server) = serve(serde_json::to_vec(&body).unwrap());
            assert!(!check_for_update(Path::new("unused"), "0.4.0", "linux-x86_64", &url, &[]).await.unwrap());
            server.join().unwrap();
        });
    }

    #[cfg(unix)]
    #[test]
    fn linux_install_and_rollback_preserve_backup() {
        let dir = directory();
        let executable = dir.join("launcher");
        let staged = dir.join("launcher.new");
        std::fs::write(&executable, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();
        install_staged(&executable, &staged, false).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("launcher.old")).unwrap(), b"old");
        rollback(&executable, false).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"old");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_executable_never_replaces_current() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let dir = directory();
            let executable = dir.join("launcher");
            let staged = dir.join("launcher.new");
            std::fs::write(&executable, b"old").unwrap();
            std::fs::write(&staged, b"not a binary").unwrap();
            crate::ensure_executable(&staged).unwrap();
            assert!(
                replace_and_restart(&executable, &staged, "0.4.0", &[])
                    .await
                    .is_err()
            );
            assert_eq!(std::fs::read(&executable).unwrap(), b"old");
            assert!(!dir.join("launcher.old").exists());
            std::fs::remove_dir_all(dir).unwrap();
        });
    }

    #[test]
    fn versions_compare_numerically_and_refuse_downgrades() {
        assert!(newer_version("0.10.0", "0.3.0").unwrap());
        assert!(!newer_version("0.3.0", "0.3.0").unwrap());
        assert!(!newer_version("0.2.9", "0.3.0").unwrap());
        assert!(newer_version("1.0.0", "0.99.9").unwrap());
        assert!(newer_version("broken", "0.3.0").is_err());
        assert!(!newer_version("1.0.0+z", "1.0.0+a").unwrap());
        assert!(!newer_version("1.0.0-rc.1", "1.0.0").unwrap());
        assert!(newer_version("1.0.0", "1.0.0-rc.1").unwrap());
    }
    #[test]
    fn transfer_verifies_hash_and_size_before_touching_current() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let dir = directory();
            let executable = dir.join("launcher");
            std::fs::write(&executable, b"current").unwrap();
            for (body, size, hash, succeeds) in [
                (
                    b"new".to_vec(),
                    3,
                    hex::encode(Sha256::digest(b"new")),
                    true,
                ),
                (
                    b"bad".to_vec(),
                    3,
                    hex::encode(Sha256::digest(b"new")),
                    false,
                ),
                (
                    b"new".to_vec(),
                    4,
                    hex::encode(Sha256::digest(b"new")),
                    false,
                ),
                (
                    b"new".to_vec(),
                    2,
                    hex::encode(Sha256::digest(b"new")),
                    false,
                ),
            ] {
                let (url, server) = serve(body);
                let release = LauncherRelease {
                    version: "0.4.0".into(),
                    url,
                    sha256: hash,
                    size,
                    file: "launcher".into(),
                };
                let result =
                    stage_release(&crate::http_client().unwrap(), &release, &executable).await;
                assert_eq!(result.is_ok(), succeeds);
                if succeeds {
                    assert_eq!(std::fs::read(result.unwrap()).unwrap(), b"new");
                }
                assert_eq!(std::fs::read(&executable).unwrap(), b"current");
                server.join().unwrap();
            }
            std::fs::remove_dir_all(dir).unwrap();
        });
    }
    #[test]
    fn windows_rename_aside_and_rollback_preserve_current() {
        let dir = directory();
        let executable = dir.join("launcher.exe");
        let staged = dir.join("new.exe");
        std::fs::write(&executable, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();
        install_staged(&executable, &staged, true).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("launcher.exe.old")).unwrap(), b"old");
        rollback(&executable, true).unwrap();
        assert_eq!(std::fs::read(&executable).unwrap(), b"old");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_install_restores_windows_current() {
        let dir = directory();
        let executable = dir.join("launcher.exe");
        std::fs::write(&executable, b"old").unwrap();
        assert!(install_staged(&executable, &dir.join("absent"), true).is_err());
        assert_eq!(std::fs::read(&executable).unwrap(), b"old");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
