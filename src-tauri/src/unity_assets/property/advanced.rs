use super::*;
use crate::unity_asset_core::{self as core, authoring::AuthoringAsset, prefab::PrefabGraph};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

type EffectiveObjects = BTreeMap<String, core::prefab::EffectiveObject>;

struct Context {
    root: std::path::PathBuf,
    graph: PrefabGraph,
    dependencies: BTreeMap<String, Vec<u8>>,
    schema: crate::unity_assets::schema::ProjectSchema,
    effective_cache: RefCell<BTreeMap<String, Arc<EffectiveObjects>>>,
    tree_cache: RefCell<BTreeMap<String, Arc<YamlPropertyTree>>>,
    read_cache: RefCell<BTreeMap<String, Value>>,
    version_cache: RefCell<BTreeMap<String, Value>>,
    effective_builds: Cell<usize>,
    tree_builds: Cell<usize>,
    read_projections: Cell<usize>,
    validation_passes: usize,
    materialized_passes: usize,
    schema_dependencies: BTreeSet<String>,
}
pub(in crate::unity_assets) fn contains_effective(
    root: &Path,
    path: &str,
    id: &str,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<bool, String> {
    Ok(Context::load_files(root, [path.to_string()], files)?
        .graph
        .effective(path)?
        .contains_key(id))
}
pub(in crate::unity_assets) fn projection(
    root: &Path,
    path: &str,
    bytes: &[u8],
) -> Result<
    (
        String,
        BTreeMap<std::path::PathBuf, String>,
        std::collections::HashMap<String, String>,
    ),
    String,
> {
    let context = Context::load_files(
        root,
        [path.to_string()],
        &BTreeMap::from([(path.into(), bytes.to_vec())]),
    )?;
    let effective = context.graph.effective(path)?;
    let text = core::authoring::projection_text(effective.into_values().map(|o| o.object))?;
    let dependencies = context
        .dependencies
        .iter()
        .map(|(p, b)| (root.join(p), blake3::hash(b).to_hex().to_string()))
        .collect();
    Ok((
        text,
        dependencies,
        context.graph.guids.into_iter().collect(),
    ))
}
impl Context {
    fn load(root: &Path, paths: impl IntoIterator<Item = String>) -> Result<Self, String> {
        Self::load_files(root, paths, &BTreeMap::new())
    }
    fn load_files(
        root: &Path,
        paths: impl IntoIterator<Item = String>,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, String> {
        let mut graph = PrefabGraph::default();
        let mut schema = crate::unity_assets::schema::ProjectSchema::load(root)?;
        let mut ambiguous_guids = BTreeSet::new();
        for (guid, paths) in schema.guid_index(root) {
            if paths.len() == 1 {
                graph.guids.insert(guid, paths[0].clone());
            } else {
                ambiguous_guids.insert(guid);
            }
        }
        let mut pending: VecDeque<_> = paths.into_iter().collect();
        let mut dependencies = BTreeMap::new();
        let mut path_aliases = BTreeMap::new();
        while let Some(path) = pending.pop_front() {
            let key = if cfg!(windows) {
                path.to_ascii_lowercase()
            } else {
                path.clone()
            };
            if path_aliases
                .get(&key)
                .is_some_and(|previous| previous != &path)
            {
                return Err("assets.path_alias: use one spelling per asset".into());
            }
            path_aliases.insert(key, path.clone());
            if graph.files.contains_key(&path) {
                continue;
            }
            if graph.files.len() >= 256 {
                return Err("property.dependency_limit".into());
            }
            let bytes = if let Some(bytes) = files.get(&path) {
                bytes.clone()
            } else {
                std::fs::read(asset_path(root, &path, false)?).map_err(|e| e.to_string())?
            };
            let file = AuthoringAsset::new(&bytes, schema.scalar_hints(&bytes)?)?;
            for instance in file.objects.values().filter(|o| o.class_id == "1001") {
                let guid = instance.data["m_SourcePrefab"]["guid"]
                    .as_str()
                    .ok_or("property.prefab_source_missing")?;
                if ambiguous_guids.contains(&guid.to_ascii_lowercase()) {
                    return Err("property.ambiguous_guid".into());
                }
                pending.push_back(
                    graph
                        .guids
                        .get(&guid.to_ascii_lowercase())
                        .ok_or("property.prefab_guid_missing")?
                        .clone(),
                );
            }
            dependencies.insert(path.clone(), bytes);
            if let Some(bytes) = schema.indexed_meta(root, &path) {
                dependencies.insert(format!("{path}.meta"), bytes.to_vec());
            }
            graph.files.insert(path, file);
        }
        if graph
            .files
            .values()
            .any(|file| file.objects.values().any(|o| o.class_id == "1001"))
        {
            for (path, file) in &graph.files {
                graph.array_templates.insert(
                    path.clone(),
                    schema.prefab_array_templates(&file.original, file)?,
                );
            }
        }
        let mut schema_dependencies = BTreeSet::new();
        for (path, bytes) in schema.captured_sources() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| "property.schema_scope")?
                .to_string_lossy()
                .replace('\\', "/");
            schema_dependencies.insert(relative.clone());
            dependencies.insert(relative, bytes.clone());
        }
        Ok(Self {
            root: root.to_path_buf(),
            schema_dependencies,
            graph,
            dependencies,
            schema,
            effective_cache: Default::default(),
            tree_cache: Default::default(),
            read_cache: Default::default(),
            version_cache: Default::default(),
            effective_builds: Cell::new(0),
            tree_builds: Cell::new(0),
            read_projections: Cell::new(0),
            validation_passes: 0,
            materialized_passes: 0,
        })
    }
    fn invalidate(&self) {
        self.effective_cache.borrow_mut().clear();
        self.tree_cache.borrow_mut().clear();
        self.read_cache.borrow_mut().clear();
        self.version_cache.borrow_mut().clear();
    }
    fn effective(&self, path: &str) -> Result<Arc<EffectiveObjects>, String> {
        if let Some(result) = self.effective_cache.borrow().get(path) {
            return Ok(Arc::clone(result));
        }
        let result = Arc::new(self.graph.effective(path)?);
        self.effective_builds.set(self.effective_builds.get() + 1);
        self.effective_cache
            .borrow_mut()
            .insert(path.into(), Arc::clone(&result));
        Ok(result)
    }
    fn versions(&self) -> Value {
        if let Some(value) = self.version_cache.borrow().get("\0all") {
            return value.clone();
        }
        let value = json!(self
            .dependencies
            .iter()
            .map(|(p, b)| (p.clone(), blake3::hash(b).to_hex().to_string()))
            .collect::<BTreeMap<_, _>>());
        self.version_cache
            .borrow_mut()
            .insert("\0all".into(), value.clone());
        value
    }
    fn versions_for(&self, path: &str) -> Result<Value, String> {
        if let Some(result) = self.version_cache.borrow().get(path) {
            return Ok(result.clone());
        }
        let mut pending = vec![path.to_string()];
        let mut seen = BTreeSet::new();
        let mut result = BTreeMap::new();
        while let Some(path) = pending.pop() {
            if !seen.insert(path.clone()) {
                continue;
            }
            for name in [path.clone(), format!("{path}.meta")] {
                if let Some(bytes) = self.dependencies.get(&name) {
                    result.insert(name, blake3::hash(bytes).to_hex().to_string());
                }
            }
            for object in self.graph.files[&path]
                .objects
                .values()
                .filter(|o| o.class_id == "1001")
            {
                pending.push(
                    self.graph
                        .guids
                        .get(
                            &object.data["m_SourcePrefab"]["guid"]
                                .as_str()
                                .ok_or("property.prefab_guid_missing")?
                                .to_ascii_lowercase(),
                        )
                        .ok_or("property.prefab_guid_missing")?
                        .clone(),
                );
            }
        }
        let result = json!(result);
        self.version_cache
            .borrow_mut()
            .insert(path.into(), result.clone());
        Ok(result)
    }
    fn tree(&self, path: &str) -> Result<Arc<YamlPropertyTree>, String> {
        if let Some(tree) = self.tree_cache.borrow().get(path) {
            return Ok(Arc::clone(tree));
        }
        let effective = self.effective(path)?;
        let mut hints = core::ScalarHints::new();
        for (id, object) in effective.iter() {
            if let Some(source) = self.graph.files[&object.source_path]
                .hints
                .get(&object.source_id)
            {
                hints.insert(id.clone(), source.clone());
            }
        }
        let mut snapshot = core::authoring::snapshot_objects(
            effective.values().map(|o| o.object.clone()),
            &hints,
        )?;
        snapshot.revision = blake3::hash(&self.graph.files[path].render(false)?)
            .to_hex()
            .to_string();
        let tree = Arc::new(YamlPropertyTree::from_snapshot(path, snapshot)?);
        self.tree_builds.set(self.tree_builds.get() + 1);
        self.tree_cache
            .borrow_mut()
            .insert(path.into(), Arc::clone(&tree));
        Ok(tree)
    }
    fn read(&self, request: &Value) -> Result<Value, String> {
        let key = json!([
            request["target"],
            request["maxDepth"],
            request["maxArrayItems"],
            request["arrayOffset"],
            request["bindingId"]
        ])
        .to_string();
        if let Some(result) = self.read_cache.borrow().get(&key) {
            return Ok(result.clone());
        }
        let target = target(request)?;
        let path = path(&target)?;
        let tree = self.tree(path)?;
        let id = object_id(&tree, &target)?.to_string();
        let mut result = read_result(&tree, request)?;
        result["dependencies"] = self.versions_for(path)?;
        let versions = self.versions();
        for path in &self.schema_dependencies {
            result["dependencies"][path] = versions[path].clone();
        }
        if let Some(object) = self.effective(path)?.get(&id) {
            let wire = core::prefab::wire_path(
                &object.object.data,
                target.property_path.as_deref().unwrap_or(""),
            )
            .unwrap_or_default();
            result["prefabLayers"] =
                serde_json::to_value(&object.layers).map_err(|e| e.to_string())?;
            result["prefabOverride"] = json!(object.layers.first().is_some_and(|layer| {
                self.graph.files[&layer.path].objects[&layer.instance_id]
                    .data
                    .pointer("/m_Modification/m_Modifications")
                    .and_then(Value::as_array)
                    .is_some_and(|mods| {
                        mods.iter().any(|m| {
                            m["target"]["guid"]
                                .as_str()
                                .is_some_and(|g| g.eq_ignore_ascii_case(&layer.source_guid))
                                && core::authoring::decimal(&m["target"]["fileID"]).as_deref()
                                    == Some(&layer.source_id)
                                && m["propertyPath"].as_str().is_some_and(|p| {
                                    p == wire || p.starts_with(&format!("{wire}."))
                                })
                        })
                    })
            }));
        }
        self.read_projections.set(self.read_projections.get() + 1);
        self.read_cache.borrow_mut().insert(key, result.clone());
        Ok(result)
    }
}

