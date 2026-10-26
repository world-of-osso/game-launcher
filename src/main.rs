#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

#[cfg(feature = "desktop")]
use dioxus::prelude::*;
use game_launcher::{FileEntry, self_update};
use serde::{Deserialize, Serialize};

#[cfg(feature = "desktop")]
static SCREENSHOT_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
#[cfg(feature = "desktop")]
static HERO_BG_DATA_URI: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    use base64::Engine;
    let bytes = include_bytes!("../assets/hero-bg.png");
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:image/png;base64,{b64}")
});

/// Public R2 bucket (worldofosso-client) behind its custom domain: the manifest at
/// `/manifest.json`, every manifest path at `/<path>`.
const BASE_URL: &str = "https://files.worldofosso.com";
const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Manifest {
    version: String,
    files: Vec<FileEntry>,
    #[serde(skip)]
    packs: Option<game_launcher::packs::PackManifest>,
}

#[cfg(feature = "desktop")]
#[derive(Clone, Debug, PartialEq)]
enum LauncherState {
    Checking,
    Ready,
    Downloading {
        current: String,
        done: usize,
        total: usize,
    },
    Error(String),
    Launching,
}

fn main() {
    let command = std::env::args().nth(1);
    if command.as_deref() == Some("--self-check") {
        println!("{LAUNCHER_VERSION}");
        return;
    }
    if matches!(
        command.as_deref(),
        Some("check-manifest" | "self-update" | "update" | "play")
    ) {
        if let Err(error) = self_update::acknowledge_start(LAUNCHER_VERSION) {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
        let runtime = tokio::runtime::Runtime::new().expect("Create updater runtime");
        if let Err(error) = runtime.block_on(update_launcher()) {
            eprintln!("Launcher update error (retry with self-update): {error}");
            if command.as_deref() == Some("self-update") {
                std::process::exit(1);
            }
        }
    }
    match command.as_deref() {
        Some("check-manifest") => return run_check_manifest(),
        Some("self-update") => return,
        Some("update") => return run_update(),
        Some("play") => return run_play(),
        #[cfg(feature = "desktop")]
        Some("screenshot") => {
            let output = std::env::args()
                .nth(2)
                .unwrap_or_else(|| "screenshot.webp".to_string());
            SCREENSHOT_PATH.set(output).unwrap();
        }
        _ => {}
    }

    #[cfg(all(windows, feature = "desktop"))]
    if let Err(error) = game_launcher::windows_shortcut::install_portable_start_menu_entry() {
        eprintln!("ERROR: {error}");
    }

    #[cfg(not(feature = "desktop"))]
    {
        eprintln!("Desktop UI requires the desktop feature; use update, play or self-update.");
        std::process::exit(1);
    }

    #[cfg(feature = "desktop")]
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new().with_window(
                dioxus::desktop::tao::window::WindowBuilder::new()
                    .with_title("World of Osso")
                    .with_decorations(false)
                    .with_inner_size(dioxus::desktop::tao::dpi::LogicalSize::new(613, 732)),
            ),
        )
        .launch(App);
}

fn launcher_manifest_url() -> String {
    std::env::var("GAME_LAUNCHER_MANIFEST_URL")
        .unwrap_or_else(|_| self_update::MANIFEST_URL.to_owned())
}

async fn update_launcher() -> Result<(), String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("Find current launcher: {error}"))?;
    self_update::cleanup_previous(&executable, LAUNCHER_VERSION)?;
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if self_update::check_for_update(
        &executable,
        LAUNCHER_VERSION,
        platform_key(),
        &launcher_manifest_url(),
        &arguments,
    )
    .await?
    {
        std::process::exit(0);
    }
    Ok(())
}

// --- UI Components ---

#[cfg(feature = "desktop")]
#[component]
fn App() -> Element {
    #[cfg(feature = "debug")]
    {
        if let Some(path) = SCREENSHOT_PATH.get() {
            dioxus_debug::use_screenshot(path);
        }
        #[cfg(debug_assertions)]
        dioxus_debug::use_debug_server();
    }

    let mut state = use_signal(|| LauncherState::Checking);
    let mut launcher_error = use_signal(|| None::<String>);
    let progress_pct = use_signal(|| 0.0f64);
    let game_dir = game_directory();

    let recheck = use_coroutine(move |mut requests: UnboundedReceiver<()>| {
        let game_dir = game_dir.clone();
        async move {
            if let Err(error) = self_update::acknowledge_start(LAUNCHER_VERSION) {
                state.set(LauncherState::Error(error));
                return;
            }
            loop {
                state.set(LauncherState::Checking);
                launcher_error.set(update_launcher().await.err());
                match check_and_sync(&game_dir, state, progress_pct).await {
                    Ok(()) => state.set(LauncherState::Ready),
                    Err(error) => state.set(LauncherState::Error(error)),
                }
                if !self_update::wait_for_recheck(&mut requests, self_update::RECHECK_INTERVAL)
                    .await
                {
                    break;
                }
            }
        }
    });

    rsx! {
        style { dangerous_inner_html: include_str!("../assets/style.css") }
        div { class: "launcher",
            TopBar {}
            HeroSection {}
            BottomBar { state, progress_pct, launcher_error, recheck }
        }
    }
}

