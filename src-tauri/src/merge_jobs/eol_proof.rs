//! Restricted proof of Unity's CRLF -> LF saves in an immutable candidate.
//! All snapshots remain raw-byte fingerprints; only a proved transformation
//! receives separate certificate evidence. No file contents are retained.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct TextFingerprint {
    pub lf_hash: String,
    pub crlf_pairs: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct FileFingerprint {
    pub raw_hash: String,
    pub bytes: u64,
    pub text: Option<TextFingerprint>,
}
pub(super) type Snapshot = BTreeMap<String, Option<FileFingerprint>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct EolNormalization {
    pub path: String,
    pub transformation: String,
    pub candidate_tree: String,
    pub candidate_blob_oid: String,
    pub candidate_mode: String,
    pub before_raw_blake3: String,
    pub before_lf_blake3: String,
    pub after_raw_blake3: String,
    pub after_raw_blob_oid: String,
    pub after_path_clean_oid: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub replaced_crlf_pairs: u64,
    pub effective_attributes: BTreeMap<String, String>,
}

fn opaque(path: &Path) -> bool {
    super::opaque(&path.to_string_lossy())
        || matches!(
            path.extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
            "zip" | "7z" | "unitypackage"
        )
}

/// A single bounded scan hashes raw and LF-normalized bytes. Keep only an
/// incomplete UTF-8 suffix (<=3 bytes) and a CR spanning two read buffers.
pub(super) fn fingerprint(path: &Path) -> Result<FileFingerprint, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    fingerprint_reader(file, !opaque(path))
}
fn fingerprint_reader(mut file: impl Read, mut text: bool) -> Result<FileFingerprint, String> {
    let mut raw = blake3::Hasher::new();
    let mut normalized = blake3::Hasher::new();
    let mut bytes = 0u64;
    let mut pairs = 0u64;
    let mut pending_cr = false;
    let mut utf8_suffix = Vec::new();
    let mut prefix = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            break;
        }
        let chunk = &buffer[..size];
        raw.update(chunk);
        bytes += size as u64;
        if !text {
            continue;
        }
        if prefix.len() < 64 {
            prefix.extend_from_slice(&chunk[..chunk.len().min(64 - prefix.len())]);
        }
        if chunk.contains(&0) {
            text = false;
            continue;
        }
        let joined;
        let utf8 = if utf8_suffix.is_empty() {
            chunk
        } else {
            joined = [utf8_suffix.as_slice(), chunk].concat();
            &joined
        };
        let next_suffix = match std::str::from_utf8(utf8) {
            Ok(_) => Vec::new(),
            Err(error) if error.error_len().is_none() => utf8[error.valid_up_to()..].to_vec(),
            Err(_) => {
                text = false;
                continue;
            }
        };
        utf8_suffix = next_suffix;
        if pending_cr {
            if chunk[0] == b'\n' {
                pairs += 1;
            } else {
                normalized.update(b"\r");
            }
            pending_cr = false;
        }
        let mut cursor = 0;
        while let Some(offset) = chunk[cursor..].iter().position(|b| *b == b'\r') {
            let cr = cursor + offset;
            normalized.update(&chunk[cursor..cr]);
            if cr + 1 == chunk.len() {
                pending_cr = true;
                cursor = chunk.len();
                break;
            }
            if chunk[cr + 1] == b'\n' {
                pairs += 1;
            } else {
                normalized.update(b"\r");
            }
            cursor = cr + 1;
        }
        normalized.update(&chunk[cursor..]);
    }
    if pending_cr {
        normalized.update(b"\r");
    }
    text &= utf8_suffix.is_empty()
        && !prefix.starts_with(b"version https://git-lfs.github.com/spec/v1\n")
        && !prefix.starts_with(b"version https://git-lfs.github.com/spec/v1\r\n");
    Ok(FileFingerprint {
        raw_hash: raw.finalize().to_hex().to_string(),
        bytes,
        text: text.then(|| TextFingerprint {
            lf_hash: normalized.finalize().to_hex().to_string(),
            crlf_pairs: pairs,
        }),
    })
}

