//! Backend-independent edits against one immutable serialized asset snapshot.
//!
//! This API contains no filesystem, Git, Editor or project state. A caller must
//! compare `revision` and persist `bytes` under its own exclusive write lock.
//! JSON pointers include the document root and use the same stable sequence
//! selectors as the merge API. Integer envelopes and string reference IDs make
//! the public JSON safe to pass through JavaScript without rounding.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use super::validation::{path_child, pptr};
use super::{
    parse, sequence_keys, validate, Asset, CoreError, Diagnostic, MergeSession, Node, NodeKind,
    PackedElement, Resolution, ScalarStyle, Severity, Span,
};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Source-proven interpretation for scalar spellings ambiguous in raw YAML.
#[derive(Debug, Clone, Copy)]
pub enum ScalarHint {
    String,
    Boolean,
    Integer,
    Float,
    PackedArray(PackedElement),
}
pub type ScalarHints = BTreeMap<String, BTreeMap<String, ScalarHint>>;
type SpanHints = HashMap<usize, ScalarHint>;

fn span_hints(asset: &Asset, hints: &ScalarHints) -> SpanHints {
    let mut result = SpanHints::new();
    fn visit(
        asset: &Asset,
        node: &Node,
        path: &str,
        fields: &BTreeMap<String, ScalarHint>,
        result: &mut SpanHints,
    ) {
        if let Some(hint) = fields.get(path) {
            if node.is_scalar() || matches!(hint, ScalarHint::PackedArray(_)) {
                result.insert(node.span.start, *hint);
            }
        }
        if pptr(node) || is_managed_reference(node) {
            return;
        }
        match &node.kind {
            NodeKind::Mapping(entries) => {
                for entry in entries {
                    visit(
                        asset,
                        &entry.value,
                        &path_child(path, &entry.key),
                        fields,
                        result,
                    );
                }
            }
            NodeKind::Sequence(items) => {
                let keys = sequence_keys(asset, node, path);
                for (index, item) in items.iter().enumerate() {
                    let key = keys
                        .as_ref()
                        .map(|keys| keys[index].clone())
                        .unwrap_or_else(|| index.to_string());
                    visit(asset, &item.value, &path_child(path, &key), fields, result);
                }
            }
            _ => {}
        }
    }
    for doc in &asset.documents {
        if let Some(fields) = hints
            .get(&doc.object_id)
            .filter(|fields| !fields.is_empty())
        {
            visit(asset, &doc.root, "", fields, &mut result);
        }
    }
    // PropertyModification.value is a string even when Unity emits it without
    // quotes (007, null, 1e3, etc.). Its interpretation comes from the source
    // field, never from the lexical shape of the override record itself.
    for doc in asset
        .documents
        .iter()
        .filter(|d| d.class_id.as_deref() == Some("1001"))
    {
        if let Some(items) = doc
            .root
            .get("PrefabInstance")
            .and_then(|n| n.get("m_Modification"))
            .and_then(|n| n.get("m_Modifications"))
            .and_then(Node::items)
        {
            for item in items {
                for key in ["value", "propertyPath"] {
                    if let Some(node) = item.value.get(key).filter(|n| n.is_scalar()) {
                        result.insert(node.span.start, ScalarHint::String);
                    }
                }
            }
        }
    }
    result
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetSnapshot {
    pub revision: String,
    pub objects: Vec<AssetObject>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetObject {
    pub object_id: String,
    pub class_id: Option<String>,
    pub root_type: String,
    pub fields: Vec<AssetField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetField {
    pub property_path: String,
    /// Serialized shape, not a substitute for a C# field schema. YAML encodes
    /// booleans and enums as integers and may omit a float's fractional part.
    pub kind: String,
    pub value: Value,
    /// Optional source-proven display type; serialized values retain their
    /// language-neutral shape (e.g. bool stays 0/1 in the raw asset API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssetOperation {
    Set {
        object_id: String,
        property_path: String,
        value: Value,
    },
    ArrayInsert {
        object_id: String,
        property_path: String,
        index: usize,
        value: Value,
    },
    ArrayRemove {
        object_id: String,
        property_path: String,
        index: usize,
    },
    ArrayMove {
        object_id: String,
        property_path: String,
        index: usize,
        to_index: usize,
    },
    /// Growing requires an explicit fill value. Unlike SerializedProperty's
    /// implicit resize this API never duplicates an arbitrary previous element.
    ArrayResize {
        object_id: String,
        property_path: String,
        size: usize,
        #[serde(default)]
        value: Option<Value>,
    },
}

impl AssetOperation {
    pub fn object_id(&self) -> &str {
        match self {
            Self::Set { object_id, .. }
            | Self::ArrayInsert { object_id, .. }
            | Self::ArrayRemove { object_id, .. }
            | Self::ArrayMove { object_id, .. }
            | Self::ArrayResize { object_id, .. } => object_id,
        }
    }
    pub fn property_path(&self) -> &str {
        match self {
            Self::Set { property_path, .. }
            | Self::ArrayInsert { property_path, .. }
            | Self::ArrayRemove { property_path, .. }
            | Self::ArrayMove { property_path, .. }
            | Self::ArrayResize { property_path, .. } => property_path,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditOutput {
    pub bytes: Vec<u8>,
    pub snapshot: AssetSnapshot,
    pub applied_operations: usize,
}

/// Inspect raw serialized values, including containers and their descendants.
/// The returned paths are immediately reusable by `edit` and merge decisions.
pub fn inspect(bytes: &[u8]) -> Result<AssetSnapshot, CoreError> {
    inspect_with_hints(bytes, &ScalarHints::new())
}

pub fn inspect_with_hints(bytes: &[u8], hints: &ScalarHints) -> Result<AssetSnapshot, CoreError> {
    snapshot(&parse(bytes)?, hints)
}

fn snapshot(asset: &Asset, hints: &ScalarHints) -> Result<AssetSnapshot, CoreError> {
    let hints = span_hints(asset, hints);
    let mut objects = Vec::with_capacity(asset.documents.len());
    for document in &asset.documents {
        let mut fields = Vec::new();
        let roots = document.root.entries().ok_or_else(|| {
            CoreError::new(
                "document_root",
                "Unity document root must be a mapping",
                document.root.span.start,
            )
        })?;
        if roots.len() != 1 {
            return Err(CoreError::new(
                "document_root",
                "Unity document must contain exactly one root type",
                document.root.span.start,
            ));
        }
        let root = &roots[0];
        let root_path = path_child("", &root.key);
        collect_fields(asset, &root.value, &root_path, false, &mut fields, &hints)?;
        objects.push(AssetObject {
            object_id: document.object_id.clone(),
            class_id: document.class_id.clone(),
            root_type: root.key.clone(),
            fields,
        });
    }
    Ok(AssetSnapshot {
        revision: asset.content_hash.clone(),
        objects,
        diagnostics: validate(asset),
    })
}

fn collect_fields(
    asset: &Asset,
    node: &Node,
    path: &str,
    include: bool,
    fields: &mut Vec<AssetField>,
    hints: &SpanHints,
) -> Result<(), CoreError> {
    if include {
        let value = node_value(asset, node, hints)?;
        fields.push(AssetField {
            property_path: path.into(),
            kind: value_kind(node, &value).into(),
            value,
            type_hint: match hints.get(&node.span.start) {
                Some(ScalarHint::Boolean) => Some("Boolean".into()),
                Some(ScalarHint::Float) => Some("Float".into()),
                Some(ScalarHint::String) => Some("String".into()),
                _ => None,
            },
        });
    }
    match &node.kind {
        NodeKind::Mapping(entries) => {
            // References are atomic values. Exposing /fileID or /rid would
            // permit callers to bypass reference semantics and exact-ID rules.
            if pptr(node) || is_managed_reference(node) {
                return Ok(());
            }
            for entry in entries {
                collect_fields(
                    asset,
                    &entry.value,
                    &path_child(path, &entry.key),
                    true,
                    fields,
                    hints,
                )?;
            }
        }
        NodeKind::Sequence(items) => {
            let keys = sequence_keys(asset, node, path);
            for (index, item) in items.iter().enumerate() {
                let key = keys
                    .as_ref()
                    .map(|keys| keys[index].clone())
                    .unwrap_or_else(|| index.to_string());
                let first_field = fields.len();
                collect_fields(
                    asset,
                    &item.value,
                    &path_child(path, &key),
                    true,
                    fields,
                    hints,
                )?;
                // The array's declared element type applies to newly inserted
                // elements too; per-index hints captured before editing do not.
                let element_hint = match hints.get(&node.span.start) {
                    Some(ScalarHint::PackedArray(PackedElement::Bool)) => Some("Boolean"),
                    Some(ScalarHint::PackedArray(PackedElement::F32 | PackedElement::F64)) => {
                        Some("Float")
                    }
                    _ => None,
                };
                if let Some(hint) = element_hint {
                    for field in &mut fields[first_field..] {
                        field.type_hint = Some(hint.into());
                    }
                }
            }
        }
        NodeKind::Scalar(_) => {
            if let Some(ScalarHint::PackedArray(element)) = hints.get(&node.span.start) {
                for (index, value) in super::packed::decode(asset.text(node.span), *element)?
                    .into_iter()
                    .enumerate()
                {
                    fields.push(AssetField {
                        property_path: path_child(path, &index.to_string()),
                        kind: value_kind(node, &value).into(),
                        value,
                        type_hint: match element {
                            PackedElement::Bool => Some("Boolean".into()),
                            PackedElement::F32 | PackedElement::F64 => Some("Float".into()),
                            _ => None,
                        },
                    });
                }
            }
        }
    }
    Ok(())
}

/// Apply operations sequentially to a private buffer. A failure exposes no
/// candidate bytes; the original input is never changed. A final validation
/// additionally checks Unity-local references and ownership constraints.
pub fn edit(bytes: &[u8], operations: &[AssetOperation]) -> Result<EditOutput, CoreError> {
    edit_with_hints(bytes, operations, &ScalarHints::new())
}

pub fn edit_with_hints(
    bytes: &[u8],
    operations: &[AssetOperation],
    hints: &ScalarHints,
) -> Result<EditOutput, CoreError> {
    let original = parse(bytes)?;
    if !original.is_writable() {
        return Err(CoreError::new(
            "unsupported_yaml",
            "asset contains unsupported or ambiguous YAML",
            0,
        ));
    }
    let mut expanded = Vec::new();
    let mut packed_targets = Vec::new();
    for (object, fields) in hints {
        for (path, hint) in fields {
            let ScalarHint::PackedArray(element) = hint else {
                continue;
            };
            if !operations.iter().any(|operation| {
                operation.object_id() == object && paths_overlap(operation.property_path(), path)
            }) {
                continue;
            }
            let node = field_at(&original, object, path)?;
            if node.is_scalar() {
                let values = super::packed::decode(original.text(node.span), *element)?;
                expanded.push((node.span, super::packed::flow(&values)?));
                packed_targets.push((object.clone(), path.clone(), *element));
            } else if node.items().is_none() {
                return Err(CoreError::new(
                    "packed_array",
                    "compact primitive array requires a scalar or sequence",
                    node.span.start,
                ));
            }
        }
    }
    let mut current = if expanded.is_empty() {
        original
    } else {
        parse(&replace_spans(&original.bytes, expanded)?)?
    };
    let mut index = 0;
    while index < operations.len() {
        let mut end = index + 1;
        if matches!(operations[index], AssetOperation::Set { .. }) {
            // Disjoint Set operations observe the same immutable snapshot and
            // can share parsing, diff construction and reference validation.
            // Arrays and overlapping paths are barriers, preserving ordering.
            let mut paths: HashMap<&str, BTreeSet<&str>> = HashMap::new();
            paths
                .entry(operations[index].object_id())
                .or_default()
                .insert(operations[index].property_path());
            while end < operations.len() && matches!(operations[end], AssetOperation::Set { .. }) {
                let operation = &operations[end];
                let seen = paths.entry(operation.object_id()).or_default();
                let path = operation.property_path();
                // A repeated scalar can coalesce even when independent writes
                // separate it. Parent/child updates remain ordering barriers.
                let duplicate = seen.contains(path);
                let ancestor = path
                    .match_indices('/')
                    .skip(1)
                    .any(|(offset, _)| seen.contains(&path[..offset]));
                let prefix = format!("{path}/");
                let descendant = seen
                    .range(prefix.as_str()..)
                    .next()
                    .is_some_and(|candidate| candidate.starts_with(&prefix));
                if ancestor
                    || descendant
                    || (duplicate
                        && !field_at(&current, operation.object_id(), path)
                            .is_ok_and(Node::is_scalar))
                {
                    break;
                }
                seen.insert(path);
                end += 1;
            }
        }
        let result = if end > index + 1 {
            apply_sets(&current, &operations[index..end], index, hints)
        } else {
            apply_operation(&current, &operations[index], hints)
        };
        current = result.map_err(|mut error| {
            if !error.message.starts_with("operation ") {
                error.message = format!("operation {index}: {}", error.message);
            }
            error
        })?;
        index = end;
    }
    if !packed_targets.is_empty() {
        let spans = span_hints(&current, hints);
        let mut encoded = Vec::new();
        for (object, path, element) in packed_targets {
            let node = match field_at(&current, &object, &path) {
                Ok(node) => node,
                Err(error) if error.code == "unknown_field" => continue,
                Err(error) => return Err(error),
            };
            let value = node_value(&current, node, &spans)?;
            let values = value.as_array().ok_or_else(|| {
                CoreError::new(
                    "packed_array",
                    "compact array result must be a logical array",
                    node.span.start,
                )
            })?;
            encoded.push((
                node.span,
                super::packed::encode(values, element)?.into_bytes(),
            ));
        }
        current = parse(&replace_spans(&current.bytes, encoded)?)?;
    }
    let snapshot = snapshot(&current, hints)?;
    Ok(EditOutput {
        bytes: current.bytes.to_vec(),
        snapshot,
        applied_operations: operations.len(),
    })
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left
            .strip_prefix(right)
            .is_some_and(|tail| tail.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|tail| tail.starts_with('/'))
}

fn replace_spans(bytes: &[u8], mut patches: Vec<(Span, Vec<u8>)>) -> Result<Vec<u8>, CoreError> {
    patches.sort_by_key(|(span, _)| span.start);
    let mut output = Vec::new();
    let mut cursor = 0;
    for (span, replacement) in patches {
        if span.start < cursor {
            return Err(CoreError::new(
                "overlapping_patch",
                "compact array spans overlap",
                span.start,
            ));
        }
        output.extend_from_slice(&bytes[cursor..span.start]);
        output.extend_from_slice(&replacement);
        cursor = span.end;
    }
    output.extend_from_slice(&bytes[cursor..]);
    Ok(output)
}

fn apply_sets(
    asset: &Asset,
    operations: &[AssetOperation],
    start: usize,
    hints: &ScalarHints,
) -> Result<Asset, CoreError> {
    let hints = span_hints(asset, hints);
    // Index once for wide documents instead of searching the mapping for each
    // scalar write. Atomic references are deliberately not indexed below slots.
    fn index_nodes<'a>(
        asset: &Asset,
        node: &'a Node,
        path: String,
        out: &mut HashMap<String, &'a Node>,
    ) {
        if path
            .strip_prefix('/')
            .is_some_and(|path| path.contains('/'))
        {
            out.insert(path.clone(), node);
        }
        if pptr(node) || is_managed_reference(node) {
            return;
        }
        match &node.kind {
            NodeKind::Mapping(entries) => {
                for entry in entries {
                    index_nodes(asset, &entry.value, path_child(&path, &entry.key), out);
                }
            }
            NodeKind::Sequence(items) => {
                let keys = sequence_keys(asset, node, &path);
                for (index, item) in items.iter().enumerate() {
                    let key = keys
                        .as_ref()
                        .map(|keys| keys[index].clone())
                        .unwrap_or_else(|| index.to_string());
                    index_nodes(asset, &item.value, path_child(&path, &key), out);
                }
            }
            _ => {}
        }
    }
    let mut indexes = HashMap::new();
    for doc in &asset.documents {
        let mut fields = HashMap::new();
        index_nodes(asset, &doc.root, String::new(), &mut fields);
        indexes.insert(doc.object_id.as_str(), fields);
    }
    let mut scalar_only = true;
    let mut scalar_patches = BTreeMap::new();
    let mut decisions = Vec::with_capacity(operations.len());
    let mut positions = HashMap::new();
    for (index, operation) in operations.iter().enumerate() {
        let AssetOperation::Set {
            object_id,
            property_path,
            value,
        } = operation
        else {
            unreachable!()
        };
        let value = (|| {
            let node = indexes
                .get(object_id.as_str())
                .and_then(|fields| fields.get(property_path))
                .copied()
                .map(Ok)
                .unwrap_or_else(|| field_at(asset, object_id, property_path))?;
            let value = decode_value(value, None)?;
            compatible(asset, node, &value, &hints)?;
            if matches!(
                node.kind,
                NodeKind::Scalar(
                    ScalarStyle::Plain
                        | ScalarStyle::Null
                        | ScalarStyle::SingleQuoted
                        | ScalarStyle::DoubleQuoted
                )
            ) && !asset.text(node.span).contains(['\r', '\n'])
            {
                if decode_value(&node_value(asset, node, &hints)?, None)? == value {
                    scalar_patches.remove(&node.span.start);
                } else {
                    scalar_patches.insert(
                        node.span.start,
                        (node.span, super::merge::json_yaml(&value)?.into_bytes()),
                    );
                }
            } else {
                scalar_only = false;
            }
            Ok::<_, CoreError>(value)
        })()
        .map_err(|mut error| {
            error.message = format!("operation {}: {}", start + index, error.message);
            error
        })?;
        let decision =
            MergeSession::field_decision(object_id, property_path, Resolution::Set { value });
        // Validation above still runs for every request. Only the immutable
        // scalar patch is coalesced, so malformed earlier writes cannot vanish.
        if let Some(position) = positions.get(&(object_id, property_path)) {
            decisions[*position] = decision;
        } else {
            positions.insert((object_id, property_path), decisions.len());
            decisions.push(decision);
        }
    }
    if scalar_only {
        let updated = parse(&replace_spans(
            &asset.bytes,
            scalar_patches.into_values().collect(),
        )?)?;
        let baseline = validate(asset);
        if let Some(error) = validate(&updated).into_iter().find(|d| {
            d.severity == Severity::Error
                && !baseline.iter().any(|old| {
                    old.code == d.code
                        && old.object_id == d.object_id
                        && old.property_path == d.property_path
                        && old.message == d.message
                })
        }) {
            return Err(CoreError::new(
                &error.code,
                &error.message,
                error.span.start,
            ));
        }
        return Ok(updated);
    }
    let shared = Arc::new(asset.clone());
    let session = MergeSession::from_shared(Arc::clone(&shared), Arc::clone(&shared), shared)?;
    let output = session.render(&decisions)?;
    if !output.ready {
        let conflict = output
            .conflicts
            .first()
            .expect("unready merge has conflicts");
        return Err(CoreError::new(&conflict.code, &conflict.message, 0));
    }
    parse(&output.bytes)
}

/// Shape/value validation without applying the edits to a source object. Prefab
/// overrides belong to the instance; their values still obey the source schema.
pub fn validate_set_values(
    bytes: &[u8],
    operations: &[AssetOperation],
    hints: &ScalarHints,
) -> Result<(), CoreError> {
    let asset = parse(bytes)?;
    let hints = span_hints(&asset, hints);
    for (index, operation) in operations.iter().enumerate() {
        let AssetOperation::Set {
            object_id,
            property_path,
            value,
        } = operation
        else {
            return Err(CoreError::new("invalid_operation", "expected Set", 0));
        };
        let result = (|| {
            let node = field_at(&asset, object_id, property_path)?;
            let value = decode_asset_value(value)?;
            compatible(&asset, node, &value, &hints)
        })();
        result.map_err(|mut error| {
            error.message = format!("operation {index}: {}", error.message);
            error
        })?;
    }
    Ok(())
}

fn apply_operation(
    asset: &Asset,
    operation: &AssetOperation,
    hints: &ScalarHints,
) -> Result<Asset, CoreError> {
    let hints = span_hints(asset, hints);
    let object_id = operation.object_id();
    let path = operation.property_path();
    let node = field_at(asset, object_id, path)?;
    let value = match operation {
        AssetOperation::Set { value, .. } => {
            let normalized = decode_value(value, None)?;
            compatible(asset, node, &normalized, &hints)?;
            normalized
        }
        _ => {
            let items = node.items().ok_or_else(|| {
                CoreError::new(
                    "type_mismatch",
                    "array operation requires an array field",
                    node.span.start,
                )
            })?;
            let mut values = items
                .iter()
                .map(|item| {
                    node_value(asset, &item.value, &hints)
                        .and_then(|value| decode_value(&value, None))
                })
                .collect::<Result<Vec<_>, _>>()?;
            match operation {
                AssetOperation::ArrayInsert { index, value, .. } => {
                    if *index > values.len() {
                        return Err(index_error(node, *index, values.len(), true));
                    }
                    let value = decode_value(value, None)?;
                    if let Some(template) = items.first() {
                        compatible(asset, &template.value, &value, &hints)?;
                    }
                    values.insert(*index, value);
                }
                AssetOperation::ArrayRemove { index, .. } => {
                    if *index >= values.len() {
                        return Err(index_error(node, *index, values.len(), false));
                    }
                    values.remove(*index);
                }
                AssetOperation::ArrayMove {
                    index, to_index, ..
                } => {
                    if *index >= values.len() {
                        return Err(index_error(node, *index, values.len(), false));
                    }
                    if *to_index >= values.len() {
                        return Err(index_error(node, *to_index, values.len(), false));
                    }
                    let value = values.remove(*index);
                    values.insert(*to_index, value);
                }
                AssetOperation::ArrayResize { size, value, .. } => {
                    if *size > values.len() {
                        let fill = value.as_ref().ok_or_else(|| {
                            CoreError::new(
                                "array_fill_required",
                                "growing an array requires an explicit value",
                                node.span.start,
                            )
                        })?;
                        let fill = decode_value(fill, None)?;
                        if let Some(template) = items.first() {
                            compatible(asset, &template.value, &fill, &hints)?;
                        }
                        // Prevent an untrusted request from allocating beyond
                        // the parser's bounded node budget before validation.
                        let limits = super::Limits::default();
                        if *size > limits.max_nodes {
                            return Err(CoreError::new(
                                "node_limit",
                                "array size exceeds asset node limit",
                                node.span.start,
                            ));
                        }
                        let added_bytes = fill
                            .to_string()
                            .len()
                            .saturating_add(2)
                            .saturating_mul(size.saturating_sub(values.len()));
                        if asset.bytes.len().saturating_add(added_bytes) > limits.max_bytes {
                            return Err(CoreError::new(
                                "byte_limit",
                                "resized array exceeds asset byte limit",
                                node.span.start,
                            ));
                        }
                        values.resize(*size, fill);
                    } else {
                        values.truncate(*size);
                    }
                }
                AssetOperation::Set { .. } => unreachable!(),
            }
            Value::Array(values)
        }
    };
    let shared = Arc::new(asset.clone());
    let session = MergeSession::from_shared(Arc::clone(&shared), Arc::clone(&shared), shared)?;
    let decision = MergeSession::field_decision(object_id, path, Resolution::Set { value });
    let output = session.render(&[decision])?;
    if !output.ready {
        let conflict = output
            .conflicts
            .first()
            .expect("unready merge has conflicts");
        return Err(CoreError::new(
            &conflict.code,
            &conflict.message,
            node.span.start,
        ));
    }
    let updated = parse(&output.bytes)?;
    if let Some(diagnostic) = updated
        .diagnostics
        .iter()
        .find(|d| d.severity == Severity::Error)
    {
        return Err(CoreError::new(
            &diagnostic.code,
            &diagnostic.message,
            diagnostic.span.start,
        ));
    }
    Ok(updated)
}

fn index_error(node: &Node, index: usize, length: usize, insert: bool) -> CoreError {
    CoreError::new(
        "array_index",
        format!(
            "index {index} is out of bounds for array length {length}{}",
            if insert {
                " (insertion at length is allowed)"
            } else {
                ""
            }
        ),
        node.span.start,
    )
}

fn field_at<'a>(asset: &'a Asset, object_id: &str, path: &str) -> Result<&'a Node, CoreError> {
    let document = asset.document(object_id).ok_or_else(|| {
        CoreError::new(
            "unknown_object",
            format!("asset has no object {object_id}"),
            0,
        )
    })?;
    if !path.starts_with('/') || path[1..].split('/').count() < 2 {
        return Err(CoreError::new(
            "property_path",
            "field path must include the root type and a field",
            0,
        ));
    }
    let mut node = &document.root;
    let mut walked = String::new();
    for token in path[1..].split('/') {
        let token = decode_token(token)?;
        if pptr(node) || is_managed_reference(node) {
            return Err(CoreError::new(
                "reference_scope",
                "assign the complete object or managed reference instead of an identity member",
                node.span.start,
            ));
        }
        node = match &node.kind {
            NodeKind::Mapping(entries) => entries
                .iter()
                .find(|entry| entry.key == token)
                .map(|entry| &entry.value),
            NodeKind::Sequence(items) => {
                let index = if token.starts_with('@') {
                    sequence_keys(asset, node, &walked)
                        .and_then(|keys| keys.iter().position(|key| key == &token))
                } else {
                    token.parse::<usize>().ok()
                };
                index
                    .and_then(|index| items.get(index))
                    .map(|item| &item.value)
            }
            NodeKind::Scalar(_) => None,
        }
        .ok_or_else(|| {
            CoreError::new(
                "unknown_field",
                format!("object {object_id} has no field {path}"),
                node.span.start,
            )
        })?;
        walked = path_child(&walked, &token);
    }
    Ok(node)
}

fn decode_token(token: &str) -> Result<String, CoreError> {
    let mut result = String::new();
    let mut chars = token.chars();
    while let Some(character) = chars.next() {
        if character != '~' {
            result.push(character);
            continue;
        }
        match chars.next() {
            Some('0') => result.push('~'),
            Some('1') => result.push('/'),
            _ => {
                return Err(CoreError::new(
                    "property_path",
                    "invalid JSON pointer escape",
                    0,
                ))
            }
        }
    }
    Ok(result)
}

fn is_managed_reference(node: &Node) -> bool {
    node.entries()
        .is_some_and(|entries| entries.len() == 1 && entries[0].key == "rid")
}

fn value_kind<'a>(node: &Node, value: &'a Value) -> &'a str {
    if pptr(node) {
        return "object_reference";
    }
    if is_managed_reference(node) {
        return "managed_reference";
    }
    match value {
        Value::Null => "null",
        Value::Bool(_) => "integer",
        Value::String(_) => "string",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::Array(_) => "array",
        Value::Object(map)
            if matches!(
                map.get("kind").and_then(Value::as_str),
                Some("int64" | "uint64")
            ) && map.len() == 2 =>
        {
            "integer"
        }
        Value::Object(map)
            if map.get("kind").and_then(Value::as_str) == Some("float64") && map.len() == 2 =>
        {
            "number"
        }
        Value::Object(_) => "object",
    }
}

fn exact_integer(raw: &str, offset: usize) -> Result<Number, CoreError> {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CoreError::new(
            "integer_range",
            "integer value must be a decimal integer string",
            offset,
        ));
    }
    raw.parse::<i64>()
        .map(Number::from)
        .or_else(|_| raw.parse::<u64>().map(Number::from))
        .map_err(|_| {
            CoreError::new(
                "integer_range",
                format!("{raw} is not an exact 64-bit decimal integer"),
                offset,
            )
        })
}

