//! One asset API for immutable Unity YAML and the live Editor. SDK/IPC adapters
//! only select a checkout; neither backend is permitted to launch an Editor.
mod storage;
pub(crate) mod schema;
mod property;
pub(crate) fn prefab_property_projection(root:&std::path::Path,path:&str,bytes:&[u8])->Result<(String,std::collections::BTreeMap<std::path::PathBuf,String>,std::collections::HashMap<String,String>),String>{
    property::advanced::projection(root,path,bytes)
}

/// Trusted semantic compiler output enters exactly the same journal/Editor
/// transaction as raw edits. Public requests cannot inject candidate bytes.
async fn commit_candidates(root:&Path,candidates:storage::Candidates)->Result<serde_json::Value,String>{
    let connected=crate::unity_bridge::is_unity_connected(&root.to_string_lossy()).await;
    if !connected {require_closed_editor(root).await?;}
    let prepare_root=root.to_path_buf();
    let prepared=tokio::task::spawn_blocking(move||storage::execute_candidates(&prepare_root,&serde_json::json!({"action":"apply_batch","_prepare_editor":connected}),Some(candidates))).await.map_err(|e|e.to_string())??;
    if !connected{return Ok(prepared);}
    let id=prepared["transaction_id"].as_str().ok_or("assets.invalid_transaction")?;
    match crate::unity_bridge::asset_api(&root.to_string_lossy(),&serde_json::json!({"action":"disk_apply","transaction_id":id,"entries":prepared["entries"],"dependencies":prepared["dependencies"]})).await {
        Ok(response)=>{storage::complete_editor(root,id)?;let mut result=prepared["result"].clone();if let Some(timings)=response.get("timings"){result["editorTimings"]=timings.clone();}Ok(result)},
        Err(error)=>{
            if !error.contains("outcome_unknown") {let rollback=storage::confirm_editor_rollback(root,id);return Err(format!("{error}; rollback={rollback:?}; transaction_id={id}"));}
            Err(format!("{error}; transaction_id={id}"))
        }
    }
}

use serde_json::{json, Value};
use std::path::Path;

const OPERATIONS: &[&str] = &["set", "array_insert", "array_remove", "array_move", "array_resize"];

pub fn capabilities(backend: &str) -> Value {
    json!({"backend":backend,"supported_operations":OPERATIONS,"atomicity":"rollback_on_failure",
        "multi_file_atomic_visibility":false,"crash_recovery":backend=="yaml",
        "durability":if backend=="yaml" {"journaled_per_file_replacement"} else {"editor_save_with_undo"},
        "source_schema":{"scopes":["Assets","embedded Packages"],"registry_package_cache":false,"unresolved_types":"diagnostic","packed_yaml_arrays_require_schema":true},
        "persist_modes":["disk"],"starts_editor":false,
        "read_source":if backend=="yaml" {"disk"} else {"editor"},
        "supported_extensions":["asset","prefab","unity","mat","anim","controller","overridecontroller","playable","mask"],
        "validation_level":"structural", "requires_closed_editor":false,"coordinates_editor":backend=="yaml",
        "prefab_inheritance":{"effective_fields":false,"write_overrides":false,"nested_instances":false},
        "logical_property_api":{"read_action":"read_property","discover_action":"discover_property","apply_action":"apply_properties",
            "prefab_effective_fields":backend=="yaml","prefab_scalar_override_revert":backend=="yaml","prefab_apply_to_source":backend=="yaml",
            "prefab_array_override_revert_apply":backend=="yaml","prefab_added_removed_topology_projection":backend=="yaml","prefab_managed_leaf_overrides":backend=="yaml",
            "managed_creation":"explicit_complete_source_verified_template","structure_creation":"materialized_gameobject_transform_monobehaviour_templates",
            "unsupported":["inherited_topology_authoring","inherited_managed_creation","inherited_managed_array_structure","apply_local_reference_remapping","constructor_execution","native_undo","nonfinite_curve_tangents"]},
        "unsupported_capabilities":["prefab_inherited_field_projection","prefab_override_authoring","nested_prefab_topology"]})
}

