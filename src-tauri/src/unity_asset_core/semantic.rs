//! Shared logical-property addressing over an immutable serialized snapshot.
//! No IO, Editor calls or UI state. View and Agent projections consume this model.
use super::{AssetObject, AssetOperation, AssetSnapshot};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct SemanticAsset {
    pub snapshot: AssetSnapshot,
    documents: HashMap<String, Document>,
    object_index: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
struct Document {
    root_type: String,
    value: Value,
    registry: HashMap<String, usize>,
    field_order: HashMap<String, Vec<String>>,
    type_hints: HashMap<String, String>,
}

#[derive(Debug)]
pub struct ResolvedProperty<'a> {
    pub value: Option<&'a Value>,
    /// Root-inclusive pointer, using stable @rid selectors inside the registry.
    pub pointer: String,
    /// Physical SerializedProperty path, for the existing presentation adapter.
    pub serialized_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyToken {
    Field(String),
    Index(usize),
}

pub fn property_tokens(path: &str) -> Result<Vec<PropertyToken>, String> {
    if path.is_empty() {
        return Ok(vec![]);
    }
    let parts: Vec<_> = path.split('.').collect();
    if parts.len() > 512 {
        return Err("property.limit: path is too deep".into());
    }
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        let part = parts[index];
        if part == "Array"
            && parts
                .get(index + 1)
                .is_some_and(|next| next.starts_with("data["))
        {
            let raw = parts[index + 1]
                .strip_prefix("data[")
                .and_then(|v| v.strip_suffix(']'))
                .ok_or("property.invalid_path: invalid array index")?;
            if raw.is_empty()
                || (raw.len() > 1 && raw.starts_with('0'))
                || !raw.bytes().all(|v| v.is_ascii_digit())
            {
                return Err("property.invalid_path: invalid array index".into());
            }
            tokens.push(PropertyToken::Index(
                raw.parse()
                    .map_err(|_| "property.invalid_path: array index overflow")?,
            ));
            index += 2;
        } else {
            if part.is_empty()
                || part.contains(['[', ']'])
                || (part == "Array" && parts.get(index + 1) == Some(&"size"))
            {
                return Err(format!("property.invalid_path: {path}"));
            }
            tokens.push(PropertyToken::Field(part.into()));
            index += 1;
        }
    }
    Ok(tokens)
}