pub(super) async fn execute(root: &Path, request: Value) -> Result<Value, String> {
    let started = std::time::Instant::now();
    if request["action"] == "discover_property" {
        let root = root.to_path_buf();
        return tokio::task::spawn_blocking(move || discover(&root, &request))
            .await
            .map_err(|e| e.to_string())?;
    }
    let root_owned = root.to_path_buf();
    let input = request.clone();
    let (context, mut results, touched, candidates) = tokio::task::spawn_blocking(move || {
        let (mut context, results, touched) = compile(&root_owned, &input)?;
        let candidates = if input["action"] == "read_property" {
            None
        } else {
            Some(prepare_candidates(
                &root_owned,
                &input,
                &mut context,
                &touched,
            )?)
        };
        Ok::<_, String>((context, results, touched, candidates))
    })
    .await
    .map_err(|e| e.to_string())??;
    if request["action"] == "read_property" {
        return Ok(results.remove(0));
    }
    let prepare_ms = started.elapsed().as_secs_f64() * 1000.0;
    let changed_files = candidates
        .as_ref()
        .unwrap()
        .0
        .iter()
        .filter(|(_, before, output)| before != &output.bytes)
        .count();
    let commit_started = std::time::Instant::now();
    let committed = super::super::commit_candidates(root, candidates.unwrap()).await?;
    let commit_ms = commit_started.elapsed().as_secs_f64() * 1000.0;
    tokio::task::spawn_blocking(move || {
    let mut context=context;
    let projection_started = std::time::Instant::now();
    if committed["persisted"] != true {
        return Err("property.outcome_unknown".into());
    }
    // Versions returned with the committed projection must describe the new disk graph.
    for path in &touched {
        context
            .dependencies
            .insert(path.clone(), context.graph.files[path].render(false)?);
    }
    context.version_cache.borrow_mut().clear();
    context.read_cache.borrow_mut().clear();
    let mut response = if request["resultMode"]=="summary" {
        let versions=context.versions();
        let assets=touched.iter().map(|path|{
            let mut dependencies=context.versions_for(path)?;
            for source in &context.schema_dependencies {dependencies[source]=versions[source].clone();}
            Ok::<_,String>(json!({"path":path,"revision":versions[path],"dependencies":dependencies}))
        }).collect::<Result<Vec<_>,_>>()?;
        json!({"ok":true,"message":"","writesApplied":request["writes"].as_array().unwrap().len(),"assets":assets,"transactionId":committed["transaction_id"]})
    } else {
    for (index, write) in request["writes"].as_array().unwrap().iter().enumerate() {
        let before = std::mem::take(&mut results[index]);
        let mut after = match context.read(write) {
            Ok(result) => result,
            Err(error) if error.contains("unknown_object") || error.contains("unknown_field") => {
                json!({"ok":true,"value":null,"target":write["target"],"children":[],"editable":false,"message":"Property was removed by a later write in this batch.","backend":"yaml","dependencies":context.versions()})
            }
            Err(error) => {
                return Err(format!(
                    "property.outcome_unknown: committed but projection failed: {error}"
                ))
            }
        };
        after["saved"] = json!(true);
        after["beforeSnapshot"] = before;
        results[index] = after;
    }
    let mut response = json!({"ok":true,"message":"","transactionId":committed["transaction_id"]});
    response["results"]=Value::Array(results);
    response
    };
    if request["profile"] == true {
        response["profile"] = json!({"prepareMs":prepare_ms,"commitMs":commit_ms,"projectionMs":projection_started.elapsed().as_secs_f64()*1000.0,"totalMs":started.elapsed().as_secs_f64()*1000.0,
        "effectiveBuilds":context.effective_builds.get(),"treeBuilds":context.tree_builds.get(),"readProjections":context.read_projections.get(),"materializedCompilePasses":context.materialized_passes,"overrideValidationPasses":context.validation_passes,"changedFiles":changed_files,"editor":committed.get("editorTimings")});
    }
    Ok(response)
    }).await.map_err(|e|format!("property.outcome_unknown: response worker: {e}"))?
}

