//! Open preview documents: subscriptions and visibility, dependency
//! watching, the per-document build worker, fallback decisions and the
//! published, revision-addressed outputs the webview loads.
//!
//! Nothing runs until a pane opens a preview: the first open creates the
//! document and its worker, the last close (or the pane's webview reloading
//! or closing) drops them. A document builds only while at least one
//! subscriber is visible. Changes made while it is hidden mark it pending;
//! it builds when it becomes visible again. A newer change cancels a running
//! build (its TeX process tree is killed) and starts over after a short
//! debounce.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

/// Receives every published snapshot: the app event in production, a
/// collector in tests.
pub type Sink = Arc<dyn Fn(&PreviewSnapshot) + Send + Sync>;

fn app_sink(app: &AppHandle) -> Sink {
    let app = app.clone();
    Arc::new(move |snapshot| {
        if let Err(error) = app.emit(UPDATED_EVENT, snapshot) {
            eprintln!("[Locus] failed to publish LaTeX preview update: {error}");
        }
    })
}

use super::build::{self, BuildInput, PdfResult, SvgResult};
use super::pipeline::{self, Diagnostic, Engine};
use super::toolchain;

pub const UPDATED_EVENT: &str = "latex-preview-updated";
const DEBOUNCE: Duration = Duration::from_millis(250);
/// Published revisions kept on disk; a page still loading an older one keeps
/// working while the next revision replaces it.
const KEEP_REVISIONS: usize = 3;
const MAX_WATCHED_DIRS: usize = 64;
const PDF_FILE: &str = "document.pdf";
const MANIFEST_FILE: &str = "manifest.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRef {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSnapshot {
    pub doc_id: String,
    pub checkout_id: String,
    /// File the preview was opened for.
    pub path: String,
    /// Root document compiled for it (differs with `% !TEX root`).
    pub root_path: String,
    /// `idle`, `pending` (changed while hidden), `building`, `ready` or
    /// `error`.
    pub status: String,
    /// `svg` or `pdf`: which output the preview shows.
    pub mode: Option<String>,
    /// Revision of the shown output.
    pub revision: u64,
    pub pages: Vec<PageRef>,
    /// Latest successful PDF (the cache), also when the SVG is shown.
    pub pdf_url: Option<String>,
    pub pdf_revision: u64,
    /// Why the PDF is shown instead of SVG (stable code).
    pub fallback_reason: Option<String>,
    pub fallback_detail: Option<String>,
    pub engine: Option<Engine>,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    pub svg_ms: Option<u64>,
    pub pdf_ms: Option<u64>,
    pub building: bool,
    /// Milliseconds since the Unix epoch of the last published change.
    pub updated_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    revision: u64,
    mode: Option<String>,
    pages: Vec<PageRef>,
    pdf_revision: u64,
    fallback_reason: Option<String>,
    fallback_detail: Option<String>,
    engine: Option<Engine>,
    updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenResult {
    pub subscription_id: String,
    pub snapshot: PreviewSnapshot,
}

/// Directory holding every document's work and published files.
pub fn preview_root() -> PathBuf {
    crate::runtime_paths::active_data_dir()
        .map(|dir| dir.join("latex-preview"))
        .unwrap_or_else(|_| std::env::temp_dir().join("locus-latex-preview"))
}

/// Path prefix of published files on the existing `locus-binary` protocol.
/// The preview registers no URI scheme of its own, so webviews that never
/// show a preview carry no extra protocol handler.
pub const PROTOCOL_PATH_PREFIX: &str = "/latex/";

/// URL prefix of published files in the webview.
pub fn url_base() -> String {
    let blob = locus_remote_protocol::BLOB_URL_PREFIX;
    format!("{}latex", blob.strip_suffix("blob/").unwrap_or(blob))
}

fn file_url(doc_id: &str, revision: u64, file: &str) -> String {
    format!("{}/{doc_id}/{revision}/{file}", url_base())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

pub fn doc_id_for(checkout_id: &str, path: &Path) -> String {
    let key = format!("{checkout_id}\u{0}{}", pipeline::normalize_key(path));
    blake3::hash(key.as_bytes()).to_hex()[..20].to_string()
}

struct Doc {
    id: String,
    checkout_id: String,
    owner_dir: String,
    path: PathBuf,
    out_dir: PathBuf,
    sink: Sink,
    change_seq: AtomicU64,
    wake: tokio::sync::Notify,
    state: Mutex<DocState>,
}

struct DocState {
    /// Subscription id -> visible.
    subscribers: HashMap<String, bool>,
    dirty: bool,
    closed: bool,
    /// A manual rebuild retries SVG even after a sticky fallback.
    force_svg: bool,
    next_revision: u64,
    /// Preamble hash whose SVG build fell back; the same preamble goes
    /// straight to PDF until it changes.
    sticky_pdf: Option<String>,
    deps: HashSet<String>,
    outputs: HashSet<String>,
    watcher: Option<RecommendedWatcher>,
    watched_dirs: Vec<PathBuf>,
    snapshot: PreviewSnapshot,
}

impl DocState {
    fn visible(&self) -> bool {
        self.subscribers.values().any(|visible| *visible)
    }
}

impl Doc {
    fn lock(&self) -> std::sync::MutexGuard<'_, DocState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn publish(&self, state: &mut DocState) {
        state.snapshot.updated_at = now_ms();
        (self.sink)(&state.snapshot);
    }

    fn pub_dir(&self, revision: u64) -> PathBuf {
        self.out_dir.join("pub").join(revision.to_string())
    }

    fn is_relevant_change(&self, path: &Path) -> bool {
        let key = pipeline::normalize_key(path);
        let state = self.lock();
        if state.outputs.contains(&key) {
            return false;
        }
        if state.deps.contains(&key) {
            return true;
        }
        !pipeline::is_build_artefact(path) && pipeline::is_relevant_source(path)
    }

    fn mark_changed(&self) {
        self.change_seq.fetch_add(1, Ordering::SeqCst);
        let mut state = self.lock();
        if state.closed {
            return;
        }
        state.dirty = true;
        if !state.visible() && !state.snapshot.building && state.snapshot.status != "pending" {
            state.snapshot.status = "pending".to_string();
            self.publish(&mut state);
        }
        drop(state);
        self.wake.notify_one();
    }
}

