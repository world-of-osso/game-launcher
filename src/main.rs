use std::collections::HashMap;
use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const BASE_URL: &str = "https://worldofosso.com";
const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");
const HMAC_SECRET: &str = "8c526f3ec373cd70aeda607a6370a1548fe83184c2c93c16b9aa289927c07dda";

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Manifest {
    version: String,
    files: Vec<FileEntry>,
    launcher: Option<LauncherUpdate>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct LauncherUpdate {
    version: String,
    sha256: String,
    platform: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct FileEntry {
    path: String,
    sha256: String,
    size: u64,
}

#[derive(Clone, Debug, PartialEq)]
enum LauncherState {
    Checking,
    UpdatingSelf,
    Ready,
    Downloading { current: String, done: usize, total: usize },
    Error(String),
    Launching,
}

fn main() {
    cleanup_old_binary();
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::tao::window::WindowBuilder::new()
                    .with_title("World of Osso")
                    .with_decorations(false),
            ),
        )
        .launch(App);
}

#[component]
fn App() -> Element {
    let mut state = use_signal(|| LauncherState::Checking);
    let progress_pct = use_signal(|| 0.0f64);
    let game_dir = game_directory();

    use_future(move || {
        let game_dir = game_dir.clone();
        async move {
            match check_and_sync(&game_dir, state, progress_pct).await {
                Ok(()) => state.set(LauncherState::Ready),
                Err(e) => state.set(LauncherState::Error(e.to_string())),
            }
        }
    });

    rsx! {
        document::Stylesheet { href: asset!("/assets/style.css") }
        div { class: "launcher",
            TopBar {}
            HeroSection {}
            BottomBar { state, progress_pct }
        }
    }
}

#[component]
fn TopBar() -> Element {
    rsx! {
        div { class: "top-bar",
            span { class: "logo", "World of Osso" }
            span { class: "separator" }
            span { class: "game-tab", "Game" }
            span { class: "version", "v{LAUNCHER_VERSION}" }
        }
    }
}

#[component]
fn HeroSection() -> Element {
    rsx! {
        div { class: "hero",
            div { class: "hero-bg" }
            div { class: "hero-content",
                div { class: "game-title", "World of Osso" }
                div { class: "game-subtitle", "A new adventure awaits" }
            }
            div { class: "news-area",
                NewsCard { tag: "Update", title: "First playable build available", date: "Mar 7, 2026" }
                NewsCard { tag: "Dev Log", title: "M2 models and terrain rendering", date: "Mar 5, 2026" }
            }
        }
    }
}

#[component]
fn NewsCard(tag: &'static str, title: &'static str, date: &'static str) -> Element {
    rsx! {
        div { class: "news-card",
            div { class: "card-tag", "{tag}" }
            div { class: "card-title", "{title}" }
            div { class: "card-date", "{date}" }
        }
    }
}

#[component]
fn BottomBar(state: Signal<LauncherState>, progress_pct: Signal<f64>) -> Element {
    let current_state = state.read().clone();
    let pct = *progress_pct.read();

    rsx! {
        div { class: "bottom-bar",
            match current_state {
                LauncherState::Checking => rsx! {
                    span { class: "status-text checking", "Checking for updates" }
                },
                LauncherState::UpdatingSelf => rsx! {
                    span { class: "status-text checking", "Updating launcher..." }
                },
                LauncherState::Downloading { ref current, done, total } => rsx! {
                    DownloadRow { current: current.clone(), done, total, pct }
                },
                LauncherState::Ready => rsx! {
                    PlayButton { state }
                },
                LauncherState::Error(ref msg) => rsx! {
                    div { class: "error-row",
                        span { class: "error-text", "{msg}" }
                        button {
                            class: "retry-button",
                            onclick: move |_| state.set(LauncherState::Checking),
                            "Retry"
                        }
                    }
                },
                LauncherState::Launching => rsx! {
                    span { class: "launching-text", "Launching..." }
                },
            }
        }
    }
}

#[component]
fn DownloadRow(current: String, done: usize, total: usize, pct: f64) -> Element {
    rsx! {
        div { class: "download-info",
            span { class: "download-label",
                strong { "Updating " }
                "{done}/{total} — {current}"
            }
            div { class: "progress-bar",
                div { class: "progress-fill", style: "width: {pct:.1}%" }
            }
        }
        span { class: "download-pct", "{pct:.0}%" }
    }
}

#[component]
fn PlayButton(state: Signal<LauncherState>) -> Element {
    rsx! {
        button {
            class: "play-button",
            onclick: move |_| {
                state.set(LauncherState::Launching);
                launch_game(&game_directory());
            },
            "Play"
        }
    }
}

fn game_directory() -> PathBuf {
    let base = dirs_next::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("WorldOfOsso")
}

fn sign_request(path: &str) -> (String, String) {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string();

    let message = format!("{timestamp}:{path}");
    let mut mac = HmacSha256::new_from_slice(HMAC_SECRET.as_bytes()).unwrap();
    mac.update(message.as_bytes());
    let sig = hex::encode(mac.finalize().into_bytes());
    (timestamp, sig)
}

fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("WorldOfOsso-Launcher/0.1")
        .build()
        .unwrap()
}

