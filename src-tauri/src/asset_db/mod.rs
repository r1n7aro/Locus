mod db;
pub(crate) mod meta_parser;
pub(crate) mod object_index;
mod scanner;
pub mod script_parser;
pub mod types;
pub mod watcher;

pub use db::AssetSearchRowDb;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rayon::prelude::*;
use rusqlite::Connection;

use crate::error::AppError;
use types::*;

const SCAN_PROGRESS_TARGET_EVENTS: u64 = 96;
const SCAN_PROGRESS_MIN_STEP: u64 = 32;
const SCAN_CANCELLED_MESSAGE: &str = "Asset database scan cancelled.";

fn ensure_scan_not_cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err(SCAN_CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

fn scan_progress_emit_step(total: u64) -> u64 {
    if total == 0 {
        1
    } else {
        (total / SCAN_PROGRESS_TARGET_EVENTS).max(SCAN_PROGRESS_MIN_STEP)
    }
}

fn maybe_emit_scan_progress<F, B>(
    on_progress: &F,
    emitted: &AtomicU64,
    total: u64,
    completed: u64,
    build_phase: B,
) where
    F: Fn(&ScanPhase) + Send + Sync,
    B: FnOnce(u64, u64) -> ScanPhase,
{
    if total == 0 {
        return;
    }

    let step = scan_progress_emit_step(total);
    let should_emit = completed == total || completed == 1 || completed % step == 0;
    if !should_emit {
        return;
    }

    let previous = emitted.fetch_max(completed, Ordering::Relaxed);
    if completed > previous {
        on_progress(&build_phase(completed, total));
    }
}

pub struct AssetDb {
    pub(crate) conn: Connection,
    project_root: PathBuf,
}

pub struct AssetDbState(pub Arc<Mutex<Option<AssetDb>>>);

pub enum LoadExistingAssetDb {
    Missing,
    Ready(AssetDb),
    NeedsRescan(AssetDbLoadIssue),
}

#[derive(Debug, Clone)]
pub struct AssetDbLoadIssue {
    pub code: &'static str,
    pub message: String,
    pub detail: Option<String>,
}

impl AssetDbLoadIssue {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn to_app_error(&self) -> AppError {
        let mut error = AppError::new(self.code, self.message.clone()).retryable(true);
        if let Some(detail) = &self.detail {
            error = error.detail(detail.clone());
        }
        error
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ParseFailureKind {
    InvalidMetaGuid,
    MissingMeta,
    ReadFailed,
}

impl ParseFailureKind {
    fn title(self) -> &'static str {
        match self {
            Self::InvalidMetaGuid => "Invalid or missing GUID in .meta",
            Self::MissingMeta => "Asset file without matching .meta",
            Self::ReadFailed => "Read failure",
        }
    }
}

#[derive(Debug, Clone)]
struct ParseFailureEntry {
    kind: ParseFailureKind,
    path: String,
    detail: String,
}

impl AssetDb {
    pub fn open(project_root: &Path) -> Result<Self, String> {
        let conn = db::open_db(project_root)?;
        Ok(Self {
            conn,
            project_root: project_root.to_path_buf(),
        })
    }

    pub fn load_existing(project_root: &Path) -> LoadExistingAssetDb {
        fn invalidate(
            db_path: &Path,
            code: &'static str,
            message: impl Into<String>,
            detail: impl Into<Option<String>>,
        ) -> LoadExistingAssetDb {
            db::delete_db_files(db_path);
            let mut issue = AssetDbLoadIssue::new(code, message);
            if let Some(detail) = detail.into() {
                issue = issue.detail(detail);
            }
            LoadExistingAssetDb::NeedsRescan(issue)
        }

        if project_root.as_os_str().is_empty() {
            return LoadExistingAssetDb::Missing;
        }

        let db_path = db::db_path(project_root);
        if !db_path.is_file() {
            return LoadExistingAssetDb::Missing;
        }

        let conn = match Connection::open(&db_path) {
            Ok(conn) => conn,
            Err(err) => {
                return invalidate(
                    &db_path,
                    "ref_graph.rescan_required.load_failed",
                    "Persisted asset database could not be loaded. Run a rescan to rebuild it.",
                    Some(err.to_string()),
                );
            }
        };

        if let Err(err) = db::configure_connection(&conn) {
            drop(conn);
            return invalidate(
                &db_path,
                "ref_graph.rescan_required.load_failed",
                "Persisted asset database could not be configured. Run a rescan to rebuild it.",
                Some(err),
            );
        }

        if let Err(err) = db::ensure_aux_tables(&conn) {
            drop(conn);
            return invalidate(
                &db_path,
                "ref_graph.rescan_required.load_failed",
                "Persisted asset database could not be prepared. Run a rescan to rebuild it.",
                Some(err),
            );
        }

        let user_version = match db::read_user_version(&conn) {
            Ok(version) => version,
            Err(err) => {
                drop(conn);
                return invalidate(
                    &db_path,
                    "ref_graph.rescan_required.load_failed",
                    "Persisted asset database metadata could not be read. Run a rescan to rebuild it.",
                    Some(err),
                );
            }
        };

        if user_version != db::ASSET_DB_VERSION {
            drop(conn);
            return invalidate(
                &db_path,
                "ref_graph.rescan_required.schema_mismatch",
                "Persisted asset database is out of date. Run a rescan to rebuild it.",
                Some(format!(
                    "Expected schema version {}, found {}",
                    db::ASSET_DB_VERSION,
                    user_version
                )),
            );
        }

        let (nodes, edges) = match db::get_stats(&conn) {
            Ok(stats) => stats,
            Err(err) => {
                drop(conn);
                return invalidate(
                    &db_path,
                    "ref_graph.rescan_required.probe_failed",
                    "Persisted asset database contents could not be verified. Run a rescan to rebuild it.",
                    Some(err),
                );
            }
        };

        let file_count = match db::get_file_count(&conn) {
            Ok(count) => count,
            Err(err) => {
                drop(conn);
                return invalidate(
                    &db_path,
                    "ref_graph.rescan_required.probe_failed",
                    "Persisted asset database contents could not be verified. Run a rescan to rebuild it.",
                    Some(err),
                );
            }
        };

        if nodes == 0 && edges == 0 && file_count == 0 {
            drop(conn);
            return invalidate(
                &db_path,
                "ref_graph.rescan_required.empty_db",
                "Persisted asset database is empty. Run a rescan to rebuild it.",
                None::<String>,
            );
        }

        LoadExistingAssetDb::Ready(Self {
            conn,
            project_root: project_root.to_path_buf(),
        })
    }

    pub fn full_scan<F>(&mut self, on_progress: F) -> Result<ScanStats, String>
    where
        F: Fn(&ScanPhase) + Send + Sync,
    {
        let cancel = AtomicBool::new(false);
        self.full_scan_with_cancel(on_progress, &cancel)
    }

    pub fn full_scan_with_cancel<F>(
        &mut self,
        on_progress: F,
        cancel: &AtomicBool,
    ) -> Result<ScanStats, String>
    where
        F: Fn(&ScanPhase) + Send + Sync,
    {
        let start = std::time::Instant::now();
        let mut stats = ScanStats::default();

        // Phase timing instrumentation. Each `phase_start` reset gives us one
        // wall-clock measurement we can compare across runs / projects to
        // decide which optimizations are worth pursuing next. Keep the print
        // format machine-greppable: `[AssetDb][timing] <phase>=<ms>ms ...`.
        let mut phase_start = std::time::Instant::now();

        ensure_scan_not_cancelled(cancel)?;
        on_progress(&ScanPhase::DirScan);
        let snapshot = scanner::scan_directory_with_cancel(&self.project_root, cancel);
        ensure_scan_not_cancelled(cancel)?;
        let t_dir_scan = phase_start.elapsed();
        stats.dirs_scanned = snapshot.dirs_scanned;
        stats.meta_files_found = snapshot.meta_files.len() as u64;
        stats.yaml_assets_found = snapshot.yaml_asset_files.len() as u64;

        eprintln!(
            "[AssetDb] snapshot: {} dirs, {} .meta, {} yaml assets, {} linked roots",
            stats.dirs_scanned,
            stats.meta_files_found,
            stats.yaml_assets_found,
            snapshot.linked_asset_roots.len()
        );
        eprintln!(
            "[AssetDb][timing] dir_scan={}ms ({} dirs, {} files)",
            t_dir_scan.as_millis(),
            stats.dirs_scanned,
            stats.meta_files_found + stats.yaml_assets_found
        );

        phase_start = std::time::Instant::now();
        ensure_scan_not_cancelled(cancel)?;
        on_progress(&ScanPhase::MetaParse {
            total: stats.meta_files_found,
            completed: 0,
        });
        let meta_progress = AtomicU64::new(0);
        let meta_progress_emitted = AtomicU64::new(0);
        let meta_outcomes: Vec<_> = snapshot
            .meta_files
            .par_iter()
            .map(|entry| {
                let outcome = (|| {
                    ensure_scan_not_cancelled(cancel).map_err(|detail| ParseFailureEntry {
                        kind: ParseFailureKind::ReadFailed,
                        path: entry.rel_path.clone(),
                        detail,
                    })?;
                    let content = match std::fs::read(&entry.abs_path) {
                        Ok(content) => content,
                        Err(err) => {
                            return Err(ParseFailureEntry {
                                kind: ParseFailureKind::ReadFailed,
                                path: entry.rel_path.clone(),
                                detail: format!("Failed to read {}: {}", entry.rel_path, err),
                            });
                        }
                    };
                    let guid = match meta_parser::extract_guid(&content) {
                        Some(g) => g,
                        None => {
                            return Err(ParseFailureEntry {
                                kind: ParseFailureKind::InvalidMetaGuid,
                                path: entry.rel_path.clone(),
                                detail: format!("No valid GUID in {}", entry.rel_path),
                            });
                        }
                    };
                    let meta_hash = hash128(&content);
                    let importer_subassets = object_index::parse_importer_subassets(&content);
                    Ok((entry.clone(), guid, meta_hash, importer_subassets))
                })();
                let completed = meta_progress.fetch_add(1, Ordering::Relaxed) + 1;
                maybe_emit_scan_progress(
                    &on_progress,
                    &meta_progress_emitted,
                    stats.meta_files_found,
                    completed,
                    |completed, total| ScanPhase::MetaParse { total, completed },
                );
                outcome
            })
            .collect();
        ensure_scan_not_cancelled(cancel)?;
        let t_meta_par = phase_start.elapsed();
        let mut parse_failures = Vec::new();
        let mut meta_results = Vec::with_capacity(meta_outcomes.len());
        let mut importer_subasset_results = Vec::new();
        for outcome in meta_outcomes {
            match outcome {
                Ok((entry, guid, meta_hash, importer_subassets)) => {
                    if !importer_subassets.is_empty() {
                        let asset_path = entry
                            .rel_path
                            .strip_suffix(".meta")
                            .unwrap_or(&entry.rel_path)
                            .to_string();
                        importer_subasset_results.push((asset_path, guid, importer_subassets));
                    }
                    meta_results.push((entry, guid, meta_hash));
                }
                Err(failure) => {
                    eprintln!("[AssetDb] warning: {}", failure.detail);
                    parse_failures.push(failure);
                }
            }
        }
        stats.duplicate_guids = build_duplicate_guid_overview(&meta_results);

        phase_start = std::time::Instant::now();
        let mut path_to_guid: HashMap<String, Guid> = HashMap::with_capacity(meta_results.len());
        let mut asset_nodes: Vec<AssetNode> = Vec::with_capacity(meta_results.len());
        let mut file_records: Vec<(String, FileRole, u64, u64, [u8; 16], Option<Guid>)> =
            Vec::with_capacity(meta_results.len() * 2);

        for (entry, guid, meta_hash) in &meta_results {
            let asset_path = entry
                .rel_path
                .strip_suffix(".meta")
                .unwrap_or(&entry.rel_path)
                .to_string();

            path_to_guid.insert(asset_path.clone(), *guid);

            let ext = Path::new(&asset_path)
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let asset_exists = self.project_root.join(&asset_path).exists();

            let initial_kind = match ext.as_str() {
                "cs" => AssetKind::Script,
                "png" | "jpg" | "jpeg" | "tga" | "psd" | "tif" | "tiff" | "bmp" | "gif" | "exr"
                | "hdr" => AssetKind::Texture,
                "wav" | "mp3" | "ogg" | "aif" | "aiff" => AssetKind::Audio,
                "shader" | "cginc" | "hlsl" | "glsl" | "compute" => AssetKind::Shader,
                "fbx" | "obj" | "blend" | "dae" | "3ds" | "max" => AssetKind::Model,
                _ => AssetKind::MetaOnly,
            };

            asset_nodes.push(AssetNode {
                guid: *guid,
                path: asset_path,
                ext,
                kind: initial_kind,
                exists_on_disk: asset_exists,
                mtime_ns: entry.mtime_ns,
                size: entry.size,
                content_hash: [0u8; 16],
                meta_hash: *meta_hash,
                parser_version: 1,
                script_class_name: None,
                script_class_lower: String::new(),
                script_namespace_lower: String::new(),
                script_full_name_lower: String::new(),
                script_type_search: String::new(),
                script_inheritance_search: String::new(),
            });

            file_records.push((
                entry.rel_path.clone(),
                FileRole::Meta,
                entry.mtime_ns,
                entry.size,
                *meta_hash,
                Some(*guid),
            ));
        }

        let t_meta_materialize = phase_start.elapsed();
        eprintln!(
            "[AssetDb] parsed {} .meta → {} GUIDs",
            stats.meta_files_found,
            path_to_guid.len()
        );
        eprintln!(
            "[AssetDb][timing] meta_par={}ms meta_materialize={}ms",
            t_meta_par.as_millis(),
            t_meta_materialize.as_millis()
        );
        if stats.duplicate_guids.group_count > 0 {
            eprintln!(
                "[AssetDb] duplicate GUIDs: {} groups, {} paths (assets={}, packages={}, cross-root={})",
                stats.duplicate_guids.group_count,
                stats.duplicate_guids.path_count,
                stats.duplicate_guids.assets_only_groups,
                stats.duplicate_guids.packages_only_groups,
                stats.duplicate_guids.cross_root_groups
            );
        }

        phase_start = std::time::Instant::now();
        ensure_scan_not_cancelled(cancel)?;
        on_progress(&ScanPhase::YamlParse {
            total: stats.yaml_assets_found,
            completed: 0,
        });

        let guid_to_path: HashMap<Guid, String> = path_to_guid
            .iter()
            .map(|(path, guid)| (*guid, path.clone()))
            .collect();

        let script_metadata_by_guid =
            script_parser::build_script_metadata_index(&self.project_root, &path_to_guid);
        let t_script_index = phase_start.elapsed();

        phase_start = std::time::Instant::now();
        let guid_to_node_idx: HashMap<Guid, usize> = asset_nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.guid, i))
            .collect();

        for (script_guid, script_meta) in &script_metadata_by_guid {
            let Some(&idx) = guid_to_node_idx.get(script_guid) else {
                continue;
            };
            let node = &mut asset_nodes[idx];
            node.content_hash = script_meta.content_hash;
            node.mtime_ns = node.mtime_ns.max(script_meta.mtime_ns);
            node.size = script_meta.size;
            node.script_class_name = Some(script_meta.class_name.clone());
            node.script_class_lower = script_meta.class_name_lower.clone();
            node.script_namespace_lower = script_meta.namespace_lower.clone();
            node.script_full_name_lower = script_meta.full_name_lower.clone();
            node.script_type_search = script_meta.type_search_lower.clone();
            node.script_inheritance_search = script_meta.inheritance_search_lower.clone();
        }
        let t_script_backfill = phase_start.elapsed();
        eprintln!(
            "[AssetDb][timing] script_index={}ms script_backfill={}ms ({} script metas)",
            t_script_index.as_millis(),
            t_script_backfill.as_millis(),
            script_metadata_by_guid.len()
        );

        phase_start = std::time::Instant::now();
        let yaml_progress = AtomicU64::new(0);
        let yaml_progress_emitted = AtomicU64::new(0);
        let yaml_outcomes: Vec<_> = snapshot
            .yaml_asset_files
            .par_iter()
            .map(|entry| {
                let outcome = (|| {
                    ensure_scan_not_cancelled(cancel).map_err(|detail| ParseFailureEntry {
                        kind: ParseFailureKind::ReadFailed,
                        path: entry.rel_path.clone(),
                        detail,
                    })?;
                    let src_guid = match path_to_guid.get(&entry.rel_path) {
                        Some(g) => *g,
                        None => {
                            return Err(ParseFailureEntry {
                                kind: ParseFailureKind::MissingMeta,
                                path: entry.rel_path.clone(),
                                detail: format!("Asset has no matching .meta: {}", entry.rel_path),
                            });
                        }
                    };

                    let content = match std::fs::read(&entry.abs_path) {
                        Ok(c) => c,
                        Err(e) => {
                            return Err(ParseFailureEntry {
                                kind: ParseFailureKind::ReadFailed,
                                path: entry.rel_path.clone(),
                                detail: format!("Failed to read {}: {}", entry.rel_path, e),
                            });
                        }
                    };

                    let refs = crate::unity_yaml::extract_refs_with_resolver(
                        &content,
                        Some(&guid_to_path),
                    );
                    let docs = crate::unity_yaml::parse_yaml_docs(&content);
                    let content_hash = hash128(&content);
                    let main_script_guid = if entry.ext.eq_ignore_ascii_case("asset") {
                        docs.iter()
                            .find(|doc| doc.doc_index == 0 && doc.class_id == 114)
                            .and_then(|doc| doc.m_script_guid)
                    } else {
                        None
                    };
                    Ok((
                        entry.clone(),
                        src_guid,
                        refs,
                        docs,
                        content_hash,
                        main_script_guid,
                    ))
                })();
                let completed = yaml_progress.fetch_add(1, Ordering::Relaxed) + 1;
                maybe_emit_scan_progress(
                    &on_progress,
                    &yaml_progress_emitted,
                    stats.yaml_assets_found,
                    completed,
                    |completed, total| ScanPhase::YamlParse { total, completed },
                );
                outcome
            })
            .collect();
        ensure_scan_not_cancelled(cancel)?;
        let t_yaml_par = phase_start.elapsed();
        let mut yaml_results = Vec::with_capacity(yaml_outcomes.len());
        for outcome in yaml_outcomes {
            match outcome {
                Ok(parsed) => yaml_results.push(parsed),
                Err(failure) => {
                    eprintln!("[AssetDb] warning: {}", failure.detail);
                    parse_failures.push(failure);
                }
            }
        }
        stats.parse_failures = parse_failures.len() as u64;

        phase_start = std::time::Instant::now();
        // Drain yaml_results by value so per-row `field_hint` / `ref_path` /
        // `entry.rel_path` strings move into edges/file_records directly
        // instead of being cloned. ~232k edges × 2 String clones removed.
        let yaml_count = yaml_results.len();
        let mut edges: Vec<RefEdge> = Vec::with_capacity(yaml_count * 16);
        let mut asset_objects: Vec<AssetObject> = Vec::new();

        for (entry, src_guid, extracted_refs, docs, content_hash, main_script_guid) in
            yaml_results.into_iter()
        {
            let mut updated_node: Option<AssetNode> = None;
            if let Some(&idx) = guid_to_node_idx.get(&src_guid) {
                let node = &mut asset_nodes[idx];
                node.kind = AssetKind::from_ext(&entry.ext);
                node.content_hash = content_hash;
                node.mtime_ns = entry.mtime_ns;
                node.size = entry.size;
                if node.kind == AssetKind::GenericAsset {
                    if let Some(script_guid) = main_script_guid {
                        if let Some(script_meta) = script_metadata_by_guid.get(&script_guid) {
                            if script_meta.inherits_scriptable_object {
                                node.script_class_name = Some(script_meta.class_name.clone());
                                node.script_class_lower = script_meta.class_name_lower.clone();
                                node.script_namespace_lower = script_meta.namespace_lower.clone();
                                node.script_full_name_lower = script_meta.full_name_lower.clone();
                                node.script_type_search = script_meta.type_search_lower.clone();
                                node.script_inheritance_search =
                                    script_meta.inheritance_search_lower.clone();
                            }
                        }
                    }
                }
                updated_node = Some(node.clone());
            }

            if let Some(node) = updated_node.as_ref() {
                asset_objects.extend(object_index::build_yaml_asset_objects(
                    node,
                    &docs,
                    |script_guid| {
                        script_metadata_by_guid.get(script_guid).map(|meta| {
                            object_index::ScriptTypeInfo {
                                class_name: meta.class_name.clone(),
                                class_name_lower: meta.class_name_lower.clone(),
                                full_name_lower: meta.full_name_lower.clone(),
                                type_search_lower: meta.type_search_lower.clone(),
                            }
                        })
                    },
                ));
            }

            for r in extracted_refs {
                edges.push(RefEdge {
                    src_guid,
                    src_file_id: r.src_file_id,
                    dst_guid: r.dst_guid,
                    dst_file_id: r.dst_file_id,
                    class_id_hint: r.class_id_hint,
                    field_hint: r.field_hint, // move
                    ref_path: r.ref_path,     // move
                });
            }

            file_records.push((
                entry.rel_path,
                FileRole::YamlAsset,
                entry.mtime_ns,
                entry.size,
                content_hash,
                Some(src_guid),
            ));
        }

        for (_asset_path, guid, importer_entries) in &importer_subasset_results {
            if let Some(&idx) = guid_to_node_idx.get(guid) {
                let node = &asset_nodes[idx];
                asset_objects.extend(object_index::build_importer_sub_asset_objects(
                    node,
                    importer_entries,
                ));
            }
        }

        let t_yaml_backfill = phase_start.elapsed();
        let raw_edge_count = edges.len();
        eprintln!(
            "[AssetDb] parsed {} yaml assets → {} edges, {} sub-asset objects",
            yaml_count,
            raw_edge_count,
            asset_objects.len()
        );
        eprintln!(
            "[AssetDb][timing] yaml_par={}ms yaml_backfill={}ms (avg {:.1} edges/yaml)",
            t_yaml_par.as_millis(),
            t_yaml_backfill.as_millis(),
            if yaml_count > 0 {
                raw_edge_count as f64 / yaml_count as f64
            } else {
                0.0
            }
        );

        phase_start = std::time::Instant::now();
        // H5: Sort edges by (src_guid, dst_guid) before batch insert. This
        // gives `idx_edges_src` near-sequential page writes (the index is
        // the dominant write-amp source for ~232k edges). The cost is one
        // ~50ms sort in exchange for hundreds of ms of B-tree page locality.
        // Sorting on (src, dst) instead of just src also makes the insert
        // order deterministic and lets a future composite index inherit
        // it for free.
        edges.sort_unstable_by(|a, b| {
            a.src_guid
                .cmp(&b.src_guid)
                .then_with(|| a.dst_guid.cmp(&b.dst_guid))
        });

        // Edges dedupe quantification: the `INSERT OR IGNORE` in
        // batch_insert_edges currently does nothing because the `edges`
        // table has no UNIQUE constraint. Drop adjacent duplicates after
        // sort (cheap window comparison, no HashSet needed) and log the
        // hit rate so we can decide whether the schema needs a real
        // UNIQUE constraint later. Two edges count as duplicates only if
        // every disambiguating field matches.
        let before_dedupe = edges.len();
        edges.dedup_by(|a, b| {
            a.src_guid == b.src_guid
                && a.src_file_id == b.src_file_id
                && a.dst_guid == b.dst_guid
                && a.dst_file_id == b.dst_file_id
                && a.class_id_hint == b.class_id_hint
                && a.field_hint == b.field_hint
                && a.ref_path == b.ref_path
        });
        let removed = before_dedupe - edges.len();
        let t_edge_prep = phase_start.elapsed();
        if before_dedupe > 0 {
            let pct = (removed as f64 / before_dedupe as f64) * 100.0;
            eprintln!(
                "[AssetDb] edges dedupe: {} → {} ({} removed, {:.2}%)",
                before_dedupe,
                edges.len(),
                removed,
                pct
            );
        }
        eprintln!(
            "[AssetDb][timing] edge_sort_dedupe={}ms",
            t_edge_prep.as_millis()
        );

        phase_start = std::time::Instant::now();
        ensure_scan_not_cancelled(cancel)?;
        on_progress(&ScanPhase::DbWrite);
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("Failed to begin transaction: {}", e))?;
        let t_tx_begin = phase_start.elapsed();

        // Wipe + reinsert in the same transaction so a mid-write crash leaves
        // the previous state intact rather than an empty DB.
        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        db::clear_all_in_tx(&tx)?;
        let t_clear = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        stats.nodes_added = db::batch_insert_assets(&tx, &asset_nodes)?;
        let t_assets = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        let objects_added = db::batch_insert_asset_objects(&tx, &asset_objects)?;
        let t_objects = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        db::batch_insert_files(&tx, &file_records)?;
        let t_files = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        db::set_scan_metrics(&tx, &stats.duplicate_guids, stats.parse_failures)?;
        let t_metrics = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        db::replace_linked_asset_roots(&tx, &snapshot.linked_asset_roots)?;
        let t_linked_roots = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        stats.edges_added = db::batch_insert_edges(&tx, &edges)?;
        let t_edges = t0.elapsed();

        ensure_scan_not_cancelled(cancel)?;
        let t0 = std::time::Instant::now();
        tx.commit()
            .map_err(|e| format!("Failed to commit: {}", e))?;
        let t_commit = t0.elapsed();

        let t0 = std::time::Instant::now();
        let duplicate_guid_report_path = match sync_duplicate_guid_report(
            &self.project_root,
            &meta_results,
            &stats.duplicate_guids,
        ) {
            Ok(path) => path,
            Err(err) => {
                eprintln!(
                    "[AssetDb] warning: failed to update duplicate GUID report: {}",
                    err
                );
                None
            }
        };
        let parse_failure_report_path =
            match sync_parse_failure_report(&self.project_root, &parse_failures) {
                Ok(path) => path,
                Err(err) => {
                    eprintln!(
                        "[AssetDb] warning: failed to update parse failure report: {}",
                        err
                    );
                    None
                }
            };
        let t_risk_reports = t0.elapsed();

        eprintln!(
            "[AssetDb][timing] db_write: tx_begin={}ms clear={}ms assets={}ms ({} rows) objects={}ms ({} sub rows) files={}ms ({} rows) metrics={}ms linked_roots={}ms ({} rows) edges={}ms ({} rows) commit={}ms",
            t_tx_begin.as_millis(),
            t_clear.as_millis(),
            t_assets.as_millis(),
            stats.nodes_added,
            t_objects.as_millis(),
            objects_added,
            t_files.as_millis(),
            file_records.len(),
            t_metrics.as_millis(),
            t_linked_roots.as_millis(),
            snapshot.linked_asset_roots.len(),
            t_edges.as_millis(),
            stats.edges_added,
            t_commit.as_millis()
        );

        if let Some(report_path) = &duplicate_guid_report_path {
            eprintln!(
                "[AssetDb] duplicate GUID report written to {}",
                report_path.display()
            );
        }
        if let Some(report_path) = &parse_failure_report_path {
            eprintln!(
                "[AssetDb] parse failure report written to {}",
                report_path.display()
            );
        }

        stats.elapsed_ms = start.elapsed().as_millis() as u64;

        eprintln!(
            "[AssetDb] scan complete: {} nodes, {} edges, {}ms",
            stats.nodes_added, stats.edges_added, stats.elapsed_ms
        );
        eprintln!(
            "[AssetDb][timing] TOTAL={}ms (dir_scan={}ms meta_par={}ms meta_mat={}ms script_idx={}ms script_bf={}ms yaml_par={}ms yaml_bf={}ms edge_prep={}ms db_write={}ms risk_reports={}ms)",
            stats.elapsed_ms,
            t_dir_scan.as_millis(),
            t_meta_par.as_millis(),
            t_meta_materialize.as_millis(),
            t_script_index.as_millis(),
            t_script_backfill.as_millis(),
            t_yaml_par.as_millis(),
            t_yaml_backfill.as_millis(),
            t_edge_prep.as_millis(),
            (t_clear + t_assets + t_objects + t_files + t_edges + t_commit + t_tx_begin).as_millis(),
            t_risk_reports.as_millis()
        );

        on_progress(&ScanPhase::Done {
            stats: stats.clone(),
        });

        Ok(stats)
    }

    pub fn get_direct_deps(&self, guid: &Guid) -> Result<Vec<RefEdge>, String> {
        db::get_direct_deps(&self.conn, guid)
    }

    pub fn get_direct_refs(&self, guid: &Guid) -> Result<Vec<RefEdge>, String> {
        db::get_direct_refs(&self.conn, guid)
    }

    pub fn get_direct_deps_for_object(
        &self,
        guid: &Guid,
        file_id: i64,
    ) -> Result<Vec<RefEdge>, String> {
        db::get_direct_deps_for_object(&self.conn, guid, file_id)
    }

    pub fn get_direct_refs_for_object(
        &self,
        guid: &Guid,
        file_id: i64,
    ) -> Result<Vec<RefEdge>, String> {
        db::get_direct_refs_for_object(&self.conn, guid, file_id)
    }

    pub fn resolve_guid_by_path(&self, path: &str) -> Result<Option<Guid>, String> {
        db::resolve_guid_by_path(&self.conn, path)
    }

    pub fn resolve_path_by_guid(&self, guid: &Guid) -> Result<Option<String>, String> {
        db::resolve_path_by_guid(&self.conn, guid)
    }

    pub fn resolve_path_and_kind_by_guid(
        &self,
        guid: &Guid,
    ) -> Result<Option<(String, AssetKind)>, String> {
        db::resolve_path_and_kind_by_guid(&self.conn, guid)
    }

    pub fn walk_deps(&self, root: &Guid, max_depth: u32) -> Result<Vec<Guid>, String> {
        db::walk_deps(&self.conn, root, max_depth)
    }

    pub fn walk_refs(&self, root: &Guid, max_depth: u32) -> Result<Vec<Guid>, String> {
        db::walk_refs(&self.conn, root, max_depth)
    }

    pub fn search_assets(
        &self,
        q: &str,
        fields: &[String],
        limit: u32,
        offset: u64,
    ) -> Result<db::SearchResult, String> {
        let predicates = db::parse_query(q)?;
        db::search_assets(&self.conn, &predicates, fields, limit, offset)
    }

    /// Asset-page free-text search. Pushes the parse + filter + rank entirely
    /// into SQLite (B-tree indexes for structured predicates, FTS5 trigram
    /// for substring matching). Returns rows already sorted by relevance.
    pub fn search_assets_for_command(
        &self,
        query: &str,
        roots: &[AssetRoot],
        limit: u32,
    ) -> Result<Vec<db::AssetSearchRowDb>, String> {
        db::search_assets_for_command(&self.conn, query, roots, limit)
    }

    pub fn find_asset_paths_referencing_script_terms(
        &self,
        lookup_terms: &[String],
    ) -> Result<Vec<String>, String> {
        let script_guids = db::find_script_guids_matching_terms(&self.conn, lookup_terms)?;
        db::find_asset_paths_referencing_any_guid(&self.conn, &script_guids)
    }

    pub fn get_stats(&self) -> Result<(u64, u64), String> {
        db::get_stats(&self.conn)
    }

    pub fn get_asset_size_bytes(&self) -> Result<u64, String> {
        db::get_asset_size_bytes(&self.conn)
    }

    /// Returns counts grouped by `AssetKind`. Variants with no rows are omitted.
    pub fn get_kind_counts(&self) -> Result<Vec<(AssetKind, u64)>, String> {
        let raw = db::get_kind_counts(&self.conn)?;
        Ok(raw
            .into_iter()
            .map(|(kind_i, count)| (AssetKind::from_i32(kind_i), count))
            .collect())
    }

    pub fn get_duplicate_guid_overview(&self) -> Result<DuplicateGuidOverview, String> {
        db::get_duplicate_guid_overview(&self.conn)
    }

    pub fn get_parse_failure_count(&self) -> Result<u64, String> {
        db::get_parse_failure_count(&self.conn)
    }

    pub fn get_asset_risks(&self) -> Result<Vec<AssetRiskEntry>, String> {
        let mut out = Vec::new();
        let missing_ref_counts = db::get_missing_reference_counts(&self.conn)?;
        let parse_failures = db::get_parse_failure_count(&self.conn)?;
        let duplicate_guids = db::get_duplicate_guid_overview(&self.conn)?;

        if missing_ref_counts.broken_references > 0 {
            out.push(AssetRiskEntry {
                kind: AssetRiskKind::BrokenReferences,
                count: missing_ref_counts.broken_references,
            });
        }
        if missing_ref_counts.missing_scripts > 0 {
            out.push(AssetRiskEntry {
                kind: AssetRiskKind::MissingScripts,
                count: missing_ref_counts.missing_scripts,
            });
        }
        if parse_failures > 0 {
            out.push(AssetRiskEntry {
                kind: AssetRiskKind::ParseFailures,
                count: parse_failures,
            });
        }
        if duplicate_guids.group_count > 0 {
            out.push(AssetRiskEntry {
                kind: AssetRiskKind::DuplicateGuids,
                count: duplicate_guids.group_count,
            });
        }

        Ok(out)
    }

    pub fn build_missing_reference_report(
        &self,
        missing_scripts_only: bool,
    ) -> Result<Option<PathBuf>, String> {
        let rows = db::get_missing_reference_rows(&self.conn, missing_scripts_only)?;
        sync_missing_reference_report(&self.project_root, missing_scripts_only, &rows)
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) fn linked_asset_roots(&self) -> Result<Vec<LinkedAssetRoot>, String> {
        db::get_linked_asset_roots(&self.conn)
    }

    pub fn batch_resolve_paths(
        &self,
        guids: &[types::Guid],
    ) -> Result<HashMap<types::Guid, String>, String> {
        db::batch_resolve_paths(&self.conn, guids)
    }
}

