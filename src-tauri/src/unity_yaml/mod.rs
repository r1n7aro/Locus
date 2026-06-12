//! Neutral Unity YAML parsing layer.
//!
//! This module owns the low-level parser, tokenizer, prefab/IR extraction,
//! reference scanning, and aggregation/formatting helpers for Unity
//! `.unity` / `.prefab` / `.asset` YAML files. It used to live under
//! `crate::ref_graph::yaml_parser`, but every consumer (diff, merge, asset
//! preview, agent tools) needed it independently of `ref_graph`, so the
//! implementation now lives here and `ref_graph::yaml_parser` is a thin
//! compatibility re-export.
//!
//! It deliberately does not own:
//! - GUID resolution / asset DB lookup (`ref_graph` proper)
//! - I/O policy (snapshot blob vs workspace fallback) — that lives in the
//!   diff / asset / merge layers
//! - UI semantics (panel building, field labels, side context) — that lives
//!   in `diff::semantic`

mod aggregation;
pub mod index;
mod parser;
mod prefab;
mod references;
#[cfg(test)]
mod tests;
mod tokenizer;

pub use parser::{
    build_go_tree, build_hierarchy_path_map, build_internal_id_map, build_world_transform_map,
    collect_guids_from_lines, collect_guids_from_ranges, find_go_by_path, format_doc_state_lines,
    format_hierarchy_tree, get_components_for_go, is_hierarchical_file, parse_yaml_docs,
    parse_yaml_docs_with_refs, resolve_references_in_lines,
    resolve_references_in_lines_skipping_fields, HierarchyNode, TransformWorldInfo, YamlDoc,
};

pub use prefab::{extract_prefab_instance_irs, extract_stripped_mappings};

pub use references::{build_refs_from_docs, extract_refs, extract_refs_with_resolver, RawYamlRef};

pub use aggregation::{
    format_hierarchy_search_results, format_hierarchy_summary,
    format_hierarchy_summary_with_options, format_override_summary, format_prefab_file_summary,
    format_prefab_instance_detail, format_scene_summary, format_scene_summary_with_options,
    summarize_prefab_instance, HierarchySearchOptions, HierarchySummaryOptions,
    SourcePrefabContext, DEFAULT_HIERARCHY_MAX_NODES,
};

pub use index::{UnityYamlDocs, UnityYamlFile};
