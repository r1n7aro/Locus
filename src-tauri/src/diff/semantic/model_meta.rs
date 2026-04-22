//! Parser for Unity ModelImporter `.meta` files (FBX/OBJ/DAE/...).
//!
//! When a `.prefab` PrefabInstance points at a model asset (e.g. `.fbx`), the
//! source asset itself is opaque binary that we cannot parse. Unity persists the
//! synthesized `fileID → (classID, name)` mapping in the sidecar `.meta` file
//! under `ModelImporter`. This module reads that mapping and returns a flat list
//! of [`ModelMetaEntry`] entries that the semantic diff layer turns into a
//! `SourcePrefabInfo`.
//!
//! Two source-of-truth fields are supported:
//!
//! * `internalIDToNameTable` (Unity 2018.3+, 64-bit recycleIDs) — list of
//!   `{ first: { <classId>: <fileId> }, second: <name> }` entries. The class id
//!   is explicit, so the resulting [`ModelMetaEntry::class_id`] is always set.
//!
//! * `fileIDToRecycleName` (legacy short fileID layout) — map of
//!   `<fileId>: <name>`. Class id is derived from `file_id / 100000` and only
//!   accepted when it matches a known component class.

use serde_yaml::Value;

/// Single entry pulled from a ModelImporter `.meta` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelMetaEntry {
    pub file_id: i64,
    /// `Some` when the meta unambiguously declares the class, `None` otherwise.
    /// Callers may apply a heuristic when this is `None`.
    pub class_id: Option<i32>,
    /// Display name for the synthesized object. Leading `//` (Unity's
    /// "implicit root" marker) is stripped, but `/` separators are preserved.
    pub node_name: String,
    /// Order in which this entry appears in the `.meta` file. Used as a stable
    /// sort key for inspector panels backed by model meta.
    pub order_index: usize,
}

/// Reason `parse_model_importer_meta_detailed` could not produce any entries.
/// This is exposed so the cache layer can surface a precise diagnostic instead
/// of the blanket "model importer meta contained no recycleID entries".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ModelMetaParseFailure {
    /// `serde_yaml::from_str` rejected the content.
    YamlInvalid { error: String },
    /// YAML parsed but the top-level `ModelImporter` mapping is absent. Most
    /// likely the file is for a different importer (TextureImporter,
    /// NativeFormatImporter, etc.) — i.e. the asset isn't actually a model.
    NoModelImporterRoot { top_level_keys: Vec<String> },
    /// `ModelImporter` exists but neither `internalIDToNameTable` nor
    /// `fileIDToRecycleName` is present. Typical when Unity has not yet
    /// imported the FBX (the importer fields haven't been written), or the
    /// `.meta` was hand-crafted as a stub.
    NoRecycleTables { importer_keys: Vec<String> },
    /// The recycle tables exist but contain no usable entries (or every entry
    /// failed to extract a fileID). Carries per-table state plus, when items
    /// were present but unparseable, a YAML dump of the first item so we can
    /// see whether Unity introduced a new schema.
    TablesPresentButEmpty {
        internal_id_table: TableProbe,
        legacy_table: TableProbe,
    },
}

/// Per-table presence state. Distinguishes "key absent" from "key present but
/// the table is empty" from "key present with N items but parser extracted
/// none". The third state is theoretical for the current parser — Unity 2022+
/// `fileIdsGeneration: 2` projects produce empty tables, not unparseable
/// items — but it remains useful as a tripwire if Unity introduces a schema
/// the parser hasn't been updated for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TableProbe {
    Absent,
    PresentEmpty,
    PresentWithItems { len: usize },
}