fn build_duplicate_guid_overview(
    meta_results: &[(scanner::FileEntry, Guid, [u8; 16])],
) -> DuplicateGuidOverview {
    let mut by_guid: HashMap<Guid, (u32, u8)> = HashMap::new();

    for (entry, guid, _) in meta_results {
        let asset_path = entry
            .rel_path
            .strip_suffix(".meta")
            .unwrap_or(&entry.rel_path);
        let root_bit = duplicate_guid_root_bit(asset_path);
        let slot = by_guid.entry(*guid).or_insert((0, 0));
        slot.0 += 1;
        slot.1 |= root_bit;
    }

    let mut out = DuplicateGuidOverview::default();
    for (_guid, (count, roots)) in by_guid {
        if count < 2 {
            continue;
        }
        out.group_count += 1;
        out.path_count += count as u64;
        match roots {
            0b01 => out.assets_only_groups += 1,
            0b10 => out.packages_only_groups += 1,
            _ => out.cross_root_groups += 1,
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DuplicateGuidReportSection {
    DifferentNameDifferentHash,
    Mixed,
    SameNameSameHash,
}

impl DuplicateGuidReportSection {
    fn title(self) -> &'static str {
        match self {
            Self::DifferentNameDifferentHash => "Different filename + different hash (higher risk)",
            Self::Mixed => "Mixed / requires manual review",
            Self::SameNameSameHash => "Same filename + same hash (lower risk)",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::DifferentNameDifferentHash => {
                "Conflicting assets do not look like identical copies and should be investigated first."
            }
            Self::Mixed => {
                "Some paths share only part of the signal, or the content hash could not be read."
            }
            Self::SameNameSameHash => {
                "Likely copied or vendored duplicates. Still worth cleaning up, but usually less urgent."
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DuplicateGuidComparison {
    Same,
    Different,
    Unavailable,
}

impl DuplicateGuidComparison {
    fn label(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Different => "different",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone)]
struct DuplicateGuidReportAsset {
    asset_path: String,
    file_name: String,
    content_hash: Option<String>,
    content_hash_display: String,
}

#[derive(Debug, Clone)]
struct DuplicateGuidReportEntry {
    guid_hex: String,
    assets: Vec<DuplicateGuidReportAsset>,
    root_bits: u8,
    file_name_comparison: DuplicateGuidComparison,
    content_hash_comparison: DuplicateGuidComparison,
    section: DuplicateGuidReportSection,
}

fn duplicate_guid_root_bit(asset_path: &str) -> u8 {
    match AssetRoot::from_rel_path(asset_path) {
        AssetRoot::Assets => 0b01,
        AssetRoot::Packages => 0b10,
        _ => 0b100,
    }
}

fn duplicate_guid_scope_label(root_bits: u8) -> &'static str {
    match root_bits {
        0b01 => "assets-only",
        0b10 => "packages-only",
        _ => "cross-root",
    }
}

fn duplicate_guid_report_path(project_root: &Path) -> PathBuf {
    project_root
        .join("Temp")
        .join("Locus")
        .join("duplicate-guid-report.txt")
}

fn duplicate_guid_asset_rel_path(entry: &scanner::FileEntry) -> String {
    entry
        .rel_path
        .strip_suffix(".meta")
        .unwrap_or(&entry.rel_path)
        .to_string()
}

fn duplicate_guid_asset_abs_path(entry: &scanner::FileEntry) -> PathBuf {
    entry.abs_path.with_extension("")
}

fn hash_file128(path: &Path) -> Result<[u8; 16], String> {
    let mut file = std::fs::File::open(path)
        .map_err(|err| format!("Failed to open {}: {}", path.display(), err))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("Failed to read {}: {}", path.display(), err))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    Ok(out)
}

fn build_duplicate_guid_report_asset(entry: &scanner::FileEntry) -> DuplicateGuidReportAsset {
    let asset_path = duplicate_guid_asset_rel_path(entry);
    let file_name = Path::new(&asset_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&asset_path)
        .to_string();
    let asset_abs_path = duplicate_guid_asset_abs_path(entry);

    let (content_hash, content_hash_display) = match std::fs::metadata(&asset_abs_path) {
        Ok(metadata) if metadata.is_file() => match hash_file128(&asset_abs_path) {
            Ok(hash) => {
                let hash_hex = guid_to_hex(&hash);
                (Some(hash_hex.clone()), hash_hex)
            }
            Err(err) => (None, format!("<read-error: {}>", err)),
        },
        Ok(metadata) if metadata.is_dir() => (None, "<directory>".to_string()),
        Ok(_) => (None, "<unsupported>".to_string()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (None, "<missing>".to_string()),
        Err(err) => (None, format!("<metadata-error: {}>", err)),
    };

    DuplicateGuidReportAsset {
        asset_path,
        file_name,
        content_hash,
        content_hash_display,
    }
}

fn classify_duplicate_guid_file_names(
    assets: &[DuplicateGuidReportAsset],
) -> DuplicateGuidComparison {
    let unique_names: BTreeSet<&str> = assets
        .iter()
        .map(|asset| asset.file_name.as_str())
        .collect();
    if unique_names.is_empty() {
        DuplicateGuidComparison::Unavailable
    } else if unique_names.len() == 1 {
        DuplicateGuidComparison::Same
    } else {
        DuplicateGuidComparison::Different
    }
}

fn classify_duplicate_guid_content_hashes(
    assets: &[DuplicateGuidReportAsset],
) -> DuplicateGuidComparison {
    if assets.iter().any(|asset| asset.content_hash.is_none()) {
        return DuplicateGuidComparison::Unavailable;
    }

    let unique_hashes: BTreeSet<&str> = assets
        .iter()
        .filter_map(|asset| asset.content_hash.as_deref())
        .collect();
    if unique_hashes.is_empty() {
        DuplicateGuidComparison::Unavailable
    } else if unique_hashes.len() == 1 {
        DuplicateGuidComparison::Same
    } else {
        DuplicateGuidComparison::Different
    }
}

fn classify_duplicate_guid_report_section(
    file_name_comparison: DuplicateGuidComparison,
    content_hash_comparison: DuplicateGuidComparison,
) -> DuplicateGuidReportSection {
    match (file_name_comparison, content_hash_comparison) {
        (DuplicateGuidComparison::Same, DuplicateGuidComparison::Same) => {
            DuplicateGuidReportSection::SameNameSameHash
        }
        (DuplicateGuidComparison::Different, DuplicateGuidComparison::Different) => {
            DuplicateGuidReportSection::DifferentNameDifferentHash
        }
        _ => DuplicateGuidReportSection::Mixed,
    }
}

fn build_duplicate_guid_report_entries(
    meta_results: &[(scanner::FileEntry, Guid, [u8; 16])],
) -> Vec<DuplicateGuidReportEntry> {
    let mut by_guid: BTreeMap<String, (Vec<&scanner::FileEntry>, u8)> = BTreeMap::new();

    for (entry, guid, _) in meta_results {
        let slot = by_guid
            .entry(guid_to_hex(guid))
            .or_insert_with(|| (Vec::new(), 0));
        slot.0.push(entry);
        slot.1 |= duplicate_guid_root_bit(&duplicate_guid_asset_rel_path(entry));
    }

    let mut out = Vec::new();
    for (guid_hex, (entries, root_bits)) in by_guid {
        if entries.len() < 2 {
            continue;
        }
        let mut assets: Vec<_> = entries
            .into_iter()
            .map(build_duplicate_guid_report_asset)
            .collect();
        assets.sort_unstable_by(|a, b| a.asset_path.cmp(&b.asset_path));
        let file_name_comparison = classify_duplicate_guid_file_names(&assets);
        let content_hash_comparison = classify_duplicate_guid_content_hashes(&assets);
        out.push(DuplicateGuidReportEntry {
            guid_hex,
            assets,
            root_bits,
            file_name_comparison,
            content_hash_comparison,
            section: classify_duplicate_guid_report_section(
                file_name_comparison,
                content_hash_comparison,
            ),
        });
    }

    out.sort_unstable_by(|a, b| {
        a.section
            .cmp(&b.section)
            .then_with(|| b.assets.len().cmp(&a.assets.len()))
            .then_with(|| a.guid_hex.cmp(&b.guid_hex))
    });
    out
}

fn count_duplicate_guid_report_section(
    entries: &[DuplicateGuidReportEntry],
    section: DuplicateGuidReportSection,
) -> usize {
    entries
        .iter()
        .filter(|entry| entry.section == section)
        .count()
}

fn render_duplicate_guid_report_section(
    report: &mut String,
    entries: &[DuplicateGuidReportEntry],
    section: DuplicateGuidReportSection,
) {
    let section_entries: Vec<_> = entries
        .iter()
        .filter(|entry| entry.section == section)
        .collect();
    if section_entries.is_empty() {
        return;
    }

    let _ = writeln!(report);
    let _ = writeln!(report, "== {} ==", section.title());
    let _ = writeln!(report, "group_count: {}", section_entries.len());
    let _ = writeln!(report, "note: {}", section.note());

    for entry in section_entries {
        let _ = writeln!(report);
        let _ = writeln!(report, "duplicate_guid: {}", entry.guid_hex);
        let _ = writeln!(
            report,
            "scope: {}",
            duplicate_guid_scope_label(entry.root_bits)
        );
        let _ = writeln!(report, "asset_count: {}", entry.assets.len());
        let _ = writeln!(
            report,
            "file_name_match: {}",
            entry.file_name_comparison.label()
        );
        let _ = writeln!(
            report,
            "content_hash_match: {}",
            entry.content_hash_comparison.label()
        );
        for asset in &entry.assets {
            let _ = writeln!(
                report,
                "- {} | file_name={} | content_hash={}",
                asset.asset_path, asset.file_name, asset.content_hash_display
            );
        }
    }
}

fn render_duplicate_guid_report(
    project_root: &Path,
    entries: &[DuplicateGuidReportEntry],
    overview: &DuplicateGuidOverview,
) -> String {
    let mut report = String::new();
    let _ = writeln!(&mut report, "Duplicate GUID Report");
    let _ = writeln!(
        &mut report,
        "generated_at: {}",
        chrono::Local::now().to_rfc3339()
    );
    let _ = writeln!(&mut report, "project_root: {}", project_root.display());
    let _ = writeln!(
        &mut report,
        "duplicate_guid_groups: {}",
        overview.group_count
    );
    let _ = writeln!(
        &mut report,
        "duplicate_asset_paths: {}",
        overview.path_count
    );
    let _ = writeln!(
        &mut report,
        "assets_only_groups: {}",
        overview.assets_only_groups
    );
    let _ = writeln!(
        &mut report,
        "packages_only_groups: {}",
        overview.packages_only_groups
    );
    let _ = writeln!(
        &mut report,
        "cross_root_groups: {}",
        overview.cross_root_groups
    );
    let _ = writeln!(
        &mut report,
        "different_name_different_hash_groups: {}",
        count_duplicate_guid_report_section(
            entries,
            DuplicateGuidReportSection::DifferentNameDifferentHash
        )
    );
    let _ = writeln!(
        &mut report,
        "mixed_groups: {}",
        count_duplicate_guid_report_section(entries, DuplicateGuidReportSection::Mixed)
    );
    let _ = writeln!(
        &mut report,
        "same_name_same_hash_groups: {}",
        count_duplicate_guid_report_section(entries, DuplicateGuidReportSection::SameNameSameHash)
    );

    render_duplicate_guid_report_section(
        &mut report,
        entries,
        DuplicateGuidReportSection::DifferentNameDifferentHash,
    );
    render_duplicate_guid_report_section(&mut report, entries, DuplicateGuidReportSection::Mixed);
    render_duplicate_guid_report_section(
        &mut report,
        entries,
        DuplicateGuidReportSection::SameNameSameHash,
    );

    report
}

fn sync_duplicate_guid_report(
    project_root: &Path,
    meta_results: &[(scanner::FileEntry, Guid, [u8; 16])],
    overview: &DuplicateGuidOverview,
) -> Result<Option<PathBuf>, String> {
    let report_path = duplicate_guid_report_path(project_root);
    if overview.group_count == 0 {
        match std::fs::remove_file(&report_path) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "Failed to remove stale duplicate GUID report at {}: {}",
                    report_path.display(),
                    err
                ));
            }
        }
        return Ok(None);
    }

    let entries = build_duplicate_guid_report_entries(meta_results);
    if entries.is_empty() {
        return Ok(None);
    }

    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Failed to create duplicate GUID report directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }

    let report = render_duplicate_guid_report(project_root, &entries, overview);
    std::fs::write(&report_path, report).map_err(|err| {
        format!(
            "Failed to write duplicate GUID report at {}: {}",
            report_path.display(),
            err
        )
    })?;

    Ok(Some(report_path))
}

