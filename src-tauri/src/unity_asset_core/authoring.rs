//! Atomic graph authoring. Ordinary field edits retain the span editor; creation
//! and topology changes render only changed documents, then validate the complete
//! candidate. Intermediate dangling references never reach disk.
use super::{semantic::SemanticAsset, *};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObjectData {
    pub id: String,
    pub class_id: String,
    pub root_type: String,
    pub data: Value,
    #[serde(default)]
    pub stripped: bool,
}

#[derive(Clone)]
pub struct AuthoringAsset {
    pub original: Vec<u8>,
    pub objects: BTreeMap<String, ObjectData>,
    original_objects: BTreeMap<String, ObjectData>,
    pub hints: ScalarHints,
}

pub fn decimal(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::to_owned)
        .or_else(|| v.as_i64().map(|n| n.to_string()))
        .or_else(|| v.get("value").and_then(Value::as_str).map(str::to_owned))
}
pub fn id(v: &str) -> Result<i64, String> {
    let n = v.parse::<i64>().map_err(|_| "property.invalid_id")?;
    if n.to_string() != v || n == 0 {
        return Err("property.invalid_id".into());
    }
    Ok(n)
}

impl AuthoringAsset {
    pub fn rebase_original(&mut self) -> Result<(), String> {
        self.original_objects = Self::new(&self.original, self.hints.clone())?.objects;
        Ok(())
    }
    pub fn new(bytes: &[u8], hints: ScalarHints) -> Result<Self, String> {
        let parsed = parse(bytes).map_err(|e| e.to_string())?;
        if !parsed.is_writable() {
            return Err("property.unsupported_yaml".into());
        }
        let semantic =
            SemanticAsset::new(inspect_with_hints(bytes, &hints).map_err(|e| e.to_string())?)?;
        let mut objects = BTreeMap::new();
        for doc in &parsed.documents {
            objects.insert(
                doc.object_id.clone(),
                ObjectData {
                    id: doc.object_id.clone(),
                    class_id: doc.class_id.clone().ok_or("property.class_required")?,
                    root_type: semantic.object(&doc.object_id)?.root_type.clone(),
                    data: semantic
                        .resolve(&doc.object_id, "", false)?
                        .value
                        .unwrap()
                        .clone(),
                    stripped: doc.stripped,
                },
            );
        }
        Ok(Self {
            original: bytes.to_vec(),
            original_objects: objects.clone(),
            objects,
            hints,
        })
    }

    pub fn semantic(&self) -> Result<SemanticAsset, String> {
        SemanticAsset::new(snapshot_objects(
            self.objects.values().cloned(),
            &self.hints,
        )?)
    }

    /// Install a compiled phase while retaining the transaction's original
    /// bytes and comparison graph. No original-file reparse for each write.
    pub fn replace_contents(&mut self, bytes: &[u8], hints: ScalarHints) -> Result<(), String> {
        let next = Self::new(bytes, hints)?;
        self.objects = next.objects;
        self.hints = next.hints;
        Ok(())
    }

    pub fn set(&mut self, object: &str, path: &str, value: Value) -> Result<(), String> {
        let model = self.semantic()?;
        let resolved = model.resolve(object, path, false)?;
        let physical = resolved.serialized_path;
        let data = &mut self
            .objects
            .get_mut(object)
            .ok_or("property.unknown_object")?
            .data;
        set_path(data, &physical, value)
    }