struct Subscription {
    doc_id: String,
    /// Webview that opened it: a reload or a closed window releases it even
    /// when the pane never got to close it.
    webview: String,
}

#[derive(Default)]
pub struct PreviewManager {
    docs: Mutex<HashMap<String, Arc<Doc>>>,
    subscriptions: Mutex<HashMap<String, Subscription>>,
}

static MANAGER: OnceLock<PreviewManager> = OnceLock::new();

/// The manager, created by the first preview that opens.
pub fn manager() -> &'static PreviewManager {
    MANAGER.get_or_init(PreviewManager::default)
}

/// Webview lifecycle hook (page reload, window destroyed). Until a preview
/// has ever opened this is one atomic load: nothing is created or locked.
pub fn release_webview(webview: &str) {
    if let Some(manager) = MANAGER.get() {
        manager.release_webview(webview);
    }
}

impl PreviewManager {
    fn doc_for_subscription(&self, subscription_id: &str) -> Result<Arc<Doc>, String> {
        let doc_id = self
            .subscriptions
            .lock()
            .map_err(|error| error.to_string())?
            .get(subscription_id)
            .map(|subscription| subscription.doc_id.clone())
            .ok_or_else(|| "unknown LaTeX preview subscription".to_string())?;
        self.docs
            .lock()
            .map_err(|error| error.to_string())?
            .get(&doc_id)
            .cloned()
            .ok_or_else(|| "the LaTeX preview was closed".to_string())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        app: &AppHandle,
        webview: &str,
        checkout_id: &str,
        owner_dir: &Path,
        path: &Path,
        visible: bool,
    ) -> Result<OpenResult, String> {
        self.open_with_sink(|| app_sink(app), webview, checkout_id, owner_dir, path, visible)
    }

    #[allow(clippy::too_many_arguments)]
    fn open_with_sink(
        &self,
        sink: impl FnOnce() -> Sink,
        webview: &str,
        checkout_id: &str,
        owner_dir: &Path,
        path: &Path,
        visible: bool,
    ) -> Result<OpenResult, String> {
        let doc_id = doc_id_for(checkout_id, path);
        let (doc, created) = {
            let mut docs = self.docs.lock().map_err(|error| error.to_string())?;
            if let Some(doc) = docs.get(&doc_id) {
                (Arc::clone(doc), false)
            } else {
                let doc = Arc::new(create_doc(sink(), &doc_id, checkout_id, owner_dir, path));
                docs.insert(doc_id.clone(), Arc::clone(&doc));
                let worker = Arc::clone(&doc);
                tauri::async_runtime::spawn(async move { run_worker(worker).await });
                (doc, true)
            }
        };
        if created {
            // Watch the file's folder at once: edits made during the first
            // build must not be lost before its dependencies are known.
            update_watch(&doc, path, Vec::new(), Vec::new());
        }
        let subscription_id = uuid::Uuid::new_v4().simple().to_string();
        self.subscriptions
            .lock()
            .map_err(|error| error.to_string())?
            .insert(
                subscription_id.clone(),
                Subscription {
                    doc_id,
                    webview: webview.to_string(),
                },
            );
        let snapshot = {
            let mut state = doc.lock();
            state.subscribers.insert(subscription_id.clone(), visible);
            state.snapshot.clone()
        };
        doc.wake.notify_one();
        Ok(OpenResult {
            subscription_id,
            snapshot,
        })
    }

    pub fn set_visible(&self, subscription_id: &str, visible: bool) -> Result<PreviewSnapshot, String> {
        let doc = self.doc_for_subscription(subscription_id)?;
        let snapshot = {
            let mut state = doc.lock();
            if let Some(entry) = state.subscribers.get_mut(subscription_id) {
                *entry = visible;
            }
            state.snapshot.clone()
        };
        doc.wake.notify_one();
        Ok(snapshot)
    }

    pub fn rebuild(&self, subscription_id: &str) -> Result<(), String> {
        let doc = self.doc_for_subscription(subscription_id)?;
        {
            let mut state = doc.lock();
            state.force_svg = true;
            state.sticky_pdf = None;
            state.dirty = true;
        }
        doc.change_seq.fetch_add(1, Ordering::SeqCst);
        doc.wake.notify_one();
        Ok(())
    }