async fn fetch_manifest(client: &reqwest::Client) -> Result<Manifest, String> {
    let path = "/api/manifest";
    let (ts, sig) = sign_request(path);
    let url = format!("{BASE_URL}{path}");

    let resp = client
        .get(&url)
        .header("x-launcher-ts", &ts)
        .header("x-launcher-sig", &sig)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Server error: {}", resp.status()));
    }

    resp.json::<Manifest>()
        .await
        .map_err(|e| format!("Parse error: {e}"))
}

async fn check_and_sync(
    game_dir: &Path,
    mut state: Signal<LauncherState>,
    mut progress: Signal<f64>,
) -> Result<(), String> {
    let client = build_client();
    let manifest = fetch_manifest(&client).await?;

    if let Some(ref launcher) = manifest.launcher {
        if launcher_needs_update(&launcher.version) {
            state.set(LauncherState::UpdatingSelf);
            self_update(&client, launcher).await?;
        }
    }

    let needed = files_needing_update(game_dir, &manifest.files).await;
    if needed.is_empty() {
        return Ok(());
    }

    let total = needed.len();
    for (i, entry) in needed.iter().enumerate() {
        state.set(LauncherState::Downloading {
            current: entry.path.clone(),
            done: i,
            total,
        });
        progress.set((i as f64 / total as f64) * 100.0);
        download_file(&client, game_dir, entry).await?;
    }
    progress.set(100.0);
    Ok(())
}

async fn files_needing_update(game_dir: &Path, files: &[FileEntry]) -> Vec<FileEntry> {
    let mut needed = Vec::new();
    for entry in files {
        if file_needs_update(game_dir, entry).await {
            needed.push(entry.clone());
        }
    }
    needed
}

async fn file_needs_update(game_dir: &Path, entry: &FileEntry) -> bool {
    let local_path = game_dir.join(&entry.path);
    match tokio::fs::read(&local_path).await {
        Ok(data) => {
            use sha2::Digest;
            hex::encode(Sha256::digest(&data)) != entry.sha256
        }
        Err(_) => true,
    }
}

async fn download_file(
    client: &reqwest::Client,
    game_dir: &Path,
    entry: &FileEntry,
) -> Result<(), String> {
    let path = format!("/files/{}", entry.path);
    let (ts, sig) = sign_request(&path);
    let url = format!("{BASE_URL}{path}");

    let bytes = client
        .get(&url)
        .header("x-launcher-ts", &ts)
        .header("x-launcher-sig", &sig)
        .send()
        .await
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

    tokio::fs::write(&local_path, &bytes)
        .await
        .map_err(|e| format!("Write error: {e}"))
}

fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let old = exe.with_extension("old");
        let _ = std::fs::remove_file(old);
    }
}

fn platform_key() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    { "linux-x86_64" }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    { "macos-x86_64" }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    { "macos-aarch64" }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    { "windows-x86_64" }
}

fn launcher_needs_update(remote_version: &str) -> bool {
    let current = env!("CARGO_PKG_VERSION");
    version_cmp(remote_version) > version_cmp(current)
}

fn version_cmp(v: &str) -> Vec<u32> {
    v.split('.').filter_map(|s| s.parse().ok()).collect()
}

async fn self_update(client: &reqwest::Client, update: &LauncherUpdate) -> Result<(), String> {
    let filename = update
        .platform
        .get(platform_key())
        .ok_or_else(|| "No launcher binary for this platform".to_string())?;

    let bytes = download_launcher_binary(client, filename).await?;
    verify_sha256(&bytes, &update.sha256)?;
    replace_current_binary(&bytes)?;
    restart_launcher()
}

async fn download_launcher_binary(
    client: &reqwest::Client,
    filename: &str,
) -> Result<Vec<u8>, String> {
    let path = format!("/files/{filename}");
    let (ts, sig) = sign_request(&path);
    let url = format!("{BASE_URL}{path}");

    client
        .get(&url)
        .header("x-launcher-ts", &ts)
        .header("x-launcher-sig", &sig)
        .send()
        .await
        .map_err(|e| format!("Download error: {e}"))?
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("Read error: {e}"))
}

fn verify_sha256(data: &[u8], expected: &str) -> Result<(), String> {
    use sha2::Digest;
    let actual = hex::encode(Sha256::digest(data));
    if actual != expected {
        return Err(format!("SHA256 mismatch: expected {expected}, got {actual}"));
    }
    Ok(())
}

fn replace_current_binary(new_bytes: &[u8]) -> Result<(), String> {
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Cannot find current exe: {e}"))?;
    let old_path = current_exe.with_extension("old");
    let new_path = current_exe.with_extension("new");

    std::fs::write(&new_path, new_bytes).map_err(|e| format!("Write new binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(&new_path, perms)
            .map_err(|e| format!("Set permissions: {e}"))?;
    }

    std::fs::rename(&current_exe, &old_path)
        .map_err(|e| format!("Rename current to .old: {e}"))?;
    std::fs::rename(&new_path, &current_exe)
        .map_err(|e| format!("Rename .new into place: {e}"))?;
    Ok(())
}

fn restart_launcher() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find exe: {e}"))?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::Command::new(exe)
        .args(&args)
        .spawn()
        .map_err(|e| format!("Restart failed: {e}"))?;
    std::process::exit(0);
}

fn launch_game(game_dir: &Path) {
    #[cfg(target_os = "windows")]
    let bin = game_dir.join("game-engine.exe");
    #[cfg(not(target_os = "windows"))]
    let bin = game_dir.join("game-engine");

    std::process::Command::new(&bin)
        .current_dir(game_dir)
        .spawn()
        .ok();
}
