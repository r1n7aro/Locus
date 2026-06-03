use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::time::Instant;

use crate::error::{AppError, AppResult};
use crate::process_util::command;

use super::context::SideFileSource;
use super::profiler::DiffProfiler;
use super::types::*;

#[derive(Debug, Clone)]
pub(crate) enum SideContentState {
    Normal,
    LfsResolved,
    LfsBinaryResolved,
    LfsPointerOnly { oid: String, size: u64 },
}

#[derive(Debug, Clone)]
pub(crate) struct ContentPair {
    pub(crate) old_content: String,
    pub(crate) new_content: String,
    pub(crate) old_bytes: Option<Vec<u8>>,
    pub(crate) new_bytes: Option<Vec<u8>>,
    pub(crate) status: String,
    pub(crate) old_file_source: SideFileSource,
    pub(crate) new_file_source: SideFileSource,
    pub(crate) old_content_state: SideContentState,
    pub(crate) new_content_state: SideContentState,
}

// ── Batch git blob reader ──

pub(crate) struct BatchBlobReader {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    cache: HashMap<String, Option<String>>,
}

impl BatchBlobReader {
    pub(crate) fn new(cwd: &str) -> Option<Self> {
        let mut child = command("git")
            .args(["cat-file", "--batch"])
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let stdin = child.stdin.take()?;
        let stdout = BufReader::new(child.stdout.take()?);
        Some(Self {
            child,
            stdin,
            stdout,
            cache: HashMap::new(),
        })
    }

    pub(crate) fn read_blob(&mut self, ref_and_path: &str) -> Option<String> {
        if let Some(cached) = self.cache.get(ref_and_path) {
            return cached.clone();
        }
        writeln!(self.stdin, "{}", ref_and_path).ok()?;
        self.stdin.flush().ok()?;
        let mut header = String::new();
        self.stdout.read_line(&mut header).ok()?;
        if header.contains("missing") {
            self.cache.insert(ref_and_path.to_string(), None);
            return None;
        }
        let size: usize = header.split_whitespace().nth(2)?.parse().ok()?;
        let mut buf = vec![0u8; size];
        self.stdout.read_exact(&mut buf).ok()?;
        // Trailing newline after blob content
        let mut nl = [0u8; 1];
        let _ = self.stdout.read_exact(&mut nl);
        let content = String::from_utf8_lossy(&buf).to_string();
        self.cache
            .insert(ref_and_path.to_string(), Some(content.clone()));
        Some(content)
    }
}

impl Drop for BatchBlobReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Git show helpers ──

