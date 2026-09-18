//! Disk transactions are independent of Git and never start or import Unity.
//! Each file replacement is atomic; the durable journal makes a multi-file
//! interruption recoverable without overwriting later external changes.
use super::*;
use crate::merge_jobs::io;
use crate::merge_jobs::FileState;
use crate::unity_asset_core as core;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::{BTreeMap, HashSet};
use std::fs;

#[derive(Serialize, Deserialize)]
struct Journal {
    id: String,
    state: String,
    before: BTreeMap<String, FileState>,
    after: BTreeMap<String, FileState>,
}

fn directory(root: &Path) -> Result<std::path::PathBuf, String> {
    let dir = io::safe_path(root, "Library/Locus/AssetApi")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn persist_journal(dir: &Path, journal: &Journal) -> Result<(), String> {
    io::atomic_json(&dir.join("journal.json"), journal)
}

fn recover(root: &Path, dir: &Path, journal: &mut Journal) -> Result<(), String> {
    for (path, before) in &journal.before {
        let current = io::capture(root, dir, path)?;
        if current.as_ref() == Some(before) {
            continue;
        }
        if current.as_ref() != journal.after.get(path) {
            journal.state = "recovery_required".into();
            persist_journal(dir, journal)?;
            return Err(format!(
                "assets.recovery_required: {path} changed externally; transaction {} preserved",
                journal.id
            ));
        }
        io::write_file(root, dir, path, &Some(before.clone()))?;
    }
    journal.state = "rolled_back".into();
    persist_journal(dir, journal)
}

fn check_pending(base: &Path) -> Result<(), String> {
    for entry in fs::read_dir(base).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let file = entry.path().join("journal.json");
        if !file.is_file() {
            continue;
        }
        let journal: Journal = serde_json::from_slice(&fs::read(file).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if matches!(journal.state.as_str(), "applying" | "recovery_required") {
            return Err(format!(
                "assets.recovery_required: recover transaction {} before further disk writes",
                journal.id
            ));
        }
    }
    Ok(())
}

pub(super) fn execute(root: &Path, request: &Value) -> Result<Value, String> {
    execute_candidates(root, request, None)
}

pub(super) type Candidates = (Vec<(String, Vec<u8>, core::EditOutput)>, BTreeMap<String, Vec<u8>>);

pub(super) fn execute_candidates(root: &Path, request: &Value, candidates: Option<Candidates>) -> Result<Value, String> {
    let action = request["action"].as_str().unwrap_or("");
    if matches!(action, "read" | "discover") {
        let path = asset_path(root, request["path"].as_str().unwrap_or(""), false)?;
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let hints = super::schema::ProjectSchema::load(root)?.scalar_hints(&bytes)?;
        return serde_json::to_value(
            core::inspect_with_hints(&bytes, &hints).map_err(|e| format!("assets.{}", e))?,
        )
        .map_err(|e| e.to_string());
    }
    let base = directory(root)?;
    let lock_path = io::safe_path(root, "Library/Locus/AssetApi/transaction.lock")?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|e| e.to_string())?;
    fs4::FileExt::try_lock(&lock).map_err(|e| format!("assets.busy: {e}"))?;
    if action == "recover" {
        let id = request["transaction_id"]
            .as_str()
            .ok_or("assets.invalid_request: transaction_id is required")?;
        uuid::Uuid::parse_str(id).map_err(|_| "assets.invalid_request: invalid transaction ID")?;
        let dir = io::safe_path(root, &format!("Library/Locus/AssetApi/{id}"))?;
        let mut journal: Journal =
            serde_json::from_slice(&fs::read(dir.join("journal.json")).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if journal.id != id {
            return Err("assets.invalid_journal: identity mismatch".into());
        }
        if journal.state == "committed" {
            return Err("assets.already_committed: create a new edit instead of reverting a committed transaction".into());
        }
        recover(root, &dir, &mut journal)?;
        return Ok(json!({"transaction_id":id,"state":journal.state}));
    }
    check_pending(&base)?;
    let (supplied, dependencies) = candidates.map(|(files,deps)|(Some(files),deps)).unwrap_or_default();
    let check_dependencies = || -> Result<(),String> {
        for (path,bytes) in &dependencies {
            let full=crate::merge_jobs::io::safe_path(root,path)?;
            if fs::read(full).map_err(|e|e.to_string())? != *bytes {return Err(format!("assets.stale_dependency: {path}"));}
        }
        Ok(())
    };
    check_dependencies()?;
    let entries = if supplied.is_some(){vec![]}else{entries(request)?};
    let mut prepared = Vec::new();
    let mut schema = if entries.is_empty() { None } else { Some(super::schema::ProjectSchema::load(root)?) };
    for entry in &entries {
        let schema = schema.as_mut().unwrap();
        let path = entry["path"].as_str().unwrap();
        let full = asset_path(root, path, true)?;
        let before = fs::read(&full).map_err(|e| e.to_string())?;
        let revision = blake3::hash(&before).to_hex().to_string();
        if entry
            .get("expected_revision")
            .and_then(Value::as_str)
            .is_some_and(|expected| expected != revision)
        {
            return Err(format!("assets.stale_revision: {path}"));
        }
        let operations: Vec<core::AssetOperation> =
            serde_json::from_value(entry["operations"].clone())
                .map_err(|e| format!("assets.invalid_operation: {e}"))?;
        let diagnostics = schema.validate(&before, &operations)?;
        let hints = schema.scalar_hints(&before)?;
        let mut output = core::edit_with_hints(&before, &operations, &hints)
            .map_err(|e| format!("assets.{e}"))?;
        // Structural changes create new paths that were absent from the input's
        // per-index hints. The response must describe the committed schema.
        let final_hints = schema.scalar_hints(&output.bytes)?;
        output.snapshot = core::inspect_with_hints(&output.bytes, &final_hints).map_err(|e|e.to_string())?;
        output.snapshot.diagnostics.extend(diagnostics);
        validate_new_external_references(root, &before, &output.bytes)?;
        prepared.push((path.to_string(), before, output));
    }
    if let Some(files)=supplied {
        if files.is_empty()||files.len()>256{return Err("assets.limit: 1 to 256 candidates required".into());}
        let mut paths=HashSet::new();
        for (path,_,_) in &files{
            let key=if cfg!(windows){path.to_ascii_lowercase()}else{path.clone()};
            if !paths.insert(key){return Err("assets.duplicate_asset: candidate paths must be distinct".into());}
        }
        let final_files=files.iter().map(|(path,_,output)|(path.clone(),output.bytes.clone())).collect::<BTreeMap<_,_>>();
        for (path,before,output) in files {
            asset_path(root,&path,true)?;
            validate_external_with_candidates(root,&before,&output.bytes,&final_files)?;
            prepared.push((path,before,output));
        }
    }
    let apply = action.starts_with("apply");
    let mut transaction_id = None;
    let preparing_editor = request["_prepare_editor"].as_bool() == Some(true);
    let mut editor_entries = Vec::new();
    if apply {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = base.join(&id);
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
        let mut journal = Journal {
            id: id.clone(),
            state: "prepared".into(),
            before: BTreeMap::new(),
            after: BTreeMap::new(),
        };
        for (path, before, output) in &prepared {
            let original = io::capture(root, &dir, path)?
                .ok_or("assets.stale_revision: target disappeared")?;
            if original.blob != blake3::hash(before).to_hex().to_string() {
                return Err(format!("assets.stale_revision: {path}"));
            }
            journal.after.insert(
                path.clone(),
                io::store_blob(&dir, &output.bytes, &original.mode)?,
            );
            journal.before.insert(path.clone(), original);
        }
        journal.state = "applying".into();
        persist_journal(&dir, &journal)?;
        if preparing_editor {
            for (path, before, output) in &prepared {
                editor_entries.push(json!({"path":path,"expected_sha256":sha2::Sha256::digest(before).iter().map(|b|format!("{b:02x}")).collect::<String>(),
                    "bytes_base64":base64::engine::general_purpose::STANDARD.encode(&output.bytes)}));
            }
        } else {
            if let Err(error)=check_dependencies(){
                let rollback=recover(root,&dir,&mut journal);
                return Err(format!("{error}; rollback={rollback:?}; transaction_id={id}"));
            }
            for (path, after) in &journal.after {
                let outcome = (|| {
                    if io::capture(root, &dir, path)?.as_ref() != journal.before.get(path) {
                        return Err(format!("assets.stale_revision: {path}"));
                    }
                    if journal.before.get(path)==Some(after){return Ok(());}
                    io::write_file(root, &dir, path, &Some(after.clone()))
                })();
                if let Err(error) = outcome {
                    let rollback = recover(root, &dir, &mut journal);
                    return Err(format!(
                        "{error}; rollback={rollback:?}; transaction_id={id}"
                    ));
                }
            }
            // Read-only inputs must still be current after replacing the targets.
            if let Err(error)=check_dependencies(){
                let rollback=recover(root,&dir,&mut journal);
                return Err(format!("{error}; rollback={rollback:?}; transaction_id={id}"));
            }
            journal.state = "committed".into();
            persist_journal(&dir,&journal).map_err(|e|format!("assets.recovery_required: files written but journal commit failed: {e}; transaction_id={id}"))?;
        }
        transaction_id = Some(id);
    }
    let results = prepared
        .into_iter()
        .map(|(path, before, output)| {
            json!({"path":path,"applied":apply,"persisted":apply,
        "previous_revision":blake3::hash(&before).to_hex().to_string(),"snapshot":output.snapshot,
        "operations_count":output.applied_operations,"diagnostics":output.snapshot.diagnostics})
        })
        .collect::<Vec<_>>();
    let result = if action.ends_with("_batch") {
        json!({"applied":apply,"persisted":apply,"results":results,"transaction_id":transaction_id})
    } else {
        let mut result = results.into_iter().next().unwrap();
        result["transaction_id"] = json!(transaction_id);
        result
    };
    if preparing_editor {
        let dependencies=dependencies.iter().map(|(path,bytes)|json!({"path":path,"expected_sha256":sha2::Sha256::digest(bytes).iter().map(|b|format!("{b:02x}")).collect::<String>()})).collect::<Vec<_>>();
        Ok(json!({"transaction_id":transaction_id,"entries":editor_entries,"dependencies":dependencies,"result":result}))
    } else {
        Ok(result)
    }
}

pub(super) fn prepare_editor(root: &Path, request: &Value) -> Result<Value, String> {
    let mut internal = request.clone();
    internal["_prepare_editor"] = json!(true);
    execute(root, &internal)
}

pub(super) fn complete_editor(root: &Path, id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id).map_err(|_| "assets.invalid_transaction")?;
    let dir = io::safe_path(root, &format!("Library/Locus/AssetApi/{id}"))?;
    let mut journal: Journal =
        serde_json::from_slice(&fs::read(dir.join("journal.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    for (path, expected) in &journal.after {
        if io::capture(root, &dir, path)?.as_ref() != Some(expected) {
            journal.state = "recovery_required".into();
            persist_journal(&dir, &journal)?;
            return Err(format!("assets.import_changed_output: Editor or external writer changed {path}; inspect transaction_id={id}"));
        }
    }
    journal.state = "committed".into();
    persist_journal(&dir, &journal)
}

pub(super) fn confirm_editor_rollback(root: &Path, id: &str) -> Result<(), String> {
    uuid::Uuid::parse_str(id).map_err(|_| "assets.invalid_transaction")?;
    let dir = io::safe_path(root, &format!("Library/Locus/AssetApi/{id}"))?;
    let mut journal: Journal =
        serde_json::from_slice(&fs::read(dir.join("journal.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    for (path, expected) in &journal.before {
        if io::capture(root, &dir, path)?.as_ref() != Some(expected) {
            journal.state = "recovery_required".into();
            persist_journal(&dir, &journal)?;
            return Err(format!("assets.recovery_required: Editor rollback could not be verified for {path}; transaction_id={id}"));
        }
    }
    journal.state = "rolled_back".into();
    persist_journal(&dir, &journal)
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod storage_tests;

fn validate_new_external_references(
    root: &Path,
    before: &[u8],
    after: &[u8],
) -> Result<(), String> {
    validate_external_with_candidates(root,before,after,&BTreeMap::new())
}

fn validate_external_with_candidates(root:&Path,before:&[u8],after:&[u8],final_files:&BTreeMap<String,Vec<u8>>)->Result<(),String>{
    let old = core::parse(before).map_err(|e| e.to_string())?.references();
    let new = core::parse(after).map_err(|e| e.to_string())?.references();
    let key = |reference: &core::AssetReference| {
        (
            reference
                .guid
                .as_ref()
                .map(|guid| guid.to_ascii_lowercase()),
            reference.file_id.clone(),
        )
    };
    let prior: HashSet<_> = old.iter().map(&key).collect();
    let wanted: HashSet<_> = new
        .iter()
        .filter(|r| r.kind == core::ReferenceKind::ExternalObject && !prior.contains(&key(r)))
        .filter_map(|r| r.guid.as_ref())
        .map(|g| g.to_ascii_lowercase())
        .filter(|g| {
            !matches!(
                g.as_str(),
                "0000000000000000e000000000000000" | "0000000000000000f000000000000000"
            )
        })
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    let mut owners: BTreeMap<String, Vec<std::path::PathBuf>> = BTreeMap::new();
    for scope in ["Assets", "Packages"] {
        for entry in walkdir::WalkDir::new(root.join(scope))
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file()
                || entry.path().extension().and_then(|e| e.to_str()) != Some("meta")
            {
                continue;
            }
            let Ok(text) = fs::read_to_string(entry.path()) else {
                continue;
            };
            if let Some(guid) = text
                .lines()
                .find_map(|l| l.strip_prefix("guid:").map(str::trim))
            {
                let guid = guid.to_ascii_lowercase();
                if wanted.contains(&guid) {
                    owners
                        .entry(guid)
                        .or_default()
                        .push(entry.path().with_extension(""));
                }
            }
        }
    }
    for guid in wanted {
        let candidates = owners.get(&guid).ok_or_else(|| {
            format!("assets.missing_reference: GUID {guid} has no asset in this project")
        })?;
        if candidates.len() != 1 {
            return Err(format!(
                "assets.ambiguous_reference: GUID {guid} has multiple owners"
            ));
        }
        if !candidates[0].is_file() {
            return Err(format!(
                "assets.missing_reference: GUID {guid} asset is missing"
            ));
        }
        let relative=candidates[0].strip_prefix(root).map_err(|e|e.to_string())?.to_string_lossy().replace('\\',"/");
        if let Ok(bytes) = final_files.get(&relative).cloned().map(Ok).unwrap_or_else(||fs::read(&candidates[0])) {
            if let Ok(asset) = core::parse(&bytes) {
                if asset.documents.iter().all(|d| d.class_id.is_some()) {
                    for reference in new.iter().filter(|r| {
                        r.guid
                            .as_deref()
                            .is_some_and(|value| value.eq_ignore_ascii_case(&guid))
                            && !prior.contains(&key(r))
                    }) {
                        if let Some(id) = &reference.file_id {
                            let inherited=asset.documents.iter().any(|d|d.class_id.as_deref()==Some("1001"));
                            if id != "0" && asset.document(id).is_none()
                                && !(inherited && super::property::advanced::contains_effective(root,&relative,id,final_files)?) {
                                return Err(format!(
                                    "assets.missing_reference: GUID {guid} has no fileID {id}"
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