impl TableProbe {
    fn describe(&self, name: &str) -> String {
        match self {
            TableProbe::Absent => format!("{}: absent", name),
            TableProbe::PresentEmpty => format!("{}: present but empty", name),
            TableProbe::PresentWithItems { len } => format!(
                "{}: present with {} item(s) but parser extracted none",
                name, len
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ModelMetaParseDetailed {
    pub entries: Vec<ModelMetaEntry>,
    /// Populated only when `entries` is empty — explains why nothing came out.
    pub failure: Option<ModelMetaParseFailure>,
}

impl ModelMetaParseFailure {
    /// Render a human-readable diagnostic. Inlined into the cache layer's
    /// `SourcePrefabLoadError::EmptyMeta` so the inspector UI can show the
    /// underlying cause without re-parsing the file.
    pub(crate) fn describe(&self) -> String {
        match self {
            ModelMetaParseFailure::YamlInvalid { error } => {
                format!("YAML parse error in .meta file: {}", error)
            }
            ModelMetaParseFailure::NoModelImporterRoot { top_level_keys } => {
                if top_level_keys.is_empty() {
                    "no `ModelImporter` key at the .meta root and the file has no top-level mapping (likely empty or corrupt)".to_string()
                } else {
                    format!(
                        "no `ModelImporter` key at the .meta root (found instead: {}) — the asset is probably not actually a model importer asset",
                        top_level_keys.join(", ")
                    )
                }
            }
            ModelMetaParseFailure::NoRecycleTables { importer_keys } => {
                format!(
                    "`ModelImporter` exists but neither `internalIDToNameTable` nor `fileIDToRecycleName` is present — Unity probably has not (re)imported the FBX yet (importer keys present: {})",
                    if importer_keys.is_empty() {
                        "<none>".to_string()
                    } else {
                        importer_keys.join(", ")
                    }
                )
            }
            ModelMetaParseFailure::TablesPresentButEmpty {
                internal_id_table,
                legacy_table,
            } => format!(
                "recycle tables yielded no entries — {}; {}",
                internal_id_table.describe("internalIDToNameTable"),
                legacy_table.describe("fileIDToRecycleName")
            ),
        }
    }
}

/// Parse the contents of a Unity `.meta` file. Returns an empty `Vec` if the
/// file is not a `ModelImporter` meta or if no entries could be extracted.
///
/// Thin wrapper around [`parse_model_importer_meta_detailed`] that drops the
/// failure reason. Existing call-sites that only need the entry list keep this
/// API; the cache/diagnostic layer should call the detailed variant directly.
pub(crate) fn parse_model_importer_meta(content: &str) -> Vec<ModelMetaEntry> {
    parse_model_importer_meta_detailed(content).entries
}

pub(crate) fn parse_model_importer_meta_detailed(content: &str) -> ModelMetaParseDetailed {
    let value: Value = match serde_yaml::from_str(content) {
        Ok(v) => v,
        Err(err) => {
            return ModelMetaParseDetailed {
                entries: Vec::new(),
                failure: Some(ModelMetaParseFailure::YamlInvalid {
                    error: err.to_string(),
                }),
            }
        }
    };

    let importer = match value.get("ModelImporter") {
        Some(v) => v,
        None => {
            // Capture top-level keys to help the user identify what kind of
            // .meta this actually is (e.g. NativeFormatImporter for .asset).
            let top_level_keys = value
                .as_mapping()
                .map(|m| {
                    m.keys()
                        .filter_map(|k| k.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            return ModelMetaParseDetailed {
                entries: Vec::new(),
                failure: Some(ModelMetaParseFailure::NoModelImporterRoot { top_level_keys }),
            };
        }
    };

    let internal_table_seq = importer
        .get("internalIDToNameTable")
        .and_then(|v| v.as_sequence());
    let legacy_table_map = importer
        .get("fileIDToRecycleName")
        .and_then(|v| v.as_mapping());

    if internal_table_seq.is_none() && legacy_table_map.is_none() {
        let importer_keys = importer
            .as_mapping()
            .map(|m| {
                m.keys()
                    .filter_map(|k| k.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        return ModelMetaParseDetailed {
            entries: Vec::new(),
            failure: Some(ModelMetaParseFailure::NoRecycleTables { importer_keys }),
        };
    }

    let mut entries = Vec::new();
    let mut order = 0usize;

    // Snapshot probes BEFORE parsing so the diagnostic can describe each
    // table's structural state independently of whether parsing succeeded.
    let internal_table_probe = match internal_table_seq {
        None => TableProbe::Absent,
        Some(seq) if seq.is_empty() => TableProbe::PresentEmpty,
        Some(seq) => TableProbe::PresentWithItems { len: seq.len() },
    };
    let legacy_table_probe = match legacy_table_map {
        None => TableProbe::Absent,
        Some(map) if map.is_empty() => TableProbe::PresentEmpty,
        Some(map) => TableProbe::PresentWithItems { len: map.len() },
    };

    // ── Modern: internalIDToNameTable (Unity 2018.3+, 64-bit recycleIDs) ──
    if let Some(table) = internal_table_seq {
        for item in table {
            let Some(first) = item.get("first").and_then(|v| v.as_mapping()) else {
                continue;
            };
            // first is a single-entry mapping: { <classID>: <fileID> }
            let mut class_id: Option<i32> = None;
            let mut file_id: Option<i64> = None;
            for (k, v) in first.iter() {
                let k_int = parse_yaml_int(k);
                let v_int = parse_yaml_int(v);
                if let (Some(ki), Some(vi)) = (k_int, v_int) {
                    class_id = Some(ki as i32);
                    file_id = Some(vi);
                    break;
                }
            }
            let Some(file_id) = file_id else {
                continue;
            };
            let node_name = item
                .get("second")
                .and_then(|v| v.as_str())
                .map(normalize_node_name)
                .unwrap_or_default();
            entries.push(ModelMetaEntry {
                file_id,
                class_id,
                node_name,
                order_index: order,
            });
            order += 1;
        }
    }

    // ── Legacy: fileIDToRecycleName (short fileID = classID*100000+N) ──
    // Only fall back if the modern table produced nothing — older importers
    // emit only the legacy form, never both meaningfully.
    if entries.is_empty() {
        if let Some(table) = legacy_table_map {
            for (k, v) in table.iter() {
                let Some(file_id) = parse_yaml_int(k) else {
                    continue;
                };
                let node_name = v.as_str().map(normalize_node_name).unwrap_or_default();
                let class_id = legacy_class_id_from_short_file_id(file_id);
                entries.push(ModelMetaEntry {
                    file_id,
                    class_id,
                    node_name,
                    order_index: order,
                });
                order += 1;
            }
        }
    }

    let failure = if entries.is_empty() {
        Some(ModelMetaParseFailure::TablesPresentButEmpty {
            internal_id_table: internal_table_probe,
            legacy_table: legacy_table_probe,
        })
    } else {
        None
    };

    ModelMetaParseDetailed { entries, failure }
}

/// Strip the implicit-root `//` prefix Unity emits in recycle names while
/// preserving any internal `/` hierarchy separators.
fn normalize_node_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("//") {
        rest.to_string()
    } else {
        trimmed.to_string()
    }
}

fn parse_yaml_int(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
}

/// For legacy short fileIDs Unity uses `classID * 100000 + N` as the file id.
/// Only accept matches for component classes that ModelImporter is known to
/// emit, so we don't fabricate class ids out of unrelated number patterns.
pub(crate) fn legacy_class_id_from_short_file_id(file_id: i64) -> Option<i32> {
    // Modern recycleIDs are way out of the legacy short range — bail early.
    // Largest known short fileID is around 1e10 (e.g. 9500000), so anything
    // beyond 1e12 is definitely a 64-bit recycleID.
    if file_id <= 0 || file_id > 1_000_000_000_000 {
        return None;
    }
    let candidate = (file_id / 100000) as i32;
    super::unity_builtin::is_model_importer_legacy_component_class_id(candidate)
        .then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_internal_id_to_name_table() {
        let meta = r#"
fileFormatVersion: 2
guid: 1234567890abcdef1234567890abcdef
ModelImporter:
  serializedVersion: 22
  internalIDToNameTable:
  - first:
      1: 919132149155446097
    second: //RootNode
  - first:
      4: 919132149155446098
    second: //RootNode
  - first:
      23: 919132149155446099
    second: //RootNode
"#;
        let entries = parse_model_importer_meta(meta);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].file_id, 919132149155446097);
        assert_eq!(entries[0].class_id, Some(1));
        assert_eq!(entries[0].node_name, "RootNode");
        assert_eq!(entries[0].order_index, 0);
        assert_eq!(entries[2].class_id, Some(23));
        assert_eq!(entries[2].order_index, 2);
    }

    #[test]
    fn parses_legacy_file_id_to_recycle_name() {
        let meta = r#"
fileFormatVersion: 2
guid: 1234567890abcdef1234567890abcdef
ModelImporter:
  fileIDToRecycleName:
    100000: //RootNode
    400000: //RootNode
    2300000: //RootNode
    3300000: //RootNode
"#;
        let entries = parse_model_importer_meta(meta);
        assert_eq!(entries.len(), 4);
        let by_fid: std::collections::HashMap<i64, &ModelMetaEntry> =
            entries.iter().map(|e| (e.file_id, e)).collect();
        assert_eq!(by_fid[&100000].class_id, Some(1));
        assert_eq!(by_fid[&400000].class_id, Some(4));
        assert_eq!(by_fid[&2300000].class_id, Some(23));
        assert_eq!(by_fid[&3300000].class_id, Some(33));
    }

    #[test]
    fn returns_empty_when_not_a_model_importer() {
        let meta = r#"
fileFormatVersion: 2
guid: 1234567890abcdef1234567890abcdef
TextureImporter:
  serializedVersion: 12
"#;
        assert!(parse_model_importer_meta(meta).is_empty());
    }

    #[test]
    fn detailed_reports_yaml_invalid() {
        let meta = ":\n: not yaml\n  - [\n";
        let detailed = parse_model_importer_meta_detailed(meta);
        assert!(detailed.entries.is_empty());
        assert!(matches!(
            detailed.failure,
            Some(ModelMetaParseFailure::YamlInvalid { .. })
        ));
    }

    #[test]
    fn detailed_reports_no_model_importer_root() {
        let meta = r#"
fileFormatVersion: 2
guid: 1234567890abcdef1234567890abcdef
TextureImporter:
  serializedVersion: 12
"#;
        let detailed = parse_model_importer_meta_detailed(meta);
        assert!(detailed.entries.is_empty());
        match detailed.failure {
            Some(ModelMetaParseFailure::NoModelImporterRoot { top_level_keys }) => {
                assert!(top_level_keys.iter().any(|k| k == "TextureImporter"));
            }
            other => panic!("expected NoModelImporterRoot, got {:?}", other),
        }
    }

    #[test]
    fn detailed_reports_no_recycle_tables() {
        // Real-world stub: ModelImporter exists but Unity hasn't (re)imported
        // the FBX, so neither recycle table is present.
        let meta = r#"
fileFormatVersion: 2
guid: 1234567890abcdef1234567890abcdef
ModelImporter:
  serializedVersion: 22
  materials:
    materialImportMode: 2
"#;
        let detailed = parse_model_importer_meta_detailed(meta);
        assert!(detailed.entries.is_empty());
        match detailed.failure {
            Some(ModelMetaParseFailure::NoRecycleTables { importer_keys }) => {
                assert!(importer_keys.iter().any(|k| k == "serializedVersion"));
                assert!(importer_keys.iter().any(|k| k == "materials"));
            }
            other => panic!("expected NoRecycleTables, got {:?}", other),
        }
    }

    #[test]
    fn detailed_reports_tables_present_but_empty() {
        let meta = r#"
ModelImporter:
  serializedVersion: 22
  internalIDToNameTable: []
  fileIDToRecycleName: {}
"#;
        let detailed = parse_model_importer_meta_detailed(meta);
        assert!(detailed.entries.is_empty());
        match detailed.failure {
            Some(ModelMetaParseFailure::TablesPresentButEmpty {
                internal_id_table,
                legacy_table,
            }) => {
                assert_eq!(internal_id_table, TableProbe::PresentEmpty);
                assert_eq!(legacy_table, TableProbe::PresentEmpty);
            }
            other => panic!("expected TablesPresentButEmpty, got {:?}", other),
        }
    }

    #[test]
    fn detailed_reports_tables_with_items_but_unparseable() {
        // Items present but the inner mapping uses an unrecognized schema —
        // the parser drops them silently. Probe should mark "present with N
        // items" so a future schema drift is visible in the diagnostic.
        let meta = r#"
ModelImporter:
  internalIDToNameTable:
  - first:
      classID: 4
      fileID: 999000111
    second: //RootNode
"#;
        let detailed = parse_model_importer_meta_detailed(meta);
        assert!(detailed.entries.is_empty());
        match detailed.failure {
            Some(ModelMetaParseFailure::TablesPresentButEmpty {
                internal_id_table,
                legacy_table,
            }) => {
                assert_eq!(internal_id_table, TableProbe::PresentWithItems { len: 1 });
                assert_eq!(legacy_table, TableProbe::Absent);
            }
            other => panic!("expected TablesPresentButEmpty, got {:?}", other),
        }
    }

    #[test]
    fn modern_table_takes_precedence_over_legacy() {
        let meta = r#"
ModelImporter:
  internalIDToNameTable:
  - first:
      4: 999000111
    second: //Foo
  fileIDToRecycleName:
    400000: //Bar
"#;
        let entries = parse_model_importer_meta(meta);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_id, 999000111);
        assert_eq!(entries[0].node_name, "Foo");
    }

    #[test]
    fn legacy_class_id_only_for_known_components() {
        assert_eq!(legacy_class_id_from_short_file_id(100000), Some(1));
        assert_eq!(legacy_class_id_from_short_file_id(400000), Some(4));
        assert_eq!(legacy_class_id_from_short_file_id(2300000), Some(23));
        assert_eq!(legacy_class_id_from_short_file_id(3300000), Some(33));
        // Unknown class — refuse to fabricate
        assert_eq!(legacy_class_id_from_short_file_id(9_900_000), None);
        // 64-bit recycleID is way out of range
        assert_eq!(legacy_class_id_from_short_file_id(919132149155446097), None);
    }

    #[test]
    fn normalizes_implicit_root_prefix() {
        assert_eq!(normalize_node_name("//RootNode"), "RootNode");
        assert_eq!(normalize_node_name("//Group/Child"), "Group/Child");
        assert_eq!(normalize_node_name("Plain"), "Plain");
    }
}