    /// Releases every subscription of a webview that reloaded or closed.
    fn release_webview(&self, webview: &str) {
        let ids = self
            .subscriptions
            .lock()
            .map(|subscriptions| {
                subscriptions
                    .iter()
                    .filter(|(_, subscription)| subscription.webview == webview)
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for id in ids {
            self.close(&id);
        }
    }

    pub fn close(&self, subscription_id: &str) {
        let doc_id = self
            .subscriptions
            .lock()
            .ok()
            .and_then(|mut subscriptions| subscriptions.remove(subscription_id))
            .map(|subscription| subscription.doc_id);
        let Some(doc_id) = doc_id else {
            return;
        };
        let doc = self
            .docs
            .lock()
            .ok()
            .and_then(|docs| docs.get(&doc_id).cloned());
        let Some(doc) = doc else {
            return;
        };
        let empty = {
            let mut state = doc.lock();
            state.subscribers.remove(subscription_id);
            state.subscribers.is_empty()
        };
        if empty {
            self.retire(&doc);
        } else {
            doc.wake.notify_one();
        }
    }

    fn retire(&self, doc: &Arc<Doc>) {
        if let Ok(mut docs) = self.docs.lock() {
            docs.remove(&doc.id);
        }
        if let Ok(mut subscriptions) = self.subscriptions.lock() {
            subscriptions.retain(|_, subscription| subscription.doc_id != doc.id);
        }
        let watcher = {
            let mut state = doc.lock();
            state.closed = true;
            state.subscribers.clear();
            state.watcher.take()
        };
        // Dropping the native watcher may join its callback thread, which
        // takes the state lock; never drop it while holding that lock.
        drop(watcher);
        doc.wake.notify_one();
    }
}

fn create_doc(sink: Sink, doc_id: &str, checkout_id: &str, owner_dir: &Path, path: &Path) -> Doc {
    let out_dir = preview_root().join(doc_id);
    let manifest = std::fs::read(out_dir.join("pub").join(MANIFEST_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
        .filter(|manifest| {
            // Restore only outputs that are still on disk.
            manifest.revision == 0
                || out_dir.join("pub").join(manifest.revision.to_string()).is_dir()
        })
        .unwrap_or_default();
    let last_revision = std::fs::read_dir(out_dir.join("pub"))
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u64>().ok())
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    let pdf_url = (manifest.pdf_revision > 0
        && out_dir
            .join("pub")
            .join(manifest.pdf_revision.to_string())
            .join(PDF_FILE)
            .is_file())
    .then(|| file_url(doc_id, manifest.pdf_revision, PDF_FILE));
    let snapshot = PreviewSnapshot {
        doc_id: doc_id.to_string(),
        checkout_id: checkout_id.to_string(),
        path: path.to_string_lossy().to_string(),
        root_path: path.to_string_lossy().to_string(),
        status: if manifest.revision > 0 { "pending" } else { "idle" }.to_string(),
        mode: manifest.mode.clone(),
        revision: manifest.revision,
        pages: manifest.pages.clone(),
        pdf_url,
        pdf_revision: manifest.pdf_revision,
        fallback_reason: manifest.fallback_reason.clone(),
        fallback_detail: manifest.fallback_detail.clone(),
        engine: manifest.engine,
        errors: Vec::new(),
        warnings: Vec::new(),
        svg_ms: None,
        pdf_ms: None,
        building: false,
        updated_at: manifest.updated_at,
    };
    Doc {
        id: doc_id.to_string(),
        checkout_id: checkout_id.to_string(),
        owner_dir: owner_dir.to_string_lossy().to_string(),
        path: path.to_path_buf(),
        out_dir,
        sink,
        change_seq: AtomicU64::new(0),
        wake: tokio::sync::Notify::new(),
        state: Mutex::new(DocState {
            subscribers: HashMap::new(),
            // Outputs from an earlier session may be stale and the
            // dependencies are unknown: build once when visible.
            dirty: true,
            closed: false,
            force_svg: false,
            next_revision: last_revision.max(manifest.revision),
            sticky_pdf: None,
            deps: HashSet::new(),
            outputs: HashSet::new(),
            watcher: None,
            watched_dirs: Vec::new(),
            snapshot,
        }),
    }
}

async fn run_worker(doc: Arc<Doc>) {
    loop {
        // Wait until the document is dirty and visible.
        loop {
            {
                let state = doc.lock();
                if state.closed {
                    return;
                }
                if state.dirty && state.visible() {
                    break;
                }
            }
            doc.wake.notified().await;
        }
        // Debounce bursts of writes (an agent often edits several times).
        loop {
            let seen = doc.change_seq.load(Ordering::SeqCst);
            tokio::time::sleep(DEBOUNCE).await;
            if doc.lock().closed {
                return;
            }
            if doc.change_seq.load(Ordering::SeqCst) == seen {
                break;
            }
        }
        let started_seq = doc.change_seq.load(Ordering::SeqCst);
        let revision = {
            let mut state = doc.lock();
            if !state.visible() {
                continue;
            }
            state.dirty = false;
            state.next_revision += 1;
            state.snapshot.building = true;
            state.snapshot.status = "building".to_string();
            doc.publish(&mut state);
            state.next_revision
        };
        let build = run_build(Arc::clone(&doc), revision);
        tokio::pin!(build);
        loop {
            tokio::select! {
                _ = &mut build => break,
                _ = doc.wake.notified() => {
                    let state = doc.lock();
                    if state.closed {
                        // Dropping the build kills its processes.
                        return;
                    }
                    if state.dirty && doc.change_seq.load(Ordering::SeqCst) != started_seq {
                        // Superseded: drop this build and start over.
                        break;
                    }
                }
            }
        }
    }
}

struct Route {
    root: PathBuf,
    engine: Engine,
    preamble_hash: String,
    /// Set when the SVG path is skipped.
    pdf_only: Option<String>,
}

fn resolve_route(doc: &Doc, toolchain: &toolchain::Toolchain) -> Result<Route, String> {
    let settings = crate::settings::global::global();
    let opened = std::fs::read(&doc.path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .map_err(|error| format!("cannot read {}: {error}", doc.path.display()))?;
    let root = match pipeline::magic_root(&opened) {
        Some(relative) => {
            let base = doc.path.parent().unwrap_or(Path::new("."));
            let candidate = base.join(relative);
            dunce::canonicalize(&candidate)
                .map_err(|_| format!("the % !TEX root file {} does not exist", candidate.display()))?
        }
        None => doc.path.clone(),
    };
    let source = if root == doc.path {
        opened
    } else {
        std::fs::read(&root)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| format!("cannot read {}: {error}", root.display()))?
    };
    let Some(preamble) = pipeline::preamble(&source) else {
        return Err(
            "This file has no \\documentclass, so it cannot be compiled on its own. Open the main document, or add `% !TEX root = main.tex` to its first lines."
                .to_string(),
        );
    };
    let engine = pipeline::choose_engine(&settings.get(crate::settings::LATEX_ENGINE), &source);
    let preamble_hash = blake3::hash(preamble.as_bytes()).to_hex().to_string();
    let state = doc.lock();
    let pdf_only = if !settings.get(crate::settings::LATEX_PREVIEW_SVG) {
        Some("svg_disabled".to_string())
    } else if toolchain.dvisvgm.is_none() {
        Some("dvisvgm_missing".to_string())
    } else if let Some(reason) = pipeline::pdf_only_reason(&preamble, engine) {
        Some(reason)
    } else if !state.force_svg && state.sticky_pdf.as_deref() == Some(preamble_hash.as_str()) {
        Some("sticky".to_string())
    } else {
        None
    };
    Ok(Route {
        root,
        engine,
        preamble_hash,
        pdf_only,
    })
}

fn safe_jobname(root: &Path) -> String {
    let stem = root
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();
    if !stem.is_empty()
        && stem
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        stem
    } else {
        "document".to_string()
    }
}

fn error_diagnostic(message: String) -> Diagnostic {
    Diagnostic {
        severity: "error".to_string(),
        file: None,
        line: None,
        message,
        context: None,
    }
}

async fn run_build(doc: Arc<Doc>, revision: u64) {
    let toolchain = toolchain::get(false).await;
    let route = match resolve_route(&doc, &toolchain) {
        Ok(route) => route,
        Err(message) => {
            finish_with_error(&doc, vec![error_diagnostic(message)], None);
            return;
        }
    };
    if !toolchain.has_engine() {
        finish_with_error(
            &doc,
            vec![error_diagnostic(
                "No TeX engine was found. Install MiKTeX or TeX Live, then re-detect the toolchain in Settings > LaTeX."
                    .to_string(),
            )],
            Some(route.engine),
        );
        return;
    }
    let settings = crate::settings::global::global();
    let input = BuildInput {
        source_dir: route
            .root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
        jobname: safe_jobname(&route.root),
        source: route.root.clone(),
        engine: route.engine,
        toolchain,
        allow_package_install: settings.get(crate::settings::LATEX_TOOLCHAIN_ALLOW_PACKAGE_INSTALL),
        work_dir: doc.out_dir.join("work"),
        driver_preamble: doc.out_dir.join("locus-svg-driver.tex"),
        owner_dir: doc.owner_dir.clone(),
    };
    {
        let mut state = doc.lock();
        state.force_svg = false;
        state.snapshot.root_path = route.root.to_string_lossy().to_string();
        state.snapshot.engine = Some(route.engine);
    }

    let svg_enabled = route.pdf_only.is_none();
    let svg_future = async {
        if svg_enabled {
            Some(build::run_svg(&input).await)
        } else {
            None
        }
    };
    let pdf_future = build::run_pdf(&input);
    tokio::pin!(svg_future);
    tokio::pin!(pdf_future);
    let mut svg: Option<Option<SvgResult>> = None;
    let mut pdf: Option<PdfResult> = None;
    while svg.is_none() || pdf.is_none() {
        tokio::select! {
            result = &mut svg_future, if svg.is_none() => {
                if let Some(SvgResult::Ready { pages, warnings, elapsed_ms, .. }) = &result {
                    publish_svg(&doc, revision, pages, warnings, *elapsed_ms);
                }
                svg = Some(result);
            }
            result = &mut pdf_future, if pdf.is_none() => {
                // The SVG may still be building; the PDF cache updates now.
                if let PdfResult::Ready { pdf: file, elapsed_ms, .. } = &result {
                    publish_pdf_file(&doc, revision, file, *elapsed_ms);
                }
                pdf = Some(result);
            }
        }
    }
    let svg = svg.flatten();
    let pdf = pdf.expect("pdf result");
    finish(&doc, revision, &route, svg, pdf, &input);
}

fn copy_into(dir: &Path, source: &Path, name: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    std::fs::copy(source, dir.join(name))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn publish_svg(doc: &Doc, revision: u64, pages: &[PathBuf], warnings: &[Diagnostic], elapsed_ms: u64) {
    let dir = doc.pub_dir(revision);
    let mut refs = Vec::with_capacity(pages.len());
    for (index, page) in pages.iter().enumerate() {
        let name = format!("p{}.svg", index + 1);
        if let Err(error) = copy_into(&dir, page, &name) {
            eprintln!("[Locus] failed to publish LaTeX preview page: {error}");
            return;
        }
        let size = std::fs::read_to_string(page)
            .ok()
            .and_then(|svg| pipeline::svg_page_size(&svg));
        refs.push(PageRef {
            url: file_url(&doc.id, revision, &name),
            width: size.map(|(width, _)| width),
            height: size.map(|(_, height)| height),
        });
    }
    let mut state = doc.lock();
    let snapshot = &mut state.snapshot;
    snapshot.revision = revision;
    snapshot.mode = Some("svg".to_string());
    snapshot.pages = refs;
    snapshot.fallback_reason = None;
    snapshot.fallback_detail = None;
    snapshot.errors.clear();
    snapshot.warnings = warnings.to_vec();
    snapshot.svg_ms = Some(elapsed_ms);
    snapshot.status = "ready".to_string();
    doc.publish(&mut state);
}

fn publish_pdf_file(doc: &Doc, revision: u64, pdf: &Path, elapsed_ms: u64) {
    if let Err(error) = copy_into(&doc.pub_dir(revision), pdf, PDF_FILE) {
        eprintln!("[Locus] failed to publish LaTeX preview PDF: {error}");
        return;
    }
    let mut state = doc.lock();
    state.snapshot.pdf_url = Some(file_url(&doc.id, revision, PDF_FILE));
    state.snapshot.pdf_revision = revision;
    state.snapshot.pdf_ms = Some(elapsed_ms);
    doc.publish(&mut state);
}

fn finish_with_error(doc: &Doc, errors: Vec<Diagnostic>, engine: Option<Engine>) {
    let mut state = doc.lock();
    state.snapshot.errors = errors;
    state.snapshot.status = "error".to_string();
    state.snapshot.building = false;
    if engine.is_some() {
        state.snapshot.engine = engine;
    }
    doc.publish(&mut state);
}

/// What a finished build shows, decided from both paths.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    /// SVG pages are shown; `pdf_warning` reports a failed PDF cache build.
    Svg { pdf_warning: Option<Diagnostic> },
    /// The PDF replaces the SVG. `sticky`: skip the SVG attempt while the
    /// preamble stays the same.
    Pdf { reason: String, detail: Option<String>, sticky: bool },
    /// Neither path built; the last good output stays next to `errors`.
    Failed { errors: Vec<Diagnostic> },
}

fn decide(pdf_only: Option<&str>, svg: Option<&SvgResult>, pdf: &PdfResult) -> Outcome {
    let pdf_errors = match pdf {
        PdfResult::Ready { .. } => None,
        PdfResult::Failed { errors, .. } => Some(errors.as_slice()),
    };
    if let Some(SvgResult::Ready { .. }) = svg {
        let pdf_warning = pdf_errors.map(|errors| Diagnostic {
            severity: "warning".to_string(),
            file: errors.first().and_then(|error| error.file.clone()),
            line: errors.first().and_then(|error| error.line),
            message: errors
                .first()
                .map(|error| format!("PDF cache build failed: {}", error.message))
                .unwrap_or_else(|| "PDF cache build failed.".to_string()),
            context: None,
        });
        return Outcome::Svg { pdf_warning };
    }
    let (reason, detail) = match svg {
        Some(SvgResult::Unsupported { reason, detail, .. }) => (reason.clone(), detail.clone()),
        Some(SvgResult::Failed { errors, .. }) => (
            "svg_build_failed".to_string(),
            errors.first().map(|error| error.message.clone()).unwrap_or_default(),
        ),
        _ => (
            pdf_only.unwrap_or("svg_unavailable").to_string(),
            String::new(),
        ),
    };
    match pdf_errors {
        None => Outcome::Pdf {
            // Settings and missing tools are rechecked on every build; a
            // document-caused fallback repeats for the same preamble.
            sticky: !matches!(reason.as_str(), "svg_disabled" | "dvisvgm_missing"),
            reason,
            detail: (!detail.is_empty()).then_some(detail),
        },
        Some(errors) => {
            let errors = if errors.is_empty() {
                match svg {
                    Some(SvgResult::Failed { errors, .. }) => errors.clone(),
                    _ => Vec::new(),
                }
            } else {
                errors.to_vec()
            };
            Outcome::Failed { errors }
        }
    }
}

fn result_deps(svg: Option<&SvgResult>, pdf: &PdfResult) -> Vec<PathBuf> {
    let mut deps = Vec::new();
    match svg {
        Some(SvgResult::Ready { deps: found, .. })
        | Some(SvgResult::Unsupported { deps: found, .. })
        | Some(SvgResult::Failed { deps: found, .. }) => deps.extend(found.iter().cloned()),
        None => {}
    }
    match pdf {
        PdfResult::Ready { deps: found, .. } | PdfResult::Failed { deps: found, .. } => {
            deps.extend(found.iter().cloned())
        }
    }
    deps
}

fn finish(
    doc: &Arc<Doc>,
    revision: u64,
    route: &Route,
    svg: Option<SvgResult>,
    pdf: PdfResult,
    input: &BuildInput,
) {
    let outcome = decide(route.pdf_only.as_deref(), svg.as_ref(), &pdf);
    let deps = result_deps(svg.as_ref(), &pdf);
    let outputs = read_outputs(input);

    let mut state = doc.lock();
    match outcome {
        Outcome::Svg { pdf_warning } => {
            state.sticky_pdf = None;
            state.snapshot.status = "ready".to_string();
            if let Some(warning) = pdf_warning {
                state.snapshot.warnings.insert(0, warning);
            }
        }
        Outcome::Pdf { reason, detail, sticky } => {
            state.sticky_pdf = sticky.then(|| route.preamble_hash.clone());
            let warnings = match &pdf {
                PdfResult::Ready { warnings, .. } => warnings.clone(),
                PdfResult::Failed { .. } => Vec::new(),
            };
            let snapshot = &mut state.snapshot;
            snapshot.revision = revision;
            snapshot.mode = Some("pdf".to_string());
            snapshot.pages.clear();
            snapshot.fallback_reason = Some(reason);
            snapshot.fallback_detail = detail;
            snapshot.errors.clear();
            snapshot.warnings = warnings;
            snapshot.status = "ready".to_string();
        }
        Outcome::Failed { errors } => {
            state.snapshot.errors = errors;
            state.snapshot.status = "error".to_string();
        }
    }
    state.snapshot.building = false;
    state.snapshot.engine = Some(route.engine);
    let keep = [state.snapshot.revision, state.snapshot.pdf_revision];
    write_manifest(doc, &state.snapshot);
    doc.publish(&mut state);
    drop(state);

    prune_revisions(doc, &keep);
    update_watch(doc, &route.root, deps, outputs);
}

fn read_outputs(input: &BuildInput) -> Vec<PathBuf> {
    let mut outputs = Vec::new();
    for mode in ["svg", "pdf"] {
        let fls = input
            .work_dir
            .join(mode)
            .join(format!("{}.fls", input.jobname));
        if let Ok(bytes) = std::fs::read(fls) {
            outputs.extend(pipeline::recorded_outputs(
                &String::from_utf8_lossy(&bytes),
                &input.source_dir,
            ));
        }
    }
    outputs
}

fn write_manifest(doc: &Doc, snapshot: &PreviewSnapshot) {
    let manifest = Manifest {
        revision: snapshot.revision,
        mode: snapshot.mode.clone(),
        pages: snapshot.pages.clone(),
        pdf_revision: snapshot.pdf_revision,
        fallback_reason: snapshot.fallback_reason.clone(),
        fallback_detail: snapshot.fallback_detail.clone(),
        engine: snapshot.engine,
        updated_at: now_ms(),
    };
    let dir = doc.out_dir.join("pub");
    if let Ok(bytes) = serde_json::to_vec_pretty(&manifest) {
        let _ = std::fs::create_dir_all(&dir);
        let temp = dir.join(format!("{MANIFEST_FILE}.tmp"));
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, dir.join(MANIFEST_FILE));
        }
    }
}

