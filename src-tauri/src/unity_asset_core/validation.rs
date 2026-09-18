use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{Asset, Diagnostic, Document, Node, NodeKind, Severity};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    LocalObject,
    ExternalObject,
    ManagedReference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetReference {
    pub host_object_id: String,
    pub property_path: String,
    pub kind: ReferenceKind,
    pub file_id: Option<String>,
    pub guid: Option<String>,
    pub rid: Option<String>,
}

/// Validate identities and references using only this immutable asset snapshot.
/// External GUIDs are returned to the project-snapshot layer for resolution.
pub fn validate(asset: &Asset) -> Vec<Diagnostic> {
    let mut out = asset.diagnostics.clone();
    let object_ids: HashSet<_> = asset
        .documents
        .iter()
        .map(|d| d.object_id.as_str())
        .collect();
    for doc in &asset.documents {
        let mut registry = HashMap::new();
        collect_registry(asset, doc, &doc.root, "", &mut registry, &mut out);
        walk(asset, doc, &doc.root, "", &object_ids, &registry, &mut out);
    }
    validate_ownership(asset, &mut out);
    out
}

fn body(doc: &Document) -> Option<&Node> {
    doc.root.entries()?.first().map(|e| &e.value)
}
fn local_id<'a>(asset: &'a Asset, node: &Node) -> Option<&'a str> {
    if node
        .get("guid")
        .and_then(|n| scalar_text(asset, n))
        .is_some_and(|g| g.bytes().any(|b| b != b'0'))
    {
        return None;
    }
    node.get("fileID").and_then(|n| scalar_text(asset, n))
}

fn validate_ownership(asset: &Asset, out: &mut Vec<Diagnostic>) {
    let docs: HashMap<_, _> = asset
        .documents
        .iter()
        .map(|d| (d.object_id.as_str(), d))
        .collect();
    let mut fathers = HashMap::new();
    for doc in &asset.documents {
        if doc.stripped {
            continue;
        }
        let Some(body) = body(doc) else {
            continue;
        };
        if let Some(owner) = body
            .get("m_GameObject")
            .and_then(|n| local_id(asset, n))
            .filter(|id| *id != "0")
        {
            if let Some(owner_doc) = docs.get(owner).filter(|d| !d.stripped) {
                if let Some(components) = self::body(owner_doc)
                    .and_then(|n| n.get("m_Component"))
                    .and_then(Node::items)
                {
                    if !components.iter().any(|i| {
                        i.value.get("component").and_then(|n| local_id(asset, n))
                            == Some(doc.object_id.as_str())
                    }) {
                        out.push(diagnostic(
                            doc,
                            body,
                            "",
                            "missing_owner_component",
                            format!(
                                "GameObject {owner} does not list component {}",
                                doc.object_id
                            ),
                        ));
                    }
                }
            }
        }
        if !matches!(doc.class_id.as_deref(), Some("4" | "224")) {
            continue;
        }
        if let Some(father) = body
            .get("m_Father")
            .and_then(|n| local_id(asset, n))
            .filter(|id| *id != "0")
        {
            fathers.insert(doc.object_id.as_str(), father);
            if let Some(parent) = docs.get(father).filter(|d| !d.stripped) {
                if let Some(children) = self::body(parent)
                    .and_then(|n| n.get("m_Children"))
                    .and_then(Node::items)
                {
                    if !children
                        .iter()
                        .any(|i| local_id(asset, &i.value) == Some(doc.object_id.as_str()))
                    {
                        out.push(diagnostic(
                            doc,
                            body,
                            "",
                            "parent_child_mismatch",
                            format!(
                                "parent Transform {father} does not list child {}",
                                doc.object_id
                            ),
                        ));
                    }
                }
            }
        }
        if let Some(children) = body.get("m_Children").and_then(Node::items) {
            let mut seen = HashSet::new();
            for child in children {
                let Some(id) = local_id(asset, &child.value) else {
                    continue;
                };
                if !seen.insert(id) {
                    out.push(diagnostic(
                        doc,
                        &child.value,
                        "",
                        "duplicate_child",
                        format!("child Transform {id} is listed twice"),
                    ));
                }
                if let Some(child_doc) = docs.get(id).filter(|d| !d.stripped) {
                    if let Some(father) = self::body(child_doc)
                        .and_then(|n| n.get("m_Father"))
                        .and_then(|n| local_id(asset, n))
                    {
                        if father != doc.object_id {
                            out.push(diagnostic(
                                doc,
                                &child.value,
                                "",
                                "child_parent_mismatch",
                                format!(
                                    "child Transform {id} points to parent {father}, not {}",
                                    doc.object_id
                                ),
                            ));
                        }
                    }
                }
            }
        }
    }
    // Linear-time cycle detection across the immutable parent graph.
    let mut visited = HashSet::new();
    for id in fathers.keys().copied() {
        if visited.contains(id) {
            continue;
        }
        let mut chain = HashSet::new();
        let mut cursor = id;
        while !visited.contains(cursor) {
            if !chain.insert(cursor) {
                if let Some(doc) = docs.get(cursor) {
                    out.push(diagnostic(
                        doc,
                        &doc.root,
                        "",
                        "transform_cycle",
                        format!("Transform parent cycle includes {cursor}"),
                    ));
                }
                break;
            }
            let Some(parent) = fathers.get(cursor) else {
                break;
            };
            cursor = parent;
        }
        visited.extend(chain);
    }
}

