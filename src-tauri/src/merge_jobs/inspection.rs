//! Read-only views of immutable job assets. Paths use the merger's own identity
//! rules so inspecting an unchanged field is sufficient to address a typed edit.
use super::*;
use crate::unity_asset_core::{self as core, Asset, Node, NodeKind};

fn child(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}
fn under(path: &str, prefix: &str) -> bool {
    prefix.is_empty() || path == prefix || path.starts_with(&format!("{prefix}/"))
}

struct Page<'a> {
    asset: &'a Asset,
    prefix: &'a str,
    offset: usize,
    limit: usize,
    scalar_limit: usize,
    total: usize,
    fields: Vec<Value>,
    objects: BTreeMap<String, Value>,
}
impl Page<'_> {
    fn walk(&mut self, doc: &core::Document, node: &Node, path: &str, stable: bool) {
        if !under(path, self.prefix) && !under(self.prefix, path) {
            return;
        }
        let keys = if matches!(node.kind, NodeKind::Sequence(_)) {
            core::sequence_keys(self.asset, node, path)
        } else {
            None
        };
        if under(path, self.prefix) {
            if self.total >= self.offset && self.fields.len() < self.limit {
                self.objects.entry(doc.object_id.clone()).or_insert_with(|| json!({
                    "object_id":doc.object_id,"class_id":doc.class_id,"stripped":doc.stripped,
                    "span":doc.span
                }));
                let (kind, count) = match &node.kind {
                    NodeKind::Scalar(_) => ("scalar", 0),
                    NodeKind::Mapping(entries) => ("mapping", entries.len()),
                    NodeKind::Sequence(items) => ("sequence", items.len()),
                };
                let mut field = json!({"object_id":doc.object_id,"property_path":path,
                    "kind":kind,"child_count":count,"span":node.span,
                    "stable_identity_path":stable,"fingerprint":blake3::Hash::from_bytes(node.fingerprint).to_hex().to_string()});
                if let NodeKind::Scalar(style) = &node.kind {
                    let text = self.asset.text(node.span);
                    let mut end = text.len().min(self.scalar_limit);
                    while !text.is_char_boundary(end) {
                        end -= 1;
                    }
                    field["scalar_text"] = json!(&text[..end]);
                    field["scalar_style"] = json!(style);
                    field["scalar_bytes"] = json!(text.len());
                    field["scalar_truncated"] = json!(end < text.len());
                }
                if matches!(node.kind, NodeKind::Sequence(_)) {
                    field["identity_addressed_children"] = json!(keys.is_some());
                }
                self.fields.push(field);
            }
            self.total += 1;
        }
        match &node.kind {
            NodeKind::Mapping(entries) => {
                for entry in entries {
                    self.walk(doc, &entry.value, &child(path, &entry.key), stable);
                }
            }
            NodeKind::Sequence(items) => {
                for (index, item) in items.iter().enumerate() {
                    let key = keys
                        .as_ref()
                        .map(|keys| keys[index].clone())
                        .unwrap_or_else(|| index.to_string());
                    self.walk(
                        doc,
                        &item.value,
                        &child(path, &key),
                        stable && keys.is_some(),
                    );
                }
            }
            NodeKind::Scalar(_) => {}
        }
    }
}

