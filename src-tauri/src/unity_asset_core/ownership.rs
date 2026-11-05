//! Objects the source deletes together stay together. Deleting a GameObject
//! also deletes its components and child Transforms and removes the
//! references its owners list (`m_Component`, `m_Children`, `m_Roots`). When
//! one of those deletions conflicts with a destination edit, every part of the
//! deletion follows the same decision: the clean parts become conflicts with a
//! reason naming the contested object, so `side="target"` keeps the whole
//! object attached and `side="source"` removes all of it.
use super::merge::{Change, ChangeKind, ChangeStatus};
use super::parser::{Asset, Node};
use super::validation::scalar_text;
use std::collections::{HashMap, HashSet};

fn body<'a>(asset: &'a Asset, id: &str) -> Option<&'a Node> {
    let doc = asset.document(id).filter(|d| !d.stripped)?;
    doc.root.entries()?.first().map(|e| &e.value)
}

fn local_id(asset: &Asset, node: Option<&Node>) -> Option<String> {
    let node = node?;
    if node.get("guid").and_then(|n| scalar_text(asset, n)).is_some_and(|g| g.bytes().any(|b| b != b'0')) {
        return None;
    }
    node.get("fileID").and_then(|n| scalar_text(asset, n)).filter(|id| *id != "0").map(str::to_string)
}

fn find(parent: &mut HashMap<String, String>, id: &str) -> String {
    let mut root = id.to_string();
    while let Some(next) = parent.get(&root).filter(|next| **next != root) {
        root = next.clone();
    }
    parent.insert(id.to_string(), root.clone());
    root
}

/// The object an owner-list element points to (`@component=`, `@child=`, `@root=`).
fn listed_object(path: &str) -> Option<&str> {
    let key = path.rsplit('/').next()?;
    ["@component=", "@child=", "@root="].iter().find_map(|prefix| key.strip_prefix(prefix))
}

/// The objects a whole owner list (`/<Class>/m_Component`, `m_Children`,
/// `m_Roots`) names in the base, for a change that replaces the list at once.
fn listed_objects(base: &Asset, object_id: &str, path: &str) -> Vec<String> {
    let mut parts = path.split('/').skip(1);
    let (Some(_class), Some(field), None) = (parts.next(), parts.next(), parts.next()) else {
        return Vec::new();
    };
    if !matches!(field, "m_Component" | "m_Children" | "m_Roots") {
        return Vec::new();
    }
    let Some(items) = body(base, object_id).and_then(|b| b.get(field)).and_then(Node::items) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let pointer = if field == "m_Component" { item.value.get("component") } else { Some(&item.value) };
            local_id(base, pointer)
        })
        .collect()
}

pub(super) fn link_deletions(base: &Asset, changes: &mut [Change]) {
    let deleted: HashMap<String, usize> = changes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.kind == ChangeKind::RemoveObject && c.base.is_some() && c.source.is_none())
        .map(|(i, c)| (c.object_id.clone(), i))
        .collect();
    if !deleted.values().any(|&i| changes[i].status == ChangeStatus::Conflict) {
        return;
    }
    let mut parent: HashMap<String, String> = deleted.keys().map(|id| (id.clone(), id.clone())).collect();
    for id in deleted.keys() {
        let Some(body) = body(base, id) else { continue };
        for link in ["m_GameObject", "m_Father"] {
            if let Some(owner) = local_id(base, body.get(link)).filter(|owner| deleted.contains_key(owner)) {
                let (a, b) = (find(&mut parent, id), find(&mut parent, &owner));
                parent.insert(a, b);
            }
        }
    }
    let mut contested: HashMap<String, String> = HashMap::new();
    for (id, &index) in &deleted {
        if changes[index].status == ChangeStatus::Conflict {
            contested.entry(find(&mut parent, id)).or_insert_with(|| id.clone());
        }
    }
    let group: HashMap<String, String> = deleted
        .keys()
        .filter_map(|id| {
            let root = find(&mut parent, id);
            contested.get(&root).map(|object| (id.clone(), object.clone()))
        })
        .collect();
    let members: HashSet<&str> = group.keys().map(String::as_str).collect();
    for change in changes.iter_mut().filter(|c| c.status == ChangeStatus::Clean && c.base.is_some()) {
        let object = if change.kind == ChangeKind::RemoveObject {
            members.contains(change.object_id.as_str()).then(|| group[&change.object_id].clone())
        } else if let Some(id) = listed_object(&change.property_path) {
            (change.source.is_none() && members.contains(id)).then(|| group[id].clone())
        } else {
            listed_objects(base, &change.object_id, &change.property_path)
                .iter()
                .find(|id| members.contains(id.as_str()))
                .map(|id| group[id.as_str()].clone())
        };
        if let Some(object) = object {
            change.status = ChangeStatus::Conflict;
            change.reason = Some(format!(
                "the source deletes this together with object {object}, whose deletion conflicts with a destination edit; resolve every change of that deletion with the same side ('target' keeps the object attached, 'source' removes all of it)"
            ));
        }
    }
}