pub fn asset_path(root: &Path, path: &str, _write: bool) -> Result<std::path::PathBuf, String> {
    if !path.starts_with("Assets/") {
        return Err("assets.path_scope: use an Assets/ path".into());
    }
    let full = crate::merge_jobs::io::safe_path(root, path)?;
    let extension = full.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["asset", "prefab", "unity", "mat", "anim", "controller", "overridecontroller", "playable", "mask"].contains(&extension.as_str()) {
        return Err("assets.unsupported_capability: asset is not a supported Unity text asset".into());
    }
    if !full.is_file() { return Err(format!("assets.not_found: {path}")); }
    Ok(full)
}

pub fn entries(request: &Value) -> Result<Vec<Value>, String> {
    let action = request["action"].as_str().unwrap_or("");
    let entries = if action.ends_with("_batch") {
        request["entries"].as_array().cloned().ok_or("assets.invalid_request: entries must be an array")?
    } else { vec![request.clone()] };
    if entries.is_empty() || entries.len() > 256 {
        return Err("assets.limit: a transaction requires 1 to 256 assets".into());
    }
    let mut paths = std::collections::HashSet::new();
    let mut count = 0;
    for entry in &entries {
        let path = entry["path"].as_str().ok_or("assets.invalid_request: path is required")?;
        // Windows path aliases must never cause two entries to edit the same file.
        let key = if cfg!(windows) { path.to_ascii_lowercase() } else { path.to_string() };
        if !paths.insert(key) { return Err("assets.duplicate_asset: combine operations for the same path".into()); }
        let operations = entry["operations"].as_array().ok_or("assets.invalid_request: operations must be an array")?;
        if operations.iter().any(|operation| operation.get("size").and_then(Value::as_u64).is_some_and(|size|size>1_000_000)) {
            return Err("assets.limit: an array may contain at most 1000000 elements".into());
        }
        let _: Vec<crate::unity_asset_core::AssetOperation> = serde_json::from_value(entry["operations"].clone())
            .map_err(|e|format!("assets.invalid_operation: {e}"))?;
        if operations.is_empty() { return Err("assets.invalid_request: operations must not be empty".into()); }
        count += operations.len();
        if action.starts_with("apply") && entry["expected_revision"].as_str().is_none_or(str::is_empty) {
            return Err("assets.revision_required: apply requires the revision returned by read".into());
        }
    }
    if count > 10_000 { return Err("assets.limit: a transaction supports at most 10000 operations".into()); }
    Ok(entries)
}

pub async fn require_closed_editor(root: &Path) -> Result<(), String> {
    if !root.join("ProjectSettings/ProjectVersion.txt").is_file() { return Ok(()); }
    let project = root.to_string_lossy().into_owned();
    let process = tokio::task::spawn_blocking(move || crate::unity_bridge::query_current_project_editor_process_uncached(project))
        .await.map_err(|e| e.to_string())?;
    match process.state {
        crate::unity_bridge::UnityEditorProcessState::NotRunning => Ok(()),
        crate::unity_bridge::UnityEditorProcessState::Running => Err("assets.editor_owns_project: connect this project's Editor to Locus for coordinated YAML writes, or close the Editor".into()),
        _ => Err(format!("assets.editor_state_unknown: cannot prove that the Editor is closed: {:?}", process.last_error)),
    }
}