pub(crate) fn canonical_number(number: Number) -> Value {
    let unsafe_integer = number
        .as_i64()
        .is_some_and(|n| !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n))
        || number.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER as u64);
    if unsafe_integer {
        let kind = if number.as_u64().is_some_and(|value| value > i64::MAX as u64) {
            "uint64"
        } else {
            "int64"
        };
        serde_json::json!({"kind":kind,"value":number.to_string()})
    } else {
        Value::Number(number)
    }
}

fn node_value(asset: &Asset, node: &Node, hints: &SpanHints) -> Result<Value, CoreError> {
    if node.is_scalar() {
        match hints.get(&node.span.start) {
            Some(ScalarHint::String) => return scalar_string(asset, node).map(Value::String),
            Some(ScalarHint::Boolean) => {
                let raw = scalar_string(asset, node)?;
                return match raw.as_str() {
                    "0" | "false" => Ok(serde_json::json!(0)),
                    "1" | "true" => Ok(serde_json::json!(1)),
                    _ => Err(CoreError::new(
                        "schema_value",
                        "serialized boolean must be 0/1 or false/true",
                        node.span.start,
                    )),
                };
            }
            Some(ScalarHint::Float) => {
                if let Ok(number) = scalar_string(asset, node)?.parse::<f64>() {
                    if number.is_finite() {
                        return if number.fract() == 0.0 && number.abs() <= MAX_SAFE_INTEGER as f64 {
                            Ok(Value::Number(Number::from(number as i64)))
                        } else {
                            Ok(Value::Number(
                                Number::from_f64(number).expect("finite float"),
                            ))
                        };
                    }
                }
            }
            Some(ScalarHint::PackedArray(element)) => {
                return super::packed::decode(asset.text(node.span), *element).map(Value::Array)
            }
            _ => {}
        }
    }
    match &node.kind {
        NodeKind::Mapping(entries) => {
            let reference = pptr(node);
            let managed = is_managed_reference(node);
            let mut map = Map::new();
            for entry in entries {
                let value = if (reference && (entry.key == "fileID" || entry.key == "guid"))
                    || (managed && entry.key == "rid")
                {
                    Value::String(scalar_string(asset, &entry.value)?)
                } else {
                    node_value(asset, &entry.value, hints)?
                };
                map.insert(entry.key.clone(), value);
            }
            Ok(Value::Object(map))
        }
        NodeKind::Sequence(items) => items
            .iter()
            .map(|item| node_value(asset, &item.value, hints))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        NodeKind::Scalar(ScalarStyle::Empty) => Ok(Value::String(String::new())),
        NodeKind::Scalar(ScalarStyle::Null) => Ok(Value::Null),
        NodeKind::Scalar(ScalarStyle::Plain) => {
            let raw = asset.text(node.span).trim();
            if let Ok(number) = raw.parse::<i64>().map(Number::from) {
                return Ok(canonical_number(number));
            }
            if let Ok(number) = raw.parse::<u64>().map(Number::from) {
                return Ok(canonical_number(number));
            }
            let digits = raw.strip_prefix('-').unwrap_or(raw);
            if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
                // Preserve an unsupported wide integer exactly on inspection;
                // decoding its envelope explicitly rejects the range on edits.
                return Ok(serde_json::json!({"kind":"int64","value":raw}));
            }
            if matches!(
                raw.to_ascii_lowercase().as_str(),
                ".nan" | ".inf" | "-.inf" | "+.inf" | "nan" | "infinity" | "-infinity"
            ) {
                let value = match raw.to_ascii_lowercase().as_str() {
                    ".nan" | "nan" => ".nan",
                    "-.inf" | "-infinity" => "-.inf",
                    _ => ".inf",
                };
                return Ok(serde_json::json!({"kind":"float64","value":value}));
            }
            if let Ok(number) = raw.parse::<f64>() {
                if number.fract() == 0.0
                    && number.abs() <= MAX_SAFE_INTEGER as f64
                    && !(number == 0.0 && number.is_sign_negative())
                {
                    // JSON consumers compare numbers by value. Unity may emit
                    // the same float as 1, 1.0 or 1e0 across serializers.
                    return Ok(Value::Number(Number::from(number as i64)));
                }
                if let Some(number) = Number::from_f64(number) {
                    return Ok(Value::Number(number));
                }
            }
            Ok(Value::String(scalar_string(asset, node)?))
        }
        NodeKind::Scalar(_) => Ok(Value::String(scalar_string(asset, node)?)),
    }
}