    /// Templates use local rid labels. Every positive label is remapped; external
    /// managed edges are rejected. Replacing a slot retains the old graph because
    /// another slot may alias it. Unity may collect unreachable entries on save.
    pub fn create_managed(
        &mut self,
        object: &str,
        path: &str,
        template: &Value,
    ) -> Result<Value, String> {
        let model = self.semantic()?;
        let slot = model.resolve(object, path, false)?;
        if slot
            .value
            .and_then(|v| v.as_object())
            .is_none_or(|m| m.len() != 1 || !m.contains_key("rid"))
        {
            return Err("property.managed_slot_required".into());
        }
        let root = template["rootRid"]
            .as_str()
            .ok_or("property.template_root_required")?;
        let entries = template["entries"]
            .as_array()
            .filter(|v| !v.is_empty() && v.len() <= 10000)
            .ok_or("property.invalid_template")?;
        let data = &mut self.objects.get_mut(object).unwrap().data;
        let mut registry = data
            .pointer("/references/RefIds")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut used = registry
            .iter()
            .filter_map(|e| decimal(&e["rid"]))
            .map(|s| s.parse::<i64>().unwrap_or(-1))
            .collect::<BTreeSet<_>>();
        let mut remap = BTreeMap::new();
        let mut next = 1i64;
        for entry in entries {
            let old = entry["rid"]
                .as_str()
                .ok_or("property.template_rid_required")?;
            if !old
                .parse::<i64>()
                .is_ok_and(|n| n >= 0 && n.to_string() == old)
                || remap.contains_key(old)
            {
                return Err("property.invalid_template_rid".into());
            }
            for key in ["class", "ns", "asm"] {
                if entry["type"][key]
                    .as_str()
                    .is_none_or(|v| key != "ns" && v.is_empty())
                {
                    return Err("property.template_type_required".into());
                }
            }
            if !entry["data"].is_object() {
                return Err("property.explicit_data_required".into());
            }
            while used.contains(&next) {
                next = next.checked_add(1).ok_or("property.rid_exhausted")?;
            }
            used.insert(next);
            remap.insert(old.to_owned(), next.to_string());
        }
        let root = remap
            .get(root)
            .ok_or("property.template_root_missing")?
            .clone();
        fn remap_data(v: &mut Value, map: &BTreeMap<String, String>) -> Result<(), String> {
            if v.as_object()
                .is_some_and(|m| m.len() == 1 && m.contains_key("rid"))
            {
                let old = decimal(&v["rid"]).ok_or("property.invalid_template_edge")?;
                v["rid"] = json!(if old == "-2" {
                    old
                } else {
                    map.get(&old)
                        .ok_or("property.template_external_edge")?
                        .clone()
                });
            } else if let Some(m) = v.as_object_mut() {
                for v in m.values_mut() {
                    remap_data(v, map)?;
                }
            } else if let Some(a) = v.as_array_mut() {
                for v in a {
                    remap_data(v, map)?;
                }
            }
            Ok(())
        }
        for entry in entries {
            let mut entry = entry.clone();
            entry["rid"] = json!(remap[entry["rid"].as_str().unwrap()]);
            remap_data(&mut entry["data"], &remap)?;
            registry.push(entry);
        }
        data["references"] = json!({"version":2,"RefIds":registry});
        set_path(data, &slot.serialized_path, json!({"rid":root}))?;
        Ok(json!({"rid":root,"remap":remap}))
    }

    pub fn edit_objects(&mut self, add: Vec<ObjectData>, remove: &[String]) -> Result<(), String> {
        if add.len() + remove.len() > 10000 {
            return Err("property.topology_limit".into());
        }
        for object in add {
            id(&object.id)?;
            if object.stripped || object.class_id == "1001" {
                return Err("property.topology_template_requires_materialized_object".into());
            }
            if !matches!(
                (object.class_id.as_str(), object.root_type.as_str()),
                ("1", "GameObject")
                    | ("4", "Transform")
                    | ("224", "RectTransform")
                    | ("114", "MonoBehaviour")
            ) {
                return Err("property.unsupported_object_template_class".into());
            }
            if !object.class_id.bytes().all(|c| c.is_ascii_digit())
                || !object
                    .root_type
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                || object.root_type.is_empty()
                || !object.data.is_object()
            {
                return Err("property.invalid_object_template".into());
            }
            if self.objects.contains_key(&object.id) {
                return Err("property.object_id_collision".into());
            }
            self.objects.insert(object.id.clone(), object);
        }
        for object in remove {
            if self.objects.remove(object).is_none() {
                return Err("property.unknown_object".into());
            }
        }
        Ok(())
    }

