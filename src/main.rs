use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const BASE_URL: &str = "https://worldofosso.com";
const HMAC_SECRET: &str = "8c526f3ec373cd70aeda607a6370a1548fe83184c2c93c16b9aa289927c07dda";

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Manifest {
    version: String,
    files: Vec<FileEntry>,
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
    Ready,
    Downloading { current: String, done: usize, total: usize },
    Error(String),
    Launching,
}

fn main() {
    dioxus::launch(App);
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
            Header {}
            Content { state, progress_pct }
        }
    }
}

#[component]
fn Header() -> Element {
    rsx! {
        div { class: "header",
            h1 { "World of Osso" }
        }
    }
}

#[component]
fn Content(state: Signal<LauncherState>, progress_pct: Signal<f64>) -> Element {
    let current_state = state.read().clone();
    let pct = *progress_pct.read();

    rsx! {
        div { class: "content",
            match current_state {
                LauncherState::Checking => rsx! {
                    p { class: "status", "Checking for updates..." }
                },
                LauncherState::Downloading { ref current, done, total } => rsx! {
                    DownloadProgress { current: current.clone(), done, total, pct }
                },
                LauncherState::Ready => rsx! {
                    PlayButton { state }
                },
                LauncherState::Error(ref msg) => rsx! {
                    p { class: "error", "{msg}" }
                    button {
                        class: "retry-button",
                        onclick: move |_| state.set(LauncherState::Checking),
                        "Retry"
                    }
                },
                LauncherState::Launching => rsx! {
                    p { class: "status", "Launching..." }
                },
            }
        }
    }
}

#[component]
fn DownloadProgress(current: String, done: usize, total: usize, pct: f64) -> Element {
    rsx! {
        p { class: "status", "Downloading {done}/{total}: {current}" }
        div { class: "progress-bar",
            div { class: "progress-fill", style: "width: {pct:.1}%" }
        }
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
            "PLAY"
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