pub(super) fn snapshot_hash(snapshot: &Snapshot) -> Result<String, String> {
    struct HashWriter(blake3::Hasher);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = HashWriter(blake3::Hasher::new());
    serde_json::to_writer(&mut output, snapshot).map_err(|e| e.to_string())?;
    Ok(output.0.finalize().to_hex().to_string())
}

fn git_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{unc}"));
        }
        if let Some(drive) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(drive);
        }
    }
    path.to_path_buf()
}
enum Input<'a> {
    Bytes(&'a [u8]),
    File(std::fs::File),
}
fn git(
    repo: &Path,
    index: Option<&Path>,
    args: &[&str],
    input: Option<Input<'_>>,
) -> Result<Vec<u8>, String> {
    let mut command = crate::process_util::command("git");
    #[cfg(windows)]
    command.args(["-c", "core.longpaths=true"]);
    command
        .arg("-C")
        .arg(git_path(repo))
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", git_path(index));
    }
    let bytes = match input {
        Some(Input::File(file)) => {
            command.stdin(Stdio::from(file));
            None
        }
        Some(Input::Bytes(bytes)) => {
            command.stdin(Stdio::piped());
            Some(bytes.to_vec())
        }
        None => None,
    };
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let writer = bytes.map(|bytes| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        std::thread::spawn(move || stdin.write_all(&bytes))
    });
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if let Some(writer) = writer {
        writer
            .join()
            .map_err(|_| "Git input writer failed")?
            .map_err(|e| e.to_string())?;
    }
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    Ok(output.stdout)
}
fn git_text(
    repo: &Path,
    index: Option<&Path>,
    args: &[&str],
    input: Option<Input<'_>>,
) -> Result<String, String> {
    String::from_utf8(git(repo, index, args, input)?)
        .map(|s| s.trim_end().to_string())
        .map_err(|e| e.to_string())
}
fn attributes(
    repo: &Path,
    index: &Path,
    path: &str,
    cached: bool,
) -> Result<BTreeMap<String, String>, String> {
    let mut args = vec!["check-attr", "-z", "--stdin"];
    if cached {
        args.push("--cached");
    }
    args.extend(["text", "eol", "filter", "working-tree-encoding", "ident"]);
    let bytes = git(
        repo,
        Some(index),
        &args,
        Some(Input::Bytes(format!("{path}\0").as_bytes())),
    )?;
    let fields: Vec<_> = bytes.split(|b| *b == 0).filter(|v| !v.is_empty()).collect();
    if fields.len() != 15 {
        return Err("Incomplete Git attribute evidence".into());
    }
    let mut result = BTreeMap::new();
    for triple in fields.chunks_exact(3) {
        if triple[0] != path.as_bytes() {
            return Err("Git returned attributes for another path".into());
        }
        result.insert(
            String::from_utf8(triple[1].to_vec()).map_err(|e| e.to_string())?,
            String::from_utf8(triple[2].to_vec()).map_err(|e| e.to_string())?,
        );
    }
    Ok(result)
}