    pub fn render(&self, validate_graph: bool) -> Result<Vec<u8>, String> {
        let parsed = parse(&self.original).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        let mut cursor = 0;
        for doc in &parsed.documents {
            bytes.extend_from_slice(&self.original[cursor..doc.span.start]);
            if let Some(object) = self.objects.get(&doc.object_id) {
                if self
                    .original_objects
                    .get(&doc.object_id)
                    .is_some_and(|old| old.data == object.data)
                {
                    bytes.extend_from_slice(&self.original[doc.span.start..doc.span.end]);
                } else {
                    let previous = &self.original_objects[&doc.object_id];
                    let entries = doc
                        .root
                        .entries()
                        .and_then(|e| e.first())
                        .and_then(|r| r.value.entries())
                        .ok_or("property.invalid_document")?;
                    bytes.extend_from_slice(
                        &self.original[doc.span.start
                            ..entries
                                .first()
                                .map(|e| e.span.start)
                                .unwrap_or(doc.span.end)],
                    );
                    let mut known = BTreeSet::new();
                    for entry in entries {
                        known.insert(entry.key.clone());
                        if let Some(value) = object.data.get(&entry.key) {
                            if previous.data.get(&entry.key) == Some(value) {
                                bytes.extend_from_slice(
                                    &self.original[entry.span.start..entry.span.end],
                                );
                            } else {
                                bytes.extend(render_field(&entry.key, value, parsed.newline)?);
                            }
                        }
                    }
                    for (key, value) in object.data.as_object().ok_or("property.object_required")? {
                        if !known.contains(key) {
                            bytes.extend(render_field(key, value, parsed.newline)?);
                        }
                    }
                }
            }
            cursor = doc.span.end;
        }
        bytes.extend_from_slice(&self.original[cursor..]);
        for (id, object) in &self.objects {
            if !self.original_objects.contains_key(id) {
                bytes.extend(render_object(object, parsed.newline)?);
            }
        }
        if validate_graph {
            let asset = parse(&bytes).map_err(|e| e.to_string())?;
            // Existing missing external/stripped references can be diagnostic,
            // but no invalid local graph is ever accepted for authoring.
            if let Some(error) = validate(&asset)
                .into_iter()
                .find(|d| d.severity == Severity::Error)
            {
                return Err(format!(
                    "property.invalid_graph: {}: {}",
                    error.code, error.message
                ));
            }
            SemanticAsset::new(
                inspect_with_hints(&bytes, &self.hints).map_err(|e| e.to_string())?,
            )?;
        }
        Ok(bytes)
    }
}

fn render_object(object: &ObjectData, newline: &str) -> Result<Vec<u8>, String> {
    let mut text = format!(
        "--- !u!{} &{}{}{newline}{}:{newline}",
        object.class_id,
        object.id,
        if object.stripped { " stripped" } else { "" },
        object.root_type
    )
    .into_bytes();
    for (key, value) in object
        .data
        .as_object()
        .ok_or("property.object_data_required")?
    {
        text.extend(render_field(key, value, newline)?);
    }
    Ok(text)
}
fn render_field(key: &str, value: &Value, newline: &str) -> Result<Vec<u8>, String> {
    let mut decoded = decode_asset_value(value).map_err(|e| format!("{key}: {e}"))?;
    if key == "references" {
        if let Some(entries) = decoded["RefIds"].as_array_mut() {
            for entry in entries {
                if let Some(rid) = entry["rid"].as_str() {
                    entry["rid"] = json!(rid.parse::<i64>().map_err(|_| "property.invalid_rid")?);
                }
                for key in ["class", "ns", "asm"] {
                    if entry["type"][key].is_null() {
                        entry["type"][key] = json!("");
                    }
                }
            }
        }
    }
    let key = if key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        key.to_owned()
    } else {
        serde_json::to_string(key).unwrap()
    };
    Ok(format!(
        "  {key}: {}{newline}",
        super::merge::json_yaml(&decoded).map_err(|e| e.to_string())?
    )
    .into_bytes())
}