fn scalar_string(asset: &Asset, node: &Node) -> Result<String, CoreError> {
    if !node.is_scalar() {
        return Err(CoreError::new(
            "reference_value",
            "reference identity must be scalar",
            node.span.start,
        ));
    }
    let raw = asset.text(node.span);
    match node.kind {
        NodeKind::Scalar(ScalarStyle::Empty) => Ok(String::new()),
        NodeKind::Scalar(ScalarStyle::Plain | ScalarStyle::Null) => {
            // Folded plain text uses YAML's line folding, but identity-looking
            // values remain lexical strings and never pass through floats.
            if !raw.contains('\n') {
                return Ok(raw.trim().into());
            }
            serde_yaml::from_str::<String>(raw)
                .map_err(|error| CoreError::new("scalar_value", error.to_string(), node.span.start))
        }
        _ => serde_yaml::from_str::<String>(raw)
            .map_err(|error| CoreError::new("scalar_value", error.to_string(), node.span.start)),
    }
}

/// Decode the transport envelope to JSON values accepted by the lossless
/// merge renderer. Exposed for adapters that share this exact-value contract.
pub fn decode_asset_value(value: &Value) -> Result<Value, CoreError> {
    decode_value(value, None)
}

fn decode_value(value: &Value, identity_key: Option<&str>) -> Result<Value, CoreError> {
    if identity_key.is_some() {
        let raw = value.as_str().ok_or_else(|| {
            CoreError::new("reference_id", "reference IDs must be decimal strings", 0)
        })?;
        return exact_integer(raw, 0).map(Value::Number);
    }
    match value {
        Value::Object(map) if map.len() == 1 && map.contains_key("$locus_packed_array") => {
            super::packed::marker(&map["$locus_packed_array"])?;
            Ok(value.clone())
        }
        Value::Object(map)
            if map.get("kind").and_then(Value::as_str) == Some("float64") && map.len() == 2 =>
        {
            Err(CoreError::new(
                "unsupported_scalar",
                "non-finite float values cannot be rewritten by this backend",
                0,
            ))
        }
        Value::Object(map)
            if matches!(
                map.get("kind").and_then(Value::as_str),
                Some("int64" | "uint64")
            ) && map.len() == 2 =>
        {
            let kind = map["kind"].as_str().expect("integer tag");
            let raw = map.get("value").and_then(Value::as_str).ok_or_else(|| {
                CoreError::new(
                    "typed_value",
                    format!("{kind} value must be a decimal string"),
                    0,
                )
            })?;
            let number = if kind == "uint64" {
                raw.parse::<u64>().map(Number::from).ok()
            } else {
                raw.parse::<i64>().map(Number::from).ok()
            };
            number.filter(|number| number.to_string() == raw).map(Value::Number)
                .ok_or_else(|| CoreError::new("integer_range", format!("{kind} value must use canonical decimal notation within its 64-bit range"), 0))
        }
        Value::Object(map) => {
            let reference = map.contains_key("fileID")
                && map
                    .keys()
                    .all(|key| matches!(key.as_str(), "fileID" | "guid" | "type"));
            let managed = map.len() == 1 && map.contains_key("rid");
            if managed {
                let raw = map.get("rid").and_then(Value::as_str).ok_or_else(|| {
                    CoreError::new(
                        "reference_id",
                        "managed reference IDs must be decimal strings",
                        0,
                    )
                })?;
                let id = raw.parse::<i64>().map_err(|_| {
                    CoreError::new(
                        "reference_id",
                        "managed rid must be a signed 64-bit decimal string",
                        0,
                    )
                })?;
                if id.to_string() != raw {
                    return Err(CoreError::new(
                        "reference_id",
                        "managed rid must use canonical decimal notation",
                        0,
                    ));
                }
                if id == -1 {
                    return Err(CoreError::new(
                        "unsupported_capability",
                        "unknown managed reference ID -1 cannot be assigned; use -2 for null",
                        0,
                    ));
                }
            }
            if reference {
                let raw = map.get("fileID").and_then(Value::as_str).ok_or_else(|| {
                    CoreError::new("reference_id", "reference IDs must be decimal strings", 0)
                })?;
                let id = raw.parse::<i64>().map_err(|_| {
                    CoreError::new(
                        "reference_id",
                        "reference fileID must be a signed 64-bit decimal string",
                        0,
                    )
                })?;
                if id.to_string() != raw {
                    return Err(CoreError::new(
                        "reference_id",
                        "reference fileID must use canonical decimal notation",
                        0,
                    ));
                }
                if id == 0 && map.len() != 1 {
                    return Err(CoreError::new(
                        "reference_value",
                        "null object references contain only fileID",
                        0,
                    ));
                }
                if map
                    .get("type")
                    .is_some_and(|value| value.as_u64().is_none())
                {
                    return Err(CoreError::new(
                        "reference_value",
                        "reference type must be a nonnegative integer",
                        0,
                    ));
                }
            }
            let mut result = Map::new();
            for (key, value) in map {
                if reference && key == "guid" && !value.is_string() {
                    return Err(CoreError::new(
                        "reference_guid",
                        "reference GUID must be a string",
                        0,
                    ));
                }
                let identity = if (reference && key == "fileID") || (managed && key == "rid") {
                    Some(key.as_str())
                } else {
                    None
                };
                result.insert(key.clone(), decode_value(value, identity)?);
            }
            Ok(Value::Object(result))
        }
        Value::Array(items) => items
            .iter()
            .map(|item| decode_value(item, None))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Number(number) => {
            let unsafe_integer = number
                .as_i64()
                .is_some_and(|n| !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n))
                || number.as_u64().is_some_and(|n| n > MAX_SAFE_INTEGER as u64)
                || number
                    .as_f64()
                    .is_some_and(|n| n.fract() == 0.0 && n.abs() > MAX_SAFE_INTEGER as f64);
            if unsafe_integer {
                return Err(CoreError::new(
                    "unsafe_integer",
                    "integers outside the JavaScript safe range require an int64 envelope",
                    0,
                ));
            }
            Ok(value.clone())
        }
        _ => Ok(value.clone()),
    }
}