fn prepare_candidates(
    root: &Path,
    request: &Value,
    context: &mut Context,
    touched: &BTreeSet<String>,
) -> Result<super::super::storage::Candidates, String> {
    let mut prepared = vec![];
    for path in touched {
        let file = context.graph.files.get_mut(path).unwrap();
        let bytes = file.render(true)?;
        let hints = context.schema.scalar_hints(&bytes)?;
        let snapshot = core::inspect_with_hints(&bytes, &hints).map_err(|e| e.to_string())?;
        file.hints = hints;
        prepared.push((
            path.clone(),
            file.original.clone(),
            core::EditOutput {
                bytes,
                snapshot,
                applied_operations: request["writes"].as_array().unwrap().len(),
            },
        ));
    }
    for (path, bytes) in context.schema.captured_sources() {
        let path = path
            .strip_prefix(root)
            .map_err(|_| "property.schema_scope")?
            .to_string_lossy()
            .replace('\\', "/");
        context.schema_dependencies.insert(path.clone());
        context.dependencies.insert(path, bytes.clone());
    }
    context.version_cache.borrow_mut().clear();
    context.read_cache.borrow_mut().clear();
    let dependencies = context
        .dependencies
        .iter()
        .filter(|(path, _)| !touched.contains(*path))
        .map(|(p, b)| (p.clone(), b.clone()))
        .collect();
    Ok((prepared, dependencies))
}