/// An object the destination deleted while the source modified it is restored
/// with `objects.restore`, which brings back what it needs to load.
pub(super) fn owner_hints(_source: &Asset, changes: &mut [Change]) {
    for change in changes.iter_mut().filter(|c| {
        c.status == ChangeStatus::Conflict && c.property_path.is_empty()
            && c.base.is_some() && c.target.is_none() && c.source.is_some()
    }) {
        let id = change.object_id.clone();
        if let Some(reason) = &mut change.reason {
            reason.push_str(&format!(
                "; to restore it, use objects.restore(path, object_id='{id}'), which also brings back the deleted GameObject, components and children it belongs to and the reference that lists it"
            ));
        }
    }
}

/// What restoring an object the destination deleted takes from the source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Restore {
    /// Source objects absent from the destination, in source document order.
    pub objects: Vec<String>,
    /// Elements of destination owner lists to take from the source, as
    /// (owner object id, element property path).
    pub attach: Vec<(String, String)>,
}

/// Fields of a document, stripped proxies included.
fn fields<'a>(asset: &'a Asset, id: &str) -> Option<&'a Node> {
    asset.document(id)?.root.entries()?.first().map(|e| &e.value)
}

fn class(asset: &Asset, id: &str) -> Option<String> {
    asset.document(id)?.root.entries()?.first().map(|e| e.key.clone())
}

/// The hierarchy node an object belongs to: its GameObject, or the prefab
/// instance of a stripped proxy.
fn node_of(source: &Asset, id: &str) -> Option<String> {
    let doc = source.document(id)?;
    let fields = fields(source, id)?;
    if doc.stripped {
        return local_id(source, fields.get("m_PrefabInstance"));
    }
    match class(source, id).as_deref() {
        Some("GameObject" | "PrefabInstance") => Some(id.to_string()),
        _ => local_id(source, fields.get("m_GameObject")),
    }
}

/// The node above `node`: through the GameObject's Transform `m_Father`, or a
/// prefab instance's `m_TransformParent`.
fn parent_node(source: &Asset, node: &str) -> Option<String> {
    let fields = fields(source, node)?;
    let father = if class(source, node).as_deref() == Some("PrefabInstance") {
        local_id(source, fields.get("m_Modification").and_then(|m| m.get("m_TransformParent")))
    } else {
        fields.get("m_Component")?.items()?.iter().find_map(|item| {
            let component = local_id(source, item.value.get("component"))?;
            matches!(class(source, &component).as_deref(), Some("Transform" | "RectTransform"))
                .then(|| local_id(source, fields_of(source, &component)?.get("m_Father")))
                .flatten()
        })
    }?;
    node_of(source, &father)
}

fn fields_of<'a>(source: &'a Asset, id: &str) -> Option<&'a Node> {
    fields(source, id)
}

/// Objects a document belongs to: its GameObject, parent Transform, prefab
/// instance or (for an instance) the Transform it is placed under.
fn owners(source: &Asset, id: &str) -> Vec<String> {
    let Some(fields) = fields(source, id) else { return Vec::new() };
    let mut owners: Vec<String> = ["m_GameObject", "m_Father", "m_PrefabInstance"]
        .iter()
        .filter_map(|key| local_id(source, fields.get(key)))
        .collect();
    if let Some(parent) = local_id(source, fields.get("m_Modification").and_then(|m| m.get("m_TransformParent"))) {
        owners.push(parent);
    }
    owners
}