pub fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
fn unescape(value: &str) -> String {
    value.replace("~1", "/").replace("~0", "~")
}
fn decimal(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_i64().map(|v| v.to_string()))
        .or_else(|| {
            value
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}
fn managed_id(value: &Value) -> Option<String> {
    value
        .as_object()
        .filter(|v| v.len() == 1)
        .and_then(|v| v.get("rid"))
        .and_then(decimal)
}
fn atomic(value: &Value) -> bool {
    value.as_object().is_some_and(|map| {
        (map.contains_key("fileID")
            && map
                .keys()
                .all(|key| matches!(key.as_str(), "fileID" | "guid" | "type")))
            || (map.len() == 2
                && matches!(
                    map.get("kind").and_then(Value::as_str),
                    Some("int64" | "uint64" | "float64")
                ))
    })
}

impl SemanticAsset {
    pub fn new(snapshot: AssetSnapshot) -> Result<Self, String> {
        let mut documents = HashMap::new();
        let mut object_index = HashMap::new();
        for (object_position, object) in snapshot.objects.iter().enumerate() {
            object_index.insert(object.object_id.clone(), object_position);
            let id: i64 = object
                .object_id
                .parse()
                .map_err(|_| "property.invalid_id: expected signed decimal fileID")?;
            if id.to_string() != object.object_id {
                return Err("property.invalid_id: noncanonical fileID".into());
            }
            let prefix = format!("/{}/", escape_pointer(&object.root_type));
            let mut value = serde_json::Map::new();
            let mut field_order: HashMap<String, Vec<String>> = HashMap::new();
            let mut type_hints = HashMap::new();
            for field in &object.fields {
                if let Some(hint) = &field.type_hint {
                    type_hints.insert(field.property_path.clone(), hint.clone());
                }
                if let Some((parent, leaf)) = field.property_path.rsplit_once('/') {
                    field_order
                        .entry(parent.into())
                        .or_default()
                        .push(unescape(leaf));
                }
                if let Some(leaf) = field
                    .property_path
                    .strip_prefix(&prefix)
                    .filter(|leaf| !leaf.contains('/'))
                {
                    value.insert(unescape(leaf), field.value.clone());
                }
            }
            let value = Value::Object(value);
            let mut registry = HashMap::new();
            if let Some(entries) = value
                .pointer("/references/RefIds")
                .and_then(Value::as_array)
            {
                for (index, entry) in entries.iter().enumerate() {
                    let rid = entry
                        .get("rid")
                        .and_then(decimal)
                        .ok_or("property.invalid_registry: missing rid")?;
                    let parsed: i64 = rid
                        .parse()
                        .map_err(|_| "property.invalid_registry: invalid rid")?;
                    if (parsed < 0 && parsed != -2)
                        || parsed.to_string() != rid
                        || registry.insert(rid, index).is_some()
                    {
                        return Err("property.invalid_registry: duplicate or invalid rid".into());
                    }
                }
            }
            if documents
                .insert(
                    object.object_id.clone(),
                    Document {
                        root_type: object.root_type.clone(),
                        value,
                        registry,
                        field_order,
                        type_hints,
                    },
                )
                .is_some()
            {
                return Err("property.invalid_id: duplicate object identity".into());
            }
        }
        Ok(Self {
            snapshot,
            documents,
            object_index,
        })
    }

    pub fn object(&self, id: &str) -> Result<&AssetObject, String> {
        self.object_index
            .get(id)
            .and_then(|index| self.snapshot.objects.get(*index))
            .ok_or_else(|| format!("property.unknown_object: {id}"))
    }

    pub fn resolve(
        &self,
        id: &str,
        path: &str,
        allow_missing_leaf: bool,
    ) -> Result<ResolvedProperty<'_>, String> {
        let document = self
            .documents
            .get(id)
            .ok_or_else(|| format!("property.unknown_object: {id}"))?;
        let mut current = &document.value;
        let mut pointer = format!("/{}", escape_pointer(&document.root_type));
        let mut serialized_path = String::new();
        let tokens = property_tokens(path)?;
        for (index, token) in tokens.iter().enumerate() {
            if let Some(rid) = managed_id(current) {
                let entry_index = document.registry.get(&rid).ok_or_else(|| {
                    format!("property.unresolved_reference: rid {rid} in host {id}")
                })?;
                current = document
                    .value
                    .pointer("/references/RefIds")
                    .and_then(Value::as_array)
                    .and_then(|entries| entries.get(*entry_index))
                    .and_then(|entry| entry.get("data"))
                    .ok_or("property.invalid_registry: managed reference data is missing")?;
                pointer = format!(
                    "/{}/references/RefIds/@rid={rid}/data",
                    escape_pointer(&document.root_type)
                );
                serialized_path = format!("references.RefIds.Array.data[{entry_index}].data");
            }
            if atomic(current) {
                return Err("property.atomic_value: reference identities and numeric envelopes are indivisible".into());
            }
            let next = match token {
                PropertyToken::Field(name) => {
                    pointer.push('/');
                    pointer.push_str(&escape_pointer(name));
                    if !serialized_path.is_empty() {
                        serialized_path.push('.');
                    }
                    serialized_path.push_str(name);
                    current.as_object().and_then(|value| value.get(name))
                }
                PropertyToken::Index(item) => {
                    let entry = current.as_array().and_then(|values| values.get(*item));
                    let key = if pointer.ends_with("/references/RefIds") {
                        entry
                            .and_then(|entry| entry.get("rid"))
                            .and_then(decimal)
                            .map(|id| format!("@rid={id}"))
                    } else {
                        None
                    }
                    .unwrap_or_else(|| item.to_string());
                    pointer.push('/');
                    pointer.push_str(&escape_pointer(&key));
                    serialized_path.push_str(&format!(".Array.data[{item}]"));
                    entry
                }
            };
            match next {
                Some(value) => current = value,
                None if allow_missing_leaf && index + 1 == tokens.len() => {
                    return Ok(ResolvedProperty {
                        value: None,
                        pointer,
                        serialized_path,
                    })
                }
                None => return Err(format!("property.unknown_field: {path} in host {id}")),
            }
        }
        Ok(ResolvedProperty {
            value: Some(current),
            pointer,
            serialized_path,
        })
    }

    pub fn lower_write(
        &self,
        id: &str,
        path: &str,
        value: &Value,
    ) -> Result<AssetOperation, String> {
        if path.is_empty() {
            return Err("property.invalid_path: writes require propertyPath".into());
        }
        let resolved = self.resolve(id, path, true)?;
        let mut operation =
            json!({"op":"set", "object_id":id, "property_path":resolved.pointer, "value":value});
        if let Some(current) = resolved.value {
            operation["value"] = super::property_values::lower(current, value)?;
        }
        if let Some(action) = value.get("action").and_then(Value::as_str) {
            if resolved.value.is_some_and(Value::is_array) {
                operation.as_object_mut().unwrap().remove("value");
                match action {
                    "resize" => {
                        operation["op"] = json!("array_resize");
                        operation["size"] = value["size"].clone();
                        if let Some(fill) = value.get("value") {
                            operation["value"] = fill.clone();
                        }
                    }
                    "insert" => {
                        operation["op"] = json!("array_insert");
                        operation["index"] = value["index"].clone();
                        operation["value"] = value
                            .get("value")
                            .ok_or(
                                "property.explicit_value_required: array insertion needs a value",
                            )?
                            .clone();
                    }
                    "delete" => {
                        operation["op"] = json!("array_remove");
                        operation["index"] = value["index"].clone();
                    }
                    "move" => {
                        operation["op"] = json!("array_move");
                        operation["index"] = value["index"].clone();
                        operation["to_index"] = value["toIndex"].clone();
                    }
                    _ => return Err(format!("property.unsupported_command: {action}")),
                }
            } else if matches!(
                action,
                "restore" | "setType" | "resize" | "insert" | "delete" | "move"
            ) {
                return Err(format!("property.unsupported_command: {action}"));
            }
        } else if let (Some(current), Some(text)) = (resolved.value, value.as_str()) {
            if matches!(
                current.get("kind").and_then(Value::as_str),
                Some("int64" | "uint64")
            ) {
                operation["value"] = json!({"kind":current["kind"],"value":text});
            }
        }
        serde_json::from_value(operation)
            .map_err(|error| format!("property.invalid_operation: {error}"))
    }

    /// Resolve against the evolving logical graph. A prior alias reassignment or
    /// array move must affect the address of later writes in this same batch.
    pub fn lower_writes(
        &self,
        writes: &[(String, String, Value)],
    ) -> Result<Vec<AssetOperation>, String> {
        self.stage_writes(writes).map(|(operations, _)| operations)
    }

    /// Return the ordered logical result as well as every original operation.
    /// Callers must validate all operations before coalescing the persisted form.
    /// The returned model's snapshot remains the input; resolve() sees staged data.
    pub fn stage_writes(
        &self,
        writes: &[(String, String, Value)],
    ) -> Result<(Vec<AssetOperation>, Self), String> {
        let mut staged = self.clone();
        let mut operations = Vec::with_capacity(writes.len());
        for (index, (id, path, value)) in writes.iter().enumerate() {
            let operation = staged
                .lower_write(id, path, value)
                .map_err(|e| format!("operation {index}: {e}"))?;
            let resolved = staged.resolve(id, path, true)?;
            let tokens = property_tokens(&resolved.serialized_path)?;
            let pointer = tokens
                .iter()
                .map(|token| match token {
                    PropertyToken::Field(name) => escape_pointer(name),
                    PropertyToken::Index(index) => index.to_string(),
                })
                .fold(String::new(), |path, token| format!("{path}/{token}"));
            let document = staged
                .documents
                .get_mut(id)
                .ok_or("property.unknown_object")?;
            let target = document
                .value
                .pointer_mut(&pointer)
                .ok_or_else(|| format!("property.unknown_field: {path}"))?;
            match &operation {
                AssetOperation::Set { value, .. } => *target = value.clone(),
                AssetOperation::ArrayInsert { index, value, .. } => {
                    let array = target.as_array_mut().ok_or("property.invalid_array")?;
                    if *index > array.len() {
                        return Err("property.invalid_index".into());
                    }
                    array.insert(*index, value.clone());
                }
                AssetOperation::ArrayRemove { index, .. } => {
                    let array = target.as_array_mut().ok_or("property.invalid_array")?;
                    if *index >= array.len() {
                        return Err("property.invalid_index".into());
                    }
                    array.remove(*index);
                }
                AssetOperation::ArrayMove {
                    index, to_index, ..
                } => {
                    let array = target.as_array_mut().ok_or("property.invalid_array")?;
                    if *index >= array.len() || *to_index >= array.len() {
                        return Err("property.invalid_index".into());
                    }
                    let value = array.remove(*index);
                    array.insert(*to_index, value);
                }
                AssetOperation::ArrayResize { size, value, .. } => {
                    if *size > 1_000_000 {
                        return Err("property.limit: array too large".into());
                    }
                    let array = target.as_array_mut().ok_or("property.invalid_array")?;
                    if *size > array.len() {
                        array.resize(
                            *size,
                            value.clone().ok_or(
                                "property.explicit_value_required: growing arrays need a value",
                            )?,
                        );
                    } else {
                        array.truncate(*size);
                    }
                }
            }
            if matches!(tokens.first(),Some(PropertyToken::Field(name)) if name=="references") {
                let entries = document
                    .value
                    .pointer("/references/RefIds")
                    .and_then(Value::as_array)
                    .ok_or("property.invalid_registry")?;
                document.registry.clear();
                for (index, entry) in entries.iter().enumerate() {
                    let rid = entry
                        .get("rid")
                        .and_then(decimal)
                        .ok_or("property.invalid_registry")?;
                    if document.registry.insert(rid, index).is_some() {
                        return Err("property.invalid_registry: duplicate rid".into());
                    }
                }
            }
            operations.push(operation);
        }
        Ok((operations, staged))
    }

    /// Preserve serialized field ordering while sharing exact normalized values
    /// with the existing Agent/Inspector projection (which consumes YAML Values).
    pub fn yaml_value(&self, id: &str) -> Result<serde_yaml::Value, String> {
        let document = self.documents.get(id).ok_or("property.unknown_object")?;
        fn convert(
            value: &Value,
            path: &str,
            order: &HashMap<String, Vec<String>>,
            hints: &HashMap<String, String>,
        ) -> Result<serde_yaml::Value, String> {
            match hints.get(path).map(String::as_str) {
                Some("Boolean") => {
                    return Ok(serde_yaml::Value::Bool(
                        value == &json!(1) || value == &json!(true),
                    ))
                }
                Some("Float") => {
                    if let Some(number) = value.as_f64() {
                        return serde_yaml::to_value(number).map_err(|e| e.to_string());
                    }
                }
                _ => {}
            }
            if let Some(kind) = value.get("kind").and_then(Value::as_str) {
                if value.as_object().is_some_and(|map| map.len() == 2) {
                    let raw = value["value"].as_str().unwrap_or("");
                    if kind == "int64" {
                        return raw
                            .parse::<i64>()
                            .map(|v| serde_yaml::Value::Number(v.into()))
                            .or_else(|_| Ok(serde_yaml::Value::String(raw.into())));
                    }
                    if kind == "uint64" {
                        return raw
                            .parse::<u64>()
                            .map(|v| serde_yaml::Value::Number(v.into()))
                            .or_else(|_| Ok(serde_yaml::Value::String(raw.into())));
                    }
                }
            }
            match value {
                Value::Object(fields) => {
                    let mut mapping = serde_yaml::Mapping::new();
                    let keys = order
                        .get(path)
                        .cloned()
                        .unwrap_or_else(|| fields.keys().cloned().collect());
                    let known: HashSet<_> = keys.iter().collect();
                    for key in keys
                        .iter()
                        .chain(fields.keys().filter(|key| !known.contains(key)))
                    {
                        if let Some(child) = fields.get(key) {
                            mapping.insert(
                                serde_yaml::Value::String(key.clone()),
                                convert(
                                    child,
                                    &format!("{path}/{}", escape_pointer(key)),
                                    order,
                                    hints,
                                )?,
                            );
                        }
                    }
                    Ok(serde_yaml::Value::Mapping(mapping))
                }
                Value::Array(values) => values
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        let key = if path.ends_with("/references/RefIds") {
                            value
                                .get("rid")
                                .and_then(decimal)
                                .map(|rid| format!("@rid={rid}"))
                        } else {
                            None
                        }
                        .unwrap_or_else(|| index.to_string());
                        convert(
                            value,
                            &format!("{path}/{}", escape_pointer(&key)),
                            order,
                            hints,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(serde_yaml::Value::Sequence),
                _ => serde_yaml::to_value(value).map_err(|e| e.to_string()),
            }
        }
        convert(
            &document.value,
            &format!("/{}", escape_pointer(&document.root_type)),
            &document.field_order,
            &document.type_hints,
        )
    }
}

#[cfg(test)]
mod tests;
