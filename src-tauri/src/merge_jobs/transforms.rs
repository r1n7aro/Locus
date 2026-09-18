use super::*;
use crate::unity_asset_core::{MergeSession, Resolution};

fn frozen_asset_schema(
    dir: &Path,
    job: &MergeJob,
    asset: &[u8],
    operations: &[crate::unity_asset_core::AssetOperation],
) -> Result<crate::unity_assets::schema::ProjectSchema, String> {
    let preview = preview_job(dir, job)?;
    if !preview.ready_to_apply {
        return Err("Resolve the current plan before validating its asset schema".into());
    }
    let mut states = snapshot::all_files(job);
    states.extend(preview.files);
    prefetch_git(dir, states.iter().filter(|(p,_)|p.ends_with(".cs") || p.ends_with(".meta"))
        .filter_map(|(_,state)|state.as_ref()))?;
    let mut files = BTreeMap::new();
    let mut wanted_guids: BTreeSet<String> = crate::unity_asset_core::parse(asset)
        .map_err(|error| error.to_string())?
        .references()
        .iter()
        .filter_map(|reference| reference.guid.clone())
        .collect();
    fn collect_guids(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if let Some(guid) = map.get("guid").and_then(Value::as_str) {
                    out.insert(guid.to_string());
                }
                for value in map.values() {
                    collect_guids(value, out);
                }
            }
            Value::Array(values) => {
                for value in values {
                    collect_guids(value, out);
                }
            }
            _ => {}
        }
    }
    collect_guids(
        &serde_json::to_value(operations).map_err(|error| error.to_string())?,
        &mut wanted_guids,
    );
    let mut referenced_paths = Vec::new();
    for (path, state) in &states {
        if !path_in_project(Path::new(&job.root), Path::new(&job.project_root), path)
            || state.is_none()
            || !(path.ends_with(".cs") || path.ends_with(".meta"))
        {
            continue;
        }
        let bytes = state_bytes(dir, state)?;
        if path.ends_with(".meta") {
            if let Ok(text) = std::str::from_utf8(&bytes) {
                if text
                    .lines()
                    .find_map(|line| line.strip_prefix("guid:").map(str::trim))
                    .is_some_and(|guid| wanted_guids.contains(guid))
                {
                    referenced_paths.push(path.trim_end_matches(".meta").to_string());
                }
            }
        }
        files.insert(path.clone(), bytes);
    }
    for path in referenced_paths {
        if let Some(state) = states.get(&path).filter(|state| state.is_some()) {
            if !files.contains_key(&path) {
                files.insert(path, state_bytes(dir, state)?);
            }
        }
    }
    Ok(crate::unity_assets::schema::ProjectSchema::from_frozen(
        files,
    ))
}

fn asset_bytes(dir: &Path, job: &MergeJob, path: &str) -> Result<Vec<u8>, String> {
    if !scope_allows(job, path) {
        return Err("Asset escapes the destination project root".into());
    }
    if !job.snapshot.files.contains_key(path) {
        return Err(
            "Asset is not in the frozen target snapshot; prepare a new job containing it".into(),
        );
    }
    let preview = preview_job(dir, job)?;
    if !preview.ready_to_apply {
        return Err(
            "Resolve the current plan before inspecting or editing its asset result".into(),
        );
    }
    let state = preview
        .files
        .get(path)
        .or_else(|| job.snapshot.files.get(path))
        .cloned()
        .flatten()
        .ok_or("Asset is absent from the current plan result")?;
    state_bytes(dir, &Some(state))
}

/// The merge adapter shares the exact asset snapshot contract; all reads are
/// against captured blobs and the current plan, never the mutable worktree.
pub fn read_asset(dir: &Path, job: &MergeJob, params: &Value) -> Result<Value, String> {
    let bytes = asset_bytes(dir, job, param_str(params, "path")?)?;
    let mut schema = frozen_asset_schema(dir, job, &bytes, &[])?;
    let hints = schema.scalar_hints(&bytes)?;
    let snapshot = crate::unity_asset_core::inspect_with_hints(&bytes, &hints)
        .map_err(|error| error.to_string())?;
    serde_json::to_value(snapshot).map_err(|error| error.to_string())
}

