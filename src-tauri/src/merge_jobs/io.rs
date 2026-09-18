use super::types::FileState;
use crate::workspace_service::worktrees::git_cli_path;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Serialize, Deserialize)]
struct Storage {
    version: u32,
    git_dir: PathBuf,
}
static STORAGE_CACHE: LazyLock<Mutex<BTreeMap<PathBuf, Option<Storage>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub fn init_storage(root: &Path, dir: &Path) -> Result<(), String> {
    let common = git_text(root, &["rev-parse", "--git-common-dir"])?;
    let git_dir = dunce::canonicalize(root.join(common)).map_err(|e| e.to_string())?;
    let storage = Storage { version: 2, git_dir };
    fs::create_dir_all(shared_blobs(dir)?).map_err(|e| e.to_string())?;
    fs::create_dir_all(dir.parent().ok_or("Missing journal parent")?.join("git-blobs-v2"))
        .map_err(|e| e.to_string())?;
    atomic_json(&dir.join("storage.json"), &storage)?;
    cache_storage(dir, Some(storage));
    Ok(())
}
fn cache_storage(dir: &Path, storage: Option<Storage>) {
    let mut cache = STORAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= 512 { cache.clear(); }
    cache.insert(dir.to_path_buf(), storage);
}
fn storage(dir: &Path) -> Result<Option<Storage>, String> {
    if let Some(value) = STORAGE_CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(dir).cloned() {
        return Ok(value);
    }
    let value = match fs::read(dir.join("storage.json")) {
        Ok(bytes) => {
            let value: Storage = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if value.version != 2 { return Err("Unsupported merge blob storage version".into()); }
            Some(value)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    cache_storage(dir, value.clone());
    Ok(value)
}
fn shared_blobs(dir: &Path) -> Result<PathBuf, String> {
    Ok(dir.parent().ok_or("Missing journal parent")?.join("blobs-v2"))
}
fn blob_folder(dir: &Path) -> Result<PathBuf, String> {
    if storage(dir)?.is_some() { shared_blobs(dir) } else { Ok(dir.join("blobs")) }
}
pub fn git_blob_oid(state: &FileState) -> Option<&str> {
    state.blob.strip_prefix("git-").filter(|oid|
        matches!(oid.len(), 40 | 64) && oid.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn git_mapping(dir: &Path, oid: &str) -> Result<PathBuf, String> {
    Ok(dir.parent().ok_or("Missing journal parent")?.join("git-blobs-v2").join(format!("{oid}.json")))
}

/// Hydrate immutable Git inputs in batches, avoiding one process per meta/script.
pub fn prefetch_git<'a>(dir: &Path, states: impl IntoIterator<Item = &'a FileState>) -> Result<(), String> {
    let oids: std::collections::BTreeSet<_> = states.into_iter().filter_map(git_blob_oid)
        .map(str::to_owned).collect();
    if oids.is_empty() { return Ok(()); }
    let storage = storage(dir)?.ok_or("Git-backed snapshot requires v2 storage")?;
    let missing: Vec<_> = oids.into_iter().filter(|oid| {
        fs::read(git_mapping(dir, oid).unwrap()).ok()
            .and_then(|bytes| serde_json::from_slice::<FileState>(&bytes).ok())
            .is_none()
    }).collect();
    if missing.is_empty() { return Ok(()); }
    let sizes = git_batch(&storage.git_dir, &["cat-file", "--batch-check=%(objectname) %(objectsize)"], &missing)?;
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut size = 0u64;
    for line in String::from_utf8(sizes).map_err(|e|e.to_string())?.lines() {
        let (oid, length) = line.split_once(' ').ok_or("Invalid Git blob size response")?;
        let length = length.parse::<u64>().map_err(|e|e.to_string())?;
        if !batch.is_empty() && (size.saturating_add(length) > 32 * 1024 * 1024 || batch.len() >= 1024) {
            batches.push(std::mem::take(&mut batch)); size = 0;
        }
        batch.push(oid.to_string()); size = size.saturating_add(length);
    }
    if !batch.is_empty() { batches.push(batch); }
    // Bound total bytes, not just file count: thousands of metas fit in one
    // process while a large scene is read in its own batch.
    for batch in batches {
        let blobs = git_blobs(&storage.git_dir, &batch)?;
        super::parallel::pool().install(|| blobs.par_iter().try_for_each(|(oid, bytes)| {
            let state = store_blob(dir, bytes, "100644")?;
            cache_json(&git_mapping(dir, oid)?, &state)
        }))?;
    }
    Ok(())
}

pub fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = crate::process_util::command("git");
    #[cfg(windows)]
    command.args(["-c", "core.longpaths=true"]);
    let output = command
        .arg("-C")
        .arg(git_cli_path(root))
        .args(args.iter().map(|arg| git_cli_path(Path::new(arg))))
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "git {} at {}: {}",
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}
pub fn git_text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(git(root, args)?)
        .map(|s| s.trim_end().to_string())
        .map_err(|e| e.to_string())
}
pub fn git_input(
    root: &Path,
    args: &[&str],
    input: &[u8],
    index: Option<&Path>,
) -> Result<String, String> {
    let mut command = crate::process_util::command("git");
    #[cfg(windows)]
    command.args(["-c", "core.longpaths=true"]);
    command
        .arg("-C")
        .arg(git_cli_path(root))
        .args(args.iter().map(|arg| git_cli_path(Path::new(arg))))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", git_cli_path(index));
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    child
        .stdin
        .take()
        .ok_or("Git stdin unavailable")?
        .write_all(input)
        .map_err(|e| e.to_string())?;
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("git {} at {}: {}",args.join(" "),root.display(),String::from_utf8_lossy(&output.stderr)));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned())
}

