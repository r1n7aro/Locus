//! Sparse file-mode placement overrides, scoped to one physical checkout.
//! Like the workspace tree they are per-user layout state, kept in the data
//! directory so reordering files never changes the project.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const DIR_NAME: &str = "workspace-file-order";
/// Older versions kept the order in `<checkout>/Locus/workspace-file-order.json`.
const LEGACY_FILE_NAME: &str = "workspace-file-order.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlacementSide {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FilePlacement {
    pub directory: String,
    pub item: String,
    pub anchor: String,
    pub side: PlacementSide,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOrder {
    schema_version: u32,
    pub revision: i64,
    pub placements: Vec<FilePlacement>,
}

impl Default for FileOrder {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            placements: Vec::new(),
        }
    }
}

fn order_lock(root: &Path) -> Result<Arc<Mutex<()>>, String> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .map_err(|e| e.to_string())?;
    Ok(Arc::clone(locks.entry(root).or_default()))
}

/// `<data dir>/workspace-file-order/<checkout id>.json`
fn path(data_dir: &Path, root: &Path) -> Result<PathBuf, String> {
    let root = crate::workspace_service::identity::normalize_existing_workspace_root(root)
        .map_err(|e| e.to_string())?;
    let checkout = crate::workspace_service::CheckoutId::from_normalized_root(&root);
    Ok(data_dir
        .join(DIR_NAME)
        .join(format!("{}.json", checkout.as_str())))
}

fn data_dir() -> Result<&'static Path, String> {
    crate::runtime_paths::active_data_dir()
}

fn relative(value: &str) -> Result<String, String> {
    let value = value.replace('\\', "/");
    if value.starts_with('/')
        || value.contains(':')
        || value.chars().any(char::is_control)
        || value.split('/').any(|part| part == "..")
    {
        return Err("File order paths must stay within the workspace".into());
    }
    let parts: Vec<_> = value
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    Ok(if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    })
}

fn key(value: &str, directory: &str) -> Result<String, String> {
    if let Some(value) = value.strip_prefix("file:") {
        let value = relative(value)?;
        if value == "." {
            return Err("The workspace root cannot be reordered".into());
        }
        let parent = value.rsplit_once('/').map(|(p, _)| p).unwrap_or(".");
        if parent != directory {
            return Err("Only entries in the same folder can be reordered".into());
        }
        Ok(format!("file:{value}"))
    } else if value.starts_with("node:")
        && value.len() > 5
        && directory == "."
        && !value.chars().any(char::is_control)
    {
        Ok(value.to_owned())
    } else {
        Err("Invalid file order item".into())
    }
}

fn validate(placements: &mut [FilePlacement]) -> Result<(), String> {
    let mut items = HashSet::new();
    for p in placements.iter_mut() {
        p.directory = relative(&p.directory)?;
        p.item = key(&p.item, &p.directory)?;
        p.anchor = key(&p.anchor, &p.directory)?;
        if p.item == p.anchor || !items.insert(p.item.clone()) {
            return Err("File order contains a duplicate or self-referencing placement".into());
        }
    }
    let edges: HashMap<_, _> = placements
        .iter()
        .map(|p| (p.item.as_str(), p.anchor.as_str()))
        .collect();
    for item in edges.keys() {
        let mut visited = HashSet::new();
        let mut current = *item;
        while let Some(anchor) = edges.get(current) {
            if !visited.insert(current) {
                return Err("File order contains a cycle".into());
            }
            current = anchor;
        }
    }
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<FileOrder, String> {
    let mut order: FileOrder =
        serde_json::from_slice(bytes).map_err(|e| format!("Failed to parse file order: {e}"))?;
    if order.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "Unsupported file order schema: {}",
            order.schema_version
        ));
    }
    validate(&mut order.placements)?;
    Ok(order)
}

/// Until the first save the checkout's legacy file seeds the order. That copy
/// is never written; an unusable one just starts the order empty.
fn read(file: &Path, root: &Path) -> Result<FileOrder, String> {
    match std::fs::read(file) {
        Ok(bytes) => parse(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok(std::fs::read(root.join("Locus").join(LEGACY_FILE_NAME))
                .ok()
                .and_then(|bytes| parse(&bytes).ok())
                .unwrap_or_default())
        }
        Err(e) => Err(format!("Failed to read file order: {e}")),
    }
}

fn write(file: &Path, order: &mut FileOrder) -> Result<(), String> {
    order.revision = order
        .revision
        .checked_add(1)
        .ok_or("File order revision is exhausted")?;
    let bytes = serde_json::to_vec_pretty(order).map_err(|e| e.to_string())?;
    crate::config::atomic_write_config(file, &bytes)
}

pub fn snapshot(root: &Path) -> Result<FileOrder, String> {
    snapshot_in(data_dir()?, root)
}