fn prune_revisions(doc: &Doc, keep: &[u64]) {
    let Ok(entries) = std::fs::read_dir(doc.out_dir.join("pub")) else {
        return;
    };
    let mut revisions = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u64>().ok())
        .collect::<Vec<_>>();
    revisions.sort_unstable_by(|a, b| b.cmp(a));
    for (index, revision) in revisions.into_iter().enumerate() {
        if index >= KEEP_REVISIONS && !keep.contains(&revision) {
            let _ = std::fs::remove_dir_all(doc.pub_dir(revision));
        }
    }
}

/// Watches the directories of the document's own inputs (the source tree
/// and workspace; never the TeX distribution or the preview output).
fn update_watch(doc: &Arc<Doc>, root: &Path, deps: Vec<PathBuf>, outputs: Vec<PathBuf>) {
    let owner = PathBuf::from(&doc.owner_dir);
    let source_dir = root.parent().map(Path::to_path_buf).unwrap_or_else(|| owner.clone());
    let opened_dir = doc.path.parent().map(Path::to_path_buf).unwrap_or_else(|| source_dir.clone());
    let preview_root_key = pipeline::normalize_key(&preview_root());
    let within = |path: &Path| {
        let key = pipeline::normalize_key(path);
        !key.starts_with(&preview_root_key)
            && [&owner, &source_dir, &opened_dir]
                .iter()
                .any(|base| key.starts_with(&pipeline::normalize_key(base)))
    };
    let mut dep_keys = HashSet::new();
    let mut dirs = vec![source_dir.clone(), opened_dir.clone()];
    dep_keys.insert(pipeline::normalize_key(root));
    dep_keys.insert(pipeline::normalize_key(&doc.path));
    for dep in deps.iter().filter(|dep| within(dep)) {
        dep_keys.insert(pipeline::normalize_key(dep));
        if let Some(parent) = dep.parent() {
            if !dirs.iter().any(|dir| pipeline::normalize_key(dir) == pipeline::normalize_key(parent)) {
                dirs.push(parent.to_path_buf());
            }
        }
    }
    dirs.truncate(MAX_WATCHED_DIRS);
    let output_keys = outputs
        .iter()
        .map(|path| pipeline::normalize_key(path))
        .filter(|key| !dep_keys.contains(key))
        .collect::<HashSet<_>>();

    let unchanged = {
        let mut state = doc.lock();
        state.deps = dep_keys;
        state.outputs = output_keys;
        state.watcher.is_some()
            && state.watched_dirs.len() == dirs.len()
            && state
                .watched_dirs
                .iter()
                .zip(dirs.iter())
                .all(|(a, b)| pipeline::normalize_key(a) == pipeline::normalize_key(b))
    };
    if unchanged {
        return;
    }
    // Callbacks hold a weak handle so a closed document is freed.
    let watcher = create_watcher(Arc::downgrade(doc), &dirs);
    let previous = {
        let mut state = doc.lock();
        if state.closed {
            return;
        }
        state.watched_dirs = dirs;
        std::mem::replace(&mut state.watcher, watcher)
    };
    drop(previous);
}