/// No exception applies to missing/added/ignored files, content changes,
/// arbitrary filters, encoding transforms or LFS. The entire proof is checked
/// before returning any certificate; failed inputs retain their raw snapshots.
pub(super) fn prove(
    repo: &Path,
    tree: &str,
    before: &Snapshot,
    after: &Snapshot,
    proof_dir: &Path,
) -> Result<Vec<EolNormalization>, String> {
    if before.keys().ne(after.keys()) {
        let changed: Vec<_> = before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .filter(|p| !before.contains_key(*p) || !after.contains_key(*p))
            .cloned()
            .collect();
        return Err(format!("Candidate source file set changed (no EOL exception for new/ignored/deleted files): {}",changed.join(", ")));
    }
    let missing: Vec<_> = before
        .iter()
        .filter(|(p, state)| state.is_none() || after[*p].is_none())
        .map(|(p, _)| p.clone())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "Candidate source files missing: {}",
            missing.join(", ")
        ));
    }
    let changed: Vec<_> = before
        .iter()
        .filter(|(p, state)| Some(*state) != after.get(*p))
        .collect();
    if changed.is_empty() {
        return Ok(vec![]);
    }
    if !matches!(tree.len(), 40 | 64)
        || !tree.bytes().all(|b| b.is_ascii_hexdigit())
        || git_text(repo, None, &["cat-file", "-t", tree], None)? != "tree"
    {
        return Err("EOL proof requires an immutable full candidate tree OID".into());
    }
    let records = git(
        repo,
        None,
        &["ls-tree", "-r", "-z", "--full-tree", tree],
        None,
    )?;
    let mut entries = BTreeMap::new();
    for record in records.split(|b| *b == 0).filter(|v| !v.is_empty()) {
        let (header, path) = std::str::from_utf8(record)
            .map_err(|e| e.to_string())?
            .split_once('\t')
            .ok_or("Invalid tree record")?;
        let fields: Vec<_> = header.split_whitespace().collect();
        if fields.len() != 3 {
            return Err("Invalid tree header".into());
        }
        if fields[1] == "blob" {
            entries.insert(
                path.to_string(),
                (fields[0].to_string(), fields[2].to_string()),
            );
        }
    }
    std::fs::create_dir_all(proof_dir).map_err(|e| e.to_string())?;
    let temporary = tempfile::Builder::new()
        .prefix("eol-proof-")
        .tempdir_in(proof_dir)
        .map_err(|e| e.to_string())?;
    let index = temporary.path().join("candidate-attributes-index");
    git(repo, Some(&index), &["read-tree", tree], None)?;
    let mut evidence = Vec::new();
    for (path, before) in changed {
        let before = before.as_ref().expect("missing checked");
        let after = after[path].as_ref().expect("missing checked");
        let (mode, oid) = entries
            .get(path)
            .ok_or_else(|| format!("{path}: changed source is not tracked in candidate"))?;
        let Some(text) = before.text.as_ref() else {
            return Err(format!(
                "{path}: changed bytes are not eligible plain UTF-8 text"
            ));
        };
        if !matches!(mode.as_str(), "100644" | "100755")
            || opaque(Path::new(path))
            || after.text.is_none()
            || text.crlf_pairs == 0
            || text.lf_hash != after.raw_hash
            || before.bytes.checked_sub(text.crlf_pairs) != Some(after.bytes)
        {
            return Err(format!(
                "{path}: source changed beyond exact CRLF to LF conversion"
            ));
        }
        let attrs = attributes(repo, &index, path, true)?;
        if attrs != attributes(repo, &index, path, false)? {
            return Err(format!(
                "{path}: working attributes differ from the candidate"
            ));
        }
        for name in ["filter", "working-tree-encoding", "ident"] {
            if !matches!(attrs[name].as_str(), "unset" | "unspecified") {
                return Err(format!("{path}: {name} prevents restricted EOL proof"));
            }
        }
        if !matches!(attrs["text"].as_str(), "set" | "auto" | "unspecified")
            || !matches!(
                attrs["eol"].as_str(),
                "lf" | "crlf" | "unset" | "unspecified"
            )
        {
            return Err(format!(
                "{path}: attributes do not permit text EOL conversion"
            ));
        }
        let full = repo.join(crate::workspace_service::worktrees::safe_relative(path)?);
        let open = || std::fs::File::open(&full).map_err(|e| e.to_string());
        let raw_oid = git_text(
            repo,
            None,
            &["hash-object", "--no-filters", "--stdin"],
            Some(Input::File(open()?)),
        )?;
        let clean_oid = git_text(
            repo,
            Some(&index),
            &["hash-object", "--path", path, "--stdin"],
            Some(Input::File(open()?)),
        )?;
        if &raw_oid != oid || &clean_oid != oid || fingerprint(&full)? != *after {
            return Err(format!("{path}: LF bytes or path-clean OID differ from the immutable candidate blob/snapshot"));
        }
        evidence.push(EolNormalization {
            path: path.clone(),
            transformation: "crlf_to_lf_only".into(),
            candidate_tree: tree.into(),
            candidate_blob_oid: oid.clone(),
            candidate_mode: mode.clone(),
            before_raw_blake3: before.raw_hash.clone(),
            before_lf_blake3: text.lf_hash.clone(),
            after_raw_blake3: after.raw_hash.clone(),
            after_raw_blob_oid: raw_oid,
            after_path_clean_oid: clean_oid,
            before_bytes: before.bytes,
            after_bytes: after.bytes,
            replaced_crlf_pairs: text.crlf_pairs,
            effective_attributes: attrs,
        });
    }
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Repo {
        _temporary: tempfile::TempDir,
        root: PathBuf,
        tree: String,
        proof: PathBuf,
    }
    impl Repo {
        fn new(path: &str, bytes: &[u8], attrs: &str, mask: bool) -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let root = temporary.path().join("repo");
            std::fs::create_dir(&root).unwrap();
            for args in [
                vec!["init", "-b", "main"],
                vec!["config", "user.name", "EOL proof"],
                vec!["config", "user.email", "eol@locus.invalid"],
                vec!["config", "core.autocrlf", "true"],
                vec!["config", "commit.gpgsign", "false"],
            ] {
                git(&root, None, &args, None).unwrap();
            }
            if mask {
                git(&root,None,&["config","filter.mask.clean","python -c \"import sys; sys.stdin.buffer.read(); sys.stdout.buffer.write(bytes([102,105,120,101,100,10]))\""],None).unwrap();
            }
            std::fs::write(root.join(".gitattributes"), attrs).unwrap();
            std::fs::write(root.join(path), bytes).unwrap();
            git(&root, None, &["add", "."], None).unwrap();
            git(&root, None, &["commit", "-m", "Candidate"], None).unwrap();
            let tree = git_text(&root, None, &["rev-parse", "HEAD^{tree}"], None).unwrap();
            let proof = temporary.path().join("proof");
            Self {
                _temporary: temporary,
                root,
                tree,
                proof,
            }
        }
        fn snap(&self, path: &str, bytes: &[u8]) -> Snapshot {
            std::fs::write(self.root.join(path), bytes).unwrap();
            [(
                path.into(),
                Some(fingerprint(&self.root.join(path)).unwrap()),
            )]
            .into()
        }
        fn check(
            &self,
            before: &Snapshot,
            after: &Snapshot,
        ) -> Result<Vec<EolNormalization>, String> {
            prove(&self.root, &self.tree, before, after, &self.proof)
        }
    }

    #[test]
    fn streaming_hash_handles_split_crlf_utf8_and_bounded_metadata() {
        struct Chunks {
            bytes: std::io::Cursor<Vec<u8>>,
            maximum: usize,
        }
        impl Read for Chunks {
            fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
                let size = output.len().min(self.maximum);
                self.bytes.read(&mut output[..size])
            }
        }
        let mut data = vec![b'x'; 65535];
        data.extend_from_slice("\r\n中\r单独\n结尾\r".as_bytes());
        let expected = String::from_utf8(data.clone())
            .unwrap()
            .replace("\r\n", "\n");
        for maximum in [1, 2, 3, 65536] {
            let stamp = fingerprint_reader(
                Chunks {
                    bytes: std::io::Cursor::new(data.clone()),
                    maximum,
                },
                true,
            )
            .unwrap();
            assert_eq!(stamp.raw_hash, blake3::hash(&data).to_hex().to_string());
            assert_eq!(
                stamp.text.as_ref().unwrap().lf_hash,
                blake3::hash(expected.as_bytes()).to_hex().to_string()
            );
            assert_eq!(stamp.text.as_ref().unwrap().crlf_pairs, 1);
            assert!(serde_json::to_vec(&stamp).unwrap().len() < 300);
        }
        for bytes in [
            b"invalid\xff\r\n".as_slice(),
            b"binary\0\r\n",
            b"version https://git-lfs.github.com/spec/v1\r\noid sha256:abc\r\n",
            b"incomplete\xe4\xb8",
        ] {
            let stamp = fingerprint_reader(
                Chunks {
                    bytes: std::io::Cursor::new(bytes.to_vec()),
                    maximum: 1,
                },
                true,
            )
            .unwrap();
            assert!(stamp.text.is_none());
        }
    }
    #[test]
    fn exact_eol_proof_preserves_index_and_records_candidate_hashes() {
        let repo = Repo::new("chosen.asset", b"field: 1\nnext: 2\n", "", false);
        let before = repo.snap("chosen.asset", b"field: 1\r\nnext: 2\r\n");
        let after = repo.snap("chosen.asset", b"field: 1\nnext: 2\n");
        let index = std::fs::read(repo.root.join(".git/index")).unwrap();
        let proof = repo.check(&before, &after).unwrap();
        assert_eq!(proof.len(), 1);
        assert_eq!(proof[0].replaced_crlf_pairs, 2);
        assert_eq!(proof[0].before_lf_blake3, proof[0].after_raw_blake3);
        assert_eq!(proof[0].after_raw_blob_oid, proof[0].candidate_blob_oid);
        assert_eq!(proof[0].after_path_clean_oid, proof[0].candidate_blob_oid);
        assert_ne!(
            snapshot_hash(&before).unwrap(),
            snapshot_hash(&after).unwrap()
        );
        assert_eq!(std::fs::read(repo.root.join(".git/index")).unwrap(), index);
        let encoded = serde_json::to_string(&proof).unwrap();
        assert!(encoded.contains("crlf_to_lf_only"));
        assert!(encoded.contains(&repo.tree));
    }
    #[test]
    fn explicit_crlf_attribute_cannot_be_fixed_only_by_disabling_autocrlf() {
        let repo = Repo::new(
            "chosen.asset",
            b"field: 1\n",
            "chosen.asset text eol=crlf\n",
            false,
        );
        let checkout = repo._temporary.path().join("checkout");
        git(
            &repo.root,
            None,
            &[
                "-c",
                "core.autocrlf=false",
                "worktree",
                "add",
                "--detach",
                checkout.to_str().unwrap(),
                "HEAD",
            ],
            None,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(checkout.join("chosen.asset")).unwrap(),
            b"field: 1\r\n"
        );
        let before = repo.snap("chosen.asset", b"field: 1\r\n");
        let after = repo.snap("chosen.asset", b"field: 1\n");
        assert_eq!(
            repo.check(&before, &after).unwrap()[0].effective_attributes["eol"],
            "crlf"
        );
    }
    #[test]
    fn content_space_final_newline_and_lone_cr_changes_are_rejected() {
        let repo = Repo::new("chosen.asset", b"field: 1\n", "", false);
        for (before, after) in [
            (b"field: 1\r\n".as_slice(), b"field: 2\n".as_slice()),
            (b"field: 1\r\n", b"field: 1 \n"),
            (b"field: 1\r\n", b"field: 1"),
            (b"field: 1\r", b"field: 1\n"),
        ] {
            let before = repo.snap("chosen.asset", before);
            let after = repo.snap("chosen.asset", after);
            assert!(repo.check(&before, &after).is_err());
        }
    }
    #[test]
    fn new_ignored_and_missing_sources_are_rejected_even_with_valid_eol_changes() {
        let repo = Repo::new("chosen.asset", b"field: 1\n", "", false);
        let before = repo.snap("chosen.asset", b"field: 1\r\n");
        let mut after = repo.snap("chosen.asset", b"field: 1\n");
        after.extend(repo.snap("ignored-settings.json", b"new ignored output\n"));
        assert!(repo
            .check(&before, &after)
            .unwrap_err()
            .contains("file set changed"));
        assert!(repo.check(&before, &Snapshot::new()).is_err());
        assert!(repo
            .check(&before, &[("chosen.asset".into(), None)].into())
            .unwrap_err()
            .contains("missing"));
        let before = repo.snap("ignored-settings.json", b"setting\r\n");
        let after = repo.snap("ignored-settings.json", b"setting\n");
        assert!(repo
            .check(&before, &after)
            .unwrap_err()
            .contains("not tracked"));
    }
    #[test]
    fn opaque_binary_encoding_and_ident_cannot_use_the_text_exception() {
        for (path, bytes, attrs) in [
            ("model.fbx", b"field: 1\n".as_slice(), ""),
            ("binary.dat", b"field: 1\n", "binary.dat -text\n"),
            ("chosen.txt", b"field: \0\n", ""),
            ("chosen.txt", b"field: \xff\n", ""),
            (
                "chosen.txt",
                b"field: 1\n",
                "chosen.txt working-tree-encoding=UTF-8\n",
            ),
            ("chosen.txt", b"field: 1\n", "chosen.txt ident\n"),
        ] {
            let repo = Repo::new(path, bytes, attrs, false);
            let crlf: Vec<_> = bytes
                .iter()
                .flat_map(|b| {
                    if *b == b'\n' {
                        vec![b'\r', b'\n']
                    } else {
                        vec![*b]
                    }
                })
                .collect();
            let before = repo.snap(path, &crlf);
            let after = repo.snap(path, bytes);
            assert!(repo.check(&before, &after).is_err(), "{path}: {attrs}");
        }
    }
    #[test]
    fn arbitrary_clean_filter_equality_does_not_authorize_a_rewrite() {
        let repo = Repo::new(
            "filtered.txt",
            b"original\n",
            "filtered.txt filter=mask\n",
            true,
        );
        let expected =
            git_text(&repo.root, None, &["rev-parse", "HEAD:filtered.txt"], None).unwrap();
        let misleading = git_text(
            &repo.root,
            None,
            &["hash-object", "--path", "filtered.txt", "--stdin"],
            Some(Input::Bytes(b"entirely different\n")),
        )
        .unwrap();
        assert_eq!(misleading, expected);
        let before = repo.snap("filtered.txt", b"entirely different\r\n");
        let after = repo.snap("filtered.txt", b"entirely different\n");
        assert!(repo.check(&before, &after).unwrap_err().contains("filter"));
    }
    #[test]
    fn immutable_candidate_and_current_disk_are_both_required() {
        let repo = Repo::new("chosen.asset", b"field: 1\n", "", false);
        let before = repo.snap("chosen.asset", b"field: 2\r\n");
        let after = repo.snap("chosen.asset", b"field: 2\n");
        git(&repo.root, None, &["add", "chosen.asset"], None).unwrap();
        git(&repo.root, None, &["commit", "-m", "Later HEAD"], None).unwrap();
        assert!(repo.check(&before, &after).is_err());
        let before = repo.snap("chosen.asset", b"field: 1\r\n");
        let after = repo.snap("chosen.asset", b"field: 1\n");
        assert!(repo.check(&before, &after).is_ok());
        std::fs::write(repo.root.join("chosen.asset"), b"later external edit\n").unwrap();
        assert!(repo.check(&before, &after).is_err());
    }
    #[test]
    fn unchanged_lfs_and_ignored_bytes_keep_their_raw_snapshot_evidence() {
        let repo = Repo::new("chosen.asset", b"field: 1\n", "", false);
        let unchanged = repo.snap("model.fbx", b"hydrated\0binary\r\n");
        assert!(repo.check(&unchanged, &unchanged).unwrap().is_empty());
        assert_eq!(
            snapshot_hash(&unchanged).unwrap(),
            snapshot_hash(&unchanged.clone()).unwrap()
        );
        let changed = repo.snap("model.fbx", b"hydrated\0binary\n");
        assert!(repo.check(&unchanged, &changed).is_err());
    }
}