fn snapshot_in(data_dir: &Path, root: &Path) -> Result<FileOrder, String> {
    let file = path(data_dir, root)?;
    let lock = order_lock(root)?;
    let _guard = lock.lock().map_err(|e| e.to_string())?;
    read(&file, root)
}

pub fn save(
    root: &Path,
    expected_revision: i64,
    directory: &str,
    placements: Vec<FilePlacement>,
) -> Result<FileOrder, String> {
    save_in(data_dir()?, root, expected_revision, directory, placements)
}

fn save_in(
    data_dir: &Path,
    root: &Path,
    expected_revision: i64,
    directory: &str,
    mut placements: Vec<FilePlacement>,
) -> Result<FileOrder, String> {
    let file = path(data_dir, root)?;
    let lock = order_lock(root)?;
    let _guard = lock.lock().map_err(|e| e.to_string())?;
    let mut order = read(&file, root)?;
    if order.revision != expected_revision {
        return Err("workspace_file_order_revision_conflict".into());
    }
    let directory = relative(directory)?;
    validate(&mut placements)?;
    if placements.iter().any(|p| p.directory != directory) {
        return Err("File order update must target one folder".into());
    }
    let mut next: Vec<_> = order
        .placements
        .iter()
        .filter(|p| p.directory != directory)
        .cloned()
        .collect();
    next.extend(placements);
    if order.placements == next {
        return Ok(order);
    }
    order.placements = next;
    write(&file, &mut order)?;
    Ok(order)
}

/// Rename anchors and descendants. A cross-folder move drops the moved entry's
/// sibling relation, while retaining ordering inside the moved folder.
pub fn relocate(root: &Path, source: &Path, target: Option<&Path>) -> Result<(), String> {
    relocate_in(data_dir()?, root, source, target)
}

