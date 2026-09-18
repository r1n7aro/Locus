//! Streaming estimates. Keep only the expensive Library measurement briefly;
//! checkout files and destination free space are always read again.
use super::*;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreePlanProgress {
    pub phase: &'static str,
    pub files: u64,
    pub total_files: Option<u64>,
    pub bytes: u64,
}

pub struct Progress<'a> {
    report: &'a (dyn Fn(WorktreePlanProgress) + Sync),
    last: Instant,
    phase: &'static str,
}

impl<'a> Progress<'a> {
    pub fn new(report: &'a (dyn Fn(WorktreePlanProgress) + Sync)) -> Self {
        Self {
            report,
            last: Instant::now(),
            phase: "",
        }
    }
    pub fn send(&mut self, phase: &'static str, files: u64, total_files: Option<u64>, bytes: u64) {
        if phase != self.phase
            || self.last.elapsed() >= Duration::from_millis(100)
            || total_files == Some(files)
        {
            (self.report)(WorktreePlanProgress {
                phase,
                files,
                total_files,
                bytes,
            });
            self.last = Instant::now();
            self.phase = phase;
        }
    }
}

pub(super) fn is_alias(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

pub(super) fn directory_bytes(
    root: &Path,
    progress: &mut Progress<'_>,
) -> Result<(u64, u64), String> {
    let metadata = match std::fs::symlink_metadata(root) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(error) => return Err(io_error(error)),
    };
    if is_alias(&metadata) {
        return Ok((0, 0));
    }
    if metadata.is_file() {
        return Ok((metadata.len(), 1));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut total = 0u64;
    let mut files = 0u64;
    while let Some(path) = pending.pop() {
        let entries = match std::fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not measure {}: {error}", path.display())),
        };
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            if entry.file_name() == ".git" {
                continue;
            }
            // On Windows DirEntry metadata comes from directory enumeration,
            // avoiding a separate file open/stat for every Library artifact.
            let metadata = match entry.metadata() {
                Ok(value) => value,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(io_error(error)),
            };
            if is_alias(&metadata) {
                continue;
            }
            if metadata.is_file() {
                total = total.saturating_add(metadata.len());
                files += 1;
            } else if metadata.is_dir() {
                pending.push(entry.path());
            }
            progress.send("cache", files, None, total);
        }
    }
    progress.send("cache", files, Some(files), total);
    Ok((total, files))
}

type Stamp = Vec<Option<(u64, Option<SystemTime>)>>;
struct CacheEntry {
    at: Instant,
    stamp: Stamp,
    bytes: u64,
    files: u64,
}
static LIBRARY_CACHE: OnceLock<Mutex<BTreeMap<String, CacheEntry>>> = OnceLock::new();

fn library_stamp(root: &Path) -> Stamp {
    ["", "ArtifactDB", "SourceAssetDB"]
        .iter()
        .map(|name| {
            std::fs::symlink_metadata(root.join(name))
                .ok()
                .map(|meta| (meta.len(), meta.modified().ok()))
        })
        .collect()
}