/// One Git process serves an immutable blob batch. Drain stdout while feeding
/// stdin so large batches cannot deadlock on the Windows anonymous-pipe buffer.
pub fn git_blobs(root: &Path, oids: &[String]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if oids.is_empty() {
        return Ok(BTreeMap::new());
    }
    if oids
        .iter()
        .any(|oid| !oid.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("Invalid blob OID".into());
    }
    let output = git_batch(root, &["cat-file", "--batch"], oids)?;
    let mut result = BTreeMap::new();
    let mut offset = 0;
    for expected in oids {
        let end = output[offset..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|n| offset + n)
            .ok_or("Truncated cat-file header")?;
        let header = std::str::from_utf8(&output[offset..end]).map_err(|e| e.to_string())?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || fields[0] != expected || fields[1] != "blob" {
            return Err(format!("Unexpected cat-file response for {expected}"));
        }
        let size = fields[2].parse::<usize>().map_err(|e| e.to_string())?;
        let start = end + 1;
        let end = start.checked_add(size).ok_or("Oversized Git blob")?;
        if end >= output.len() || output[end] != b'\n' {
            return Err("Truncated cat-file blob".into());
        }
        result.insert(expected.clone(), output[start..end].to_vec());
        offset = end + 1;
    }
    Ok(result)
}

fn git_batch(root: &Path, args: &[&str], oids: &[String]) -> Result<Vec<u8>, String> {
    git_output_input(root, args, format!("{}\n", oids.join("\n")).into_bytes())
}