fn relocate_in(
    data_dir: &Path,
    root: &Path,
    source: &Path,
    target: Option<&Path>,
) -> Result<(), String> {
    let file = path(data_dir, root)?;
    let lock = order_lock(root)?;
    let _guard = lock.lock().map_err(|e| e.to_string())?;
    let mut order = read(&file, root)?;
    if order.placements.is_empty() {
        return Ok(());
    }
    let root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    let Some(source) = source
        .strip_prefix(&root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
    else {
        return Ok(());
    };
    let target = target
        .and_then(|p| p.strip_prefix(&root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"));
    let rewrite = |value: &str| -> Option<String> {
        if value == source {
            return target.clone();
        }
        if let Some(suffix) = value.strip_prefix(&format!("{source}/")) {
            return target.as_ref().map(|t| format!("{t}/{suffix}"));
        }
        Some(value.to_owned())
    };
    let rewrite_key = |value: &str| -> Option<String> {
        if let Some(file) = value.strip_prefix("file:") {
            rewrite(file).map(|p| format!("file:{p}"))
        } else {
            Some(value.to_owned())
        }
    };
    let next = order
        .placements
        .iter()
        .filter_map(|p| {
            let directory = rewrite(&p.directory)?;
            let item = rewrite_key(&p.item)?;
            let anchor = rewrite_key(&p.anchor)?;
            if key(&item, &directory).is_err() || key(&anchor, &directory).is_err() {
                return None;
            }
            Some(FilePlacement {
                directory,
                item,
                anchor,
                side: p.side.clone(),
            })
        })
        .collect::<Vec<_>>();
    if next != order.placements {
        order.placements = next;
        write(&file, &mut order)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn placement(directory: &str, item: &str, anchor: &str) -> FilePlacement {
        FilePlacement {
            directory: directory.into(),
            item: item.into(),
            anchor: anchor.into(),
            side: PlacementSide::Before,
        }
    }

    #[test]
    fn lazy_sparse_storage_reloads_and_keeps_directory_scopes() {
        let temp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        assert_eq!(snapshot_in(data.path(), temp.path()).unwrap(), FileOrder::default());
        assert!(!temp.path().join("Locus").exists());
        let root = vec![placement(".", "file:c.txt", "node:sessions")];
        let saved = save_in(data.path(), temp.path(), 0, ".", root.clone()).unwrap();
        assert_eq!(saved, snapshot_in(data.path(), temp.path()).unwrap());
        let raw: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path(data.path(), temp.path()).unwrap()).unwrap()).unwrap();
        assert_eq!(raw["schemaVersion"], 1);
        assert_eq!(raw["placements"].as_array().unwrap().len(), 1);
        assert!(raw.get("nodes").is_none());
        assert!(raw["placements"][0].get("position").is_none());
        let nested = placement("Assets", "file:Assets/b.cs", "file:Assets/a.cs");
        let saved = save_in(data.path(), temp.path(), 1, "Assets", vec![nested.clone()]).unwrap();
        assert_eq!(saved.placements, [root[0].clone(), nested.clone()]);
        let restored = save_in(data.path(), temp.path(), 2, ".", vec![]).unwrap();
        assert_eq!(restored.placements, [nested]);
        assert_eq!(snapshot_in(data.path(), temp.path()).unwrap(), restored);
    }

    #[test]
    fn rejects_stale_revisions_bad_scopes_and_cycles_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let saved = save_in(
            data.path(),
            temp.path(),
            0,
            ".",
            vec![placement(".", "file:b", "file:a")],
        )
        .unwrap();
        assert!(save_in(data.path(), temp.path(), 0, ".", vec![])
            .unwrap_err()
            .contains("revision_conflict"));
        for invalid in [
            vec![placement(".", "file:../a", "file:b")],
            vec![placement(".", "file:F:/a", "file:b")],
            vec![placement(".", "file:folder/a", "file:b")],
            vec![placement("folder", "node:sessions", "file:folder/a")],
            vec![placement("other", "file:other/a", "file:other/b")],
            vec![placement(".", "file:a", "file:a")],
            vec![
                placement(".", "file:a", "file:b"),
                placement(".", "file:b", "file:a"),
            ],
        ] {
            assert!(save_in(data.path(), temp.path(), 1, ".", invalid).is_err());
        }
        assert_eq!(snapshot_in(data.path(), temp.path()).unwrap(), saved);
        assert!(!temp.path().join("Locus").exists());
    }

    #[test]
    fn rename_delete_and_folder_moves_update_sparse_paths() {
        let temp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        save_in(data.path(), &root, 0, ".", vec![placement(".", "file:c", "file:a")]).unwrap();
        relocate_in(data.path(), &root, &root.join("a"), Some(&root.join("b"))).unwrap();
        assert_eq!(snapshot_in(data.path(), &root).unwrap().placements[0].anchor, "file:b");
        save_in(data.path(), &root, 2, "c", vec![placement("c", "file:c/z", "file:c/y")]).unwrap();
        relocate_in(data.path(), &root, &root.join("c"), Some(&root.join("nested/c"))).unwrap();
        assert_eq!(
            snapshot_in(data.path(), &root).unwrap().placements,
            [placement("nested/c", "file:nested/c/z", "file:nested/c/y")]
        );
        relocate_in(data.path(), &root, &root.join("nested/c/y"), None).unwrap();
        assert!(snapshot_in(data.path(), &root).unwrap().placements.is_empty());
    }

    #[test]
    fn incompatible_or_corrupt_files_are_never_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(data.path().join(DIR_NAME)).unwrap();
        for raw in [
            r#"{"schemaVersion":99,"revision":0,"placements":[]}"#,
            "not json",
        ] {
            std::fs::write(path(data.path(), temp.path()).unwrap(), raw).unwrap();
            assert!(snapshot_in(data.path(), temp.path()).is_err());
            assert!(save_in(data.path(), temp.path(), 0, ".", vec![]).is_err());
            assert_eq!(std::fs::read_to_string(path(data.path(), temp.path()).unwrap()).unwrap(), raw);
        }
    }

    #[test]
    fn checkout_files_seed_the_user_order_and_are_never_written() {
        let temp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("Locus").join(LEGACY_FILE_NAME);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let raw = r#"{"schemaVersion":1,"revision":4,"placements":[{"directory":".","item":"file:b","anchor":"file:a","side":"before"}]}"#;
        std::fs::write(&legacy, raw).unwrap();

        let seeded = snapshot_in(data.path(), temp.path()).unwrap();
        assert_eq!(seeded.revision, 4);
        assert_eq!(seeded.placements, [placement(".", "file:b", "file:a")]);
        let saved = save_in(data.path(), temp.path(), 4, ".", vec![]).unwrap();
        assert_eq!(saved.revision, 5);
        assert_eq!(snapshot_in(data.path(), temp.path()).unwrap(), saved);
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), raw);
        let file = path(data.path(), temp.path()).unwrap();
        assert!(file.starts_with(data.path().join(DIR_NAME)));
        // Each checkout keeps its own order.
        assert_ne!(path(data.path(), other.path()).unwrap(), file);
        assert_eq!(snapshot_in(data.path(), other.path()).unwrap(), FileOrder::default());

        // An unusable checkout file starts the order empty and stays as it is.
        let corrupt = other.path().join("Locus").join(LEGACY_FILE_NAME);
        std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
        std::fs::write(&corrupt, "not json").unwrap();
        assert_eq!(snapshot_in(data.path(), other.path()).unwrap(), FileOrder::default());
        save_in(data.path(), other.path(), 0, ".", vec![placement(".", "file:d", "file:c")]).unwrap();
        assert_eq!(std::fs::read_to_string(&corrupt).unwrap(), "not json");
    }
}