#[cfg(feature = "desktop")]
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

#[cfg(feature = "desktop")]
#[component]
fn HeroSection() -> Element {
    let hero_bg = &*HERO_BG_DATA_URI;
    rsx! {
        div { class: "hero",
            div { class: "hero-bg", style: "background-image: url('{hero_bg}')" }
            div { class: "hero-content",
                div { class: "game-title", "World of Osso" }
                div { class: "game-subtitle", "A new adventure awaits" }
            }
            div { class: "news-area",
                NewsCard { tag: "Update", title: "First playable build available", date: "Mar 7, 2026", url: "https://worldofosso.com/news/first-playable-build.html" }
                NewsCard { tag: "Dev Log", title: "M2 models and terrain rendering", date: "Mar 5, 2026", url: "https://worldofosso.com/news/m2-models-terrain-rendering.html" }
            }
        }
    }
}

#[cfg(feature = "desktop")]
#[component]
fn NewsCard(
    tag: &'static str,
    title: &'static str,
    date: &'static str,
    url: &'static str,
) -> Element {
    rsx! {
        div { class: "news-card", onclick: move |_| { let _ = open::that(url); },
            div { class: "card-tag", "{tag}" }
            div { class: "card-title", "{title}" }
            div { class: "card-date", "{date}" }
        }
    }
}

