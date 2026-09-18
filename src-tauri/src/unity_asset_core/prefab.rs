//! Recursive textual Prefab projection and ownership-aware override authoring.
//! The caller supplies an immutable dependency set. No filesystem/Editor access.
use super::{
    authoring::{self, AuthoringAsset, ObjectData},
    *,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub path: String,
    pub instance_id: String,
    pub source_guid: String,
    pub source_id: String,
}
#[derive(Clone, Debug)]
pub struct EffectiveObject {
    pub object: ObjectData,
    pub layers: Vec<Layer>,
    pub source_path: String,
    pub source_id: String,
}
pub struct OverrideEdit {
    pub layer: Layer,
    pub property: String,
    pub value: Option<Value>,
}
#[derive(Default)]
pub struct PrefabGraph {
    pub files: BTreeMap<String, AuthoringAsset>,
    pub guids: BTreeMap<String, String>,
    /// Source-proven zero shapes for arrays with no serialized element prototype.
    pub array_templates: BTreeMap<String, BTreeMap<String, BTreeMap<String, Value>>>,
}

impl PrefabGraph {
    pub fn effective(&self, path: &str) -> Result<BTreeMap<String, EffectiveObject>, String> {
        self.visit(path, &mut BTreeSet::new(), &mut BTreeMap::new())
    }
    fn visit(
        &self,
        path: &str,
        visiting: &mut BTreeSet<String>,
        cache: &mut BTreeMap<String, BTreeMap<String, EffectiveObject>>,
    ) -> Result<BTreeMap<String, EffectiveObject>, String> {
        if let Some(result) = cache.get(path) {
            return Ok(result.clone());
        }
        if visiting.len() >= 32 || !visiting.insert(path.into()) {
            return Err("property.prefab_cycle_or_depth".into());
        }
        let file = self
            .files
            .get(path)
            .ok_or("property.prefab_dependency_missing")?;
        let mut result = BTreeMap::new();
        let mut additions = vec![];
        let mut instance_roots: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut stripped: BTreeMap<(String, String, String), Vec<String>> = BTreeMap::new();
        for object in file.objects.values().filter(|o| o.stripped) {
            if let (Some(instance), Some(guid), Some(source)) = (
                authoring::decimal(&object.data["m_PrefabInstance"]["fileID"]),
                object.data["m_CorrespondingSourceObject"]["guid"].as_str(),
                authoring::decimal(&object.data["m_CorrespondingSourceObject"]["fileID"]),
            ) {
                stripped
                    .entry((instance, guid.to_ascii_lowercase(), source))
                    .or_default()
                    .push(object.id.clone());
            }
        }
        for object in file
            .objects
            .values()
            .filter(|o| !o.stripped && o.class_id != "1001")
        {
            result.insert(
                object.id.clone(),
                EffectiveObject {
                    object: object.clone(),
                    layers: vec![],
                    source_path: path.into(),
                    source_id: object.id.clone(),
                },
            );
        }
        for instance in file.objects.values().filter(|o| o.class_id == "1001") {
            let guid = instance.data["m_SourcePrefab"]["guid"]
                .as_str()
                .ok_or("property.prefab_source_missing")?;
            let source_path = self
                .guids
                .get(&guid.to_ascii_lowercase())
                .ok_or_else(|| format!("property.prefab_guid_missing: {guid}"))?;
            let mut children = self.visit(source_path, visiting, cache)?;
            let mut ids = BTreeMap::new();
            for source_id in children.keys() {
                // Unity's serialized virtual identity is the positive XOR of
                // source/instance IDs. Explicit stripped identities take priority.
                let explicit = stripped
                    .get(&(
                        instance.id.clone(),
                        guid.to_ascii_lowercase(),
                        source_id.clone(),
                    ))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if explicit.len() > 1 {
                    return Err("property.prefab_ambiguous_stripped_identity".into());
                }
                let mapped = if let Some(id) = explicit.first() {
                    id.clone()
                } else {
                    let id = ((authoring::id(source_id)? ^ authoring::id(&instance.id)?)
                        & i64::MAX)
                        .to_string();
                    authoring::id(&id)?;
                    id
                };
                ids.insert(source_id.clone(), mapped);
            }
            for (field, component) in [("m_AddedGameObjects", false), ("m_AddedComponents", true)] {
                for record in instance.data["m_Modification"][field]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    let target = &record["targetCorrespondingSourceObject"];
                    if !same_guid(&target["guid"], guid) {
                        return Err("property.prefab_added_target_scope".into());
                    }
                    let source = authoring::decimal(&target["fileID"])
                        .ok_or("property.prefab_added_target_missing")?;
                    let parent = ids
                        .get(&source)
                        .ok_or("property.prefab_added_target_missing")?
                        .clone();
                    let added = authoring::decimal(&record["addedObject"]["fileID"])
                        .ok_or("property.prefab_added_object_missing")?;
                    if record["addedObject"].get("guid").is_some() {
                        return Err("property.prefab_added_object_scope".into());
                    }
                    let index = record["insertIndex"]
                        .as_i64()
                        .ok_or("property.prefab_added_index")?;
                    additions.push((parent, added, index, component));
                }
            }
            let mut removed_ids = BTreeSet::new();
            for removed in ["m_RemovedComponents", "m_RemovedGameObjects"] {
                for reference in instance.data["m_Modification"][removed]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    if !same_guid(&reference["guid"], guid) {
                        return Err("property.prefab_removed_target_scope".into());
                    }
                    if let Some(id) = authoring::decimal(&reference["fileID"]) {
                        removed_ids.insert(id);
                    }
                }
            }
            loop {
                let before = removed_ids.len();
                for (id, child) in &children {
                    let owner = authoring::decimal(&child.object.data["m_GameObject"]["fileID"]);
                    let parent = authoring::decimal(&child.object.data["m_Father"]["fileID"]);
                    if owner.as_ref().is_some_and(|id| removed_ids.contains(id)) {
                        removed_ids.insert(id.clone());
                    }
                    if matches!(child.object.class_id.as_str(), "4" | "224")
                        && parent.as_ref().is_some_and(|id| removed_ids.contains(id))
                    {
                        removed_ids.insert(id.clone());
                        if let Some(owner) = owner {
                            removed_ids.insert(owner);
                        }
                    }
                }
                if removed_ids.len() == before {
                    break;
                }
            }
            children.retain(|id, _| !removed_ids.contains(id));
            instance_roots.insert(
                instance.id.clone(),
                children
                    .iter()
                    .filter(|(_, o)| {
                        matches!(o.object.class_id.as_str(), "4" | "224")
                            && authoring::decimal(&o.object.data["m_Father"]["fileID"]).as_deref()
                                == Some("0")
                    })
                    .map(|(id, _)| ids[id].clone())
                    .collect(),
            );
            for id in &removed_ids {
                ids.insert(id.clone(), "0".into());
            }
            for child in children.values_mut() {
                for (field, nested) in [("m_Component", true), ("m_Children", false)] {
                    if let Some(items) = child
                        .object
                        .data
                        .get_mut(field)
                        .and_then(Value::as_array_mut)
                    {
                        items.retain(|item| {
                            let reference = if nested { &item["component"] } else { item };
                            authoring::decimal(&reference["fileID"])
                                .is_none_or(|id| !removed_ids.contains(&id))
                        });
                    }
                }
                remap_pptrs(&mut child.object.data, &ids);
                if matches!(child.object.class_id.as_str(), "4" | "224")
                    && authoring::decimal(&child.object.data["m_Father"]["fileID"]).as_deref()
                        == Some("0")
                {
                    if let Some(parent) = instance.data.pointer("/m_Modification/m_TransformParent")
                    {
                        child.object.data["m_Father"] = parent.clone();
                    }
                }
            }
            let mut modifications = instance.data["m_Modification"]["m_Modifications"]
                .as_array()
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            // Array sizes establish the shape before leaf overlays. Stable sorting
            // preserves the last record for duplicate paths, independent of YAML order.
            modifications.sort_by_key(|m| {
                let p = m["propertyPath"].as_str().unwrap_or("");
                if p.ends_with(".Array.size") {
                    (0, p.matches('.').count())
                } else {
                    (1, 0)
                }
            });
            for modification in modifications {
                if !same_guid(&modification["target"]["guid"], guid) {
                    return Err("property.prefab_override_scope".into());
                }
                let id = authoring::decimal(&modification["target"]["fileID"])
                    .ok_or("property.prefab_target_missing")?;
                let Some(child) = children.get_mut(&id) else {
                    continue;
                }; // Unity retains stale overrides for removed objects.
                let property = modification["propertyPath"]
                    .as_str()
                    .ok_or("property.prefab_path_missing")?;
                let path_to_resolve = property.strip_suffix(".Array.size").unwrap_or(property);
                if let Err(error) = resolved_path(&child.object.data, path_to_resolve) {
                    // Unity retains overrides of elements removed from a source.
                    // Keep the serialized record so growing the source restores it.
                    if error == "property.prefab_inactive_array_override" {
                        continue;
                    }
                    return Err(error);
                }
                if let Some(array_path) = property.strip_suffix(".Array.size") {
                    let size = modification["value"]
                        .as_str()
                        .and_then(|s| s.parse::<usize>().ok())
                        .filter(|n| *n <= 1_000_000)
                        .ok_or("property.invalid_prefab_array_size")?;
                    let mut values = value_at(&child.object.data, array_path)?
                        .as_array()
                        .ok_or("property.prefab_array_required")?
                        .clone();
                    if size > values.len() {
                        let template = values
                            .last()
                            .cloned()
                            .or_else(|| {
                                let (physical, _) =
                                    resolved_path(&child.object.data, array_path).ok()?;
                                let pointer = semantic::property_tokens(&physical)
                                    .ok()?
                                    .into_iter()
                                    .fold(format!("/{}", child.object.root_type), |p, t| match t {
                                        semantic::PropertyToken::Field(k) => {
                                            format!("{p}/{}", semantic::escape_pointer(&k))
                                        }
                                        semantic::PropertyToken::Index(_) => format!("{p}/*"),
                                    });
                                self.array_templates
                                    .get(&child.source_path)?
                                    .get(&child.source_id)?
                                    .get(&pointer)
                                    .cloned()
                            })
                            .ok_or("property.prefab_array_template_required")?;
                        values.resize(size, template);
                    } else {
                        values.truncate(size);
                    }
                    set_value(&mut child.object.data, array_path, json!(values))?;
                } else {
                    let previous = value_at(&child.object.data, property)?;
                    let value = decode_override(previous, modification)?;
                    set_value(&mut child.object.data, property, value)?;
                }
            }
            for (source_id, mut child) in children {
                let mapped = ids[&source_id].clone();
                child.object.id = mapped.clone();
                child.layers.insert(
                    0,
                    Layer {
                        path: path.into(),
                        instance_id: instance.id.clone(),
                        source_guid: guid.to_ascii_lowercase(),
                        source_id,
                    },
                );
                if result.insert(mapped, child).is_some() {
                    return Err("property.prefab_identity_collision".into());
                }
            }
        }
        // Resolve after all instances: an added child can itself be a nested
        // Prefab instance whose stripped transform appeared later in the file.
        for (parent, added, index, component) in additions {
            let object = result
                .get(&added)
                .ok_or("property.prefab_added_object_missing")?;
            let owner = if component {
                "m_GameObject"
            } else {
                "m_Father"
            };
            if authoring::decimal(&object.object.data[owner]["fileID"]).as_deref()
                != Some(parent.as_str())
            {
                return Err("property.prefab_added_owner_mismatch".into());
            }
            if !component && !matches!(object.object.class_id.as_str(), "4" | "224") {
                return Err("property.prefab_added_transform_required".into());
            }
            let parent_object = result
                .get_mut(&parent)
                .ok_or("property.prefab_added_target_missing")?;
            let field = if component {
                "m_Component"
            } else {
                "m_Children"
            };
            let list = parent_object.object.data[field]
                .as_array_mut()
                .ok_or("property.prefab_added_target_type")?;
            let reference = json!({"fileID":added});
            let item = if component {
                json!({"component":reference})
            } else {
                reference
            };
            if list.contains(&item) {
                return Err("property.prefab_duplicate_added_object".into());
            }
            let position = if index == -1 {
                list.len()
            } else {
                usize::try_from(index).map_err(|_| "property.prefab_added_index")?
            };
            if position > list.len() {
                return Err("property.prefab_added_index".into());
            }
            list.insert(position, item);
        }
        for object in result
            .values_mut()
            .filter(|o| o.object.class_id == "1660057539")
        {
            for reference in object.object.data["m_Roots"]
                .as_array_mut()
                .into_iter()
                .flatten()
            {
                if let Some(roots) =
                    authoring::decimal(&reference["fileID"]).and_then(|id| instance_roots.get(&id))
                {
                    if roots.len() != 1 {
                        return Err("property.prefab_scene_root_ambiguous".into());
                    }
                    reference["fileID"] = json!(roots[0]);
                }
            }
        }
        visiting.remove(path);
        cache.insert(path.into(), result.clone());
        Ok(result)
    }

    pub fn override_value(
        &mut self,
        layer: &Layer,
        property: &str,
        value: Option<Value>,
    ) -> Result<(), String> {
        self.override_values(&[OverrideEdit {
            layer: layer.clone(),
            property: property.into(),
            value,
        }])
    }

    /// Replacing an array owns its complete element overlay. Drop stale indices
    /// before writing size/leaves; unrelated properties and instances are retained.
    pub fn override_array(
        &mut self,
        layer: &Layer,
        property: &str,
        value: &Value,
    ) -> Result<(), String> {
        fn leaves(
            layer: &Layer,
            path: &str,
            value: &Value,
            out: &mut Vec<OverrideEdit>,
        ) -> Result<(), String> {
            if let Some(array) = value.as_array() {
                out.push(OverrideEdit {
                    layer: layer.clone(),
                    property: format!("{path}.Array.size"),
                    value: Some(json!(array.len())),
                });
                for (i, v) in array.iter().enumerate() {
                    leaves(layer, &format!("{path}.Array.data[{i}]"), v, out)?;
                }
            } else if value.get("rid").is_some() {
                return Err("property.prefab_managed_array_structure_unsupported".into());
            } else if value.is_object()
                && value.get("fileID").is_none()
                && value.get("kind").is_none()
            {
                for (k, v) in value.as_object().unwrap() {
                    leaves(layer, &format!("{path}.{k}"), v, out)?;
                }
            } else {
                encode_override(layer, path, value)?;
                out.push(OverrideEdit {
                    layer: layer.clone(),
                    property: path.into(),
                    value: Some(value.clone()),
                });
            }
            Ok(())
        }
        if !value.is_array() {
            return Err("property.prefab_array_required".into());
        }
        let mut replacements = vec![];
        leaves(layer, property, value, &mut replacements)?;
        let mut edits = vec![];
        let prefix = format!("{property}.");
        if let Some(records) = self
            .files
            .get(&layer.path)
            .and_then(|f| f.objects.get(&layer.instance_id))
            .and_then(|o| o.data.pointer("/m_Modification/m_Modifications"))
            .and_then(Value::as_array)
        {
            for record in records {
                if same_guid(&record["target"]["guid"], &layer.source_guid)
                    && authoring::decimal(&record["target"]["fileID"]).as_deref()
                        == Some(&layer.source_id)
                {
                    if let Some(p) = record["propertyPath"]
                        .as_str()
                        .filter(|p| *p == property || p.starts_with(&prefix))
                    {
                        edits.push(OverrideEdit {
                            layer: layer.clone(),
                            property: p.into(),
                            value: None,
                        });
                    }
                }
            }
        }
        edits.extend(replacements);
        self.override_values(&edits)
    }

    pub fn revert_subtree(&mut self, layer: &Layer, property: &str) -> Result<(), String> {
        validate_override_path(property)?;
        let prefix = format!("{property}.");
        let records = self
            .files
            .get(&layer.path)
            .and_then(|f| f.objects.get(&layer.instance_id))
            .and_then(|o| o.data.pointer("/m_Modification/m_Modifications"))
            .and_then(Value::as_array)
            .ok_or("property.prefab_modifications_missing")?;
        let mut edits = vec![OverrideEdit {
            layer: layer.clone(),
            property: property.into(),
            value: None,
        }];
        for record in records {
            if same_guid(&record["target"]["guid"], &layer.source_guid)
                && authoring::decimal(&record["target"]["fileID"]).as_deref()
                    == Some(&layer.source_id)
            {
                if let Some(path) = record["propertyPath"]
                    .as_str()
                    .filter(|p| p.starts_with(&prefix))
                {
                    edits.push(OverrideEdit {
                        layer: layer.clone(),
                        property: path.into(),
                        value: None,
                    });
                }
            }
        }
        self.override_values(&edits)
    }

    /// Validate every request before changing any instance. Repeated keys only
    /// coalesce after validation; each instance's override list is scanned once.
    pub fn override_values(&mut self, edits: &[OverrideEdit]) -> Result<(), String> {
        type Key = (String, String, String);
        let mut groups: BTreeMap<(String, String), BTreeMap<Key, (usize, Option<Value>)>> =
            BTreeMap::new();
        for (index, edit) in edits.iter().enumerate() {
            let layer = &edit.layer;
            if edit.property.is_empty() {
                return Err("property.path_required".into());
            }
            validate_override_path(&edit.property)?;
            authoring::id(&layer.source_id)?;
            let instance = self
                .files
                .get(&layer.path)
                .and_then(|f| f.objects.get(&layer.instance_id))
                .ok_or("property.prefab_instance_missing")?;
            if !same_guid(&instance.data["m_SourcePrefab"]["guid"], &layer.source_guid) {
                return Err("property.prefab_override_scope".into());
            }
            if instance
                .data
                .pointer("/m_Modification/m_Modifications")
                .and_then(Value::as_array)
                .is_none()
            {
                return Err("property.prefab_modifications_missing".into());
            }
            let record = edit
                .value
                .as_ref()
                .map(|value| encode_override(layer, &edit.property, value))
                .transpose()?;
            groups
                .entry((layer.path.clone(), layer.instance_id.clone()))
                .or_default()
                .insert(
                    (
                        layer.source_guid.to_ascii_lowercase(),
                        layer.source_id.clone(),
                        edit.property.clone(),
                    ),
                    (index, record),
                );
        }
        for ((path, instance), replacements) in groups {
            let list = self
                .files
                .get_mut(&path)
                .unwrap()
                .objects
                .get_mut(&instance)
                .unwrap()
                .data
                .pointer_mut("/m_Modification/m_Modifications")
                .unwrap()
                .as_array_mut()
                .unwrap();
            list.retain(|record| {
                !replacements.contains_key(&(
                    record["target"]["guid"]
                        .as_str()
                        .unwrap_or("")
                        .to_ascii_lowercase(),
                    authoring::decimal(&record["target"]["fileID"]).unwrap_or_default(),
                    record["propertyPath"].as_str().unwrap_or("").to_string(),
                ))
            });
            let mut ordered = replacements.into_values().collect::<Vec<_>>();
            ordered.sort_by_key(|(index, _)| *index);
            list.extend(ordered.into_iter().filter_map(|(_, record)| record));
        }
        Ok(())
    }
}