fn discover(root: &Path, request: &Value) -> Result<Value, String> {
    let target = target(request)?;
    let path = path(&target)?;
    let context = Context::load(root, [path.to_string()])?;
    let tree = context.tree(path)?;
    let max = request["maxResults"].as_u64().unwrap_or(128).clamp(1, 1024) as usize;
    let query = request["query"].as_str().unwrap_or("").to_lowercase();
    let mut matches = vec![];
    let mut scanned = 0;
    for object in &tree.semantic().snapshot.objects {
        if target
            .target_file_id
            .or(target.object_file_id)
            .is_some_and(|id| id.to_string() != object.object_id)
        {
            continue;
        }
        let mut input = request.clone();
        input["target"]["targetFileId"] = json!(object.object_id);
        let result = read_result(&tree, &input)?;
        scanned += 1;
        let mut pending = vec![(result, 0)];
        while let Some((node, depth)) = pending.pop() {
            let property = node["propertyPath"].as_str().unwrap_or("");
            if !property.starts_with("references")
                && (query.is_empty() || property.to_lowercase().contains(&query))
                && request["fieldName"]
                    .as_str()
                    .is_none_or(|name| node["name"] == name)
                && request["fieldType"]
                    .as_str()
                    .is_none_or(|ty| node["valueType"] == ty)
            {
                let mut entry = node.clone();
                entry.as_object_mut().unwrap().remove("children");
                entry["target"] = json!({"kind":"asset","path":path,"targetFileId":object.object_id,"propertyPath":property});
                entry["depth"] = json!(depth);
                matches.push(entry);
                if matches.len() >= max {
                    break;
                }
            }
            for child in node["children"].as_array().into_iter().flatten().rev() {
                pending.push((child.clone(), depth + 1));
            }
        }
        if matches.len() >= max {
            break;
        }
    }
    Ok(
        json!({"ok":true,"message":"","target":target,"matches":matches,"truncated":matches.len()>=max,"scannedObjects":scanned,"backend":"yaml","revision":tree.semantic().snapshot.revision,"dependencies":context.versions()}),
    )
}