/// Public service boundary used identically by the Python HTTP SDK, TypeScript
/// IPC and the CLI acceptance driver. Callers hold the checkout mutation gate.
pub async fn execute(root: &Path, mut request: Value) -> Result<Value, String> {
    if request.as_object().is_none_or(|map|map.keys().any(|key|key.starts_with('_'))) {
        return Err("assets.invalid_request: internal request fields are not public API".into());
    }
    let backend = request.get("backend").and_then(Value::as_str).unwrap_or("yaml").to_owned();
    if !matches!(backend.as_str(), "yaml" | "live") { return Err("assets.invalid_backend: expected yaml or live".into()); }
    let action = request["action"].as_str().ok_or("assets.invalid_request: action is required")?.to_owned();
    if matches!(action.as_str(), "read_property" | "discover_property" | "apply_properties") {
        return property::execute(root, request).await;
    }
    if action == "capabilities" { return Ok(capabilities(&backend)); }
    if !matches!(action.as_str(), "read" | "discover" | "preview" | "apply" | "preview_batch" | "apply_batch" | "recover") {
        return Err(format!("assets.invalid_action: {action}"));
    }
    if request.get("persist").and_then(Value::as_str).is_some_and(|v| v != "disk") {
        return Err("assets.unsupported_capability: apply persists to disk; use preview for a non-mutating result".into());
    }
    if request.get("persist").is_some_and(|value| !value.is_string()) {
        return Err("assets.invalid_request: persist must be disk".into());
    }
    let mutating = action.starts_with("apply") || action == "recover";
    let mut materialized_ids=None;
    let mut materialized_edits=std::collections::BTreeMap::new();
    let inspect_scope=|path:&str,operations:Option<&Value>| -> Result<Option<std::collections::HashSet<String>>,String> {
        let bytes=std::fs::read(asset_path(root,path,false)?).map_err(|e|e.to_string())?;
        let parsed=crate::unity_asset_core::parse_shared(&bytes).map_err(|e|format!("assets.{e}"))?;
        if !parsed.documents.iter().any(|doc|doc.stripped||doc.class_id.as_deref()==Some("1001")){return Ok(None);}
        let ids=parsed.documents.iter().filter(|doc|!doc.stripped&&doc.class_id.as_deref()!=Some("1001"))
            .map(|doc|doc.object_id.clone()).collect::<std::collections::HashSet<_>>();
        if ids.is_empty() || operations.and_then(Value::as_array).is_some_and(|ops|ops.iter().any(|op|op["object_id"].as_str().is_none_or(|id|!ids.contains(id)))) {
            return Err("assets.unsupported_capability: inherited Prefab fields require an effective-value adapter".into());
        }
        Ok(Some(ids))
    };
    if matches!(action.as_str(), "read" | "discover") {
        asset_path(root, request["path"].as_str().ok_or("assets.invalid_request: path is required")?, false)?;
        materialized_ids=inspect_scope(request["path"].as_str().unwrap(),None)?;
    } else if action != "recover" {
        for entry in entries(&request)? {
            asset_path(root, entry["path"].as_str().unwrap(), true)?;
            if let Some(ids)=inspect_scope(entry["path"].as_str().unwrap(),entry.get("operations"))? {
                materialized_edits.insert(entry["path"].as_str().unwrap().to_string(),ids);
            }
        }
    }
    let coordinate_editor = if backend=="yaml" && mutating {
        if crate::unity_bridge::is_unity_connected(&root.to_string_lossy()).await {true}
        else {require_closed_editor(root).await?;false}
    } else {false};
    let mut result = if backend == "yaml" {
        let root = root.to_path_buf();
        let input = request.clone();
        if coordinate_editor && action.starts_with("apply") {
            let prepare_root=root.clone();
            let prepared=tokio::task::spawn_blocking(move ||storage::prepare_editor(&prepare_root,&input)).await.map_err(|e|e.to_string())??;
            let id=prepared["transaction_id"].as_str().ok_or("assets.invalid_transaction")?.to_owned();
            let response=crate::unity_bridge::asset_api(&root.to_string_lossy(),&json!({"action":"disk_apply","transaction_id":id,"entries":prepared["entries"]})).await;
            match response {
                Ok(response) => {
                    let finish_root=root.clone();let finish_id=id.clone();
                    tokio::task::spawn_blocking(move||storage::complete_editor(&finish_root,&finish_id)).await.map_err(|e|e.to_string())??;
                    let mut result=prepared["result"].clone();if let Some(timings)=response.get("timings"){result["editorTimings"]=timings.clone();}result
                },
                Err(error) => {
                    if !error.contains("outcome_unknown") {
                        let recovery_root=root.clone();let recovery_id=id.clone();
                        let rollback=tokio::task::spawn_blocking(move||storage::confirm_editor_rollback(&recovery_root,&recovery_id)).await.map_err(|e|e.to_string())?;
                        return Err(format!("{error}; rollback={rollback:?}; transaction_id={id}"));
                    }
                    return Err(format!("{error}; transaction_id={id}"));
                }
            }
        } else if coordinate_editor && action=="recover" {
            return Err("assets.editor_owns_project: close Editor before journal recovery".into());
        } else {
            tokio::task::spawn_blocking(move || storage::execute(&root, &input)).await.map_err(|e|e.to_string())??
        }
    } else {
        if action == "recover" { return Err("assets.unsupported_capability: YAML journals use backend='yaml'".into()); }
        let schema_diagnostics=if matches!(action.as_str(),"preview"|"apply"|"preview_batch"|"apply_batch") {
            let root=root.to_path_buf();let input=request.clone();
            tokio::task::spawn_blocking(move|| -> Result<std::collections::BTreeMap<String,Vec<Value>>,String> {
                let mut schema=schema::ProjectSchema::load(&root)?;
                let mut reports=std::collections::BTreeMap::new();
                for entry in entries(&input)? {
                    let path=asset_path(&root,entry["path"].as_str().unwrap(),true)?;
                    let bytes=std::fs::read(path).map_err(|e|e.to_string())?;
                    let operations:Vec<crate::unity_asset_core::AssetOperation>=serde_json::from_value(entry["operations"].clone()).map_err(|e|e.to_string())?;
                    let diagnostics=schema.validate(&bytes,&operations)?.into_iter().map(serde_json::to_value)
                        .collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
                    reports.insert(entry["path"].as_str().unwrap().to_string(),diagnostics);
                }
                Ok(reports)
            }).await.map_err(|e|e.to_string())??
        } else {std::collections::BTreeMap::new()};
        if action == "discover" { request["action"] = json!("read"); }
        // Internal C# transport encodes arbitrary operation values once as JSON;
        // this does not alter the public, language-neutral operation contract.
        fn encode_operations(entry: &mut Value) -> Result<(), String> {
            if let Some(operations) = entry.get_mut("operations").and_then(Value::as_array_mut) {
                for operation in operations {
                    if let Some(value) = operation.as_object_mut().and_then(|v| v.remove("value")) {
                        operation["value_json"] = json!(serde_json::to_string(&value).map_err(|e| e.to_string())?);
                    }
                }
            }
            Ok(())
        }
        encode_operations(&mut request)?;
        if let Some(entries) = request.get_mut("entries").and_then(Value::as_array_mut) { for entry in entries { encode_operations(entry)?; } }
        let mut response=crate::unity_bridge::asset_api(&root.to_string_lossy(), &request).await?;
        if !schema_diagnostics.is_empty() { merge_live_schema_diagnostics(&mut response,&schema_diagnostics); }
        response
    };
    if matches!(action.as_str(), "read" | "discover") {
        if let Some(ids)=materialized_ids {
            filter_materialized_snapshot(&mut result,&ids);
        }
        result = filter_snapshot(result, &request, action == "discover")?;
        result["capabilities"]=json!({"representation":"serialized","prefab_inherited_fields":false,"managed_type_creation":false});
    } else if !materialized_edits.is_empty() {
        filter_materialized_edits(&mut result,&materialized_edits);
    }
    result["backend"] = json!(backend);
    if let Some(path) = request.get("path") { result["path"] = path.clone(); }
    Ok(result)
}

