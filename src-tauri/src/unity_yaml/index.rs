//! Public, reusable parse + index views over a Unity YAML file.
//!
//! Two layered types are exposed:
//!
//! - [`UnityYamlDocs`] is the *minimal* view: just the parsed `YamlDoc`
//!   vector and the line-split text. Use this for flat asset previews
//!   (`.asset` / `.controller` / `.mat` / `.anim`) where downstream code
//!   only needs the docs and lines and would never look at scene
//!   indices. Building it is the cheapest possible parse.
//!
//! - [`UnityYamlFile`] is the *indexed* view: docs + lines plus the
//!   GameObject hierarchy forest, a fileID → doc map and the
//!   GameObject → owned-component lookup. Use this for scene/prefab
//!   previews and for diff/merge consumers that need O(1) component
//!   lookup. The extra index pass is unconditional, which is fine for
//!   files large enough to actually have a hierarchy.
//!
//! Splitting the two avoids the cost regression where flat asset
//! previews were paying for `build_go_tree` + component indexing they
//! never read.
//!
//! Both types stay free of GUID resolution, I/O policy and UI
//! labelling — that's the caller's job.
//!
//! Note on `component_index`: the algorithm here must stay byte-for-byte
//! identical to `crate::diff::semantic::scene::build_component_index`.
//! The two exist in parallel because scene.rs builds it from a borrowed
//! `&[YamlDoc]` slice in the middle of a hot diff path, where threading
//! a full `UnityYamlFile` would cause lifetime churn. There is an
//! equivalence test in `unity_yaml::tests` that pins the two together —
//! if you change one and not the other it will fail.

use std::collections::HashMap;

use super::{build_go_tree, parse_yaml_docs, HierarchyNode, YamlDoc};

/// Minimal parsed Unity YAML view: docs + line-split text. Cheap to
/// build — no hierarchy / index passes.
pub struct UnityYamlDocs {
    /// Owned parsed YAML documents.
    pub docs: Vec<YamlDoc>,
    /// Owned line-split file content. Inspector / field-rendering code
    /// reads from this.
    pub lines: Vec<String>,
}

impl UnityYamlDocs {
    /// Parse `content` into docs + lines, with no derived indices.
    pub fn parse(content: &[u8]) -> Self {
        let docs = parse_yaml_docs(content);
        let text = String::from_utf8_lossy(content);
        let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        Self { docs, lines }
    }
}

/// Parsed Unity YAML file plus the most commonly needed derived
/// indices. Use this for scene/prefab previews and diff/merge consumers
/// that need hierarchy or component lookups. For flat asset previews
/// that only consume docs/lines, prefer [`UnityYamlDocs`] to avoid the
/// extra index pass.
pub struct UnityYamlFile {
    /// Owned parsed YAML documents.
    pub docs: Vec<YamlDoc>,
    /// Owned line-split file content. Inspector / field-rendering code
    /// reads from this.
    pub lines: Vec<String>,
    /// GameObject hierarchy forest (multiple roots are normal for scenes).
    pub hierarchy_roots: Vec<HierarchyNode>,
    /// fileID → docs index. Only non-stripped docs are inserted; stripped
    /// docs share fileIDs with their source prefab and would collide.
    pub doc_by_file_id: HashMap<i64, usize>,
    /// GameObject fileID → owned component doc indices.
    ///
    /// Mirrors `diff::semantic::scene::build_component_index`: a doc with
    /// `m_GameObject` set is inserted under that GameObject's fileID, and
    /// GameObject (`class_id == 1`) / PrefabInstance (`class_id == 1001`)
    /// docs are *also* inserted under their own fileID so callers can do
    /// `component_index.get(&go_id)` and find the GameObject itself in the
    /// same vector.
    pub component_index: HashMap<i64, Vec<usize>>,
}

impl UnityYamlFile {
    /// Parse `content` and build all derived indices in one pass.
    pub fn parse(content: &[u8]) -> Self {
        let base = UnityYamlDocs::parse(content);
        Self::from_docs(base)
    }

    /// Promote a [`UnityYamlDocs`] into a fully indexed [`UnityYamlFile`]
    /// without re-parsing. Useful when a caller started flat and decided
    /// it actually needs the indices.
    pub fn from_docs(base: UnityYamlDocs) -> Self {
        let UnityYamlDocs { docs, lines } = base;
        let hierarchy_roots = build_go_tree(&docs);

        let mut doc_by_file_id = HashMap::with_capacity(docs.len());
        for (i, d) in docs.iter().enumerate() {
            if !d.is_stripped {
                doc_by_file_id.insert(d.file_id, i);
            }
        }

        let mut component_index: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, doc) in docs.iter().enumerate() {
            if let Some(go_id) = doc.m_game_object_id {
                component_index.entry(go_id).or_default().push(i);
            }
            if doc.class_id == 1 || doc.class_id == 1001 {
                component_index.entry(doc.file_id).or_default().push(i);
            }
        }
        for v in component_index.values_mut() {
            v.sort();
            v.dedup();
        }

        Self {
            docs,
            lines,
            hierarchy_roots,
            doc_by_file_id,
            component_index,
        }
    }
}
