#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::collections::HashSet;
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

/// Local copy of manifest-v2.json and its ETag, so an unchanged manifest costs one 304.
const MANIFEST_V2: &str = ".manifest-v2.json";
const MANIFEST_V2_ETAG: &str = ".manifest-v2.etag";
/// ETag of the manifest whose files were last installed and pruned completely.
const INSTALLED_MANIFEST: &str = ".installed-manifest";

/// Fetches manifest-v2 conditionally: its ~600 MB body is downloaded only when the ETag
/// changed. `None` when the server publishes no v2 manifest.
async fn fetch_v2_manifest(
    client: &reqwest::Client,
    game_dir: &Path,
) -> Result<Option<(game_launcher::packs::PackManifest, Option<String>)>, String> {
    let body_path = game_dir.join(MANIFEST_V2);
    let etag_path = game_dir.join(MANIFEST_V2_ETAG);
    let cached_etag = std::fs::read_to_string(&etag_path)
        .ok()
        .filter(|_| body_path.is_file());
    let mut request = client.get(file_url("manifest-v2.json"));
    if let Some(etag) = &cached_etag {
        request = request.header("If-None-Match", etag);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Fetch v2 manifest: {error}"))?;
    let parse = |body: &[u8]| {
        serde_json::from_slice::<game_launcher::packs::PackManifest>(body)
            .map_err(|error| format!("Parse v2 manifest: {error}"))
    };
    match response.status() {
        reqwest::StatusCode::NOT_FOUND => return Ok(None),
        reqwest::StatusCode::NOT_MODIFIED => {
            let body = std::fs::read(&body_path)
                .map_err(|error| format!("Read {}: {error}", body_path.display()))?;
            return Ok(Some((parse(&body)?, cached_etag)));
        }
        status if !status.is_success() => return Err(format!("Fetch v2 manifest: {status}")),
        _ => {}
    }
    let etag = extract_response_header(&response, "etag");
    let body = response
        .bytes()
        .await
        .map_err(|error| format!("Fetch v2 manifest: {error}"))?;
    let packs = parse(&body)?;
    std::fs::create_dir_all(game_dir)
        .map_err(|error| format!("Create {}: {error}", game_dir.display()))?;
    // The ETag goes first and last, so a body and an ETag on disk never disagree.
    remove_if_present(&etag_path)?;
    std::fs::write(&body_path, &body)
        .map_err(|error| format!("Save {}: {error}", body_path.display()))?;
    if let Some(etag) = &etag {
        std::fs::write(&etag_path, etag)
            .map_err(|error| format!("Save {}: {error}", etag_path.display()))?;
    }
    Ok(Some((packs, etag)))
}

fn remove_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("Remove {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Resolve the manifest and the ETag identifying it (v2 only).
async fn resolve_manifest(
    client: &reqwest::Client,
    game_dir: &Path,
) -> Result<(Manifest, Option<String>), String> {
    if let Some((packs, etag)) = fetch_v2_manifest(client, game_dir).await? {
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
        let manifest = Manifest {
            version: "2".into(),
            files,
            packs: Some(packs),
        };
        return Ok((manifest, etag));
    }
    let cached = load_manifest_cache();
    let manifest = match fetch_manifest(client, cached.as_ref()).await? {
        Some(m) => m,
        None => cached
            .map(|c| c.manifest)
            .ok_or_else(|| "No manifest available".to_string())?,
    };
    Ok((manifest, None))
}

/// Brings `game_dir` to the published manifest. A manifest with the same ETag as the last
/// complete install returns at once, without loading receipts or touching installed files.
/// `progress(entry, done, total)` reports each placed file, starting with `done == 0`.
async fn sync_game(
    client: &reqwest::Client,
    game_dir: &Path,
    mut progress: impl FnMut(&FileEntry, usize, usize),
) -> Result<(), String> {
    let (manifest, etag) = resolve_manifest(client, game_dir).await?;
    let installed = game_dir.join(INSTALLED_MANIFEST);
    if etag.is_some() && std::fs::read_to_string(&installed).ok() == etag {
        return Ok(());
    }
    let files = filter_platform_files(&manifest.files);
    let needed = files_needing_update(game_dir, &files).await?;
    if let Some(first) = needed.first() {
        progress(first, 0, needed.len());
        let cold = files
            .iter()
            .all(|entry| !game_dir.join(&entry.path).exists());
        sync_downloads(client, game_dir, &needed, &manifest, cold, |entry, done| {
            progress(entry, done, needed.len())
        })
        .await?;
    }
    prune_unlisted(game_dir, &files)?;
    if let Some(etag) = etag {
        std::fs::write(&installed, etag)
            .map_err(|error| format!("Save {}: {error}", installed.display()))?;
    }
    Ok(())
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
    let listed = listed_paths(files);
    let reused =
        game_launcher::reuse::reuse_local_content(game_dir, &mut cache, needed, &listed).await;
    // Receipts of moved or copied files must persist even when a later placement fails.
    cache.save(game_dir)?;
    reused
}

fn listed_paths(files: &[FileEntry]) -> HashSet<&str> {
    files.iter().map(|entry| entry.path.as_str()).collect()
}

/// After a complete sync, files from earlier builds that the manifest no longer lists go.
fn prune_unlisted(game_dir: &Path, files: &[FileEntry]) -> Result<usize, String> {
    let mut cache = game_launcher::cache::FileHashCache::load(game_dir)?;
    let removed = game_launcher::reuse::prune_unlisted(game_dir, &mut cache, &listed_paths(files));
    cache.save(game_dir)?;
    removed
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
    mut state: Signal<LauncherState>,
    mut progress: Signal<f64>,
) -> Result<(), String> {
    sync_game(&build_client(), game_dir, |entry, done, total| {
        state.set(LauncherState::Downloading {
            current: entry.path.clone(),
            done,
            total,
        });
        progress.set(done as f64 / total as f64 * 100.0);
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
        cli_sync_game(&game_directory()).await;
        println!("Game is up to date");
    });
}

fn run_play() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let game_dir = game_directory();
        cli_sync_game(&game_dir).await;
        println!("Launching...");
        let arguments = std::env::args_os().skip(2).collect::<Vec<_>>();
        if let Err(error) = launch_game(&game_dir, &arguments) {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
    });
}

async fn cli_sync_game(game_dir: &Path) {
    if let Err(error) = sync_game(&build_client(), game_dir, |entry, done, total| {
        println!("  [{done}/{total}] {}", entry.path);
    })
    .await
    {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}