fn parse_failure_report_path(project_root: &Path) -> PathBuf {
    project_root
        .join("Temp")
        .join("Locus")
        .join("parse-failures-report.txt")
}

fn render_parse_failure_report(project_root: &Path, failures: &[ParseFailureEntry]) -> String {
    let mut report = String::new();
    let _ = writeln!(&mut report, "Asset Parse Failure Report");
    let _ = writeln!(
        &mut report,
        "generated_at: {}",
        chrono::Local::now().to_rfc3339()
    );
    let _ = writeln!(&mut report, "project_root: {}", project_root.display());
    let _ = writeln!(&mut report, "failure_count: {}", failures.len());

    for kind in [
        ParseFailureKind::InvalidMetaGuid,
        ParseFailureKind::MissingMeta,
        ParseFailureKind::ReadFailed,
    ] {
        let section: Vec<_> = failures.iter().filter(|entry| entry.kind == kind).collect();
        if section.is_empty() {
            continue;
        }
        let _ = writeln!(&mut report);
        let _ = writeln!(&mut report, "== {} ==", kind.title());
        let _ = writeln!(&mut report, "count: {}", section.len());
        for entry in section {
            let _ = writeln!(&mut report, "- {}", entry.path);
            let _ = writeln!(&mut report, "  detail: {}", entry.detail);
        }
    }

    report
}