#[cfg(feature = "desktop")]
#[component]
fn BottomBar(
    state: Signal<LauncherState>,
    progress_pct: Signal<f64>,
    launcher_error: Signal<Option<String>>,
    recheck: Coroutine<()>,
) -> Element {
    let current_state = state.read().clone();
    let pct = *progress_pct.read();

    rsx! {
        div { class: "bottom-bar",
            if let Some(message) = launcher_error.read().as_ref() {
                div { class: "error-row", role: "alert",
                    span { class: "error-text", "Launcher update: {message}" }
                    button { class: "retry-button", onclick: move |_| recheck.send(()), "Retry launcher update" }
                }
            }
            match current_state {
                LauncherState::Checking => rsx! {
                    span { class: "status-text checking", "Checking for updates" }
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
                            onclick: move |_| recheck.send(()),
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

#[cfg(feature = "desktop")]
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

#[cfg(feature = "desktop")]
#[component]
fn PlayButton(state: Signal<LauncherState>) -> Element {
    rsx! {
        button {
            class: "play-button",
            onclick: move |_| {
                state.set(LauncherState::Launching);
                if let Err(error) = launch_game(&game_directory(), &[]) {
                    state.set(LauncherState::Error(error));
                }
            },
            "Play"
        }
    }
}

// --- Platform ---

fn game_directory() -> PathBuf {
    if let Some(directory) = std::env::var_os("GAME_LAUNCHER_GAME_DIR") {
        return PathBuf::from(directory);
    }
    dirs_next::data_dir()
        .expect("Find user data directory")
        .join("WorldOfOsso")
}

fn platform_key() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x86_64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-aarch64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x86_64"
    }
}

fn game_engine_binary() -> String {
    format!("game-engine-{}", platform_key())
}

/// Filter manifest to only include files for the current platform.
/// Skips `game-engine-*` entries for other platforms.
fn filter_platform_files(files: &[FileEntry]) -> Vec<FileEntry> {
    let my_binary = game_engine_binary();
    files
        .iter()
        .filter(|e| !e.path.starts_with("game-engine-") || e.path == my_binary)
        .cloned()
        .collect()
}

fn launch_game(game_dir: &Path, arguments: &[std::ffi::OsString]) -> Result<(), String> {
    game_launcher::spawn_player(game_dir, &game_engine_binary(), arguments).map(|_| ())
}

fn base_url() -> String {
    std::env::var("GAME_LAUNCHER_BASE_URL").unwrap_or_else(|_| BASE_URL.to_owned())
}

fn file_url(path: &str) -> String {
    format!("{}/{path}", base_url())
}

fn build_client() -> reqwest::Client {
    game_launcher::http_client().expect("Create launcher HTTP client")
}

// --- Manifest ---

#[derive(Serialize, Deserialize, Clone, Debug)]
struct CachedManifest {
    manifest: Manifest,
    etag: Option<String>,
    last_modified: Option<String>,
}

fn manifest_cache_path() -> PathBuf {
    game_directory().join(".manifest_cache.json")
}

fn load_manifest_cache() -> Option<CachedManifest> {
    let data = std::fs::read_to_string(manifest_cache_path()).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_manifest_cache(manifest: &Manifest, etag: Option<String>, last_modified: Option<String>) {
    let cached = CachedManifest {
        manifest: manifest.clone(),
        etag,
        last_modified,
    };
    if let Ok(json) = serde_json::to_string(&cached) {
        let _ = std::fs::write(manifest_cache_path(), json);
    }
}

fn add_cache_headers(
    req: reqwest::RequestBuilder,
    cached: &CachedManifest,
) -> reqwest::RequestBuilder {
    let mut req = req;
    if let Some(ref etag) = cached.etag {
        req = req.header("If-None-Match", etag);
    }
    if let Some(ref lm) = cached.last_modified {
        req = req.header("If-Modified-Since", lm);
    }
    req
}

async fn parse_manifest_response(resp: reqwest::Response) -> Result<Manifest, String> {
    let etag = extract_response_header(&resp, "etag");
    let last_modified = extract_response_header(&resp, "last-modified");
    let manifest: Manifest = resp.json().await.map_err(|e| format!("Parse error: {e}"))?;
    save_manifest_cache(&manifest, etag, last_modified);
    Ok(manifest)
}

fn extract_response_header(resp: &reqwest::Response, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

async fn fetch_manifest(
    client: &reqwest::Client,
    cached: Option<&CachedManifest>,
) -> Result<Option<Manifest>, String> {
    let mut req = client.get(file_url("manifest.json"));

    if let Some(c) = cached {
        req = add_cache_headers(req, c);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("Server error: {}", resp.status()));
    }

    parse_manifest_response(resp).await.map(Some)
}

/// Resolve manifest, using cache on 304.
async fn resolve_manifest(client: &reqwest::Client) -> Result<Manifest, String> {
    let response = client
        .get(file_url("manifest-v2.json"))
        .send()
        .await
        .map_err(|error| format!("Fetch v2 manifest: {error}"))?;
    if response.status() != reqwest::StatusCode::NOT_FOUND {
        if !response.status().is_success() {
            return Err(format!("Fetch v2 manifest: {}", response.status()));
        }
        let packs: game_launcher::packs::PackManifest = response
            .json()
            .await
            .map_err(|error| format!("Parse v2 manifest: {error}"))?;
        packs.validate()?;
        let files = packs
            .files
            .iter()
            .map(|file| FileEntry {
                path: file.path.clone(),
                size: file.size,
                sha256: file.sha256.clone(),
            })
            .collect();
        return Ok(Manifest {
            version: "2".into(),
            files,
            packs: Some(packs),
        });
    }
    let cached = load_manifest_cache();
    match fetch_manifest(client, cached.as_ref()).await? {
        Some(m) => Ok(m),
        None => cached
            .map(|c| c.manifest)
            .ok_or_else(|| "No manifest available".to_string()),
    }
}

// --- File sync ---

async fn files_needing_update(
    game_dir: &Path,
    files: &[FileEntry],
) -> Result<Vec<FileEntry>, String> {
    let mut cache = game_launcher::cache::FileHashCache::load(game_dir)?;
    let mut needed = Vec::new();
    for entry in files {
        if cache.needs_update(game_dir, entry).await? {
            needed.push(entry.clone());
        }
    }
    cache.save(game_dir)?;
    Ok(needed)
}

async fn sync_downloads(
    client: &reqwest::Client,
    game_dir: &Path,
    needed: &[FileEntry],
    manifest: &Manifest,
    cold: bool,
    mut progress: impl FnMut(&FileEntry, usize),
) -> Result<(), String> {
    game_launcher::packs::reject_running_game(game_dir)?;
    let mut cache = game_launcher::cache::FileHashCache::load(game_dir)?;
    let mut completed = |entry: &FileEntry, count| {
        cache.record_verified(game_dir, entry)?;
        progress(entry, count);
        Ok(())
    };
    let result = if let Some(packs) = &manifest.packs {
        let paths = needed
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        game_launcher::packs::sync_packs(
            client,
            &base_url(),
            game_dir,
            packs,
            &paths,
            cold,
            &mut completed,
        )
        .await
    } else {
        game_launcher::download_files(client, &base_url(), game_dir, needed, &mut completed).await
    };
    match (result, cache.save(game_dir)) {
        (Err(transfer), Err(save)) => Err(format!("{transfer}; {save}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

// --- Sync orchestration ---

#[cfg(feature = "desktop")]
async fn check_and_sync(
    game_dir: &Path,
    state: Signal<LauncherState>,
    progress: Signal<f64>,
) -> Result<(), String> {
    let client = build_client();
    let manifest = resolve_manifest(&client).await?;

    let files = filter_platform_files(&manifest.files);
    let needed = files_needing_update(game_dir, &files).await?;
    if needed.is_empty() {
        return Ok(());
    }

    let cold = files
        .iter()
        .all(|entry| !game_dir.join(&entry.path).exists());
    sync_files(&client, game_dir, &needed, &manifest, cold, state, progress).await
}

#[cfg(feature = "desktop")]
async fn sync_files(
    client: &reqwest::Client,
    game_dir: &Path,
    needed: &[FileEntry],
    manifest: &Manifest,
    cold: bool,
    mut state: Signal<LauncherState>,
    mut progress: Signal<f64>,
) -> Result<(), String> {
    state.set(LauncherState::Downloading {
        current: needed[0].path.clone(),
        done: 0,
        total: needed.len(),
    });
    sync_downloads(client, game_dir, needed, manifest, cold, |entry, done| {
        state.set(LauncherState::Downloading {
            current: entry.path.clone(),
            done,
            total: needed.len(),
        });
        progress.set(done as f64 / needed.len() as f64 * 100.0);
    })
    .await
}

// --- CLI subcommands ---

fn run_check_manifest() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = build_client();
        let cached = load_manifest_cache();

        if let Some(ref c) = cached {
            eprintln!(
                "Cache: etag={:?} last_modified={:?}",
                c.etag, c.last_modified
            );
            eprintln!("Cached manifest version: {}", c.manifest.version);
            eprintln!("Cached files: {}", c.manifest.files.len());
        } else {
            eprintln!("Cache: none");
        }

        match fetch_manifest(&client, cached.as_ref()).await {
            Ok(Some(m)) => println!(
                "DOWNLOADED — version: {}, files: {}",
                m.version,
                m.files.len()
            ),
            Ok(None) => println!("NOT MODIFIED — using cached manifest"),
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        }
    });
}

fn run_update() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = build_client();
        let game_dir = game_directory();
        let manifest = match resolve_manifest(&client).await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        };

        let files = filter_platform_files(&manifest.files);
        let needed = files_needing_update(&game_dir, &files)
            .await
            .unwrap_or_else(|error| {
                eprintln!("ERROR: {error}");
                std::process::exit(1);
            });
        if needed.is_empty() {
            println!("All {} files up to date", files.len());
            return;
        }

        println!("Downloading {}/{} files...", needed.len(), files.len());
        let cold = files
            .iter()
            .all(|entry| !game_dir.join(&entry.path).exists());
        cli_download_files(&client, &game_dir, &needed, &manifest, cold).await;
        println!("Done");
    });
}

fn run_play() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let client = build_client();
        let game_dir = game_directory();
        let manifest = match resolve_manifest(&client).await {
            Ok(m) => m,
            Err(e) => {
                eprintln!("ERROR: {e}");
                std::process::exit(1);
            }
        };

        let files = filter_platform_files(&manifest.files);
        let needed = files_needing_update(&game_dir, &files)
            .await
            .unwrap_or_else(|error| {
                eprintln!("ERROR: {error}");
                std::process::exit(1);
            });
        if !needed.is_empty() {
            println!("Updating {}/{} files...", needed.len(), files.len());
            let cold = files
                .iter()
                .all(|entry| !game_dir.join(&entry.path).exists());
            cli_download_files(&client, &game_dir, &needed, &manifest, cold).await;
        }

        println!("Launching...");
        let arguments = std::env::args_os().skip(2).collect::<Vec<_>>();
        if let Err(error) = launch_game(&game_dir, &arguments) {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
    });
}

async fn cli_download_files(
    client: &reqwest::Client,
    game_dir: &Path,
    needed: &[FileEntry],
    manifest: &Manifest,
    cold: bool,
) {
    if let Err(error) = sync_downloads(client, game_dir, needed, manifest, cold, |entry, done| {
        println!("  [{}/{}] {}", done, needed.len(), entry.path);
    })
    .await
    {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}