pub(super) fn inspect(dir: &Path, job: &MergeJob, params: &Value) -> Result<Value, String> {
    let path = param_str(params, "path")?;
    // Validate repository-relative syntax, but do not inspect the current disk:
    // a frozen deleted asset remains inspectable after subsequent checkout edits.
    if path.starts_with('/')
        || path.contains('\\')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.eq_ignore_ascii_case(".git"))
        || path.contains(':')
    {
        return Err("inspect_asset path must be a safe repository-relative asset path".into());
    }
    if !scope_allows(job, path) {
        return Err("Asset is outside this merge job's destination project scope".into());
    }
    let version = params
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("target");
    let mut preview_info = Value::Null;
    let mut selected_commit = None;
    let state = match version {
        "target" => job.snapshot.files.get(path).cloned().ok_or("Asset was not part of the frozen target snapshot; prepare a job that includes this asset")?,
        "result" => {
            let preview = preview_job(dir, job)?;
            let state = preview.files.get(path).or_else(||job.snapshot.files.get(path)).cloned().ok_or("Asset is absent from the frozen target and selected result")?;
            preview_info = json!({"plan_hash":preview.plan_hash,"ready_to_apply":preview.ready_to_apply,"issues":preview.issues});
            state
        }
        "source" | "base" => {
            let requested = params.get("commit").and_then(Value::as_str);
            let commits: BTreeMap<_,_> = job.deltas.iter().filter(|d|requested.map(|c|c == d.commit).unwrap_or(true)).map(|d|(d.commit.as_str(),d.parent.as_str())).collect();
            if commits.len() != 1 { return Err("Source/base inspection requires one exact selected commit OID when the job contains multiple commits".into()); }
            let (commit, parent) = commits.into_iter().next().unwrap();
            selected_commit = Some(commit.to_string());
            if let Some(delta) = job.deltas.iter().find(|d|d.path == path && d.commit == commit) {
                if version == "source" { delta.source.clone() } else { delta.base.clone() }
            } else {
                // Read the pinned immutable tree, never a moving branch or source checkout.
                tree_file(Path::new(&job.root), dir, if version == "source" {commit} else {parent}, path)?
            }
        }
        _ => return Err("inspect_asset version must be target, source, base, or result".into()),
    };
    let mut result = json!({"job_id":job.id,"path":path,"version":version,"commit":selected_commit,
        "exists":state.is_some(),"blob":state.as_ref().map(|s|&s.blob),"preview":preview_info,
        "objects":[],"fields":[],"references":[],"total":0,"next_offset":null,"next_reference_offset":null});
    let Some(state) = state else {
        result["kind"] = json!("absent");
        return Ok(result);
    };
    let bytes = read_blob(dir, &state)?;
    if opaque(path) || is_lfs_pointer(&bytes) || bytes.contains(&0) {
        result["kind"] = json!("binary");
        result["structural"] = json!(false);
        return Ok(result);
    }
    if !unity_yaml(path, &bytes) {
        result["kind"] = json!("text");
        result["structural"] = json!(false);
        return Ok(result);
    }
    let asset = match core::parse_shared(&bytes) {
        Ok(asset) => asset,
        Err(error) => {
            result["kind"] = json!("unsupported_unity_yaml");
            result["structural"] = json!(false);
            result["diagnostics"] = json!([error]);
            return Ok(result);
        }
    };
    let object = params.get("object_id").and_then(Value::as_str);
    if object.is_some_and(|id| asset.document(id).is_none()) {
        return Err("Object ID does not exist in this asset version".into());
    }
    let prefix = params
        .get("property_path")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !prefix.is_empty() && !prefix.starts_with('/') {
        return Err("property_path must be an RFC 6901 pointer".into());
    }
    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .clamp(1, 1000) as usize;
    let mut page = Page {
        asset: &asset,
        prefix,
        offset,
        limit,
        scalar_limit: params
            .get("scalar_limit")
            .and_then(Value::as_u64)
            .unwrap_or(4096)
            .clamp(1, 65536) as usize,
        total: 0,
        fields: vec![],
        objects: BTreeMap::new(),
    };
    for doc in &asset.documents {
        if object.map(|id| id == doc.object_id).unwrap_or(true) {
            page.walk(doc, &doc.root, "", true);
        }
    }
    let references: Vec<_> = asset
        .references()
        .into_iter()
        .filter(|reference| {
            object
                .map(|id| id == reference.host_object_id)
                .unwrap_or(true)
        })
        .collect();
    let ref_offset = params
        .get("reference_offset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    result["kind"] = json!("unity_yaml");
    result["structural"] = json!(true);
    result["writable_syntax"] = json!(asset.is_writable());
    result["diagnostics"] = json!(core::validate(&asset));
    result["objects"] = json!(page.objects.into_values().collect::<Vec<_>>());
    result["fields"] = json!(page.fields);
    result["total"] = json!(page.total);
    result["next_offset"] = json!(offset.checked_add(limit).filter(|next| *next < page.total));
    result["references"] = json!(references
        .iter()
        .skip(ref_offset)
        .take(limit)
        .collect::<Vec<_>>());
    result["reference_total"] = json!(references.len());
    result["next_reference_offset"] = json!(ref_offset
        .checked_add(limit)
        .filter(|next| *next < references.len()));
    Ok(result)
}