/// Build a private candidate first, then reduce sequential operations to
/// deterministic final Set decisions. Existing source/object/file overlap
/// validation remains authoritative and no new persisted job schema is needed.
pub fn edit_asset(
    dir: &Path,
    job: &mut MergeJob,
    params: &Value,
    apply: bool,
) -> Result<Value, String> {
    if params
        .get("persist")
        .is_some_and(|value| value.as_str() != Some("plan"))
    {
        return Err("Merge asset edits require persist='plan'; materialize with the outer merge plan apply operation".into());
    }
    let path = param_str(params, "path")?;
    let expected = params
        .get("expected_revision")
        .map(|value| value.as_str().ok_or("expected_revision must be a string"))
        .transpose()?;
    if apply && expected.is_none() {
        return Err("expected_revision is required when staging asset edits".into());
    }
    let operations: Vec<crate::unity_asset_core::AssetOperation> = serde_json::from_value(
        params
            .get("operations")
            .cloned()
            .ok_or("operations is required")?,
    )
    .map_err(|error| format!("Invalid asset operations: {error}"))?;
    let bytes = asset_bytes(dir, job, path)?;
    let revision = blake3::hash(&bytes).to_hex().to_string();
    if expected.is_some_and(|expected| expected != revision) {
        return Err("stale_revision: merge asset result changed since it was read".into());
    }
    let mut schema = frozen_asset_schema(dir, job, &bytes, &operations)?;
    let schema_diagnostics = schema.validate(&bytes, &operations)?;
    let hints = schema.scalar_hints(&bytes)?;
    let output = crate::unity_asset_core::edit_with_hints(&bytes, &operations, &hints)
        .map_err(|error| error.to_string())?;
    let raw_snapshot =
        crate::unity_asset_core::inspect(&bytes).map_err(|error| error.to_string())?;
    let packed_fields: BTreeMap<(String, String), crate::unity_asset_core::PackedElement> =
        raw_snapshot
            .objects
            .iter()
            .flat_map(|object| {
                let hints = &hints;
                object.fields.iter().filter_map(move |field| {
                    if field.kind == "array" {
                        return None;
                    }
                    match hints
                        .get(&object.object_id)
                        .and_then(|fields| fields.get(&field.property_path))
                    {
                        Some(crate::unity_asset_core::ScalarHint::PackedArray(element)) => Some((
                            (object.object_id.clone(), field.property_path.clone()),
                            *element,
                        )),
                        _ => None,
                    }
                })
            })
            .collect();
    let mut touched: Vec<(String, String)> = operations
        .iter()
        .map(|operation| {
            let path = packed_fields
                .keys()
                .find(|(object, path)| {
                    object == operation.object_id()
                        && (path == operation.property_path()
                            || operation
                                .property_path()
                                .strip_prefix(path)
                                .is_some_and(|tail| tail.starts_with('/')))
                })
                .map(|(_, path)| path.as_str())
                .unwrap_or(operation.property_path());
            (operation.object_id().into(), path.into())
        })
        .collect();
    touched.sort();
    touched.dedup();
    let scopes: Vec<_> = touched
        .iter()
        .filter(|(object, property)| {
            !touched.iter().any(|(other_object, other)| {
                other_object == object
                    && other != property
                    && property
                        .strip_prefix(other)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        })
        .cloned()
        .collect();
    let mut candidate = job.clone();
    for (object_id, property_path) in scopes {
        let field = output
            .snapshot
            .objects
            .iter()
            .find(|object| object.object_id == object_id)
            .and_then(|object| {
                object
                    .fields
                    .iter()
                    .find(|field| field.property_path == property_path)
            })
            .ok_or_else(|| {
                format!("Edited field {object_id}:{property_path} has no final value")
            })?;
        let mut value = field.value.clone();
        for ((packed_object, packed_path), element) in &packed_fields {
            if packed_object != &object_id {
                continue;
            }
            let Some(relative) = packed_path
                .strip_prefix(&property_path)
                .filter(|tail| tail.is_empty() || tail.starts_with('/'))
            else {
                continue;
            };
            if let Some(target) = value.pointer_mut(relative) {
                if let Some(values) = target.as_array() {
                    *target = crate::unity_asset_core::packed_array_value(*element, values)
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        let value = crate::unity_asset_core::decode_asset_value(&value)
            .map_err(|error| error.to_string())?;
        // An existing exact source-field decision is replaced just as with
        // fields.set; ancestor/source/object/file conflicts still fail preview.
        set(
            &mut candidate,
            "fields.set",
            &json!({"path":path,"object_id":object_id,
            "property_path":property_path,"value":value}),
        )?;
    }
    let preview = preview_job(dir, &candidate)?;
    if !preview.ready_to_apply {
        return Err(format!(
            "asset_plan_conflict: {}",
            serde_json::to_string(&preview.issues).map_err(|error| error.to_string())?
        ));
    }
    // Verify decisions reconstruct the same complete value result before they
    // are persisted. This catches provenance/order interactions in a merge.
    let rendered = asset_bytes(dir, &candidate, path)?;
    let mut final_snapshot = crate::unity_asset_core::inspect_with_hints(&rendered, &hints)
        .map_err(|error| error.to_string())?;
    if serde_json::to_value(&final_snapshot.objects).map_err(|error| error.to_string())?
        != serde_json::to_value(&output.snapshot.objects).map_err(|error| error.to_string())?
    {
        return Err(
            "asset_plan_conflict: persisted selections do not reproduce the requested asset result"
                .into(),
        );
    }
    if apply {
        job.selection = candidate.selection;
    }
    final_snapshot.diagnostics.extend(schema_diagnostics);
    Ok(
        json!({"ok":true,"backend":"yaml","destination":"merge_plan","applied":apply,
        "persisted":false,"operations_count":output.applied_operations,"snapshot":final_snapshot,
        "previous_revision":revision,"revision":final_snapshot.revision,"diagnostics":final_snapshot.diagnostics}),
    )
}

fn overlaps(a: &str, b: &str) -> bool {
    a == b
        || a.strip_prefix(b).is_some_and(|v| v.starts_with('/'))
        || b.strip_prefix(a).is_some_and(|v| v.starts_with('/'))
}
pub fn apply(
    dir: &Path,
    job: &MergeJob,
    result: &mut BTreeMap<String, Option<FileState>>,
    issues: &mut Vec<Value>,
) -> Result<(), String> {
    let mut groups: BTreeMap<(String, Option<String>), Vec<&FieldChoice>> = BTreeMap::new();
    for choice in job.selection.fields.values() {
        groups
            .entry((choice.path.clone(), choice.commit.clone()))
            .or_default()
            .push(choice);
    }
    for ((path, commit), choices) in groups {
        if !scope_allows(job, &path) {
            issues.push(error_value(
                "outside_destination_scope",
                &path,
                "Field operation escapes the destination project root",
            ));
            continue;
        }
        let conflict = job.selection.files.contains_key(&path)
            || choices.iter().any(|choice| {
                job.selection
                    .objects
                    .values()
                    .any(|object| object.path == path && object.object_id == choice.object_id)
                    || job.selection.fields.values().any(|other| {
                        other.path == path
                            && other.object_id == choice.object_id
                            && other.property_path != choice.property_path
                            && overlaps(&other.property_path, &choice.property_path)
                    })
                    || job
                        .deltas
                        .iter()
                        .filter(|d| d.path == path)
                        .flat_map(|d| &d.changes)
                        .any(|change| {
                            change["object_id"].as_str() == Some(&choice.object_id)
                                && change["property_path"]
                                    .as_str()
                                    .map(|p| overlaps(p, &choice.property_path))
                                    .unwrap_or(false)
                                && job
                                    .selection
                                    .decisions
                                    .get(change["id"].as_str().unwrap_or(""))
                                    .map(|v| {
                                        !matches!(
                                            resolution_name(v),
                                            "exclude" | "defer" | "target"
                                        )
                                    })
                                    .unwrap_or(false)
                        })
            });
        if conflict {
            issues.push(error_value("overlapping_selection",&path,"Explicit field transformations overlap a selected source, object, file, or ancestor field operation"));
            continue;
        }
        let target = result.get(&path).cloned().unwrap_or(None);
        let current = state_bytes(dir, &target)?;
        let (base, source) = if let Some(commit) = commit {
            let delta = job
                .deltas
                .iter()
                .find(|d| d.path == path && d.commit == commit)
                .ok_or("Field take refers to an unselected source file version")?;
            (
                state_bytes(dir, &delta.base)?,
                state_bytes(dir, &delta.source)?,
            )
        } else {
            (current.clone(), current.clone())
        };
        let session = match crate::unity_asset_core::prepare_merge(&base, &current, &source) {
            Ok(session) => session,
            Err(error) => {
                issues.push(error_value(
                    "field_transform_parse",
                    &path,
                    error.to_string(),
                ));
                continue;
            }
        };
        let decisions = choices
            .iter()
            .map(|choice| {
                let resolution: Resolution =
                    serde_json::from_value(choice.resolution.clone()).map_err(|e| e.to_string())?;
                Ok(MergeSession::field_decision(
                    &choice.object_id,
                    &choice.property_path,
                    resolution,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        match session.render(&decisions){
            Ok(output)if output.ready=>{result.insert(path,Some(store_blob(dir,&output.bytes,target.as_ref().map(|s|s.mode.as_str()).unwrap_or("100644"))?));},
            Ok(output)=>issues.push(json!({"code":"field_transform_conflict","path":path,"conflicts":output.conflicts,"diagnostics":output.diagnostics})),
            Err(error)=>issues.push(error_value("field_transform",&path,error.to_string())),
        }
    }
    Ok(())
}

pub fn set(job: &mut MergeJob, action: &str, params: &Value) -> Result<(), String> {
    let path = param_str(params, "path")?;
    let object = param_str(params, "object_id")?;
    let property = param_str(params, "property_path")?;
    if !property.starts_with('/') {
        return Err("property_path must be a JSON pointer".into());
    }
    if !job.snapshot.files.contains_key(path) {
        return Err(
            "This asset is not in the frozen target snapshot; prepare a new job containing it"
                .into(),
        );
    }
    let commit = params
        .get("commit")
        .and_then(Value::as_str)
        .map(str::to_string);
    let matching: Vec<_> = job
        .deltas
        .iter()
        .filter(|d| d.path == path && commit.as_ref().map(|c| c == &d.commit).unwrap_or(true))
        .flat_map(|d| &d.changes)
        .filter(|change| {
            change["object_id"].as_str() == Some(object)
                && change["property_path"].as_str() == Some(property)
        })
        .filter_map(|change| change["id"].as_str().map(str::to_string))
        .collect();
    let resolution = match action {
        "fields.set" => {
            json!({"kind":"set","value":params.get("value").ok_or("A typed value is required")?})
        }
        "fields.delete" => json!({"kind":"delete"}),
        "fields.take" => json!({"kind":param_str(params,"side")?}),
        _ => return Err("Unsupported field transform".into()),
    };
    // Replacing the decision for one known field is a conflict resolution. A
    // field absent from the source catalog is an explicit target transformation.
    if matching.len() == 1 {
        job.selection
            .decisions
            .insert(matching[0].clone(), resolution);
        return Ok(());
    }
    if matching.len() > 1 {
        return Err("The field has changes in multiple selected commits; specify its commit to resolve provenance".into());
    }
    let commit = if action == "fields.take" {
        let candidates: Vec<_> = job
            .deltas
            .iter()
            .filter(|d| d.path == path && commit.as_ref().map(|c| c == &d.commit).unwrap_or(true))
            .collect();
        if candidates.len() != 1 {
            return Err("Field take requires one selected source commit version".into());
        }
        Some(candidates[0].commit.clone())
    } else {
        None
    };
    let choice = FieldChoice {
        path: path.into(),
        object_id: object.into(),
        property_path: property.into(),
        commit,
        resolution,
        group: None,
    };
    job.selection
        .fields
        .insert(format!("{path}:{object}:{property}"), choice);
    Ok(())
}

pub fn move_object(dir: &Path, job: &mut MergeJob, params: &Value) -> Result<(), String> {
    let path = param_str(params, "path")?;
    let child = param_str(params, "object_id")?;
    let parent = param_str(params, "parent_id")?;
    let preview = preview_job(dir, job)?;
    if !preview.ready_to_apply {
        return Err("Resolve the current plan before reparenting its result".into());
    }
    let state = preview
        .files
        .get(path)
        .or_else(|| job.snapshot.files.get(path))
        .cloned()
        .unwrap_or(None);
    let bytes = state_bytes(dir, &state)?;
    let asset = crate::unity_asset_core::parse(&bytes).map_err(|e| e.to_string())?;
    fn transform<'a>(
        asset: &'a crate::unity_asset_core::Asset,
        id: &str,
    ) -> Result<(&'static str, &'a crate::unity_asset_core::Node), String> {
        let document = asset
            .document(id)
            .ok_or_else(|| format!("Transform fileID {id} does not exist"))?;
        if let Some(node) = document.root.get("Transform") {
            Ok(("Transform", node))
        } else if let Some(node) = document.root.get("RectTransform") {
            Ok(("RectTransform", node))
        } else {
            Err(
                "objects.move expects a Transform/RectTransform fileID, not its GameObject name/ID"
                    .into(),
            )
        }
    }
    fn ids(
        asset: &crate::unity_asset_core::Asset,
        node: &crate::unity_asset_core::Node,
    ) -> Result<Vec<String>, String> {
        node.get("m_Children")
            .and_then(|n| n.items())
            .ok_or("Transform children are not a sequence")?
            .iter()
            .map(|item| {
                item.value
                    .get("fileID")
                    .and_then(|n| n.scalar(asset))
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| "Child lacks a fileID".into())
            })
            .collect()
    }
    fn pptr(id: &str) -> Result<Value, String> {
        Ok(
            json!({"fileID":id.parse::<i64>().map_err(|_|"fileID must be an exact signed 64-bit integer string")?}),
        )
    }
    let (child_type, child_node) = transform(&asset, child)?;
    let old_parent = child_node
        .get("m_Father")
        .and_then(|n| n.get("fileID"))
        .and_then(|n| n.scalar(&asset))
        .ok_or("Transform has no parent reference")?
        .trim();
    let group = uuid::Uuid::new_v4().to_string();
    let mut updates = vec![(
        child.to_string(),
        format!("/{child_type}/m_Father"),
        pptr(parent)?,
    )];
    let mut parents: BTreeSet<String> = [old_parent.to_string(), parent.to_string()]
        .into_iter()
        .filter(|id| id != "0")
        .collect();
    for parent_id in std::mem::take(&mut parents) {
        let (kind, node) = transform(&asset, &parent_id)?;
        let mut children = ids(&asset, node)?;
        children.retain(|id| id != child);
        if parent_id == parent {
            let position = params
                .get("position")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(children.len());
            if position > children.len() {
                return Err("Reparent position exceeds the destination child count".into());
            }
            children.insert(position, child.to_string());
        }
        updates.push((
            parent_id,
            format!("/{kind}/m_Children"),
            Value::Array(
                children
                    .iter()
                    .map(|id| pptr(id))
                    .collect::<Result<_, _>>()?,
            ),
        ));
    }
    for (object_id, property_path, value) in updates {
        let choice = FieldChoice {
            path: path.into(),
            object_id: object_id.clone(),
            property_path: property_path.clone(),
            commit: None,
            resolution: json!({"kind":"set","value":value}),
            group: Some(group.clone()),
        };
        job.selection
            .fields
            .insert(format!("{path}:{object_id}:{property_path}"), choice);
    }
    Ok(())
}