pub(super) fn library_bytes(root: &Path, progress: &mut Progress<'_>) -> Result<u64, String> {
    let key = path_key(root);
    let stamp = library_stamp(root);
    let cache = LIBRARY_CACHE.get_or_init(Default::default);
    if let Some(entry) = cache.lock().map_err(io_error)?.get(&key) {
        if entry.at.elapsed() < Duration::from_secs(30) && entry.stamp == stamp {
            progress.send("cache", entry.files, Some(entry.files), entry.bytes);
            return Ok(entry.bytes);
        }
    }
    progress.send("cache", 0, None, 0);
    let (bytes, files) = directory_bytes(root, progress)?;
    // Do not cache a measurement taken while Unity was updating its databases.
    if library_stamp(root) == stamp {
        let mut cache = cache.lock().map_err(io_error)?;
        cache.retain(|_, entry| entry.at.elapsed() < Duration::from_secs(30));
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert(
            key,
            CacheEntry {
                at: Instant::now(),
                stamp,
                bytes,
                files,
            },
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual filesystem performance comparison"]
    fn benchmark_library_scan() {
        fn previous_scan(root: &Path) -> u64 {
            let mut pending = vec![root.to_path_buf()];
            let mut bytes = 0;
            while let Some(path) = pending.pop() {
                let metadata = std::fs::symlink_metadata(&path).unwrap();
                if is_alias(&metadata) {
                    continue;
                }
                if metadata.is_file() {
                    bytes += metadata.len();
                } else if metadata.is_dir() {
                    for entry in std::fs::read_dir(path).unwrap() {
                        let entry = entry.unwrap();
                        if entry.file_name() != ".git" {
                            pending.push(entry.path());
                        }
                    }
                }
            }
            bytes
        }
        let temp = tempfile::tempdir().unwrap();
        for group in 0..32 {
            let directory = temp.path().join(group.to_string());
            std::fs::create_dir(&directory).unwrap();
            for file in 0..512 {
                std::fs::write(directory.join(file.to_string()), [0; 128]).unwrap();
            }
        }
        let start = Instant::now();
        let old_bytes = previous_scan(temp.path());
        let old_time = start.elapsed();
        let start = Instant::now();
        let (new_bytes, files) = directory_bytes(temp.path(), &mut Progress::new(&|_| {})).unwrap();
        let new_time = start.elapsed();
        assert_eq!(old_bytes, new_bytes);
        assert_eq!(files, 16384);
        library_bytes(temp.path(), &mut Progress::new(&|_| {})).unwrap();
        let start = Instant::now();
        assert_eq!(
            library_bytes(temp.path(), &mut Progress::new(&|_| {})).unwrap(),
            new_bytes
        );
        std::println!("Library scan: files={files}, previous={old_time:?}, enumerated={new_time:?}, cached={:?}", start.elapsed());
    }

    #[test]
    fn directory_measurement_skips_git_and_reports_real_file_totals() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("nested")).unwrap();
        std::fs::create_dir_all(temp.path().join(".git")).unwrap();
        std::fs::write(temp.path().join("nested/a"), [0; 64]).unwrap();
        std::fs::write(temp.path().join("b"), [0; 32]).unwrap();
        std::fs::write(temp.path().join(".git/ignored"), [0; 128]).unwrap();
        let events = Mutex::new(Vec::new());
        let report = |event| events.lock().unwrap().push(event);
        assert_eq!(
            directory_bytes(temp.path(), &mut Progress::new(&report)).unwrap(),
            (96, 2)
        );
        let events = events.lock().unwrap();
        let last = events.last().unwrap();
        assert_eq!((last.files, last.total_files, last.bytes), (2, Some(2), 96));
    }

    #[test]
    fn library_measurement_reuses_recent_result_and_invalidates_changed_database() {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("Library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("ArtifactDB"), [0; 128]).unwrap();
        let events = Mutex::new(Vec::new());
        let report = |event| events.lock().unwrap().push(event);
        assert_eq!(
            library_bytes(&library, &mut Progress::new(&report)).unwrap(),
            128
        );
        events.lock().unwrap().clear();
        assert_eq!(
            library_bytes(&library, &mut Progress::new(&report)).unwrap(),
            128
        );
        assert_eq!(events.lock().unwrap().len(), 1); // Cached result only, no scan.
        std::fs::write(library.join("ArtifactDB"), [0; 256]).unwrap();
        assert_eq!(
            library_bytes(&library, &mut Progress::new(&report)).unwrap(),
            256
        );
    }

    #[cfg(windows)]
    #[test]
    fn measurement_does_not_follow_directory_junctions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("excluded"), [0; 1024]).unwrap();
        junction::create(&outside, root.join("linked")).unwrap();
        assert_eq!(
            directory_bytes(&root, &mut Progress::new(&|_| {})).unwrap(),
            (0, 0)
        );
        junction::delete(root.join("linked")).unwrap();
    }
}