fn compatible(
    asset: &Asset,
    node: &Node,
    value: &Value,
    hints: &SpanHints,
) -> Result<(), CoreError> {
    let mismatch = || {
        CoreError::new(
            "type_mismatch",
            "value does not match the existing serialized field shape",
            node.span.start,
        )
    };
    match (&node.kind, value) {
        (NodeKind::Mapping(entries), Value::Object(map)) => {
            if pptr(node) {
                if !map.contains_key("fileID")
                    || map
                        .keys()
                        .any(|key| !matches!(key.as_str(), "fileID" | "guid" | "type"))
                {
                    return Err(mismatch());
                }
                return Ok(());
            }
            if entries.len() != map.len()
                || entries.iter().any(|entry| !map.contains_key(&entry.key))
            {
                return Err(CoreError::new(
                    "object_shape",
                    "object assignment must retain its existing fields",
                    node.span.start,
                ));
            }
            for entry in entries {
                compatible(asset, &entry.value, &map[&entry.key], hints)?;
            }
            Ok(())
        }
        (NodeKind::Sequence(items), Value::Array(values)) => {
            if let Some(template) = items.first() {
                for value in values {
                    compatible(asset, &template.value, value, hints)?;
                }
            }
            Ok(())
        }
        (NodeKind::Scalar(_), _) => {
            if let Some(hint) = hints.get(&node.span.start) {
                let accepts = match hint {
                    ScalarHint::String => value.is_string(),
                    ScalarHint::Boolean => {
                        value.is_boolean() || matches!(value.as_i64(), Some(0 | 1))
                    }
                    ScalarHint::Integer => value.as_i64().is_some() || value.as_u64().is_some(),
                    ScalarHint::Float => value.is_number(),
                    ScalarHint::PackedArray(_) => value.is_array(),
                };
                return if accepts { Ok(()) } else { Err(mismatch()) };
            }
            let existing = node_value(asset, node, hints)?;
            let accepts = match existing {
                Value::Number(_) | Value::Bool(_) => value.is_number() || value.is_boolean(),
                Value::Object(ref map)
                    if matches!(
                        map.get("kind").and_then(Value::as_str),
                        Some("int64" | "uint64")
                    ) =>
                {
                    value.is_number()
                }
                Value::String(_) => value.is_string(),
                Value::Null => value.is_null(),
                _ => false,
            };
            if accepts {
                Ok(())
            } else {
                Err(mismatch())
            }
        }
        _ => Err(mismatch()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const HEADER: &str =
        "%YAML 1.1\n%TAG !u! tag:unity3d.com,2011:\n--- !u!114 &11400000\nMonoBehaviour:\n";
    fn fixture(body: &str) -> Vec<u8> {
        format!("{HEADER}{body}").into_bytes()
    }
    fn set(path: &str, value: Value) -> AssetOperation {
        AssetOperation::Set {
            object_id: "11400000".into(),
            property_path: format!("/MonoBehaviour/{path}"),
            value,
        }
    }
    fn operation(op: Value) -> AssetOperation {
        serde_json::from_value(op).unwrap()
    }
    fn field<'a>(snapshot: &'a AssetSnapshot, path: &str) -> &'a Value {
        &snapshot.objects[0]
            .fields
            .iter()
            .find(|field| field.property_path == format!("/MonoBehaviour/{path}"))
            .unwrap()
            .value
    }

    #[test]
    fn no_op_preserves_bom_crlf_comments_and_revision() {
        let bytes = format!(
            "\u{feff}{}",
            String::from_utf8(fixture(
                "  name: 'hello' # retain\n  unknown: {x: 1, y: 2}\n"
            ))
            .unwrap()
            .replace('\n', "\r\n")
        )
        .into_bytes();
        let result = edit(&bytes, &[]).unwrap();
        assert_eq!(result.bytes, bytes);
        assert_eq!(
            result.snapshot.revision,
            blake3::hash(&bytes).to_hex().to_string()
        );
    }

    #[test]
    fn scalar_phase_coalesces_interleaved_paths_without_touching_restored_bytes() {
        let bytes = fixture("  amount: 007 # keep lexical spelling\n  other: 9\n");
        let output = edit(
            &bytes,
            &[
                set("amount", json!(8)),
                set("other", json!(10)),
                set("amount", json!(7)),
            ],
        )
        .unwrap();
        assert!(String::from_utf8(output.bytes)
            .unwrap()
            .contains("amount: 007 # keep lexical spelling"));
        assert_eq!(field(&output.snapshot, "other"), &json!(10));
        assert!(edit(
            &bytes,
            &[
                set("amount", json!({"bad":true})),
                set("other", json!(10)),
                set("amount", json!(7))
            ]
        )
        .is_err());
    }

    #[test]
    fn scalar_index_does_not_bypass_atomic_ids_or_graph_validation() {
        let bytes=fixture("  amount: 1\n  node: {rid: 1}\n  references:\n    version: 2\n    RefIds:\n    - rid: 1\n      type: {class: Node, ns: , asm: Assembly-CSharp}\n      data: {amount: 3}\n");
        assert!(edit(
            &bytes,
            &[set("amount", json!(2)), set("node/rid", json!(2))]
        )
        .is_err());
        assert!(edit(
            &bytes,
            &[
                set("amount", json!(2)),
                set("references/RefIds/@rid=1/rid", json!(2))
            ]
        )
        .is_err());
    }

    #[test]
    fn exact_int64_and_reference_ids_round_trip() {
        let bytes = fixture("  count: 9007199254740993\n  target: {fileID: 9007199254740995, guid: aabbccdd00112233445566778899aabb, type: 2}\n  min: -9223372036854775808\n");
        let initial = inspect(&bytes).unwrap();
        assert_eq!(
            field(&initial, "count"),
            &json!({"kind":"int64", "value":"9007199254740993"})
        );
        assert_eq!(field(&initial, "target")["fileID"], "9007199254740995");
        let result = edit(&bytes, &[
            set("count", json!({"kind":"int64", "value":"9223372036854775807"})),
            set("target", json!({"fileID":"9007199254740997","guid":"aabbccdd00112233445566778899aabb","type":2})),
        ]).unwrap();
        assert_eq!(
            field(&result.snapshot, "count")["value"],
            "9223372036854775807"
        );
        assert_eq!(
            field(&result.snapshot, "target")["fileID"],
            "9007199254740997"
        );
        let text = String::from_utf8(result.bytes).unwrap();
        assert!(text.contains("count: 9223372036854775807"));
        assert!(text.contains("fileID: 9007199254740997"));
        assert_eq!(
            edit(&bytes, &[set("count", json!(9007199254740993_i64))])
                .unwrap_err()
                .code,
            "unsafe_integer"
        );
    }

    #[test]
    fn scalar_edits_preserve_unselected_bytes_and_quote_strings() {
        let bytes = fixture("  count: 10\n  label: 'before'\n  enabled: 0\n  vector: {x: 1, y: 2, z: 3}\n  untouched: {a:  12, strange: 'yes'} # exact\n");
        let output = edit(
            &bytes,
            &[
                set("count", json!(12)),
                set("label", json!("after: # safe\n中文")),
                set("enabled", json!(true)),
                set("vector/x", json!(1.5)),
            ],
        )
        .unwrap();
        assert_eq!(
            field(&output.snapshot, "label"),
            &json!("after: # safe\n中文")
        );
        assert_eq!(field(&output.snapshot, "enabled"), &json!(1));
        assert!(String::from_utf8(output.bytes)
            .unwrap()
            .contains("  untouched: {a:  12, strange: 'yes'} # exact\n"));
    }

    #[test]
    fn sequential_array_edits_have_explicit_deterministic_values() {
        let bytes = fixture("  values: [10, 20, 30]\n");
        let base = json!({"object_id":"11400000","property_path":"/MonoBehaviour/values"});
        let operations = [
            json!({"op":"array_insert","index":1,"value":15}),
            json!({"op":"array_move","index":3,"to_index":0}),
            json!({"op":"array_remove","index":2}),
            json!({"op":"array_resize","size":5,"value":99}),
        ]
        .into_iter()
        .map(|mut value| {
            value
                .as_object_mut()
                .unwrap()
                .extend(base.as_object().unwrap().clone());
            operation(value)
        })
        .collect::<Vec<_>>();
        let output = edit(&bytes, &operations).unwrap();
        assert_eq!(
            field(&output.snapshot, "values"),
            &json!([30, 10, 20, 99, 99])
        );
        assert_eq!(output.applied_operations, 4);
    }

    #[test]
    fn array_rewrite_preserves_nested_values_empty_strings_and_exact_integers() {
        let bytes = fixture("  values:\n  - name:\n    count: 9007199254740993\n    target: {fileID: 0}\n  - name: 'two'\n    count: 2\n    target: {fileID: 0}\n");
        let output = edit(&bytes, &[operation(json!({"op":"array_move","object_id":"11400000", "property_path":"/MonoBehaviour/values","index":0,"to_index":1}))]).unwrap();
        assert_eq!(
            field(&output.snapshot, "values")[1]["count"]["value"],
            "9007199254740993"
        );
        assert_eq!(field(&output.snapshot, "values")[1]["name"], "");
        assert_eq!(
            field(&output.snapshot, "values")[1]["target"]["fileID"],
            "0"
        );
    }

    #[test]
    fn invalid_batch_is_all_or_nothing_and_never_guesses_target_or_array_defaults() {
        let bytes = fixture("  value: 1\n  values: [1]\n");
        for (tail, expected) in [
            (set("missing", json!(3)), "unknown_field"),
            (
                operation(
                    json!({"op":"set","object_id":"999","property_path":"/MonoBehaviour/value","value":3}),
                ),
                "unknown_object",
            ),
            (
                operation(
                    json!({"op":"array_remove","object_id":"11400000","property_path":"/MonoBehaviour/values","index":1}),
                ),
                "array_index",
            ),
            (
                operation(
                    json!({"op":"array_resize","object_id":"11400000","property_path":"/MonoBehaviour/values","size":2}),
                ),
                "array_fill_required",
            ),
            (
                set("value", json!({"action":"resize","size":10})),
                "type_mismatch",
            ),
        ] {
            let error = edit(&bytes, &[set("value", json!(2)), tail]).unwrap_err();
            assert_eq!(error.code, expected);
            assert!(error.message.starts_with("operation 1:"));
            assert_eq!(field(&inspect(&bytes).unwrap(), "value"), &json!(1));
        }
    }

    #[test]
    fn managed_registry_uses_stable_rid_paths_and_validates_links() {
        let bytes = fixture("  root: {rid: 9007199254740993}\n  references:\n    version: 2\n    RefIds:\n    - rid: 9007199254740993\n      type: {class: Node, ns: Test, asm: Assembly-CSharp}\n      data:\n        label: before\n        next: {rid: -2}\n");
        let output = edit(
            &bytes,
            &[set(
                "references/RefIds/@rid=9007199254740993/data/label",
                json!("after"),
            )],
        )
        .unwrap();
        assert_eq!(field(&output.snapshot, "root")["rid"], "9007199254740993");
        assert_eq!(
            field(
                &output.snapshot,
                "references/RefIds/@rid=9007199254740993/data/label"
            ),
            &json!("after")
        );
        assert!(edit(&bytes, &[set("root", json!({"rid":"123"}))]).is_err());
    }

    #[test]
    fn paths_escape_slashes_and_tildes_and_reject_invalid_pointer_escapes() {
        let bytes = fixture("  'key/with~chars': 1\n");
        let output = edit(&bytes, &[set("key~1with~0chars", json!(2))]).unwrap();
        assert_eq!(field(&output.snapshot, "key~1with~0chars"), &json!(2));
        assert_eq!(
            edit(&bytes, &[set("key~2", json!(2))]).unwrap_err().code,
            "property_path"
        );
    }

    #[test]
    fn references_require_exact_string_ids_and_reject_dangling_local_targets() {
        let bytes = fixture("  target: {fileID: 0}\n");
        assert_eq!(
            edit(&bytes, &[set("target", json!({"fileID":1}))])
                .unwrap_err()
                .code,
            "reference_id"
        );
        assert!(edit(&bytes, &[set("target", json!({"fileID":"123"}))]).is_err());
        assert!(edit(&bytes, &[set("target", json!({"fileID":"0"}))]).is_ok());
    }

    #[test]
    fn independent_set_batch_scales_and_overlapping_sets_keep_order() {
        let body = (0..1024)
            .map(|index| format!("  field{index}: {index}\n"))
            .collect::<String>();
        let bytes = fixture(&body);
        let operations = (0..1024)
            .map(|index| set(&format!("field{index}"), json!(index + 1)))
            .collect::<Vec<_>>();
        let output = edit(&bytes, &operations).unwrap();
        assert_eq!(field(&output.snapshot, "field1023"), &json!(1024));
        let overlapping = edit(
            &fixture("  vector: {x: 1, y: 2}\n"),
            &[
                set("vector", json!({"x":3,"y":4})),
                set("vector/x", json!(9)),
            ],
        )
        .unwrap();
        assert_eq!(
            field(&overlapping.snapshot, "vector"),
            &json!({"x":9,"y":4})
        );
    }

    #[test]
    fn unsupported_nonfinite_arrays_fail_without_coercing_values_to_strings() {
        let bytes = fixture("  values: [1, .nan]\n  huge: 999999999999999999999999999999999999\n");
        let snapshot = inspect(&bytes).unwrap();
        assert_eq!(
            field(&snapshot, "values")[1],
            json!({"kind":"float64","value":".nan"})
        );
        assert_eq!(
            field(&snapshot, "huge")["value"],
            "999999999999999999999999999999999999"
        );
        let error = edit(&bytes, &[operation(json!({"op":"array_move","object_id":"11400000","property_path":"/MonoBehaviour/values","index":0,"to_index":1}))]).unwrap_err();
        assert_eq!(error.code, "unsupported_scalar");
    }

    #[test]
    fn numeric_spellings_share_canonical_values_and_strings_keep_their_shape() {
        let bytes = fixture("  integer: 1\n  decimal: 1.0\n  exponent: 1e0\n  empty:\n  quoted: '1'\n  explicitNull: null\n  enabled: 0\n");
        let snapshot = inspect(&bytes).unwrap();
        assert_eq!(field(&snapshot, "integer"), field(&snapshot, "decimal"));
        assert_eq!(field(&snapshot, "integer"), field(&snapshot, "exponent"));
        assert_eq!(field(&snapshot, "empty"), &json!(""));
        assert_eq!(field(&snapshot, "quoted"), &json!("1"));
        assert_eq!(field(&snapshot, "explicitNull"), &Value::Null);
        let output = edit(
            &bytes,
            &[
                set("integer", json!(2.0)),
                set("decimal", json!(2)),
                set("enabled", json!(true)),
            ],
        )
        .unwrap();
        assert_eq!(
            field(&output.snapshot, "integer"),
            field(&output.snapshot, "decimal")
        );
        assert_eq!(field(&output.snapshot, "enabled"), &json!(1));
        assert!(edit(&bytes, &[set("empty", Value::Null)]).is_err());
        assert!(edit(&bytes, &[set("quoted", json!(1))]).is_err());
    }

    #[test]
    fn scalar_evidence_preserves_numeric_looking_strings_through_array_rewrites() {
        let bytes =
            fixture("  name: 123\n  labels: [001, null, true]\n  untouched: {odd:  17} # retain\n");
        let hints = ScalarHints::from([(
            "11400000".into(),
            BTreeMap::from([
                ("/MonoBehaviour/name".into(), ScalarHint::String),
                ("/MonoBehaviour/labels/0".into(), ScalarHint::String),
                ("/MonoBehaviour/labels/1".into(), ScalarHint::String),
                ("/MonoBehaviour/labels/2".into(), ScalarHint::String),
            ]),
        )]);
        let snapshot = inspect_with_hints(&bytes, &hints).unwrap();
        assert_eq!(field(&snapshot, "name"), &json!("123"));
        assert_eq!(field(&snapshot, "labels"), &json!(["001", "null", "true"]));
        let operations = vec![
            set("name", json!("456")),
            operation(
                json!({"op":"array_move","object_id":"11400000","property_path":"/MonoBehaviour/labels","index":0,"to_index":2}),
            ),
        ];
        assert!(edit(&bytes, &operations).is_err());
        let output = edit_with_hints(&bytes, &operations, &hints).unwrap();
        assert_eq!(
            field(&output.snapshot, "labels"),
            &json!(["null", "true", "001"])
        );
        assert!(String::from_utf8(output.bytes)
            .unwrap()
            .contains("  untouched: {odd:  17} # retain\n"));
    }

    #[test]
    fn actual_unity2022_compact_int_array_reads_and_edits_with_lossless_outer_bytes() {
        // Exact serialized field from the real unity-sample-project SDK fixture.
        let bytes = fixture("  amount: 10\n  numbers: 010000000200000003000000\n  untouched: 010000000200000003000000 # retain raw\n");
        let hints = ScalarHints::from([(
            "11400000".into(),
            BTreeMap::from([(
                "/MonoBehaviour/numbers".into(),
                ScalarHint::PackedArray(PackedElement::I32),
            )]),
        )]);
        let snapshot = inspect_with_hints(&bytes, &hints).unwrap();
        assert_eq!(field(&snapshot, "numbers"), &json!([1, 2, 3]));
        assert_eq!(field(&snapshot, "numbers/1"), &json!(2));
        assert_eq!(edit_with_hints(&bytes, &[], &hints).unwrap().bytes, bytes);
        let prefix = json!({"object_id":"11400000","property_path":"/MonoBehaviour/numbers"});
        let mut ops = [
            json!({"op":"array_insert","index":1,"value":9}),
            json!({"op":"array_move","index":3,"to_index":0}),
            json!({"op":"array_remove","index":2}),
            json!({"op":"array_resize","size":5,"value":7}),
        ]
        .into_iter()
        .map(|mut value| {
            value
                .as_object_mut()
                .unwrap()
                .extend(prefix.as_object().unwrap().clone());
            operation(value)
        })
        .collect::<Vec<_>>();
        ops.insert(1, set("numbers/0", json!(-5)));
        let output = edit_with_hints(&bytes, &ops, &hints).unwrap();
        assert_eq!(field(&output.snapshot, "numbers"), &json!([3, -5, 2, 7, 7]));
        let text = String::from_utf8(output.bytes).unwrap();
        assert!(text.contains("numbers: 03000000fbffffff020000000700000007000000\n"));
        assert!(text.contains("  untouched: 010000000200000003000000 # retain raw\n"));
        assert!(
            edit_with_hints(&bytes, &[set("numbers/0", json!(2147483648_i64))], &hints).is_err()
        );
        let empty = edit_with_hints(&bytes, &[set("numbers", json!([]))], &hints).unwrap();
        assert_eq!(field(&empty.snapshot, "numbers"), &json!([]));
    }

    #[test]
    fn unsigned_transport_tags_are_exact_and_round_trip_whole_packed_arrays() {
        let max = json!({"kind":"uint64","value":"18446744073709551615"});
        let bytes =
            fixture("  wide: 18446744073709551615\n  values: 0100000000000000ffffffffffffffff\n");
        let hints = ScalarHints::from([(
            "11400000".into(),
            BTreeMap::from([(
                "/MonoBehaviour/values".into(),
                ScalarHint::PackedArray(PackedElement::U64),
            )]),
        )]);
        let snapshot = inspect_with_hints(&bytes, &hints).unwrap();
        assert_eq!(field(&snapshot, "wide"), &max);
        assert_eq!(field(&snapshot, "values"), &json!([1, max]));
        let output = edit_with_hints(&bytes, &[set("values", json!([max, 1]))], &hints).unwrap();
        assert_eq!(field(&output.snapshot, "values"), &json!([max, 1]));
        assert!(String::from_utf8(output.bytes)
            .unwrap()
            .contains("values: ffffffffffffffff0100000000000000"));
        for (kind, raw) in [
            ("int64", "9223372036854775808"),
            ("int64", "-9223372036854775809"),
            ("uint64", "18446744073709551616"),
            ("uint64", "-1"),
            ("uint64", "01"),
            ("uint64", "+1"),
            ("uint64", "-0"),
        ] {
            assert_eq!(
                decode_asset_value(&json!({"kind":kind,"value":raw}))
                    .unwrap_err()
                    .code,
                "integer_range",
                "{kind}:{raw}"
            );
        }
        assert_eq!(decode_asset_value(&max).unwrap(), json!(u64::MAX));
    }
}