fn compile(
    root: &Path,
    request: &Value,
) -> Result<(Context, Vec<Value>, BTreeSet<String>), String> {
    let writes = if request["action"] == "read_property" {
        vec![request]
    } else {
        request["writes"]
            .as_array()
            .ok_or("property.writes_required")?
            .iter()
            .collect::<Vec<_>>()
    };
    if writes.is_empty() || writes.len() > 10000 {
        return Err("property.limit".into());
    }
    let paths = writes
        .iter()
        .map(|w| target(w).and_then(|t| Ok(path(&t)?.to_string())))
        .collect::<Result<BTreeSet<_>, String>>()?;
    let mut context = Context::load(root, paths.clone())?;
    if request["action"] == "read_property" {
        let result = context.read(request)?;
        return Ok((context, vec![result], BTreeSet::new()));
    }
    let versions = context.versions();
    let mut results = vec![];
    let mut touched = BTreeSet::new();
    let mut checked_dependencies = BTreeMap::new();
    let inherited_paths = paths
        .iter()
        .filter_map(|path| match context.effective(path) {
            Ok(objects) if objects.values().any(|object| !object.layers.is_empty()) => {
                Some(Ok(path.clone()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<BTreeSet<_>, String>>()?;
    for write in &writes {
        let target = target(write)?;
        let path = path(&target)?;
        if write["expectedRevision"].as_str() != versions[path].as_str() {
            return Err(format!("assets.stale_revision: {path}"));
        }
        if write["writeMode"].as_str().is_some_and(|v| v != "commit") {
            return Err("property.unsupported_preview".into());
        }
        if inherited_paths.contains(path) {
            let expected = write["expectedDependencies"]
                .as_object()
                .ok_or("property.dependencies_required")?;
            // Validate the transitive closure of this target, not unrelated batch members.
            let closure = context.versions_for(path)?;
            if closure
                .as_object()
                .unwrap()
                .iter()
                .any(|(p, v)| expected.get(p) != Some(v))
            {
                return Err("assets.stale_dependency".into());
            }
        }
        if let Some(expected) = write["expectedDependencies"].as_object() {
            if expected.len() > 4096 {
                return Err("property.dependency_limit".into());
            }
            for (path, revision) in expected {
                if !checked_dependencies.contains_key(path) {
                    let actual = if let Some(revision) = versions.get(path) {
                        revision.clone()
                    } else {
                        if !(path.starts_with("Assets/") || path.starts_with("Packages/")) {
                            return Err("property.dependency_scope".into());
                        }
                        let full = crate::merge_jobs::io::safe_path(&context.root, path)?;
                        let bytes = std::fs::read(full)
                            .map_err(|_| format!("assets.stale_dependency: {path}"))?;
                        context.dependencies.insert(path.clone(), bytes.clone());
                        json!(blake3::hash(&bytes).to_hex().to_string())
                    };
                    checked_dependencies.insert(path.clone(), actual);
                }
                if checked_dependencies[path] != *revision {
                    return Err(format!("assets.stale_dependency: {path}"));
                }
            }
        }
        if request["resultMode"] != "summary" {
            results.push(context.read(write).unwrap_or(Value::Null));
        }
    }
    let mut index = 0;
    while index < writes.len() {
        let count = compile_materialized_run(&mut context, &writes[index..], &mut touched)?;
        if count > 0 {
            index += count;
            continue;
        }
        let count = compile_override_run(&mut context, &writes[index..], &mut touched)?;
        if count > 0 {
            index += count;
            continue;
        }
        let count = compile_inherited_array_run(&mut context, &writes[index..], &mut touched)?;
        if count > 0 {
            index += count;
            continue;
        }
        let write = writes[index];
        index += 1;
        let target = target(write)?;
        let path = path(&target)?.to_string();
        let property = target.property_path.as_deref().unwrap_or("");
        let value = write.get("value").ok_or("property.value_required")?;
        let effective = context.effective(&path)?;
        let id = target
            .target_file_id
            .or(target.object_file_id)
            .map(|v| v.to_string())
            .or_else(|| (effective.len() == 1).then(|| effective.keys().next().unwrap().clone()))
            .ok_or("property.ambiguous_target")?;
        let object = effective.get(&id).ok_or("property.unknown_object")?.clone();
        match value["action"].as_str() {
            Some("editObjects") => {
                if !object.layers.is_empty() {
                    return Err("property.topology_requires_materialized_layer".into());
                }
                let add: Vec<core::authoring::ObjectData> =
                    serde_json::from_value(value.get("add").cloned().unwrap_or(json!([])))
                        .map_err(|e| e.to_string())?;
                let added = add.iter().map(|o| o.id.clone()).collect::<Vec<_>>();
                let remove: Vec<String> =
                    serde_json::from_value(value.get("remove").cloned().unwrap_or(json!([])))
                        .map_err(|e| e.to_string())?;
                let file = context.graph.files.get_mut(&path).unwrap();
                file.edit_objects(add, &remove)?;
                for update in value["updates"].as_array().into_iter().flatten() {
                    file.set(
                        update["objectId"]
                            .as_str()
                            .ok_or("property.object_required")?,
                        update["propertyPath"]
                            .as_str()
                            .ok_or("property.path_required")?,
                        update["value"].clone(),
                    )?;
                }
                let final_bytes = file.render(true)?;
                context
                    .schema
                    .validate_created_objects(&final_bytes, &added)?;
                file.hints = context.schema.scalar_hints(&final_bytes)?;
                // Structural updates carry the same field validation as ordinary writes.
                for update in value["updates"].as_array().into_iter().flatten() {
                    let op = file.semantic()?.lower_write(
                        update["objectId"].as_str().unwrap(),
                        update["propertyPath"].as_str().unwrap(),
                        &update["value"],
                    )?;
                    context.schema.validate_properties(&final_bytes, &[op])?;
                }
                touched.insert(path);
            }
            Some("createManaged") => {
                if !object.layers.is_empty() {
                    return Err("property.creation_requires_materialized_layer".into());
                }
                let file = context.graph.files.get_mut(&path).unwrap();
                let created = file.create_managed(&id, property, &value["template"])?;
                let rids = created["remap"]
                    .as_object()
                    .unwrap()
                    .values()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect::<Vec<_>>();
                context.schema.validate_creation(
                    root,
                    &file.render(true)?,
                    &id,
                    property,
                    &rids,
                )?;
                file.hints = context.schema.scalar_hints(&file.render(false)?)?;
                touched.insert(path);
            }
            Some("revert") => {
                let layer = object
                    .layers
                    .first()
                    .ok_or("property.prefab_layer_required")?;
                core::prefab::value_at(&object.object.data, property)?;
                let wire = core::prefab::wire_path(&object.object.data, property)?;
                context.graph.revert_subtree(layer, &wire)?;
                touched.insert(path);
            }
            Some("applyToSource") => {
                let level = value["level"]
                    .as_u64()
                    .filter(|v| *v > 0 && *v <= object.layers.len() as u64)
                    .ok_or("property.invalid_apply_level")? as usize;
                let current = core::prefab::value_at(&object.object.data, property)?.clone();
                let wire = core::prefab::wire_path(&object.object.data, property)?;
                if current.is_object()
                    && !matches!(current["kind"].as_str(), Some("int64" | "uint64"))
                {
                    return Err("property.apply_scalar_required".into());
                }
                fn local_reference(value: &Value) -> bool {
                    if value.get("rid").is_some() {
                        return true;
                    }
                    if value.get("fileID").is_some() {
                        return value.get("guid").is_none()
                            && core::authoring::decimal(&value["fileID"]).as_deref() != Some("0");
                    }
                    value
                        .as_array()
                        .is_some_and(|a| a.iter().any(local_reference))
                        || value
                            .as_object()
                            .is_some_and(|m| m.values().any(local_reference))
                }
                if current.is_array() && local_reference(&current) {
                    return Err("property.apply_array_reference_remap_required".into());
                }
                if level == object.layers.len() {
                    let file = context.graph.files.get_mut(&object.source_path).unwrap();
                    let op = file
                        .semantic()?
                        .lower_write(&object.source_id, property, &current)?;
                    let bytes = file.render(false)?;
                    context
                        .schema
                        .validate_properties(&bytes, std::slice::from_ref(&op))?;
                    let output = core::edit_with_hints(&bytes, &[op], &file.hints)
                        .map_err(|e| e.to_string())?;
                    file.replace_contents(
                        &output.bytes,
                        context.schema.scalar_hints(&output.bytes)?,
                    )?;
                    touched.insert(object.source_path.clone());
                } else {
                    if current.is_array() {
                        context
                            .graph
                            .override_array(&object.layers[level], &wire, &current)?;
                    } else {
                        context.graph.override_value(
                            &object.layers[level],
                            &wire,
                            Some(current),
                        )?;
                    }
                    touched.insert(object.layers[level].path.clone());
                }
                for layer in &object.layers[..level] {
                    context.graph.revert_subtree(layer, &wire)?;
                    touched.insert(layer.path.clone());
                }
            }
            _ if !object.layers.is_empty() => {
                // Validate against the effective source schema before encoding an override.
                let file = &context.graph.files[&object.source_path];
                let op = file
                    .semantic()?
                    .lower_write(&object.source_id, property, value)?;
                context
                    .schema
                    .validate_properties(&file.render(false)?, &[op])?;
                let previous = core::prefab::value_at(&object.object.data, property)?;
                if previous.is_array() || previous.get("rid").is_some() {
                    return Err("property.prefab_scalar_required".into());
                }
                context
                    .graph
                    .override_value(&object.layers[0], property, Some(value.clone()))?;
                touched.insert(path);
            }
            _ => {
                return Err("property.unsupported_command".into());
            }
        }
        // Commands/structural writes are ordering barriers. Rebuild from the
        // resulting graph before compiling any later ordinary override run.
        context.invalidate();
    }
    // Resolve every final target before entering the journal; errors here cannot
    // become a misleading saved result after an otherwise successful commit.
    for write in request["writes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|_| request["resultMode"] != "summary")
    {
        if let Err(error) = context.read(write) {
            if !error.contains("unknown_object") && !error.contains("unknown_field") {
                return Err(error);
            }
        }
    }
    Ok((context, results, touched))
}

/// Independent materialized scalars may be interleaved across files. Group them
/// per file, retaining every input in that file's order. Graph/structural commands
/// and inherited writes end the phase; non-scalar runs retain global file order.
fn compile_materialized_run(
    context: &mut Context,
    writes: &[&Value],
    touched: &mut BTreeSet<String>,
) -> Result<usize, String> {
    let first = target(writes[0])?;
    let asset_path = path(&first)?.to_string();
    let scalar = |v: &Value| {
        v.is_number()
            || v.is_boolean()
            || v.is_string()
            || matches!(v["kind"].as_str(), Some("int64" | "uint64"))
    };
    let scalar_run = scalar(&writes[0]["value"]);
    let mut groups: BTreeMap<String, Vec<(String, String, Value)>> = BTreeMap::new();
    let mut count = 0;
    for write in writes {
        let target = target(write)?;
        let current_path = path(&target)?.to_string();
        if !scalar_run && current_path != asset_path {
            break;
        }
        let value = write.get("value").ok_or("property.value_required")?;
        if scalar_run && !scalar(value) {
            break;
        }
        if matches!(
            value["action"].as_str(),
            Some("createManaged" | "editObjects" | "revert" | "applyToSource")
        ) {
            break;
        }
        let effective = context.effective(&current_path)?;
        let id = target
            .target_file_id
            .or(target.object_file_id)
            .map(|id| id.to_string())
            .or_else(|| (effective.len() == 1).then(|| effective.keys().next().unwrap().clone()))
            .ok_or("property.ambiguous_target")?;
        let object = effective.get(&id).ok_or("property.unknown_object")?;
        if !object.layers.is_empty() {
            break;
        }
        groups.entry(current_path).or_default().push((
            id,
            target.property_path.unwrap_or_default(),
            value.clone(),
        ));
        count += 1;
    }
    if count == 0 {
        return Ok(0);
    }
    for (asset_path, logical) in groups {
        let file = context.graph.files.get_mut(&asset_path).unwrap();
        let bytes = file.render(false)?;
        let operations = file.semantic()?.lower_writes(&logical)?;
        context.schema.validate_properties(&bytes, &operations)?;
        let output =
            core::edit_with_hints(&bytes, &operations, &file.hints).map_err(|e| e.to_string())?;
        let hints = context.schema.scalar_hints(&output.bytes)?;
        file.replace_contents(&output.bytes, hints)?;
        context.materialized_passes += 1;
        touched.insert(asset_path);
    }
    context.invalidate();
    Ok(count)
}

fn compile_override_run(
    context: &mut Context,
    writes: &[&Value],
    touched: &mut BTreeSet<String>,
) -> Result<usize, String> {
    let first = target(writes[0])?;
    let asset_path = path(&first)?.to_string();
    let effective = context.effective(&asset_path)?;
    let mut edits = vec![];
    let mut sources: BTreeMap<
        String,
        (
            Vec<u8>,
            core::semantic::SemanticAsset,
            Vec<core::AssetOperation>,
        ),
    > = BTreeMap::new();
    for write in writes {
        let target = target(write)?;
        if path(&target)? != asset_path {
            break;
        }
        let value = write.get("value").ok_or("property.value_required")?;
        if value.get("action").is_some() {
            break;
        }
        let id = target
            .target_file_id
            .or(target.object_file_id)
            .map(|v| v.to_string())
            .or_else(|| (effective.len() == 1).then(|| effective.keys().next().unwrap().clone()))
            .ok_or("property.ambiguous_target")?;
        let object = effective.get(&id).ok_or("property.unknown_object")?;
        let Some(layer) = object.layers.first() else {
            break;
        };
        let property = target.property_path.as_deref().unwrap_or("");
        let previous = core::prefab::value_at(&object.object.data, property)?;
        if previous.is_array() || previous.get("rid").is_some() {
            break;
        }
        if !sources.contains_key(&object.source_path) {
            let file = &context.graph.files[&object.source_path];
            sources.insert(
                object.source_path.clone(),
                (file.render(false)?, file.semantic()?, vec![]),
            );
        }
        let (_, model, operations) = sources.get_mut(&object.source_path).unwrap();
        if model.resolve(&object.source_id, property, false).is_err() {
            break;
        }
        let operation = model.lower_write(&object.source_id, property, value)?;
        let core::AssetOperation::Set { value, .. } = &operation else {
            return Err("property.prefab_scalar_required".into());
        };
        edits.push(core::prefab::OverrideEdit {
            layer: layer.clone(),
            property: core::prefab::wire_path(&object.object.data, property)?,
            value: Some(value.clone()),
        });
        operations.push(operation);
    }
    if edits.is_empty() {
        return Ok(0);
    }
    for (path, (bytes, _, operations)) in sources {
        context.schema.validate_properties(&bytes, &operations)?;
        context.validation_passes += 1;
        core::validate_set_values(&bytes, &operations, &context.graph.files[&path].hints)
            .map_err(|e| e.to_string())?;
    }
    context.graph.override_values(&edits)?;
    touched.insert(asset_path);
    context.invalidate();
    Ok(edits.len())
}

/// Structural inherited array edits form one ordered phase. Validate the staged
/// effective schema (new indices need not exist in the base), edit it once, then
/// lower only the affected arrays back to size/leaf Prefab override records.
fn compile_inherited_array_run(
    context: &mut Context,
    writes: &[&Value],
    touched: &mut BTreeSet<String>,
) -> Result<usize, String> {
    let first = target(writes[0])?;
    let asset_path = path(&first)?.to_string();
    let effective = context.effective(&asset_path)?;
    let mut logical = vec![];
    for write in writes {
        let target = target(write)?;
        if path(&target)? != asset_path {
            break;
        }
        let value = write.get("value").ok_or("property.value_required")?;
        if matches!(
            value["action"].as_str(),
            Some("editObjects" | "createManaged" | "revert" | "applyToSource")
        ) {
            break;
        }
        let id = target
            .target_file_id
            .or(target.object_file_id)
            .map(|v| v.to_string())
            .ok_or("property.ambiguous_target")?;
        if effective
            .get(&id)
            .ok_or("property.unknown_object")?
            .layers
            .is_empty()
        {
            break;
        }
        logical.push((id, target.property_path.unwrap_or_default(), value.clone()));
    }
    if logical.is_empty() {
        return Ok(0);
    }
    let bytes = core::authoring::projection_text(effective.values().map(|o| o.object.clone()))?
        .into_bytes();
    let hints = context.schema.scalar_hints(&bytes)?;
    let model = AuthoringAsset::new(&bytes, hints.clone())?.semantic()?;
    let (operations, staged) = model.stage_writes(&logical)?;
    context.schema.validate_properties(&bytes, &operations)?;
    context.validation_passes += 1;
    let mut final_objects = vec![];
    for (id, object) in effective.iter() {
        let mut final_object = object.object.clone();
        final_object.data = staged.resolve(id, "", false)?.value.unwrap().clone();
        final_objects.push(final_object);
    }
    let final_bytes = core::authoring::projection_text(final_objects)?.into_bytes();
    let final_asset =
        AuthoringAsset::new(&final_bytes, context.schema.scalar_hints(&final_bytes)?)?;
    final_asset.render(true)?;
    let mut arrays: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for ((id, path, _), op) in logical.iter().zip(&operations) {
        if !matches!(op,core::AssetOperation::Set{value,..} if !value.is_array()) {
            arrays.entry(id.clone()).or_default().insert(path.clone());
        }
    }
    // Only outermost arrays need a replacement; they include nested edits.
    for roots in arrays.values_mut() {
        let all = roots.clone();
        roots.retain(|p| {
            !all.iter()
                .any(|q| q != p && p.starts_with(&format!("{q}.")))
        });
    }
    let mut scalars = vec![];
    for (id, path, _) in &logical {
        if arrays.get(id).is_some_and(|roots| {
            roots
                .iter()
                .any(|r| path == r || path.starts_with(&format!("{r}.")))
        }) {
            continue;
        }
        let object = &final_asset.objects[id];
        let value = core::prefab::value_at(&object.data, path)?.clone();
        if value.get("rid").is_some() {
            return Err("property.prefab_scalar_required".into());
        }
        scalars.push(core::prefab::OverrideEdit {
            layer: effective[id].layers[0].clone(),
            property: core::prefab::wire_path(&object.data, path)?,
            value: Some(value),
        });
    }
    for (id, roots) in arrays {
        let object = &final_asset.objects[&id];
        for path in roots {
            let value = core::prefab::value_at(&object.data, &path)?;
            let wire = core::prefab::wire_path(&object.data, &path)?;
            context
                .graph
                .override_array(&effective[&id].layers[0], &wire, value)?;
        }
    }
    context.graph.override_values(&scalars)?;
    touched.insert(asset_path.clone());
    context.invalidate();
    // Summary responses skip tree rendering, but still prove the emitted
    // override representation reconstructs before any candidate is committed.
    context.effective(&asset_path)?;
    Ok(logical.len())
}