/// Owner lists and how their elements name an object.
const LISTS: &[(&str, Option<&str>, &str)] = &[
    ("m_Component", Some("component"), "@component="),
    ("m_Children", None, "@child="),
    ("m_Roots", None, "@root="),
];

fn listed(asset: &Asset, list: &Node, field: Option<&str>) -> Vec<String> {
    list.items()
        .into_iter()
        .flatten()
        .filter_map(|item| local_id(asset, match field {
            Some(field) => item.value.get(field),
            None => Some(&item.value),
        }))
        .collect()
}

/// Everything restoring `object_id` brings back: the object, the deleted
/// part of the hierarchy it belongs to (up to the nearest node the
/// destination kept, with that node's whole deleted subtree), and the owner
/// list elements that attach it to what the destination kept.
pub fn restore_closure(target: &Asset, source: &Asset, object_id: &str) -> Result<Restore, String> {
    if source.document(object_id).is_none() {
        return Err(format!("The source has no object {object_id}"));
    }
    if target.document(object_id).is_some() {
        return Err(format!("The destination still has object {object_id}; objects.restore brings back objects the destination deleted"));
    }
    let absent = |id: &str| target.document(id).is_none() && source.document(id).is_some();
    let mut start = object_id.to_string();
    if let Some(mut node) = node_of(source, object_id).filter(|node| absent(node)) {
        while let Some(parent) = parent_node(source, &node).filter(|parent| absent(parent)) {
            node = parent;
        }
        start = node;
    }
    let mut owned: HashMap<String, Vec<String>> = HashMap::new();
    for doc in &source.documents {
        for owner in owners(source, &doc.object_id) {
            owned.entry(owner).or_default().push(doc.object_id.clone());
        }
    }
    let mut members: HashSet<String> = HashSet::from([start.clone(), object_id.to_string()]);
    let mut queue = vec![start, object_id.to_string()];
    while let Some(member) = queue.pop() {
        let mut next: Vec<String> = owned.get(&member).cloned().unwrap_or_default();
        // A member component or proxy brings its own GameObject or instance.
        if let Some(fields) = fields(source, &member) {
            for key in ["m_GameObject", "m_PrefabInstance"] {
                next.extend(local_id(source, fields.get(key)));
            }
        }
        for id in next {
            if absent(&id) && members.insert(id.clone()) {
                queue.push(id);
            }
        }
    }
    let objects: Vec<String> = source.documents.iter().map(|d| d.object_id.clone()).filter(|id| members.contains(id)).collect();
    let mut attach = Vec::new();
    for doc in &source.documents {
        let id = doc.object_id.as_str();
        if members.contains(id) || target.document(id).is_none() {
            continue;
        }
        let (Some(name), Some(fields)) = (class(source, id), fields(source, id)) else { continue };
        let mut lists: Vec<(String, &Node, Option<&str>, &str)> = LISTS
            .iter()
            .filter_map(|(list, field, key)| fields.get(list).map(|node| (format!("/{name}/{list}"), node, *field, *key)))
            .collect();
        if let Some(modification) = fields.get("m_Modification") {
            for list in ["m_AddedGameObjects", "m_AddedComponents"] {
                if let Some(node) = modification.get(list) {
                    lists.push((format!("/{name}/m_Modification/{list}"), node, Some("addedObject"), "@added="));
                }
            }
        }
        for (path, node, field, key) in lists {
            let kept: HashSet<String> = path
                .rsplit('/')
                .next()
                .and_then(|list| {
                    let kept = fields_of(target, id)?;
                    let node = if path.contains("/m_Modification/") { kept.get("m_Modification")?.get(list)? } else { kept.get(list)? };
                    Some(listed(target, node, field).into_iter().collect())
                })
                .unwrap_or_default();
            for element in listed(source, node, field) {
                if members.contains(&element) && !kept.contains(&element) {
                    attach.push((id.to_string(), format!("{path}/{key}{element}")));
                }
            }
        }
    }
    Ok(Restore { objects, attach })
}