fn create_watcher(doc: Weak<Doc>, dirs: &[PathBuf]) -> Option<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        let Some(doc) = doc.upgrade() else {
            return;
        };
        if event.paths.iter().any(|path| doc.is_relevant_change(path)) {
            doc.mark_changed();
        }
    })
    .map_err(|error| eprintln!("[Locus] LaTeX preview watcher unavailable: {error}"))
    .ok()?;
    for dir in dirs {
        if dir.is_dir() {
            if let Err(error) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                eprintln!("[Locus] cannot watch {} for LaTeX preview: {error}", dir.display());
            }
        }
    }
    Some(watcher)
}

/// Serves `/<doc>/<revision>/<file>` from the published outputs. Only page
/// SVGs and the PDF of a revision directory are reachable.
pub fn serve(path: &str) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::{header, Response, StatusCode};
    let not_found = || {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(Vec::new())
            .unwrap_or_default()
    };
    let parts = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    let [doc_id, revision, file] = parts.as_slice() else {
        return not_found();
    };
    let valid_doc = doc_id.len() == 20 && doc_id.chars().all(|ch| ch.is_ascii_hexdigit());
    let valid_revision = !revision.is_empty() && revision.chars().all(|ch| ch.is_ascii_digit());
    let mime = if *file == PDF_FILE {
        "application/pdf"
    } else if file
        .strip_prefix('p')
        .and_then(|rest| rest.strip_suffix(".svg"))
        .is_some_and(|number| !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit()))
    {
        "image/svg+xml"
    } else {
        return not_found();
    };
    if !valid_doc || !valid_revision {
        return not_found();
    }
    let full = preview_root().join(doc_id).join("pub").join(revision).join(file);
    match std::fs::read(&full) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime)
            .header(header::CONTENT_LENGTH, bytes.len())
            .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(bytes)
            .unwrap_or_else(|_| not_found()),
        Err(_) => not_found(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_ids_are_stable_and_path_insensitive_on_windows() {
        let a = doc_id_for("c1", Path::new("C:/Docs/Main.tex"));
        let b = doc_id_for("c1", Path::new("C:/Docs/Main.tex"));
        assert_eq!(a, b);
        assert_eq!(a.len(), 20);
        assert_ne!(a, doc_id_for("c2", Path::new("C:/Docs/Main.tex")));
        if cfg!(windows) {
            assert_eq!(a, doc_id_for("c1", Path::new("c:\\docs\\main.tex")));
        }
    }

    #[test]
    fn published_urls_route_through_the_binary_protocol() {
        let url = file_url("0123456789abcdef0123", 3, "p1.svg");
        let path = url
            .strip_prefix("http://locus-binary.localhost")
            .expect("served by locus-binary");
        assert_eq!(
            path.strip_prefix(PROTOCOL_PATH_PREFIX),
            Some("0123456789abcdef0123/3/p1.svg")
        );
    }

    #[test]
    fn webview_hooks_do_not_create_the_manager() {
        // Page loads and closed windows call this even when no preview was
        // ever opened; it must not allocate or lock anything then.
        release_webview("main");
        assert!(MANAGER.get().is_none());
    }

    #[test]
    fn serve_rejects_paths_outside_published_files() {
        for path in [
            "/../secret/1/p1.svg",
            "/0123456789abcdef0123/1/../../x",
            "/0123456789abcdef0123/1/main.tex",
            "/0123456789abcdef0123/x/p1.svg",
            "/0123456789abcdef0123/1/p.svg",
            "/short/1/p1.svg",
        ] {
            assert_eq!(serve(path).status(), 404, "{path}");
        }
    }

    fn error(message: &str, line: Option<u32>) -> Diagnostic {
        Diagnostic {
            severity: "error".to_string(),
            file: Some("main.tex".to_string()),
            line,
            message: message.to_string(),
            context: None,
        }
    }

    fn svg_ready() -> SvgResult {
        SvgResult::Ready {
            pages: vec![PathBuf::from("p1.svg")],
            deps: Vec::new(),
            warnings: Vec::new(),
            elapsed_ms: 1,
        }
    }

    fn pdf_ready() -> PdfResult {
        PdfResult::Ready {
            pdf: PathBuf::from("main.pdf"),
            deps: Vec::new(),
            warnings: Vec::new(),
            elapsed_ms: 1,
        }
    }

    fn pdf_failed(errors: Vec<Diagnostic>) -> PdfResult {
        PdfResult::Failed {
            errors,
            deps: Vec::new(),
        }
    }

    #[test]
    fn svg_wins_and_a_failed_pdf_cache_becomes_a_warning() {
        assert_eq!(
            decide(None, Some(&svg_ready()), &pdf_ready()),
            Outcome::Svg { pdf_warning: None }
        );
        let Outcome::Svg { pdf_warning: Some(warning) } =
            decide(None, Some(&svg_ready()), &pdf_failed(vec![error("Boom", Some(3))]))
        else {
            panic!("expected SVG with a warning");
        };
        assert_eq!(warning.severity, "warning");
        assert_eq!(warning.line, Some(3));
        assert!(warning.message.contains("Boom"));
    }

    #[test]
    fn falls_back_to_pdf_with_the_reason() {
        assert_eq!(
            decide(Some("package:pdfpages"), None, &pdf_ready()),
            Outcome::Pdf { reason: "package:pdfpages".to_string(), detail: None, sticky: true }
        );
        let unsupported = SvgResult::Unsupported {
            reason: "pdf_specials".to_string(),
            detail: "code".to_string(),
            deps: Vec::new(),
        };
        assert_eq!(
            decide(None, Some(&unsupported), &pdf_ready()),
            Outcome::Pdf { reason: "pdf_specials".to_string(), detail: Some("code".to_string()), sticky: true }
        );
        let failed = SvgResult::Failed { errors: vec![error("Driver clash", None)], deps: Vec::new() };
        assert_eq!(
            decide(None, Some(&failed), &pdf_ready()),
            Outcome::Pdf {
                reason: "svg_build_failed".to_string(),
                detail: Some("Driver clash".to_string()),
                sticky: true
            }
        );
        // Settings and missing tools are rechecked every build.
        for reason in ["svg_disabled", "dvisvgm_missing"] {
            let Outcome::Pdf { sticky, .. } = decide(Some(reason), None, &pdf_ready()) else {
                panic!("expected PDF");
            };
            assert!(!sticky, "{reason}");
        }
    }

    #[test]
    fn document_errors_keep_the_last_output() {
        let svg_failed = SvgResult::Failed { errors: vec![error("svg side", Some(2))], deps: Vec::new() };
        assert_eq!(
            decide(None, Some(&svg_failed), &pdf_failed(vec![error("pdf side", Some(4))])),
            Outcome::Failed { errors: vec![error("pdf side", Some(4))] }
        );
        // A PDF failure without a parsed error reports the SVG side's errors.
        assert_eq!(
            decide(None, Some(&svg_failed), &pdf_failed(Vec::new())),
            Outcome::Failed { errors: vec![error("svg side", Some(2))] }
        );
        assert_eq!(
            decide(Some("engine:lualatex"), None, &pdf_failed(vec![error("x", None)])),
            Outcome::Failed { errors: vec![error("x", None)] }
        );
    }

    #[test]
    fn jobnames_stay_tex_safe() {
        assert_eq!(safe_jobname(Path::new("C:/a/thesis-v2.tex")), "thesis-v2");
        assert_eq!(safe_jobname(Path::new("C:/a/论文 草稿.tex")), "document");
    }
}