pub fn snapshot_objects(
    objects: impl IntoIterator<Item = ObjectData>,
    hints: &ScalarHints,
) -> Result<AssetSnapshot, String> {
    fn fields(
        value: &Value,
        path: &str,
        hints: Option<&BTreeMap<String, ScalarHint>>,
        out: &mut Vec<AssetField>,
    ) {
        let kind = if value.get("fileID").is_some() {
            "reference"
        } else if value.is_array() {
            "array"
        } else if value.is_object() {
            "object"
        } else if value.is_string() {
            "string"
        } else if value.is_number() {
            "number"
        } else if value.is_boolean() {
            "boolean"
        } else {
            "null"
        };
        out.push(AssetField {
            property_path: path.into(),
            kind: kind.into(),
            value: value.clone(),
            type_hint: hints
                .and_then(|h| {
                    h.get(path)
                        .or_else(|| h.get(&super::prefab::array_hint_pattern(path)))
                })
                .and_then(|h| match h {
                    ScalarHint::String => Some("String".into()),
                    ScalarHint::Boolean => Some("Boolean".into()),
                    ScalarHint::Float => Some("Float".into()),
                    _ => None,
                })
                .or_else(|| {
                    if value.is_boolean() {
                        Some("Boolean".into())
                    } else if value.as_number().is_some_and(serde_json::Number::is_f64) {
                        Some("Float".into())
                    } else {
                        None
                    }
                }),
        });
        if let Some(map) = value.as_object() {
            for (key, v) in map {
                fields(
                    v,
                    &format!("{path}/{}", semantic::escape_pointer(key)),
                    hints,
                    out,
                );
            }
        }
        if let Some(array) = value.as_array() {
            for (i, v) in array.iter().enumerate() {
                let key = if path.ends_with("/references/RefIds") {
                    decimal(&v["rid"])
                        .map(|s| format!("@rid={s}"))
                        .unwrap_or(i.to_string())
                } else {
                    i.to_string()
                };
                fields(v, &format!("{path}/{key}"), hints, out);
            }
        }
    }
    let mut result = vec![];
    for object in objects {
        let mut out = vec![];
        let mut field_hints = hints.get(&object.id).cloned().unwrap_or_default();
        // Serialized arrays have a uniform element schema; preserve proven leaf
        // types when Prefab size overrides introduce indices absent in the source.
        for (path, hint) in hints.get(&object.id).into_iter().flatten() {
            field_hints
                .entry(super::prefab::array_hint_pattern(path))
                .or_insert(*hint);
        }
        for (key, value) in object.data.as_object().ok_or("property.object_required")? {
            fields(
                value,
                &format!("/{}/{}", object.root_type, semantic::escape_pointer(key)),
                Some(&field_hints),
                &mut out,
            );
        }
        result.push(AssetObject {
            object_id: object.id,
            class_id: Some(object.class_id),
            root_type: object.root_type,
            fields: out,
        });
    }
    Ok(AssetSnapshot {
        revision: String::new(),
        objects: result,
        diagnostics: vec![],
    })
}

/// Only for hierarchy/metadata presentation. Opaque scalar spellings remain
/// strings here; this projection is never a candidate for persistence.
pub fn projection_text(objects: impl IntoIterator<Item = ObjectData>) -> Result<String, String> {
    fn preserve_opaque(value: &mut Value) {
        if value["kind"] == "int64"
            && value["value"].as_str().is_some_and(|v| {
                v.parse::<i64>().is_err() || v.parse::<i64>().unwrap().to_string() != v
            })
        {
            *value = value["value"].clone();
            return;
        }
        if let Some(map) = value.as_object_mut() {
            for v in map.values_mut() {
                preserve_opaque(v);
            }
        } else if let Some(array) = value.as_array_mut() {
            for v in array {
                preserve_opaque(v);
            }
        }
    }
    let mut text = b"%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n".to_vec();
    for mut object in objects {
        preserve_opaque(&mut object.data);
        text.extend(render_object(&object, "\n")?);
    }
    String::from_utf8(text).map_err(|e| e.to_string())
}

pub fn set_path(data: &mut Value, path: &str, value: Value) -> Result<(), String> {
    let tokens = semantic::property_tokens(path)?;
    if tokens.is_empty() {
        return Err("property.path_required".into());
    }
    let mut current = data;
    for token in &tokens[..tokens.len() - 1] {
        current = match token {
            semantic::PropertyToken::Field(k) => current.get_mut(k),
            semantic::PropertyToken::Index(i) => current.get_mut(*i),
        }
        .ok_or("property.unknown_field")?;
    }
    let target = match tokens.last().unwrap() {
        semantic::PropertyToken::Field(k) => current.get_mut(k),
        semantic::PropertyToken::Index(i) => current.get_mut(*i),
    }
    .ok_or("property.unknown_field")?;
    *target = value;
    Ok(())
}

#[cfg(test)]
#[path = "authoring_tests.rs"]
mod tests;