fn filter_materialized_snapshot(snapshot: &mut Value, ids: &std::collections::HashSet<String>) {
    if let Some(objects)=snapshot["objects"].as_array_mut() {
        objects.retain(|object|object["object_id"].as_str().is_some_and(|id|ids.contains(id)));
    }
    if let Some(diagnostics)=snapshot["diagnostics"].as_array_mut() {
        if !diagnostics.iter().any(|diagnostic|diagnostic["code"]=="unsupported_prefab_inheritance") {
            diagnostics.push(json!({"code":"unsupported_prefab_inheritance","severity":"warning",
                "message":"Inherited Prefab objects are not part of the materialized serialized API",
                "object_id":null,"property_path":null,"span":{"start":0,"end":0}}));
        }
    }
}

fn filter_materialized_edits(result: &mut Value, scopes: &std::collections::BTreeMap<String,std::collections::HashSet<String>>) {
    fn filter(entry: &mut Value, scopes: &std::collections::BTreeMap<String,std::collections::HashSet<String>>) {
        let Some(ids)=entry["path"].as_str().and_then(|path|scopes.get(path)) else {return;};
        let diagnostics=if let Some(snapshot)=entry.get_mut("snapshot") {
            filter_materialized_snapshot(snapshot,ids); Some(snapshot["diagnostics"].clone())
        } else {None};
        if let Some(diagnostics)=diagnostics {entry["diagnostics"]=diagnostics;}
    }
    if let Some(entries)=result.get_mut("results").and_then(Value::as_array_mut) {
        for entry in entries {filter(entry,scopes);}
    } else {filter(result,scopes);}
}