impl Asset {
    pub fn references(&self) -> Vec<AssetReference> {
        let mut out = Vec::new();
        for doc in &self.documents {
            collect_references(self, doc, &doc.root, "", &mut out);
        }
        out
    }
}

pub(crate) fn path_child(path: &str, key: &str) -> String {
    format!("{}/{}", path, key.replace('~', "~0").replace('/', "~1"))
}

pub(crate) fn scalar_text<'a>(asset: &'a Asset, node: &Node) -> Option<&'a str> {
    node.scalar(asset).map(str::trim)
}

pub(crate) fn pptr(node: &Node) -> bool {
    node.get("fileID").is_some()
        && node.entries().is_some_and(|entries| {
            entries
                .iter()
                .all(|e| matches!(e.key.as_str(), "fileID" | "guid" | "type"))
        })
}

fn diagnostic(doc: &Document, node: &Node, path: &str, code: &str, message: String) -> Diagnostic {
    Diagnostic {
        code: code.into(),
        message,
        severity: Severity::Error,
        span: node.span,
        object_id: Some(doc.object_id.clone()),
        property_path: Some(path.into()),
    }
}

fn collect_registry<'a>(
    asset: &'a Asset,
    doc: &Document,
    node: &'a Node,
    path: &str,
    ids: &mut HashMap<String, &'a Node>,
    out: &mut Vec<Diagnostic>,
) {
    match &node.kind {
        NodeKind::Mapping(entries) => {
            if let Some(ref_ids) = node.get("RefIds") {
                if node.get("version").is_some() {
                    if let Some(items) = ref_ids.items() {
                        for item in items {
                            if let Some(rid) =
                                item.value.get("rid").and_then(|n| scalar_text(asset, n))
                            {
                                if rid.parse::<i64>().is_err() {
                                    out.push(diagnostic(doc,&item.value,path,"invalid_rid",format!("managed reference ID {rid} is not a signed 64-bit integer")));
                                } else if ids.insert(rid.to_owned(), &item.value).is_some() {
                                    out.push(diagnostic(
                                        doc,
                                        &item.value,
                                        path,
                                        "duplicate_rid",
                                        format!(
                                            "duplicate managed reference rid {rid} in host {}",
                                            doc.object_id
                                        ),
                                    ));
                                }
                                if item.value.get("type").is_none() {
                                    out.push(diagnostic(
                                        doc,
                                        &item.value,
                                        path,
                                        "incomplete_managed_reference",
                                        format!("rid {rid} must preserve its type identity"),
                                    ));
                                }
                            } else {
                                out.push(diagnostic(
                                    doc,
                                    &item.value,
                                    path,
                                    "missing_rid",
                                    "RefIds entry has no rid".into(),
                                ));
                            }
                        }
                    } else if !ref_ids.scalar(asset).is_some_and(|s| s.trim().is_empty()) {
                        out.push(diagnostic(
                            doc,
                            ref_ids,
                            path,
                            "invalid_reference_registry",
                            "RefIds must be a sequence or an empty registry".into(),
                        ));
                    }
                }
            }
            for entry in entries {
                collect_registry(
                    asset,
                    doc,
                    &entry.value,
                    &path_child(path, &entry.key),
                    ids,
                    out,
                );
            }
        }
        NodeKind::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                collect_registry(
                    asset,
                    doc,
                    &item.value,
                    &path_child(path, &index.to_string()),
                    ids,
                    out,
                );
            }
        }
        _ => {}
    }
}