pub fn git_output_input(root: &Path, args: &[&str], input: Vec<u8>) -> Result<Vec<u8>, String> {
    let mut command = crate::process_util::command("git");
    #[cfg(windows)] command.args(["-c", "core.longpaths=true"]);
    let mut child = command.arg("-C").arg(git_cli_path(root)).args(args)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e|e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("Missing Git batch stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let output = child.wait_with_output().map_err(|e|e.to_string())?;
    writer.join().map_err(|_|"Git input thread failed")?.map_err(|e|e.to_string())?;
    if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into_owned()); }
    Ok(output.stdout)
}
pub fn repository(root: &Path) -> Result<PathBuf, String> {
    dunce::canonicalize(git_text(root, &["rev-parse", "--show-toplevel"])?)
        .map_err(|e| e.to_string())
}
pub fn journal_root(root: &Path) -> Result<PathBuf, String> {
    let common = git_text(root, &["rev-parse", "--git-common-dir"])?;
    let path = Path::new(&common);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let path = path.join("locus").join("merge-jobs");
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}
pub fn job_dir(root: &Path, id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid merge job ID")?;
    Ok(journal_root(root)?.join(id))
}
/// No symlink/reparse traversal is permitted for source writes, even through an ancestor.
pub fn safe_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if relative.is_empty()
        || relative.contains('\\')
        || relative.contains(':')
        || relative.contains('\0')
    {
        return Err(format!("Unsupported repository path: {relative}"));
    }
    let relative_path = Path::new(relative);
    if relative_path
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("Path is outside the checkout: {relative}"));
    }
    if relative_path.components().any(|part| {
        part.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".git")
    }) {
        return Err("A merge cannot modify Git administration files".into());
    }
    let mut current = root.to_path_buf();
    for part in relative_path.components() {
        #[cfg(windows)]
        {
            let name = part.as_os_str().to_string_lossy();
            let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
            if name.ends_with('.')
                || name.ends_with(' ')
                || name.chars().any(|c| "<>\"|?*".contains(c))
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || ((stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.len() == 4
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            {
                return Err(format!(
                    "Windows reserved/ambiguous path is not a merge destination: {relative}"
                ));
            }
        }
        current.push(part);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Symlink path requires an explicit external dependency policy: {relative}"
                ));
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(format!(
                        "Reparse point is not a merge destination: {relative}"
                    ));
                }
            }
        }
    }
    Ok(current)
}
pub fn store_blob(dir: &Path, bytes: &[u8], mode: &str) -> Result<FileState, String> {
    let blob = blake3::hash(bytes).to_hex().to_string();
    let folder = blob_folder(dir)?;
    if storage(dir)?.is_none() { fs::create_dir_all(&folder).map_err(|e| e.to_string())?; }
    let path = folder.join(&blob);
    if !path.exists() {
        // Atomic publication: parallel jobs must never observe a partial blob.
        let mut temp = tempfile::NamedTempFile::new_in(&folder).map_err(|e| e.to_string())?;
        temp.write_all(bytes).map_err(|e| e.to_string())?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {},
            Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists
                || (path.is_file() && fs::read(&path).is_ok_and(|existing| existing == bytes)) => {},
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(FileState {
        blob,
        mode: mode.into(),
    })
}
pub fn read_blob(dir: &Path, state: &FileState) -> Result<Vec<u8>, String> {
    if let Some(oid) = git_blob_oid(state) {
        prefetch_git(dir, [state])?;
        let resolved: FileState = serde_json::from_slice(&fs::read(git_mapping(dir, oid)?)
            .map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if resolved.blob.starts_with("git-") { return Err("Invalid recursive Git blob mapping".into()); }
        return read_blob(dir, &resolved);
    }
    if state.blob.len() != 64 || !state.blob.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid snapshot blob".into());
    }
    let bytes = fs::read(blob_folder(dir)?.join(&state.blob)).map_err(|e| e.to_string())?;
    if blake3::hash(&bytes).to_hex().as_str() != state.blob {
        return Err("Corrupt merge snapshot blob".into());
    }
    Ok(bytes)
}
pub fn capture(root: &Path, dir: &Path, path: &str) -> Result<Option<FileState>, String> {
    let full = safe_path(root, path)?;
    let size = fs::metadata(&full).map(|s| s.len()).unwrap_or(0);
    super::parallel::with_bytes(size, || match fs::read(&full) {
        Ok(bytes) => {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                if fs::metadata(&full)
                    .map_err(|e| e.to_string())?
                    .permissions()
                    .mode()
                    & 0o111
                    != 0
                {
                    "100755"
                } else {
                    "100644"
                }
            };
            #[cfg(not(unix))]
            let mode = "100644";
            Ok(Some(store_blob(dir, &bytes, mode)?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Cannot snapshot {path}: {e}")),
    })
}
pub fn tree_file(
    root: &Path,
    dir: &Path,
    tree: &str,
    path: &str,
) -> Result<Option<FileState>, String> {
    let output = git(root, &["ls-tree", "-z", tree, "--", path])?;
    if output.is_empty() {
        return Ok(None);
    }
    let line = String::from_utf8(output).map_err(|e| e.to_string())?;
    let record = line.split('\t').next().ok_or("Invalid ls-tree record")?;
    let fields: Vec<_> = record.split_whitespace().collect();
    if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") {
        return Err(format!(
            "Unsupported source mode for {path}; symlinks and submodules require explicit handling"
        ));
    }
    let bytes = git(root, &["cat-file", "blob", fields[2]])?;
    Ok(Some(store_blob(dir, &bytes, fields[0])?))
}

pub fn tree_files(
    root: &Path,
    dir: &Path,
    tree: &str,
    paths: &[String],
) -> Result<BTreeMap<String, Option<FileState>>, String> {
    let mut result: BTreeMap<_, _> = paths.iter().map(|p| (p.clone(), None)).collect();
    let records = git(root, &["ls-tree", "-r", "-z", tree])?;
    let mut entries = BTreeMap::new();
    for raw in records.split(|b| *b == 0).filter(|b| !b.is_empty()) {
        let record = std::str::from_utf8(raw).map_err(|e| e.to_string())?;
        let Some((header, path)) = record.split_once('\t') else {
            return Err("Invalid Git tree record".into());
        };
        if !result.contains_key(path) {
            continue;
        }
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 || fields[1] != "blob" || !matches!(fields[0], "100644" | "100755") {
            return Err(format!("Unsupported source file mode for {path}"));
        }
        entries.insert(
            path.to_string(),
            (fields[0].to_string(), fields[2].to_string()),
        );
    }
    let oids: Vec<_> = entries
        .values()
        .map(|(_, oid)| oid.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let blobs = git_blobs(root, &oids)?;
    for (path, (mode, oid)) in entries {
        result.insert(path, Some(store_blob(dir, &blobs[&oid], &mode)?));
    }
    Ok(result)
}
pub fn write_file(
    root: &Path,
    dir: &Path,
    path: &str,
    state: &Option<FileState>,
) -> Result<(), String> {
    let full = safe_path(root, path)?;
    if let Some(state) = state {
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create parent for {}: {e}", full.display()))?;
        }
        let bytes = read_blob(dir, state)?;
        if is_lfs_pointer(&bytes) {
            return Err(format!("Refusing to write an unmaterialized Git LFS pointer into {path}; select an available complete LFS object"));
        }
        atomic_replace(&full, |temp| {
            temp.write_all(&bytes).map_err(|e| e.to_string())
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &full,
                fs::Permissions::from_mode(if state.mode == "100755" { 0o755 } else { 0o644 }),
            )
            .map_err(|e| e.to_string())?;
        }
    } else if full.exists() {
        fs::remove_file(&full).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\n")
        || bytes.starts_with(b"version https://git-lfs.github.com/spec/v1\r\n")
}
pub fn materialize_lfs(root: &Path, dir: &Path, state: &FileState) -> Result<FileState, String> {
    let pointer = read_blob(dir, state)?;
    if !is_lfs_pointer(&pointer) {
        return Ok(state.clone());
    }
    let text = std::str::from_utf8(&pointer).map_err(|_| "Invalid Git LFS pointer encoding")?;
    let oid = text
        .lines()
        .find_map(|line| line.strip_prefix("oid sha256:"))
        .filter(|oid| oid.len() == 64 && oid.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Invalid Git LFS pointer OID")?;
    let size = text
        .lines()
        .find_map(|line| line.strip_prefix("size "))
        .and_then(|size| size.parse::<u64>().ok())
        .ok_or("Invalid Git LFS pointer size")?;
    let common = git_text(root, &["rev-parse", "--git-common-dir"])?;
    let common = Path::new(&common);
    let common = if common.is_absolute() {
        common.to_path_buf()
    } else {
        root.join(common)
    };
    let storage = match git_text(root, &["config", "--path", "lfs.storage"]) {
        Ok(storage) => {
            let storage = Path::new(&storage);
            if storage.is_absolute() {
                storage.to_path_buf()
            } else {
                common.join(storage)
            }
        }
        Err(_) => common.join("lfs"),
    };
    let path = storage
        .join("objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid);
    let bytes=std::fs::read(&path).map_err(|_|format!("LFS object {oid} is not available locally; fetch the selected commit's LFS objects, then retry preview (pointer bytes will never be applied)"))?;
    use sha2::Digest;
    if bytes.len() as u64 != size
        || sha2::Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
            != oid.to_ascii_lowercase()
    {
        return Err(format!("LFS object {oid} failed size/SHA-256 verification"));
    }
    store_blob(dir, &bytes, &state.mode)
}
fn atomic_replace(
    path: &Path,
    write: impl FnOnce(&mut tempfile::NamedTempFile) -> Result<(), String>,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Missing parent for {}", path.display()))?;
    let filename = path
        .file_name()
        .ok_or_else(|| format!("Missing filename for {}", path.display()))?;
    // tempfile's Windows backend passes both paths directly to native APIs.
    // Keep std's verbatim prefix for both the temporary file and destination;
    // canonicalizing only the destination (or using dunce) is insufficient.
    #[cfg(windows)]
    let parent = fs::canonicalize(parent)
        .map_err(|e| format!("Could not resolve parent for {}: {e}", path.display()))?;
    #[cfg(not(windows))]
    let parent = parent.to_path_buf();
    let destination = parent.join(filename);
    let mut temp = tempfile::NamedTempFile::new_in(&parent)
        .map_err(|e| format!("Could not create replacement for {}: {e}", path.display()))?;
    write(&mut temp)
        .map_err(|e| format!("Could not write replacement for {}: {e}", path.display()))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| format!("Could not sync replacement for {}: {e}", path.display()))?;
    temp.persist(&destination)
        .map_err(|e| format!("Could not atomically replace {}: {e}", path.display()))?;
    Ok(())
}
pub fn atomic_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    atomic_replace(path, |temp| {
        let mut writer = std::io::BufWriter::with_capacity(256 * 1024, temp);
        serde_json::to_writer(&mut writer, value).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())
    })
}

/// Reconstructible caches need atomic publication, not a per-entry disk flush.
/// Durable job/transaction journals continue to use atomic_json.
pub fn cache_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let parent = path.parent().ok_or("Missing cache parent")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e|e.to_string())?;
    let bytes = serde_json::to_vec(value).map_err(|e|e.to_string())?;
    temp.write_all(&bytes).map_err(|e|e.to_string())?;
    match temp.persist_noclobber(path) {
        Ok(_) => {},
        Err(_) if fs::read(path).is_ok_and(|existing| existing == bytes) => {},
        Err(error) => return Err(format!("Could not publish cache {}: {error}",path.display())),
    }
    Ok(())
}
pub fn lock(root: &Path) -> Result<fs::File, String> {
    let path = journal_root(root)?.join("operation.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    fs4::FileExt::try_lock(&file).map_err(|e| format!("Another merge operation is active: {e}"))?;
    Ok(file)
}
pub fn index_entries(root: &Path) -> Result<(String, String), String> {
    Ok((
        String::from_utf8(git(root, &["ls-files", "--stage", "-z"])?).map_err(|e| e.to_string())?,
        String::from_utf8(git(root, &["ls-files", "-v", "-z"])?).map_err(|e| e.to_string())?,
    ))
}
pub fn check_files(
    root: &Path,
    _dir: &Path,
    files: &BTreeMap<String, Option<FileState>>,
) -> Result<(), String> {
    super::parallel::pool().install(|| files.par_iter().try_for_each(|(path, state)| {
        if &fingerprint(root, path, state.as_ref())? != state {
            return Err(format!(
                "stale: {path} changed after the snapshot; replan before writing"
            ));
        }
        Ok(())
    }))
}

/// Check raw worktree bytes without allocating/writing a snapshot blob. Streaming
/// hashing also keeps revalidation memory independent of the largest scene.
pub fn fingerprint(root: &Path, path: &str, known: Option<&FileState>) -> Result<Option<FileState>, String> {
    let full = safe_path(root, path)?;
    let mut file = match fs::File::open(&full) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Cannot verify {path}: {e}")),
    };
    let mut hash = blake3::Hasher::new();
    hash.update_reader(&mut file).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata().map_err(|e| e.to_string())?.permissions().mode() & 0o111 != 0 { "100755" } else { "100644" }
    };
    #[cfg(not(unix))]
    let mode = known.map(|s| s.mode.as_str()).unwrap_or("100644");
    #[cfg(unix)] let _ = known;
    Ok(Some(FileState { blob: hash.finalize().to_hex().to_string(), mode: mode.into() }))
}