fn encode_override(layer: &Layer, property: &str, value: &Value) -> Result<Value, String> {
    let reference = value.get("fileID").is_some();
    let integer = matches!(value["kind"].as_str(), Some("int64" | "uint64"));
    decode_asset_value(value).map_err(|e| e.to_string())?;
    if !reference && ((value.is_object() && !integer) || value.is_array() || value.is_null()) {
        return Err("property.prefab_scalar_or_reference_required".into());
    }
    let text = if reference {
        String::new()
    } else if integer {
        value["value"].as_str().unwrap().to_string()
    } else if let Some(s) = value.as_str() {
        s.into()
    } else if let Some(b) = value.as_bool() {
        if b { "1" } else { "0" }.into()
    } else {
        value.to_string()
    };
    Ok(
        json!({"target":{"fileID":layer.source_id,"guid":layer.source_guid,"type":3},"propertyPath":property,"value":text,"objectReference":if reference{value.clone()}else{json!({"fileID":"0"})}}),
    )
}

fn same_guid(value: &Value, guid: &str) -> bool {
    value.as_str().is_some_and(|s| s.eq_ignore_ascii_case(guid))
}

pub fn array_hint_pattern(pointer: &str) -> String {
    pointer
        .split('/')
        .map(|s| {
            if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
                "*"
            } else {
                s
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub fn value_at<'a>(data: &'a Value, path: &str) -> Result<&'a Value, String> {
    let (physical, _) = resolved_path(data, path)?;
    let mut current = data;
    for token in semantic::property_tokens(&physical)? {
        current = match token {
            semantic::PropertyToken::Field(k) => current.get(&k),
            semantic::PropertyToken::Index(i) => current.get(i),
        }
        .ok_or_else(|| format!("property.unsupported_prefab_path: {path}"))?;
    }
    Ok(current)
}

/// Convert logical aliases and Unity's managedReferences[rid] wire paths to
/// one host-scoped registry location. No registry/identity crosses a component.
fn resolved_path(data: &Value, path: &str) -> Result<(String, String), String> {
    fn entry(data: &Value, rid: &str) -> Result<usize, String> {
        data.pointer("/references/RefIds")
            .and_then(Value::as_array)
            .and_then(|v| {
                v.iter()
                    .position(|e| authoring::decimal(&e["rid"]).as_deref() == Some(rid))
            })
            .ok_or_else(|| format!("property.unresolved_prefab_managed_reference: {rid}"))
    }
    let mut current = data;
    let mut physical = String::new();
    let mut wire = String::new();
    let mut rest = path;
    if let Some(tail) = path.strip_prefix("managedReferences[") {
        let (rid, tail) = tail
            .split_once("]")
            .ok_or("property.invalid_prefab_managed_path")?;
        let i = entry(data, rid)?;
        current = &data["references"]["RefIds"][i]["data"];
        physical = format!("references.RefIds.Array.data[{i}].data");
        wire = format!("managedReferences[{rid}]");
        rest = tail
            .strip_prefix('.')
            .ok_or("property.invalid_prefab_managed_path")?;
    }
    for token in semantic::property_tokens(rest)? {
        if current
            .as_object()
            .is_some_and(|m| m.len() == 1 && m.contains_key("rid"))
        {
            let rid = authoring::decimal(&current["rid"]).ok_or("property.invalid_rid")?;
            let i = entry(data, &rid)?;
            current = &data["references"]["RefIds"][i]["data"];
            physical = format!("references.RefIds.Array.data[{i}].data");
            wire = format!("managedReferences[{rid}]");
        }
        let (suffix, next) = match token {
            semantic::PropertyToken::Field(k) => (k.clone(), current.get(&k)),
            semantic::PropertyToken::Index(i) => {
                if current.as_array().is_some_and(|a| i >= a.len()) {
                    return Err("property.prefab_inactive_array_override".into());
                }
                (format!("Array.data[{i}]"), current.get(i))
            }
        };
        current = next.ok_or_else(|| format!("property.unsupported_prefab_path: {path}"))?;
        for p in [&mut physical, &mut wire] {
            if !p.is_empty() {
                p.push('.');
            }
            p.push_str(&suffix);
        }
    }
    Ok((physical, wire))
}
pub fn wire_path(data: &Value, path: &str) -> Result<String, String> {
    Ok(resolved_path(data, path)?.1)
}
pub fn set_value(data: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let (physical, _) = resolved_path(data, path)?;
    authoring::set_path(data, &physical, value)
}
fn validate_override_path(path: &str) -> Result<(), String> {
    let mut path = path.strip_suffix(".Array.size").unwrap_or(path);
    if let Some(tail) = path.strip_prefix("managedReferences[") {
        let (rid, tail) = tail
            .split_once("].")
            .ok_or("property.invalid_prefab_managed_path")?;
        authoring::id(rid)?;
        path = tail;
    }
    semantic::property_tokens(path)?;
    Ok(())
}
fn decode_override(previous: &Value, modification: &Value) -> Result<Value, String> {
    if previous.get("fileID").is_some() {
        return Ok(modification["objectReference"].clone());
    }
    let raw = &modification["value"];
    let text = raw.as_str().map(str::to_owned).unwrap_or_else(|| {
        if raw.is_null() {
            String::new()
        } else {
            raw.to_string()
        }
    });
    if previous.is_string() {
        return Ok(json!(text));
    }
    if previous.is_boolean() {
        return match text.as_str() {
            "0" => Ok(json!(false)),
            "1" => Ok(json!(true)),
            _ => Err("property.invalid_prefab_boolean".into()),
        };
    }
    if previous.get("kind").is_some() {
        return Ok(json!({"kind":previous["kind"],"value":text}));
    }
    if previous.is_number() {
        return serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                if let Value::Number(n) = v {
                    if previous.as_number().is_some_and(serde_json::Number::is_f64) {
                        n.as_f64()
                            .and_then(serde_json::Number::from_f64)
                            .map(Value::Number)
                    } else {
                        Some(super::edit::canonical_number(n))
                    }
                } else {
                    None
                }
            })
            .ok_or("property.invalid_prefab_number".into());
    }
    Err("property.unsupported_prefab_value".into())
}
fn remap_pptrs(value: &mut Value, ids: &BTreeMap<String, String>) {
    if value.get("fileID").is_some() && value.get("guid").is_none() {
        if let Some(id) = authoring::decimal(&value["fileID"]).and_then(|id| ids.get(&id)) {
            value["fileID"] = json!(id);
        }
    } else if let Some(map) = value.as_object_mut() {
        for v in map.values_mut() {
            remap_pptrs(v, ids);
        }
    } else if let Some(array) = value.as_array_mut() {
        for v in array {
            remap_pptrs(v, ids);
        }
    }
}