fn walk(
    asset: &Asset,
    doc: &Document,
    node: &Node,
    path: &str,
    object_ids: &HashSet<&str>,
    registry: &HashMap<String, &Node>,
    out: &mut Vec<Diagnostic>,
) {
    if pptr(node) {
        if let Some(id) = node.get("fileID").and_then(|n| scalar_text(asset, n)) {
            if id.parse::<i64>().is_err() {
                out.push(diagnostic(
                    doc,
                    node,
                    path,
                    "invalid_file_id",
                    format!("fileID {id} is not a signed 64-bit integer"),
                ));
            } else {
                let external = node
                    .get("guid")
                    .and_then(|n| scalar_text(asset, n))
                    .is_some_and(|g| g.bytes().any(|b| b != b'0'));
                if !external && id != "0" && doc.class_id.is_some() && !object_ids.contains(id) {
                    out.push(diagnostic(
                        doc,
                        node,
                        path,
                        "dangling_local_reference",
                        format!("local fileID {id} has no object in this asset snapshot"),
                    ));
                }
            }
        }
        if let Some(guid) = node.get("guid").and_then(|n| scalar_text(asset, n)) {
            if guid.len() != 32 || !guid.bytes().all(|b| b.is_ascii_hexdigit()) {
                out.push(diagnostic(
                    doc,
                    node,
                    path,
                    "invalid_guid",
                    format!("invalid 128-bit GUID {guid}"),
                ));
            }
        }
    }
    if let Some(entries) = node.entries() {
        if entries.len() == 1 && entries[0].key == "rid" {
            if let Some(rid) = scalar_text(asset, &entries[0].value) {
                match rid.parse::<i64>() {
                    Ok(-2 | -1) => {}
                    Ok(_) if registry.contains_key(rid) => {}
                    Ok(_) => out.push(diagnostic(
                        doc,
                        node,
                        path,
                        "dangling_managed_reference",
                        format!("rid {rid} is absent from host {}'s registry", doc.object_id),
                    )),
                    Err(_) => out.push(diagnostic(
                        doc,
                        node,
                        path,
                        "invalid_rid",
                        format!("rid {rid} is not a signed 64-bit integer"),
                    )),
                }
            }
        }
        // Owner and component links are bidirectional. This catches selective
        // component additions/removals that omitted the corresponding owner edit.
        if let Some(components) = node.get("m_Component").and_then(Node::items) {
            let mut seen = HashSet::new();
            for item in components {
                if let Some(id) = item
                    .value
                    .get("component")
                    .and_then(|n| n.get("fileID"))
                    .and_then(|n| scalar_text(asset, n))
                {
                    if !seen.insert(id) {
                        out.push(diagnostic(
                            doc,
                            &item.value,
                            path,
                            "duplicate_component",
                            format!("component fileID {id} is listed twice"),
                        ));
                    }
                    if let Some(component) = asset.document(id) {
                        let owner = component
                            .root
                            .entries()
                            .and_then(|e| e.first())
                            .map(|e| &e.value)
                            .and_then(|n| n.get("m_GameObject"))
                            .and_then(|n| n.get("fileID"))
                            .and_then(|n| scalar_text(asset, n));
                        if owner.is_some_and(|owner| owner != doc.object_id) {
                            out.push(diagnostic(
                                doc,
                                &item.value,
                                path,
                                "component_owner_mismatch",
                                format!("component {id} belongs to another GameObject"),
                            ));
                        }
                    }
                }
            }
        }
        for entry in entries {
            walk(
                asset,
                doc,
                &entry.value,
                &path_child(path, &entry.key),
                object_ids,
                registry,
                out,
            );
        }
    } else if let Some(items) = node.items() {
        for (index, item) in items.iter().enumerate() {
            walk(
                asset,
                doc,
                &item.value,
                &path_child(path, &index.to_string()),
                object_ids,
                registry,
                out,
            );
        }
    }
}

fn collect_references(
    asset: &Asset,
    doc: &Document,
    node: &Node,
    path: &str,
    out: &mut Vec<AssetReference>,
) {
    if pptr(node) {
        let guid = node
            .get("guid")
            .and_then(|n| scalar_text(asset, n))
            .map(str::to_owned);
        let external = guid
            .as_deref()
            .is_some_and(|g| g.bytes().any(|b| b != b'0'));
        out.push(AssetReference {
            host_object_id: doc.object_id.clone(),
            property_path: path.into(),
            kind: if external {
                ReferenceKind::ExternalObject
            } else {
                ReferenceKind::LocalObject
            },
            file_id: node
                .get("fileID")
                .and_then(|n| scalar_text(asset, n))
                .map(str::to_owned),
            guid,
            rid: None,
        });
    }
    if let Some(entries) = node.entries() {
        if entries.len() == 1 && entries[0].key == "rid" {
            out.push(AssetReference {
                host_object_id: doc.object_id.clone(),
                property_path: path.into(),
                kind: ReferenceKind::ManagedReference,
                file_id: None,
                guid: None,
                rid: scalar_text(asset, &entries[0].value).map(str::to_owned),
            });
        }
        for entry in entries {
            collect_references(asset, doc, &entry.value, &path_child(path, &entry.key), out);
        }
    } else if let Some(items) = node.items() {
        for (index, item) in items.iter().enumerate() {
            collect_references(
                asset,
                doc,
                &item.value,
                &path_child(path, &index.to_string()),
                out,
            );
        }
    }
}