pub fn capture_like(
    root: &Path,
    _dir: &Path,
    path: &str,
    known: Option<&FileState>,
) -> Result<Option<FileState>, String> {
    fingerprint(root, path, known)
}

/// Git-compatible index.lock transaction. The guarded real index is never used
/// as a temporary staging area; a shadow copy is prepared and installed once.
pub struct IndexTransaction {
    path: PathBuf,
    lock_path: PathBuf,
    lock: Option<fs::File>,
    active: bool,
    pub original: Vec<u8>,
}
impl IndexTransaction {
    pub fn begin(root: &Path) -> Result<Self, String> {
        let name = git_text(root, &["rev-parse", "--git-path", "index"])?;
        let path = Path::new(&name);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let mut lock_name = path.as_os_str().to_os_string();
        lock_name.push(".lock");
        let lock_path = PathBuf::from(lock_name);
        let lock = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("Destination index is busy: {e}"))?;
        let original = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                drop(lock);
                let _ = fs::remove_file(&lock_path);
                return Err(e.to_string());
            }
        };
        Ok(Self {
            path,
            lock_path,
            lock: Some(lock),
            active: true,
            original,
        })
    }
    pub fn install(mut self, bytes: &[u8]) -> Result<(), String> {
        let current = fs::read(&self.path).unwrap_or_default();
        if current != self.original {
            return Err(
                "stale: index changed despite its Git lock; preserved external index".into(),
            );
        }
        let mut lock = self.lock.take().ok_or("Index transaction already closed")?;
        lock.write_all(bytes).map_err(|e| e.to_string())?;
        lock.sync_all().map_err(|e| e.to_string())?;
        drop(lock);
        self.active = false;
        if let Err(error) = fs::rename(&self.lock_path, &self.path) {
            let _ = fs::remove_file(&self.lock_path);
            return Err(error.to_string());
        }
        Ok(())
    }
}
impl Drop for IndexTransaction {
    fn drop(&mut self) {
        self.lock.take();
        if self.active {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

pub fn prepare_index(
    root: &Path,
    dir: &Path,
    seed: &[u8],
    entries: &BTreeMap<String, Option<FileState>>,
    paths: &[String],
) -> Result<Vec<u8>, String> {
    let shadow = dir.join(format!("index-{}", uuid::Uuid::new_v4().simple()));
    if seed.is_empty() {
        git_input(root, &["read-tree", "--empty"], b"", Some(&shadow))?;
    } else {
        fs::write(&shadow, seed).map_err(|e| e.to_string())?;
    }
    let oid_len = git_text(root, &["rev-parse", "HEAD"])?.len();
    for path in paths {
        let state = entries
            .get(path)
            .ok_or("No selected output for index path")?;
        let entry = if let Some(state) = state {
            let bytes = read_blob(dir, state)?;
            let oid = git_input(
                root,
                &["hash-object", "-w", "--stdin", "--path", path],
                &bytes,
                None,
            )?;
            format!("{} {}\t{}\0", state.mode, oid, path)
        } else {
            format!("0 {}\t{}\0", "0".repeat(oid_len), path)
        };
        git_input(
            root,
            &["update-index", "-z", "--index-info"],
            entry.as_bytes(),
            Some(&shadow),
        )?;
    }
    fs::read(shadow).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn git_helpers_adapt_verbatim_roots_and_long_indexes_without_rebasing() {
        let (temp, repo) = crate::workspace_service::worktrees::tests::fixture();
        let verbatim_repo = std::fs::canonicalize(&repo).unwrap();
        let real_index = std::fs::read(repo.join(".git/index")).unwrap();
        let mut project = repo.join("Projects");
        while project.to_string_lossy().len() < 310 {
            project.push("long-io-component-123456789");
        }
        std::fs::create_dir_all(&project).unwrap();
        assert!(crate::workspace_service::worktrees::path_components_equal(
            &std::fs::canonicalize(repository(&verbatim_repo).unwrap()).unwrap(),
            &verbatim_repo,
        )
        .unwrap());
        if let Err(error) = repository(&project) {
            assert!(error.contains("git rev-parse --show-toplevel at"), "{error}");
            assert!(error.contains(&project.to_string_lossy().to_string()), "{error}");
            assert!(error.contains("Filename too long"), "{error}");
        }
        let index_parent = long_parent(temp.path());
        let shadow_index = std::fs::canonicalize(&index_parent)
            .unwrap()
            .join("shadow-index");
        git_input(
            &verbatim_repo,
            &["read-tree", "HEAD"],
            b"",
            Some(&shadow_index),
        )
        .unwrap();
        assert!(shadow_index.is_file());
        let blob = git_input(
            &verbatim_repo,
            &["hash-object", "-w", "--stdin"],
            b"exact blob bytes\n",
            None,
        )
        .unwrap();
        assert_eq!(
            git_blobs(&verbatim_repo, &[blob.clone()]).unwrap()[&blob],
            b"exact blob bytes\n"
        );
        assert_eq!(std::fs::read(repo.join(".git/index")).unwrap(), real_index);
    }

    #[cfg(windows)]
    fn long_parent(root: &Path) -> PathBuf {
        let parent = root
            .join("a".repeat(90))
            .join("b".repeat(90))
            .join("c".repeat(90));
        fs::create_dir_all(&parent).unwrap();
        assert!(parent.to_string_lossy().len() > 260);
        parent
    }

    #[test]
    #[cfg(windows)]
    fn atomic_write_file_supports_long_target_creation_and_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("checkout");
        let parent = long_parent(&root);
        let target = parent.join("asset.asset");
        let relative = target
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let dir = temp.path().join("snapshots");
        for bytes in [
            b"first frozen worktree bytes\n".as_slice(),
            b"replacement dirty worktree bytes\n".as_slice(),
        ] {
            let state = store_blob(&dir, bytes, "100644").unwrap();
            write_file(&root, &dir, &relative, &Some(state)).unwrap();
            assert_eq!(fs::read(&target).unwrap(), bytes);
            assert_eq!(
                fs::read_dir(&parent).unwrap().count(),
                1,
                "temporary replacement leaked"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn atomic_json_supports_long_paths_and_preserves_existing_bytes_on_failure() {
        struct FailingJson;
        impl serde::Serialize for FailingJson {
            fn serialize<S: serde::Serializer>(&self, _serializer: S) -> Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("fixture serialization failure"))
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let parent = long_parent(temp.path());
        let target = parent.join("journal.json");
        for revision in [1, 2] {
            let value = serde_json::json!({"revision":revision});
            atomic_json(&target, &value).unwrap();
            assert_eq!(
                fs::read(&target).unwrap(),
                serde_json::to_vec(&value).unwrap()
            );
            assert_eq!(fs::read_dir(&parent).unwrap().count(), 1);
        }
        let before = fs::read(&target).unwrap();
        let error = atomic_json(&target, &FailingJson).unwrap_err();
        assert!(
            error.contains(&target.to_string_lossy().to_string()),
            "{error}"
        );
        assert!(error.contains("fixture serialization failure"));
        assert_eq!(fs::read(&target).unwrap(), before);
        assert_eq!(
            fs::read_dir(&parent).unwrap().count(),
            1,
            "failed replacement leaked"
        );
    }

    #[test]
    #[cfg(windows)]
    fn atomic_persist_failure_reports_long_target_and_removes_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let parent = long_parent(temp.path());
        let target = parent.join("existing-directory");
        fs::create_dir(&target).unwrap();
        let error = atomic_json(&target, &serde_json::json!({"value":1})).unwrap_err();
        assert!(
            error.contains(&target.to_string_lossy().to_string()),
            "{error}"
        );
        assert!(error.contains("atomically replace"));
        assert!(target.is_dir());
        assert_eq!(
            fs::read_dir(&parent).unwrap().count(),
            1,
            "failed persistence leaked"
        );
    }
}