fn sync_parse_failure_report(
    project_root: &Path,
    failures: &[ParseFailureEntry],
) -> Result<Option<PathBuf>, String> {
    let report_path = parse_failure_report_path(project_root);
    if failures.is_empty() {
        match std::fs::remove_file(&report_path) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "Failed to remove stale parse failure report at {}: {}",
                    report_path.display(),
                    err
                ));
            }
        }
        return Ok(None);
    }

    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Failed to create parse failure report directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }

    let report = render_parse_failure_report(project_root, failures);
    std::fs::write(&report_path, report).map_err(|err| {
        format!(
            "Failed to write parse failure report at {}: {}",
            report_path.display(),
            err
        )
    })?;

    Ok(Some(report_path))
}

fn missing_reference_report_path(project_root: &Path, missing_scripts_only: bool) -> PathBuf {
    project_root
        .join("Temp")
        .join("Locus")
        .join(if missing_scripts_only {
            "missing-scripts-report.txt"
        } else {
            "broken-references-report.txt"
        })
}

fn render_missing_reference_report(
    project_root: &Path,
    missing_scripts_only: bool,
    rows: &[db::MissingReferenceRow],
) -> String {
    let mut report = String::new();
    let title = if missing_scripts_only {
        "Missing Script Report"
    } else {
        "Broken Reference Report"
    };
    let _ = writeln!(&mut report, "{}", title);
    let _ = writeln!(
        &mut report,
        "generated_at: {}",
        chrono::Local::now().to_rfc3339()
    );
    let _ = writeln!(&mut report, "project_root: {}", project_root.display());
    let _ = writeln!(&mut report, "reference_count: {}", rows.len());

    let affected_assets: BTreeSet<&str> = rows.iter().map(|row| row.src_path.as_str()).collect();
    let _ = writeln!(&mut report, "affected_assets: {}", affected_assets.len());

    let mut grouped: BTreeMap<&str, Vec<&db::MissingReferenceRow>> = BTreeMap::new();
    for row in rows {
        grouped.entry(row.src_path.as_str()).or_default().push(row);
    }

    for (src_path, group) in grouped {
        let _ = writeln!(&mut report);
        let _ = writeln!(&mut report, "asset: {}", src_path);
        let _ = writeln!(&mut report, "kind: {}", group[0].src_kind.camel_str());
        let _ = writeln!(&mut report, "count: {}", group.len());
        for row in group {
            let path_hint = row
                .ref_path
                .as_deref()
                .or(row.field_hint.as_deref())
                .unwrap_or("?");
            let _ = writeln!(
                &mut report,
                "- {} -> {}",
                path_hint,
                guid_to_hex(&row.dst_guid)
            );
        }
    }

    report
}