/// The whole manager against a real TeX: worker, watcher, publication,
/// visibility gating and fallback. Ignored by default (needs MiKTeX or TeX
/// Live with dvisvgm and pdfpages):
/// `cargo test --lib latex_preview::manager::live_tests -- --ignored`.
#[cfg(test)]
mod live_tests {
    use super::*;
    use std::time::Instant;

    type Seen = Arc<Mutex<Vec<PreviewSnapshot>>>;

    fn collector() -> (Seen, impl FnOnce() -> Sink) {
        let seen: Seen = Arc::new(Mutex::new(Vec::new()));
        let pushed = Arc::clone(&seen);
        let sink = move || -> Sink {
            Arc::new(move |snapshot: &PreviewSnapshot| {
                pushed.lock().unwrap().push(snapshot.clone());
            })
        };
        (seen, sink)
    }

    fn count(seen: &Seen) -> usize {
        seen.lock().unwrap().len()
    }

    async fn wait_for(
        seen: &Seen,
        after: usize,
        predicate: impl Fn(&PreviewSnapshot) -> bool,
    ) -> PreviewSnapshot {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(found) = seen
                .lock()
                .unwrap()
                .iter()
                .skip(after)
                .find(|snapshot| predicate(snapshot))
                .cloned()
            {
                return found;
            }
            if Instant::now() > deadline {
                let trail = seen
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|s| format!("{}#{} {:?} {:?} {:?}", s.status, s.revision, s.mode, s.fallback_reason, s.errors))
                    .collect::<Vec<_>>();
                panic!("timed out; snapshots: {trail:#?}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn settled(snapshot: &PreviewSnapshot) -> bool {
        !snapshot.building && (snapshot.status == "ready" || snapshot.status == "error")
    }

    fn served(url: &str) -> tauri::http::Response<Vec<u8>> {
        serve(url.strip_prefix(url_base().as_str()).expect("preview url"))
    }

    async fn tex_available() -> bool {
        let toolchain = toolchain::get(true).await;
        let ok = toolchain.xelatex.is_some() && toolchain.dvisvgm.is_some();
        if !ok {
            std::eprintln!("skipping: xelatex or dvisvgm not installed");
        }
        ok
    }

    fn document(body: &str) -> String {
        format!("\\documentclass{{article}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn rebuilds_on_change_only_while_visible() {
        if !tex_available().await {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("paper");
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.tex");
        std::fs::write(&main, document("First version.")).unwrap();

        let manager = PreviewManager::default();
        let (seen, sink) = collector();
        let opened = manager
            .open_with_sink(sink, "test", "checkout-a", temp.path(), &main, true)
            .unwrap();
        let first = wait_for(&seen, 0, settled).await;
        assert_eq!(first.status, "ready", "{:?}", first.errors);
        assert_eq!(
            first.mode.as_deref(),
            Some("svg"),
            "fallback {:?}: {:?}",
            first.fallback_reason,
            first.fallback_detail
        );
        assert_eq!(first.pages.len(), 1);
        let page = served(&first.pages[0].url);
        assert_eq!(page.status(), 200);
        assert!(String::from_utf8_lossy(page.body()).contains("<svg"));
        let pdf = served(first.pdf_url.as_deref().expect("pdf cache"));
        assert!(pdf.body().starts_with(b"%PDF"));

        // Visible: an edit rebuilds by itself.
        let mark = count(&seen);
        std::fs::write(&main, document("Second version.")).unwrap();
        let second = wait_for(&seen, mark, |s| settled(s) && s.revision > first.revision).await;
        assert_eq!(second.status, "ready");

        // Hidden: an edit only marks the preview pending.
        manager.set_visible(&opened.subscription_id, false).unwrap();
        let mark = count(&seen);
        std::fs::write(&main, document("Third version.")).unwrap();
        wait_for(&seen, mark, |s| s.status == "pending").await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            seen.lock().unwrap().iter().skip(mark).all(|s| s.status != "building"),
            "a hidden preview must not build"
        );

        // Shown again: the pending change builds.
        manager.set_visible(&opened.subscription_id, true).unwrap();
        let third = wait_for(&seen, mark, |s| settled(s) && s.revision > second.revision).await;
        assert_eq!(third.status, "ready");

        manager.close(&opened.subscription_id);
        assert!(manager.docs.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(preview_root().join(&first.doc_id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore]
    async fn pdf_only_documents_fall_back_and_errors_keep_the_last_output() {
        if !tex_available().await {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("slides.tex");
        let source = |body: &str| {
            format!("\\documentclass{{article}}\n\\usepackage{{pdfpages}}\n\\begin{{document}}\n{body}\n\\end{{document}}\n")
        };
        std::fs::write(&main, source("Uses pdfpages.")).unwrap();

        let manager = PreviewManager::default();
        let (seen, sink) = collector();
        let opened = manager
            .open_with_sink(sink, "test", "checkout-b", temp.path(), &main, true)
            .unwrap();
        let first = wait_for(&seen, 0, settled).await;
        assert_eq!(first.status, "ready", "{:?}", first.errors);
        assert_eq!(first.mode.as_deref(), Some("pdf"));
        assert_eq!(first.fallback_reason.as_deref(), Some("package:pdfpages"));
        assert!(first.pages.is_empty());
        let pdf_url = first.pdf_url.clone().expect("pdf");

        let mark = count(&seen);
        std::fs::write(&main, source("Broken \\undefinedmacro here.")).unwrap();
        let failed = wait_for(&seen, mark, |s| settled(s) && s.status == "error").await;
        assert_eq!(failed.errors[0].line, Some(4), "{:?}", failed.errors);
        assert!(failed.errors[0].message.contains("Undefined control sequence"));
        // The last good PDF stays visible next to the error.
        assert_eq!(failed.pdf_url.as_deref(), Some(pdf_url.as_str()));
        assert_eq!(served(&pdf_url).status(), 200);

        manager.close(&opened.subscription_id);
        let _ = std::fs::remove_dir_all(preview_root().join(&first.doc_id));
    }
}