fn merge_live_schema_diagnostics(result: &mut Value, reports: &std::collections::BTreeMap<String,Vec<Value>>) {
    fn merge(entry: &mut Value, reports: &std::collections::BTreeMap<String,Vec<Value>>) {
        let path=entry.get("path").and_then(Value::as_str).unwrap_or("").to_string();
        let mut diagnostics=entry.get("snapshot").and_then(|snapshot|snapshot.get("diagnostics"))
            .and_then(Value::as_array).cloned().unwrap_or_default();
        for diagnostic in entry.get("diagnostics").and_then(Value::as_array).into_iter().flatten()
            .chain(reports.get(&path).into_iter().flatten()) {
            if !diagnostics.contains(diagnostic) {diagnostics.push(diagnostic.clone());}
        }
        entry["diagnostics"]=json!(diagnostics);
        if let Some(snapshot)=entry.get_mut("snapshot") {snapshot["diagnostics"]=json!(diagnostics);}
    }
    if let Some(entries)=result.get_mut("results").and_then(Value::as_array_mut) {
        for entry in entries {merge(entry,reports);}
    } else {merge(result,reports);}
}

fn filter_snapshot(mut snapshot: Value, request: &Value, discover: bool) -> Result<Value, String> {
    let object = request.get("object_id").and_then(Value::as_str);
    let prefix = request.get("property_path").and_then(Value::as_str).unwrap_or("");
    let query = request.get("query").and_then(Value::as_str).unwrap_or("").to_lowercase();
    if !prefix.is_empty() && !prefix.starts_with('/') { return Err("assets.invalid_path: property_path must be an RFC 6901 pointer".into()); }
    let objects = snapshot["objects"].as_array_mut().ok_or("assets.invalid_response: snapshot objects missing")?;
    if let Some(id) = object {
        if !objects.iter().any(|obj| obj["object_id"].as_str()==Some(id)) { return Err(format!("assets.object_not_found: {id}")); }
        objects.retain(|obj| obj["object_id"].as_str()==Some(id));
    }
    let mut matches = Vec::new();
    for obj in objects {
        let id = obj["object_id"].clone();
        let fields = obj["fields"].as_array_mut().ok_or("assets.invalid_response: snapshot fields missing")?;
        fields.retain(|field| {
            let path = field["property_path"].as_str().unwrap_or("");
            (prefix.is_empty() || path == prefix || path.strip_prefix(prefix).is_some_and(|s|s.starts_with('/')))
                && (query.is_empty() || path.to_lowercase().contains(&query) || field["value"].to_string().to_lowercase().contains(&query))
        });
        for field in fields.iter() { let mut field=field.clone(); field["object_id"]=id.clone(); matches.push(field); }
    }
    if !discover { return Ok(snapshot); }
    let offset = request.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = request.get("limit").and_then(Value::as_u64).unwrap_or(100).clamp(1,1000) as usize;
    let total = matches.len();
    let next = offset.saturating_add(limit);
    Ok(json!({"revision":snapshot["revision"],"matches":matches.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),
        "total":total,"truncated":next<total,"next_offset":if next<total {Some(next)} else {None}}))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contract_tests;