fn sync_missing_reference_report(
    project_root: &Path,
    missing_scripts_only: bool,
    rows: &[db::MissingReferenceRow],
) -> Result<Option<PathBuf>, String> {
    let report_path = missing_reference_report_path(project_root, missing_scripts_only);
    if rows.is_empty() {
        match std::fs::remove_file(&report_path) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "Failed to remove stale missing reference report at {}: {}",
                    report_path.display(),
                    err
                ));
            }
        }
        return Ok(None);
    }

    if let Some(parent) = report_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Failed to create missing reference report directory {}: {}",
                parent.display(),
                err
            )
        })?;
    }

    let report = render_missing_reference_report(project_root, missing_scripts_only, rows);
    std::fs::write(&report_path, report).map_err(|err| {
        format!(
            "Failed to write missing reference report at {}: {}",
            report_path.display(),
            err
        )
    })?;

    Ok(Some(report_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path as StdPath;
    use uuid::Uuid;

    #[cfg(unix)]
    fn create_dir_symlink(source: &StdPath, link: &StdPath) -> std::io::Result<()> {
        std::os::unix::fs::symlink(source, link)
    }

    #[cfg(windows)]
    fn create_dir_symlink(source: &StdPath, link: &StdPath) -> std::io::Result<()> {
        std::os::windows::fs::symlink_dir(source, link)
    }

    fn create_dir_symlink_or_skip(source: &StdPath, link: &StdPath) -> bool {
        match create_dir_symlink(source, link) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("skipping symlink test; failed to create directory symlink: {error}");
                false
            }
        }
    }

    fn temp_project_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("locus-assetdb-load-{}", Uuid::new_v4()));
        std::fs::create_dir_all(root.join("Assets")).expect("create temp Unity project");
        root
    }

    fn test_file_entry(rel_path: &str) -> scanner::FileEntry {
        scanner::FileEntry {
            rel_path: rel_path.to_string(),
            abs_path: PathBuf::from(rel_path),
            ext: "meta".to_string(),
            mtime_ns: 1,
            size: 1,
        }
    }

    fn write_test_asset_with_meta(
        root: &Path,
        rel_asset_path: &str,
        content: &[u8],
    ) -> scanner::FileEntry {
        let asset_abs_path = root.join(rel_asset_path);
        if let Some(parent) = asset_abs_path.parent() {
            std::fs::create_dir_all(parent).expect("create asset parent directory");
        }
        std::fs::write(&asset_abs_path, content).expect("write asset file");

        let rel_meta_path = format!("{}.meta", rel_asset_path);
        let meta_abs_path = root.join(&rel_meta_path);
        std::fs::write(&meta_abs_path, b"fileFormatVersion: 2\n").expect("write meta file");

        let meta_metadata = std::fs::metadata(&meta_abs_path).expect("read meta metadata");
        scanner::FileEntry {
            rel_path: rel_meta_path.replace('\\', "/"),
            abs_path: meta_abs_path,
            ext: "meta".to_string(),
            mtime_ns: scanner::get_mtime_ns(&meta_metadata),
            size: meta_metadata.len(),
        }
    }

    #[test]
    fn duplicate_guid_overview_splits_by_root() {
        let guid_assets = parse_guid_hex("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let guid_packages = parse_guid_hex("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let guid_cross = parse_guid_hex("cccccccccccccccccccccccccccccccc").unwrap();

        let meta_results = vec![
            (
                test_file_entry("Assets/Foo/A.prefab.meta"),
                guid_assets,
                [0u8; 16],
            ),
            (
                test_file_entry("Assets/Bar/A.prefab.meta"),
                guid_assets,
                [0u8; 16],
            ),
            (
                test_file_entry("Packages/pkg.one/B.prefab.meta"),
                guid_packages,
                [0u8; 16],
            ),
            (
                test_file_entry("Packages/pkg.two/B.prefab.meta"),
                guid_packages,
                [0u8; 16],
            ),
            (
                test_file_entry("Assets/Baz/C.prefab.meta"),
                guid_cross,
                [0u8; 16],
            ),
            (
                test_file_entry("Packages/pkg.three/C.prefab.meta"),
                guid_cross,
                [0u8; 16],
            ),
        ];

        let overview = build_duplicate_guid_overview(&meta_results);
        assert_eq!(overview.group_count, 3);
        assert_eq!(overview.path_count, 6);
        assert_eq!(overview.assets_only_groups, 1);
        assert_eq!(overview.packages_only_groups, 1);
        assert_eq!(overview.cross_root_groups, 1);
    }

    #[test]
    fn full_scan_indexes_assets_inside_symlinked_asset_folders() {
        let temp = tempfile::tempdir().expect("create temp dir");
        let root = temp.path().join("project");
        let external = temp.path().join("shared-assets");
        std::fs::create_dir_all(root.join("Assets")).expect("create assets dir");
        std::fs::create_dir_all(&external).expect("create external target");
        std::fs::write(
            external.join("Hero.prefab"),
            b"%YAML 1.1\n--- !u!1 &1000\nGameObject:\n  m_Name: Hero\n",
        )
        .expect("write linked prefab");
        std::fs::write(
            external.join("Hero.prefab.meta"),
            b"fileFormatVersion: 2\nguid: 11111111111111111111111111111111\n",
        )
        .expect("write linked prefab meta");

        if !create_dir_symlink_or_skip(&external, &root.join("Assets/Linked")) {
            return;
        }

        let mut graph = AssetDb::open(&root).expect("open asset db");
        let stats = graph.full_scan(|_| {}).expect("scan asset db");
        assert_eq!(stats.meta_files_found, 1);
        assert_eq!(stats.yaml_assets_found, 1);
        assert_eq!(
            graph
                .resolve_guid_by_path("Assets/Linked/Hero.prefab")
                .expect("resolve linked asset path"),
            Some(parse_guid_hex("11111111111111111111111111111111").unwrap())
        );

        let linked_roots = graph
            .linked_asset_roots()
            .expect("load cached linked asset roots");
        assert_eq!(linked_roots.len(), 1);
        assert_eq!(linked_roots[0].link_rel_path, "Assets/Linked");
        assert_eq!(
            linked_roots[0].target_path,
            dunce::canonicalize(&external).expect("canonical linked target")
        );
    }

    #[test]
    fn duplicate_guid_report_is_written_to_temp_locus_and_cleared_when_resolved() {
        let root = temp_project_root();
        let guid = parse_guid_hex("dddddddddddddddddddddddddddddddd").unwrap();
        let meta_results = vec![
            (
                write_test_asset_with_meta(&root, "Assets/Foo/Shared.prefab", b"same-bytes"),
                guid,
                [0u8; 16],
            ),
            (
                write_test_asset_with_meta(&root, "Packages/com.demo/Shared.prefab", b"same-bytes"),
                guid,
                [0u8; 16],
            ),
        ];

        let overview = build_duplicate_guid_overview(&meta_results);
        let report_path = sync_duplicate_guid_report(&root, &meta_results, &overview)
            .expect("write duplicate guid report")
            .expect("report path should be returned");

        assert_eq!(report_path, duplicate_guid_report_path(&root));
        let report = std::fs::read_to_string(&report_path).expect("read duplicate guid report");
        assert!(report.contains("same_name_same_hash_groups: 1"));
        assert!(report.contains("== Same filename + same hash (lower risk) =="));
        assert!(report.contains("duplicate_guid: dddddddddddddddddddddddddddddddd"));
        assert!(report.contains("scope: cross-root"));
        assert!(report.contains("file_name_match: same"));
        assert!(report.contains("content_hash_match: same"));
        assert!(
            report.contains("- Assets/Foo/Shared.prefab | file_name=Shared.prefab | content_hash=")
        );
        assert!(report.contains(
            "- Packages/com.demo/Shared.prefab | file_name=Shared.prefab | content_hash="
        ));

        let resolved_results = vec![(
            write_test_asset_with_meta(&root, "Assets/Foo/Shared.prefab", b"same-bytes"),
            guid,
            [0u8; 16],
        )];
        let resolved_overview = build_duplicate_guid_overview(&resolved_results);
        let cleared = sync_duplicate_guid_report(&root, &resolved_results, &resolved_overview)
            .expect("clear duplicate guid report");
        assert!(cleared.is_none());
        assert!(!report_path.exists(), "stale report should be removed");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_guid_report_separates_high_risk_and_low_risk_sections() {
        let root = temp_project_root();
        let low_risk_guid = parse_guid_hex("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee").unwrap();
        let high_risk_guid = parse_guid_hex("ffffffffffffffffffffffffffffffff").unwrap();
        let meta_results = vec![
            (
                write_test_asset_with_meta(&root, "Assets/Foo/Shared.prefab", b"same-copy"),
                low_risk_guid,
                [0u8; 16],
            ),
            (
                write_test_asset_with_meta(&root, "Packages/com.demo/Shared.prefab", b"same-copy"),
                low_risk_guid,
                [0u8; 16],
            ),
            (
                write_test_asset_with_meta(&root, "Assets/Bar/Alpha.prefab", b"alpha-bytes"),
                high_risk_guid,
                [0u8; 16],
            ),
            (
                write_test_asset_with_meta(&root, "Packages/com.demo/Beta.prefab", b"beta-bytes"),
                high_risk_guid,
                [0u8; 16],
            ),
        ];

        let overview = build_duplicate_guid_overview(&meta_results);
        let report_path = sync_duplicate_guid_report(&root, &meta_results, &overview)
            .expect("write duplicate guid report")
            .expect("report path should be returned");
        let report = std::fs::read_to_string(&report_path).expect("read duplicate guid report");

        assert!(report.contains("different_name_different_hash_groups: 1"));
        assert!(report.contains("same_name_same_hash_groups: 1"));
        assert!(report.contains("== Different filename + different hash (higher risk) =="));
        assert!(report.contains("== Same filename + same hash (lower risk) =="));
        assert!(report.contains("duplicate_guid: ffffffffffffffffffffffffffffffff"));
        assert!(report.contains("file_name_match: different"));
        assert!(report.contains("content_hash_match: different"));
        assert!(
            report.contains("- Assets/Bar/Alpha.prefab | file_name=Alpha.prefab | content_hash=")
        );
        assert!(report
            .contains("- Packages/com.demo/Beta.prefab | file_name=Beta.prefab | content_hash="));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_existing_blank_db_requires_rescan() {
        let root = temp_project_root();
        let db_path = db::db_path(&root);
        let conn = db::open_db(&root).expect("create blank asset db");
        drop(conn);

        match AssetDb::load_existing(&root) {
            LoadExistingAssetDb::NeedsRescan(issue) => {
                assert_eq!(issue.code, "ref_graph.rescan_required.empty_db");
            }
            _ => panic!("blank db should require a rescan"),
        }

        assert!(
            !db_path.exists(),
            "blank db should be deleted after invalidation"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_existing_schema_mismatch_requires_rescan() {
        let root = temp_project_root();
        let db_path = db::db_path(&root);
        let conn = db::open_db(&root).expect("create asset db");
        conn.execute_batch("PRAGMA user_version = 0")
            .expect("downgrade schema version");
        drop(conn);

        match AssetDb::load_existing(&root) {
            LoadExistingAssetDb::NeedsRescan(issue) => {
                assert_eq!(issue.code, "ref_graph.rescan_required.schema_mismatch");
            }
            _ => panic!("schema mismatch should require a rescan"),
        }

        assert!(
            !db_path.exists(),
            "mismatched db should be deleted after invalidation"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