/// Returns raw bytes from `git show`. Callers convert to String as needed.
pub(crate) async fn git_show_file(
    cwd: &str,
    ref_and_path: &str,
) -> Result<Option<Vec<u8>>, AppError> {
    let output = command("git")
        .args(["-c", "core.quotePath=false", "show", ref_and_path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| AppError::new("diff.git_show", format!("git show failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("does not exist")
            || stderr.contains("bad revision")
            || stderr.contains("fatal: path")
            || stderr.contains("exists on disk, but not in")
        {
            return Ok(None);
        }
        return Err(AppError::new(
            "diff.git_show",
            format!("git show failed: {}", stderr.trim()),
        ));
    }

    Ok(Some(output.stdout))
}

pub(crate) fn git_show_file_sync(cwd: &str, ref_and_path: &str) -> Option<String> {
    let output = command("git")
        .args(["-c", "core.quotePath=false", "show", ref_and_path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

// ── Binary detection ──

pub(crate) fn is_binary(content: &str) -> bool {
    let check_len = content.len().min(8192);
    content.as_bytes()[..check_len].contains(&0)
}

pub(crate) fn is_binary_bytes(bytes: &[u8]) -> bool {
    let check_len = bytes.len().min(8192);
    bytes[..check_len].contains(&0)
}

// ── Binary preview helpers ──

fn is_previewable_binary_ext(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tga" | "psd" | "fbx"
    )
}

pub(crate) fn detect_binary_kind(path: &str, bytes: &[u8]) -> Option<BinaryPreviewKind> {
    // Magic bytes take priority
    if bytes.len() >= 4 && bytes[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return Some(BinaryPreviewKind::Image); // PNG
    }
    if bytes.len() >= 3 && bytes[..3] == [0xff, 0xd8, 0xff] {
        return Some(BinaryPreviewKind::Image); // JPEG
    }
    if bytes.len() >= 4 && &bytes[..4] == b"8BPS" {
        return Some(BinaryPreviewKind::Psd);
    }
    if bytes.len() >= 11 && &bytes[..11] == b"Kaydara FBX" {
        return Some(BinaryPreviewKind::Model); // FBX ASCII/Binary
    }
    // Extension fallback
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tga" => Some(BinaryPreviewKind::Image),
        "psd" => Some(BinaryPreviewKind::Psd),
        "fbx" => Some(BinaryPreviewKind::Model),
        _ => None,
    }
}

pub(crate) fn within_binary_threshold(kind: BinaryPreviewKind, size: u64) -> bool {
    match kind {
        BinaryPreviewKind::Image => size <= 20 * 1024 * 1024, // 20MB
        BinaryPreviewKind::Psd => size <= 50 * 1024 * 1024,   // 50MB
        BinaryPreviewKind::Model => size <= 25 * 1024 * 1024, // 25MB
    }
}

pub(crate) fn mime_for_ext(path: &str) -> String {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "bmp" => "image/bmp",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "tga" => "image/x-tga",
        "psd" => "application/x-photoshop",
        "fbx" => "application/octet-stream",
        _ => "application/octet-stream",
    }
    .into()
}

// ── LFS pointer detection & smudge ──

/// Known binary extensions that should never be smudged for diff — return
/// immediately as binary to avoid downloading/reading large blobs.
fn is_known_binary_ext(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        // 3D models
        "fbx" | "obj" | "blend" | "dae" | "3ds" | "max" | "ma" | "mb"
        // Textures / images
        | "png" | "jpg" | "jpeg" | "psd" | "tga" | "tif" | "tiff"
        | "exr" | "hdr" | "bmp" | "gif" | "ico" | "webp"
        // Audio
        | "wav" | "mp3" | "ogg" | "aif" | "aiff" | "flac"
        // Video
        | "mp4" | "mov" | "avi" | "webm" | "mkv"
        // Fonts
        | "ttf" | "otf" | "woff" | "woff2"
        // Compiled / binary
        | "dll" | "so" | "a" | "dylib" | "exe" | "o" | "lib"
        // Compressed
        | "zip" | "7z" | "gz" | "rar" | "tar" | "bz2" | "xz"
        // Unity binary
        | "unitypackage" | "cubemap"
    )
}

/// Extract the file path portion from a git ref spec like "HEAD:path" or ":path".
fn path_from_rev_spec(rev_path: &str) -> &str {
    rev_path.find(':').map_or(rev_path, |i| &rev_path[i + 1..])
}

pub(crate) struct LfsPointer {
    oid: String,
    size: u64,
}

#[derive(Debug, Clone, Default)]
struct LfsResolveMetrics {
    total_ms: u64,
    utf8_decode_ms: u64,
    pointer_parse_ms: u64,
    binary_probe_ms: u64,
    smudge_ms: Option<u64>,
    raw_bytes: usize,
    resolved_bytes: Option<usize>,
    state_label: &'static str,
    path_kind: &'static str,
    smudge_result: &'static str,
}

#[derive(Debug)]
struct LfsResolveOutcome {
    content: Option<String>,
    state: SideContentState,
    bytes: Option<Vec<u8>>,
    metrics: LfsResolveMetrics,
}

fn finish_lfs_outcome(
    started: Instant,
    mut metrics: LfsResolveMetrics,
    content: Option<String>,
    state: SideContentState,
    bytes: Option<Vec<u8>>,
) -> LfsResolveOutcome {
    metrics.total_ms = started.elapsed().as_millis() as u64;
    LfsResolveOutcome {
        content,
        state,
        bytes,
        metrics,
    }
}

fn record_lfs_metrics(profiler: &mut DiffProfiler, side: &str, metrics: &LfsResolveMetrics) {
    if metrics.raw_bytes == 0 && metrics.total_ms == 0 && metrics.state_label.is_empty() {
        return;
    }

    profiler.record_fetch_side(&format!("{}_lfs_total", side), metrics.total_ms);
    profiler.record_fetch_side(&format!("{}_lfs_utf8_decode", side), metrics.utf8_decode_ms);
    profiler.record_fetch_side(
        &format!("{}_lfs_pointer_parse", side),
        metrics.pointer_parse_ms,
    );
    profiler.record_fetch_side(
        &format!("{}_lfs_binary_probe", side),
        metrics.binary_probe_ms,
    );
    if let Some(smudge_ms) = metrics.smudge_ms {
        profiler.record_fetch_side(&format!("{}_lfs_smudge", side), smudge_ms);
    }

    profiler.record_fetch_note(format!(
        "{}_lfs state={} pathKind={} rawBytes={} resolvedBytes={} smudge={}",
        side,
        if metrics.state_label.is_empty() {
            "unknown"
        } else {
            metrics.state_label
        },
        if metrics.path_kind.is_empty() {
            "unknown"
        } else {
            metrics.path_kind
        },
        metrics.raw_bytes,
        metrics
            .resolved_bytes
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string()),
        if metrics.smudge_result.is_empty() {
            "-"
        } else {
            metrics.smudge_result
        },
    ));
}

pub(crate) fn parse_lfs_pointer(content: &str) -> Option<LfsPointer> {
    if !content.starts_with("version https://git-lfs.github.com/spec/v1\n") {
        return None;
    }
    if content.len() > 1024 {
        return None;
    }
    let mut oid = None;
    let mut size = None;
    for line in content.lines().skip(1) {
        if let Some(rest) = line.strip_prefix("oid ") {
            oid = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("size ") {
            size = rest.parse::<u64>().ok();
        }
    }
    Some(LfsPointer {
        oid: oid?,
        size: size?,
    })
}

async fn git_cat_file_filtered(cwd: &str, rev_path: &str) -> Option<Vec<u8>> {
    let output = command("git")
        .args(["cat-file", "--filters", rev_path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

pub(crate) fn git_cat_file_filtered_sync(cwd: &str, rev_path: &str) -> Option<Vec<u8>> {
    let output = command("git")
        .args(["cat-file", "--filters", rev_path])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .ok()?;
    if output.status.success() {
        Some(output.stdout)
    } else {
        None
    }
}

/// Detect LFS pointer in raw git content and attempt to smudge.
/// Returns (resolved content, side state, optional raw bytes for binary preview).
async fn resolve_lfs_if_needed(
    cwd: &str,
    raw: Option<Vec<u8>>,
    rev_path: &str,
) -> LfsResolveOutcome {
    let started = Instant::now();
    let mut metrics = LfsResolveMetrics {
        raw_bytes: raw.as_ref().map_or(0, |bytes| bytes.len()),
        ..LfsResolveMetrics::default()
    };

    let Some(raw_bytes) = raw else {
        return finish_lfs_outcome(started, metrics, None, SideContentState::Normal, None);
    };

    // Try to interpret as UTF-8 text to check for LFS pointer
    let utf8_started = Instant::now();
    let text = match std::str::from_utf8(&raw_bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            // Not valid UTF-8 — definitely binary, not an LFS pointer
            metrics.utf8_decode_ms = utf8_started.elapsed().as_millis() as u64;
            metrics.state_label = "normal";
            metrics.path_kind = "raw_binary";
            metrics.resolved_bytes = Some(raw_bytes.len());
            return finish_lfs_outcome(
                started,
                metrics,
                None,
                SideContentState::Normal,
                Some(raw_bytes),
            );
        }
    };
    metrics.utf8_decode_ms = utf8_started.elapsed().as_millis() as u64;

    let pointer_started = Instant::now();
    let pointer = parse_lfs_pointer(&text);
    metrics.pointer_parse_ms = pointer_started.elapsed().as_millis() as u64;

    let Some(pointer) = pointer else {
        // Not an LFS pointer — return text content, check binary later
        let binary_probe_started = Instant::now();
        let is_binary = is_binary_bytes(&raw_bytes);
        metrics.binary_probe_ms = binary_probe_started.elapsed().as_millis() as u64;

        if is_binary {
            metrics.state_label = "normal";
            metrics.path_kind = "non_lfs_binary";
            metrics.resolved_bytes = Some(raw_bytes.len());
            return finish_lfs_outcome(
                started,
                metrics,
                String::from_utf8(raw_bytes.clone()).ok(),
                SideContentState::Normal,
                Some(raw_bytes),
            );
        }
        metrics.state_label = "normal";
        metrics.path_kind = "non_lfs_text";
        return finish_lfs_outcome(started, metrics, Some(text), SideContentState::Normal, None);
    };

    let file_path = path_from_rev_spec(rev_path);
    let previewable = is_previewable_binary_ext(file_path);

    // Fast path: known binary extension → skip smudge unless previewable
    if is_known_binary_ext(file_path) {
        if previewable {
            // Previewable binary in LFS — smudge to get actual bytes for preview
            metrics.path_kind = "previewable_binary";
            let smudge_started = Instant::now();
            let smudged = git_cat_file_filtered(cwd, rev_path).await;
            metrics.smudge_ms = Some(smudge_started.elapsed().as_millis() as u64);
            match smudged {
                Some(bytes) => {
                    metrics.state_label = "lfs_binary_resolved";
                    metrics.smudge_result = "ok";
                    metrics.resolved_bytes = Some(bytes.len());
                    return finish_lfs_outcome(
                        started,
                        metrics,
                        None,
                        SideContentState::LfsBinaryResolved,
                        Some(bytes),
                    );
                }
                None => {
                    metrics.state_label = "lfs_pointer_only";
                    metrics.smudge_result = "missing";
                    return finish_lfs_outcome(
                        started,
                        metrics,
                        None,
                        SideContentState::LfsPointerOnly {
                            oid: pointer.oid,
                            size: pointer.size,
                        },
                        None,
                    );
                }
            }
        }
        metrics.state_label = "lfs_binary_resolved";
        metrics.path_kind = "binary_skip_smudge";
        metrics.smudge_result = "skipped";
        return finish_lfs_outcome(
            started,
            metrics,
            None,
            SideContentState::LfsBinaryResolved,
            None,
        );
    }

    // Potentially text (e.g. Unity YAML in LFS) — attempt smudge
    metrics.path_kind = "text_smudge";
    let smudge_started = Instant::now();
    let smudged = git_cat_file_filtered(cwd, rev_path).await;
    metrics.smudge_ms = Some(smudge_started.elapsed().as_millis() as u64);

    match smudged {
        Some(bytes) => {
            let binary_probe_started = Instant::now();
            let is_binary = is_binary_bytes(&bytes);
            metrics.binary_probe_ms = binary_probe_started.elapsed().as_millis() as u64;

            if is_binary {
                // Binary content after smudge
                metrics.state_label = "lfs_binary_resolved";
                metrics.smudge_result = "ok_binary";
                metrics.resolved_bytes = Some(bytes.len());
                finish_lfs_outcome(
                    started,
                    metrics,
                    None,
                    SideContentState::LfsBinaryResolved,
                    Some(bytes),
                )
            } else {
                metrics.state_label = "lfs_resolved";
                metrics.smudge_result = "ok_text";
                metrics.resolved_bytes = Some(bytes.len());
                finish_lfs_outcome(
                    started,
                    metrics,
                    Some(String::from_utf8_lossy(&bytes).to_string()),
                    SideContentState::LfsResolved,
                    None,
                )
            }
        }
        None => {
            // Smudge failed — LFS object not downloaded
            metrics.state_label = "lfs_pointer_only";
            metrics.smudge_result = "missing";
            finish_lfs_outcome(
                started,
                metrics,
                None,
                SideContentState::LfsPointerOnly {
                    oid: pointer.oid,
                    size: pointer.size,
                },
                None,
            )
        }
    }
}

/// Synchronous version for load_side_text_file.
pub(crate) fn resolve_lfs_sync(cwd: &str, raw: Option<String>, rev_path: &str) -> Option<String> {
    let content = raw?;
    if parse_lfs_pointer(&content).is_none() {
        return Some(content);
    }
    // Fast path: known binary extension → no point smudging for text diff
    if is_known_binary_ext(path_from_rev_spec(rev_path)) {
        return None;
    }
    // LFS pointer — try to smudge, return None if binary or unavailable
    let bytes = git_cat_file_filtered_sync(cwd, rev_path)?;
    if is_binary_bytes(&bytes) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

// ── File type detection ──

pub(crate) fn lang_from_path(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next()?.to_lowercase();
    match ext.as_str() {
        "cs" | "csharp" => Some("csharp".into()),
        "ts" | "tsx" => Some("typescript".into()),
        "js" | "jsx" => Some("javascript".into()),
        "json" => Some("json".into()),
        "xml" | "html" | "htm" | "svg" | "csproj" | "sln" | "asmdef" => Some("xml".into()),
        "css" | "scss" => Some("css".into()),
        "rs" => Some("rust".into()),
        "py" => Some("python".into()),
        "yaml" | "yml" | "unity" | "prefab" | "mat" | "asset" | "controller" | "anim"
        | "overridecontroller" | "mixer" | "physicmaterial" | "physicsmaterial2d" | "flare"
        | "mask" | "fontsettings" | "preset" | "lighting" | "terrainlayer"
        | "rendertexture" | "signal" | "playable" | "cubemap" | "guiskin" | "brush" => {
            Some("yaml".into())
        }
        "sh" | "bash" | "zsh" => Some("bash".into()),
        "md" | "markdown" => Some("markdown".into()),
        "toml" => Some("ini".into()),
        "diff" | "patch" => Some("diff".into()),
        _ => None,
    }
}

pub(crate) fn is_unity_yaml(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    matches!(
        ext.as_str(),
        "unity"
            | "prefab"
            | "mat"
            | "asset"
            | "controller"
            | "anim"
            | "overridecontroller"
            | "mixer"
            | "physicmaterial"
            | "physicsmaterial2d"
            | "flare"
            | "mask"
            | "fontsettings"
            | "preset"
            | "lighting"
            | "terrainlayer"
            | "rendertexture"
            | "signal"
            | "playable"
            | "cubemap"
            | "guiskin"
            | "brush"
    )
}

pub(crate) fn unity_asset_kind(path: &str) -> UnityAssetKind {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "unity" => UnityAssetKind::Scene,
        "prefab" => UnityAssetKind::Prefab,
        "mat" => UnityAssetKind::Material,
        "anim" => UnityAssetKind::AnimationClip,
        "controller" | "overridecontroller" => UnityAssetKind::AnimatorController,
        _ => UnityAssetKind::GenericYaml,
    }
}

// ── Content fetching ──

pub(crate) async fn fetch_content(
    cwd: &str,
    request: &FileDiffRequest,
    undo_mgr: &crate::vcs::UndoManager,
    profiler: &mut DiffProfiler,
) -> AppResult<ContentPair> {
    let path = &request.file_path;
    let old_path = request.old_path.as_deref().unwrap_or(path);

    match request.source {
        DiffSource::GitCommit => {
            let hash = request.commit_hash.as_deref().ok_or_else(|| {
                AppError::new("diff.missing_param", "commitHash is required for gitCommit")
            })?;
            let old_ref = format!("{}^:{}", hash, old_path);
            let new_ref = format!("{}:{}", hash, path);

            // Parallel fetch: both git show calls run concurrently
            let t0 = Instant::now();
            let (old_raw_res, new_raw_res) =
                tokio::join!(git_show_file(cwd, &old_ref), git_show_file(cwd, &new_ref),);
            let git_show_ms = t0.elapsed().as_millis() as u64;
            profiler.record_fetch_side("git_show_parallel", git_show_ms);
            let old_raw = old_raw_res?;
            let new_raw = new_raw_res?;

            // Parallel LFS resolve: both sides run concurrently
            let t1 = Instant::now();
            let (old_lfs, new_lfs) = tokio::join!(
                resolve_lfs_if_needed(cwd, old_raw, &old_ref),
                resolve_lfs_if_needed(cwd, new_raw, &new_ref),
            );
            let lfs_ms = t1.elapsed().as_millis() as u64;
            profiler.record_fetch_side("lfs_resolve_parallel", lfs_ms);
            record_lfs_metrics(profiler, "old", &old_lfs.metrics);
            record_lfs_metrics(profiler, "new", &new_lfs.metrics);

            let status = match (&old_lfs.content, &new_lfs.content) {
                (None, Some(_)) => "A",
                (Some(_), None) => "D",
                _ if old_path != path => "R",
                _ => "M",
            };
            Ok(ContentPair {
                old_content: old_lfs.content.unwrap_or_default(),
                new_content: new_lfs.content.unwrap_or_default(),
                old_bytes: old_lfs.bytes,
                new_bytes: new_lfs.bytes,
                status: status.to_string(),
                old_file_source: SideFileSource::GitRef(format!("{}^", hash)),
                new_file_source: SideFileSource::GitRef(hash.to_string()),
                old_content_state: old_lfs.state,
                new_content_state: new_lfs.state,
            })
        }
        DiffSource::GitStaged => {
            let old_ref = format!("HEAD:{}", old_path);
            let new_ref = format!(":{}", path);

            // Parallel fetch: both git show calls run concurrently
            let t0 = Instant::now();
            let (old_raw_res, new_raw_res) =
                tokio::join!(git_show_file(cwd, &old_ref), git_show_file(cwd, &new_ref),);
            let git_show_ms = t0.elapsed().as_millis() as u64;
            profiler.record_fetch_side("git_show_parallel", git_show_ms);
            let old_raw = old_raw_res?;
            let new_raw = new_raw_res?;

            // Parallel LFS resolve: both sides run concurrently
            let t1 = Instant::now();
            let (old_lfs, new_lfs) = tokio::join!(
                resolve_lfs_if_needed(cwd, old_raw, &old_ref),
                resolve_lfs_if_needed(cwd, new_raw, &new_ref),
            );
            let lfs_ms = t1.elapsed().as_millis() as u64;
            profiler.record_fetch_side("lfs_resolve_parallel", lfs_ms);
            record_lfs_metrics(profiler, "old", &old_lfs.metrics);
            record_lfs_metrics(profiler, "new", &new_lfs.metrics);

            let status = match (&old_lfs.content, &new_lfs.content) {
                (None, Some(_)) => "A",
                (Some(_), None) => "D",
                _ if old_path != path => "R",
                _ => "M",
            };
            Ok(ContentPair {
                old_content: old_lfs.content.unwrap_or_default(),
                new_content: new_lfs.content.unwrap_or_default(),
                old_bytes: old_lfs.bytes,
                new_bytes: new_lfs.bytes,
                status: status.to_string(),
                old_file_source: SideFileSource::GitRef("HEAD".into()),
                new_file_source: SideFileSource::GitIndex,
                old_content_state: old_lfs.state,
                new_content_state: new_lfs.state,
            })
        }
        DiffSource::GitUnstaged => {
            let preferred_old_ref = format!(":{}", path);
            let fallback_old_ref = if old_path != path {
                Some(format!(":{}", old_path))
            } else {
                None
            };
            let t0 = Instant::now();
            let mut old_ref = preferred_old_ref.clone();
            let mut old_raw = git_show_file(cwd, &old_ref).await?;
            let mut used_fallback_old_path = false;
            if old_raw.is_none() {
                if let Some(fallback_old_ref) = fallback_old_ref.as_ref() {
                    old_ref = fallback_old_ref.clone();
                    old_raw = git_show_file(cwd, &old_ref).await?;
                    used_fallback_old_path = old_raw.is_some();
                }
            }
            profiler.record_fetch_side("old_git_index", t0.elapsed().as_millis() as u64);
            let old_lfs = resolve_lfs_if_needed(cwd, old_raw, &old_ref).await;
            record_lfs_metrics(profiler, "old", &old_lfs.metrics);
            let full_path = Path::new(cwd).join(path);
            let t1 = Instant::now();
            let (new, new_cs, new_bin) = match tokio::fs::read(&full_path).await {
                Ok(bytes) => {
                    if is_binary_bytes(&bytes) {
                        (None, SideContentState::LfsBinaryResolved, Some(bytes))
                    } else {
                        (
                            Some(String::from_utf8_lossy(&bytes).to_string()),
                            SideContentState::Normal,
                            None,
                        )
                    }
                }
                Err(_) => (None, SideContentState::Normal, None),
            };
            profiler.record_fetch_side("new_workspace", t1.elapsed().as_millis() as u64);
            let status = match (&old_lfs.content, &new) {
                (None, Some(_)) => "A",
                (Some(_), None) => "D",
                _ if used_fallback_old_path => "R",
                _ => "M",
            };
            Ok(ContentPair {
                old_content: old_lfs.content.unwrap_or_default(),
                new_content: new.unwrap_or_default(),
                old_bytes: old_lfs.bytes,
                new_bytes: new_bin,
                status: status.to_string(),
                old_file_source: SideFileSource::GitIndex,
                new_file_source: SideFileSource::Workspace,
                old_content_state: old_lfs.state,
                new_content_state: new_cs,
            })
        }
        DiffSource::ChatCheckpoint => {
            let session_id = request.session_id.as_deref().ok_or_else(|| {
                AppError::new(
                    "diff.missing_param",
                    "sessionId is required for chatCheckpoint",
                )
            })?;
            let message_id = request.assistant_message_id.as_deref().ok_or_else(|| {
                AppError::new(
                    "diff.missing_param",
                    "assistantMessageId is required for chatCheckpoint",
                )
            })?;
            let entry = undo_mgr
                .find_entry(session_id, message_id)
                .await
                .ok_or_else(|| {
                    AppError::new(
                        "diff.checkpoint_expired",
                        "Undo checkpoint not found (it may have expired after restart)",
                    )
                })?;
            let old_ref = format!("{}:{}", entry.checkpoint.id, old_path);
            let t0 = Instant::now();
            let old_raw = git_show_file(cwd, &old_ref).await?;
            profiler.record_fetch_side("old_git_show", t0.elapsed().as_millis() as u64);
            let old_lfs = resolve_lfs_if_needed(cwd, old_raw, &old_ref).await;
            record_lfs_metrics(profiler, "old", &old_lfs.metrics);
            let full_path = Path::new(cwd).join(path);
            let t1 = Instant::now();
            let (new, new_cs, new_bin) = match tokio::fs::read(&full_path).await {
                Ok(bytes) => {
                    if is_binary_bytes(&bytes) {
                        (None, SideContentState::LfsBinaryResolved, Some(bytes))
                    } else {
                        (
                            Some(String::from_utf8_lossy(&bytes).to_string()),
                            SideContentState::Normal,
                            None,
                        )
                    }
                }
                Err(_) => (None, SideContentState::Normal, None),
            };
            profiler.record_fetch_side("new_workspace", t1.elapsed().as_millis() as u64);
            let status = match (&old_lfs.content, &new) {
                (None, Some(_)) => "A",
                (Some(_), None) => "D",
                _ if old_path != path => "R",
                _ => "M",
            };
            Ok(ContentPair {
                old_content: old_lfs.content.unwrap_or_default(),
                new_content: new.unwrap_or_default(),
                old_bytes: old_lfs.bytes,
                new_bytes: new_bin,
                status: status.to_string(),
                old_file_source: SideFileSource::GitRef(entry.checkpoint.id.clone()),
                new_file_source: SideFileSource::Workspace,
                old_content_state: old_lfs.state,
                new_content_state: new_cs,
            })
        }
        DiffSource::GitConflictBaseToLeft => {
            let old_ref = format!(":1:{}", old_path); // base (stage 1)
            let new_ref = format!(":2:{}", path); // ours (stage 2)
            let t0 = Instant::now();
            let old_raw = git_show_file(cwd, &old_ref).await?;
            profiler.record_fetch_side("old_stage1", t0.elapsed().as_millis() as u64);
            let t1 = Instant::now();
            let new_raw = git_show_file(cwd, &new_ref).await?;
            profiler.record_fetch_side("new_stage2", t1.elapsed().as_millis() as u64);
            let old_lfs = resolve_lfs_if_needed(cwd, old_raw, &old_ref).await;
            let new_lfs = resolve_lfs_if_needed(cwd, new_raw, &new_ref).await;
            record_lfs_metrics(profiler, "old", &old_lfs.metrics);
            record_lfs_metrics(profiler, "new", &new_lfs.metrics);
            let status = match (&old_lfs.content, &new_lfs.content) {
                (None, Some(_)) => "A",
                (Some(_), None) => "D",
                _ => "M",
            };
            Ok(ContentPair {
                old_content: old_lfs.content.unwrap_or_default(),
                new_content: new_lfs.content.unwrap_or_default(),
                old_bytes: old_lfs.bytes,
                new_bytes: new_lfs.bytes,
                status: status.to_string(),
                old_file_source: SideFileSource::GitStage(1),
                new_file_source: SideFileSource::GitStage(2),
                old_content_state: old_lfs.state,
                new_content_state: new_lfs.state,
            })
        }
        DiffSource::GitConflictBaseToRight => {
            let old_ref = format!(":1:{}", old_path); // base (stage 1)
            let new_ref = format!(":3:{}", path); // theirs (stage 3)
            let t0 = Instant::now();
            let old_raw = git_show_file(cwd, &old_ref).await?;
            profiler.record_fetch_side("old_stage1", t0.elapsed().as_millis() as u64);
            let t1 = Instant::now();
            let new_raw = git_show_file(cwd, &new_ref).await?;
            profiler.record_fetch_side("new_stage3", t1.elapsed().as_millis() as u64);
            let old_lfs = resolve_lfs_if_needed(cwd, old_raw, &old_ref).await;
            let new_lfs = resolve_lfs_if_needed(cwd, new_raw, &new_ref).await;
            record_lfs_metrics(profiler, "old", &old_lfs.metrics);
            record_lfs_metrics(profiler, "new", &new_lfs.metrics);
            let status = match (&old_lfs.content, &new_lfs.content) {
                (None, Some(_)) => "A",
                (Some(_), None) => "D",
                _ => "M",
            };
            Ok(ContentPair {
                old_content: old_lfs.content.unwrap_or_default(),
                new_content: new_lfs.content.unwrap_or_default(),
                old_bytes: old_lfs.bytes,
                new_bytes: new_lfs.bytes,
                status: status.to_string(),
                old_file_source: SideFileSource::GitStage(1),
                new_file_source: SideFileSource::GitStage(3),
                old_content_state: old_lfs.state,
                new_content_state: new_lfs.state,
            })
        }
    }
}
