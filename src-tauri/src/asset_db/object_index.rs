use std::collections::HashSet;
use std::path::Path;

use serde_yaml::Value;

use super::db::derive_search_cols;
use super::types::{asset_object_key, AssetKind, AssetNode, AssetObject, Guid};
use crate::unity_yaml::YamlDoc;

#[derive(Debug, Clone)]
pub(crate) struct ScriptTypeInfo {
    pub class_name: String,
    pub class_name_lower: String,
    pub full_name_lower: String,
    pub type_search_lower: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ImporterSubAsset {
    pub file_id: i64,
    pub class_id: Option<i32>,
    pub name: String,
    pub order_index: usize,
}

pub(crate) fn main_asset_object(asset: &AssetNode) -> AssetObject {
    let (root_i, path_lower, file_name_lower, stem_lower) = derive_search_cols(&asset.path);
    let display_name = asset
        .path
        .rsplit('/')
        .next()
        .unwrap_or(asset.path.as_str())
        .to_string();
    let type_name = asset
        .script_class_name
        .as_ref()
        .filter(|name| !name.is_empty() && asset.kind == AssetKind::GenericAsset)
        .cloned()
        .unwrap_or_else(|| asset.kind.camel_str().to_string());
    let type_lower = type_name.to_ascii_lowercase();
    let type_search = merge_type_terms([
        asset.kind.camel_str(),
        asset.ext.as_str(),
        type_lower.as_str(),
        asset.script_type_search.as_str(),
    ]);

    AssetObject {
        object_key: asset_object_key(&asset.guid, None),
        asset_guid: asset.guid,
        file_id: None,
        path: asset.path.clone(),
        kind: asset.kind,
        root: super::types::AssetRoot::from_i32(root_i),
        path_lower,
        file_name_lower,
        name: display_name,
        name_lower: stem_lower,
        type_name,
        type_lower,
        type_search,
        script_class_name: asset.script_class_name.clone(),
        script_class_lower: asset.script_class_lower.clone(),
        is_main: true,
        is_sub_asset: false,
        searchable: asset.exists_on_disk && asset.kind != AssetKind::MetaOnly,
        target_id: None,
        sort_index: 0,
    }
}

pub(crate) fn build_yaml_asset_objects<F>(
    asset: &AssetNode,
    docs: &[YamlDoc],
    mut script_lookup: F,
) -> Vec<AssetObject>
where
    F: FnMut(&Guid) -> Option<ScriptTypeInfo>,
{
    let mut out = Vec::new();
    let mut seen_file_ids = HashSet::new();
    for doc in docs {
        if doc.file_id == 0 {
            continue;
        }
        if !yaml_doc_should_index_asset_object(asset, doc) {
            continue;
        }
        // Duplicate fileIDs (e.g. botched merges) would collide on
        // object_key and strand the first copy's FTS row — first doc wins.
        if !seen_file_ids.insert(doc.file_id) {
            continue;
        }
        let script_type = doc
            .m_script_guid
            .as_ref()
            .and_then(|guid| script_lookup(guid));
        let type_name = script_type
            .as_ref()
            .map(|info| info.class_name.clone())
            .filter(|name| !name.is_empty())
            .or_else(|| (!doc.type_name.is_empty()).then(|| doc.type_name.clone()))
            .unwrap_or_else(|| unity_class_name(doc.class_id).to_string());
        let type_lower = type_name.to_ascii_lowercase();
        let display_name = doc
            .m_name
            .as_ref()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if doc.doc_index == 0 {
                    file_stem(&asset.path).to_string()
                } else {
                    type_name.clone()
                }
            });
        let script_class_lower = script_type
            .as_ref()
            .map(|info| info.class_name_lower.clone())
            .unwrap_or_default();
        let script_class_name = script_type.as_ref().map(|info| info.class_name.clone());
        let script_full_name = script_type
            .as_ref()
            .map(|info| info.full_name_lower.as_str())
            .unwrap_or("");
        let script_type_search = script_type
            .as_ref()
            .map(|info| info.type_search_lower.as_str())
            .unwrap_or("");
        let unity_type_lower = unity_class_name(doc.class_id).to_ascii_lowercase();
        let class_terms = class_aliases(doc.class_id).join(" ");
        let type_search = merge_type_terms([
            asset.kind.camel_str(),
            type_lower.as_str(),
            script_class_lower.as_str(),
            script_full_name,
            script_type_search,
            unity_type_lower.as_str(),
            class_terms.as_str(),
        ]);

        let searchable = yaml_doc_is_searchable_sub_asset(asset, doc);
        out.push(asset_object_from_parts(
            asset,
            Some(doc.file_id),
            display_name,
            type_name,
            type_lower,
            type_search,
            script_class_name,
            script_class_lower,
            doc.doc_index == 0,
            doc.doc_index > 0,
            searchable,
            Some(format!("doc:{}", doc.file_id)),
            doc.doc_index as i64 + 1,
        ));
    }
    out
}

/// A non-main YAML doc is a Unity sub-asset only when it is addressable from
/// outside its file: extra documents of a multi-object `.asset`, or
/// AudioMixer group/snapshot controllers (class 243/245). Scene/prefab
/// interiors (GameObjects, components, prefab instances) and controller
/// state machines are not — nothing can reference them cross-file, and every
/// `asset_objects` reader filters on `searchable = 1`, so persisting them
/// produced write-only rows (a 57 MiB scene expanded into ~35k rows and
/// stalled the watcher's FTS delete path).
fn yaml_doc_is_sub_asset(asset: &AssetNode, doc: &YamlDoc) -> bool {
    if asset.kind == AssetKind::GenericAsset {
        return true;
    }
    matches!(doc.class_id, 243 | 245)
}

fn yaml_doc_should_index_asset_object(asset: &AssetNode, doc: &YamlDoc) -> bool {
    if doc.doc_index == 0 {
        return true;
    }
    if asset.ext.eq_ignore_ascii_case("playable") {
        return false;
    }
    yaml_doc_is_sub_asset(asset, doc)
}

fn yaml_doc_is_searchable_sub_asset(asset: &AssetNode, doc: &YamlDoc) -> bool {
    if !asset.exists_on_disk || doc.doc_index == 0 {
        return false;
    }
    yaml_doc_is_sub_asset(asset, doc)
}

pub(crate) fn build_importer_sub_asset_objects(
    asset: &AssetNode,
    entries: &[ImporterSubAsset],
) -> Vec<AssetObject> {
    let searchable = asset.exists_on_disk && asset.kind != AssetKind::MetaOnly;
    entries
        .iter()
        .filter(|entry| entry.file_id != 0)
        .map(|entry| {
            let type_name = entry
                .class_id
                .map(unity_class_name)
                .unwrap_or("SubAsset")
                .to_string();
            let type_lower = type_name.to_ascii_lowercase();
            let display_name = if entry.name.trim().is_empty() {
                type_name.clone()
            } else {
                entry.name.trim().to_string()
            };
            let class_terms = entry
                .class_id
                .map(class_aliases)
                .unwrap_or_default()
                .join(" ");
            let type_search = merge_type_terms([
                asset.kind.camel_str(),
                asset.ext.as_str(),
                type_lower.as_str(),
                class_terms.as_str(),
            ]);
            asset_object_from_parts(
                asset,
                Some(entry.file_id),
                display_name,
                type_name,
                type_lower,
                type_search,
                None,
                String::new(),
                false,
                true,
                searchable,
                None,
                10_000 + entry.order_index as i64,
            )
        })
        .collect()
}

/// Byte markers that must appear in a .meta file for any of the sub-asset
/// tables below to be non-empty. The full `serde_yaml` deserialization is the
/// dominant cost of the meta-parse phase, and most metas (NativeFormatImporter
/// / DefaultImporter / MonoImporter — prefabs, .asset files, scripts, folders)
/// contain none of these keys, so a substring precheck skips the parse
/// entirely without ever producing a false negative.
const IMPORTER_SUBASSET_MARKERS: &[&[u8]] = &[
    b"internalIDToNameTable",
    b"fileIDToRecycleName",
    b"spriteSheet",
];

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && haystack.windows(needle.len()).any(|window| window == needle)
}

pub(crate) fn parse_importer_subassets(content: &[u8]) -> Vec<ImporterSubAsset> {
    if !IMPORTER_SUBASSET_MARKERS
        .iter()
        .any(|marker| contains_subslice(content, marker))
    {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(content);
    let Ok(value) = serde_yaml::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(root) = value.as_mapping() else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for (key, importer) in root {
        let Some(key) = key.as_str() else {
            continue;
        };
        if !key.ends_with("Importer") {
            continue;
        }
        parse_internal_id_to_name_table(importer, &mut entries);
        if key == "TextureImporter" {
            parse_texture_sprites(importer, &mut entries);
        }
        if key == "ModelImporter" && entries.is_empty() {
            parse_legacy_file_id_to_recycle_name(importer, &mut entries);
        }
    }
    dedupe_importer_entries(entries)
}

fn asset_object_from_parts(
    asset: &AssetNode,
    file_id: Option<i64>,
    display_name: String,
    type_name: String,
    type_lower: String,
    type_search: String,
    script_class_name: Option<String>,
    script_class_lower: String,
    is_main: bool,
    is_sub_asset: bool,
    searchable: bool,
    target_id: Option<String>,
    sort_index: i64,
) -> AssetObject {
    let (root_i, path_lower, file_name_lower, _) = derive_search_cols(&asset.path);
    AssetObject {
        object_key: asset_object_key(&asset.guid, file_id),
        asset_guid: asset.guid,
        file_id,
        path: asset.path.clone(),
        kind: asset.kind,
        root: super::types::AssetRoot::from_i32(root_i),
        path_lower,
        file_name_lower,
        name_lower: display_name.to_ascii_lowercase(),
        name: display_name,
        type_name,
        type_lower,
        type_search,
        script_class_name,
        script_class_lower,
        is_main,
        is_sub_asset,
        searchable,
        target_id,
        sort_index,
    }
}

fn parse_internal_id_to_name_table(importer: &Value, entries: &mut Vec<ImporterSubAsset>) {
    let Some(table) = importer
        .get("internalIDToNameTable")
        .and_then(|value| value.as_sequence())
    else {
        return;
    };
    for item in table {
        let Some(first) = item.get("first").and_then(|value| value.as_mapping()) else {
            continue;
        };
        let mut class_id: Option<i32> = None;
        let mut file_id: Option<i64> = None;
        for (k, v) in first {
            if let (Some(k_int), Some(v_int)) = (parse_yaml_int(k), parse_yaml_int(v)) {
                class_id = Some(k_int as i32);
                file_id = Some(v_int);
                break;
            }
        }
        let Some(file_id) = file_id else {
            continue;
        };
        let name = item
            .get("second")
            .and_then(|value| value.as_str())
            .map(normalize_importer_name)
            .unwrap_or_default();
        entries.push(ImporterSubAsset {
            file_id,
            class_id,
            name,
            order_index: entries.len(),
        });
    }
}

fn parse_texture_sprites(importer: &Value, entries: &mut Vec<ImporterSubAsset>) {
    let Some(sprites) = importer
        .get("spriteSheet")
        .and_then(|value| value.get("sprites"))
        .and_then(|value| value.as_sequence())
    else {
        return;
    };
    for sprite in sprites {
        let Some(file_id) = sprite.get("internalID").and_then(parse_yaml_int) else {
            continue;
        };
        let name = sprite
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Sprite".to_string());
        entries.push(ImporterSubAsset {
            file_id,
            class_id: Some(213),
            name,
            order_index: entries.len(),
        });
    }
}

fn parse_legacy_file_id_to_recycle_name(importer: &Value, entries: &mut Vec<ImporterSubAsset>) {
    let Some(table) = importer
        .get("fileIDToRecycleName")
        .and_then(|value| value.as_mapping())
    else {
        return;
    };
    for (k, v) in table {
        let Some(file_id) = parse_yaml_int(k) else {
            continue;
        };
        let name = v.as_str().map(normalize_importer_name).unwrap_or_default();
        let class_id =
            crate::diff::semantic::model_meta::legacy_class_id_from_short_file_id(file_id);
        entries.push(ImporterSubAsset {
            file_id,
            class_id,
            name,
            order_index: entries.len(),
        });
    }
}

fn dedupe_importer_entries(entries: Vec<ImporterSubAsset>) -> Vec<ImporterSubAsset> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        if entry.file_id == 0 || !seen.insert(entry.file_id) {
            continue;
        }
        out.push(entry);
    }
    out
}

fn parse_yaml_int(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_u64().map(|n| n as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<i64>().ok()))
}

fn normalize_importer_name(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed.strip_prefix("//").unwrap_or(trimmed).to_string()
}

fn unity_class_name(class_id: i32) -> &'static str {
    crate::diff::semantic::unity_class_name(class_id)
}

fn class_aliases(class_id: i32) -> Vec<&'static str> {
    match class_id {
        1 => vec!["gameobject", "game_object"],
        4 | 224 => vec!["transform"],
        21 => vec!["material", "mat"],
        43 => vec!["mesh"],
        74 => vec!["animationclip", "animation", "anim"],
        82 => vec!["audiosource", "audio"],
        83 => vec!["audioclip", "audio"],
        91 => vec!["animatorcontroller", "controller"],
        95 => vec!["animator"],
        241 => vec!["audiomixer", "audio_mixer", "mixer"],
        243 => vec![
            "audiomixergroup",
            "audio_mixer_group",
            "audiomixergroupcontroller",
            "mixergroup",
            "mixer",
        ],
        244 => vec!["audiomixereffect", "audio_mixer_effect", "mixer"],
        245 => vec![
            "audiomixersnapshot",
            "audio_mixer_snapshot",
            "snapshot",
            "mixer",
        ],
        1001 => vec!["prefabinstance", "prefab"],
        114 => vec!["monobehaviour", "component", "script"],
        213 => vec!["sprite", "texture"],
        _ => Vec::new(),
    }
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    Path::new(name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name)
}

fn merge_type_terms<'a>(terms: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in terms {
        for part in raw.split_whitespace() {
            let normalized = normalize_type_term(part);
            if normalized.is_empty() || !seen.insert(normalized.clone()) {
                continue;
            }
            out.push(normalized);
        }
    }
    out.join(" ")
}

fn normalize_type_term(raw: &str) -> String {
    raw.trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '.' && ch != '_')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texture_node(exists_on_disk: bool) -> AssetNode {
        AssetNode {
            guid: [1u8; 16],
            path: "Assets/Atlas.png".to_string(),
            ext: "png".to_string(),
            kind: AssetKind::Texture,
            exists_on_disk,
            mtime_ns: 0,
            size: 0,
            content_hash: [0u8; 16],
            meta_hash: [0u8; 16],
            parser_version: 1,
            script_class_name: None,
            script_class_lower: String::new(),
            script_namespace_lower: String::new(),
            script_full_name_lower: String::new(),
            script_type_search: String::new(),
            script_inheritance_search: String::new(),
        }
    }

    fn asset_node(path: &str, kind: AssetKind, exists_on_disk: bool) -> AssetNode {
        AssetNode {
            guid: [2u8; 16],
            path: path.to_string(),
            ext: Path::new(path)
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("")
                .to_string(),
            kind,
            exists_on_disk,
            mtime_ns: 0,
            size: 0,
            content_hash: [0u8; 16],
            meta_hash: [0u8; 16],
            parser_version: 1,
            script_class_name: None,
            script_class_lower: String::new(),
            script_namespace_lower: String::new(),
            script_full_name_lower: String::new(),
            script_type_search: String::new(),
            script_inheritance_search: String::new(),
        }
    }

    fn yaml_doc(
        file_id: i64,
        class_id: i32,
        type_name: &str,
        m_name: Option<&str>,
        doc_index: usize,
    ) -> YamlDoc {
        YamlDoc {
            file_id,
            class_id,
            type_name: type_name.to_string(),
            line_start: 0,
            line_end: 0,
            m_name: m_name.map(str::to_string),
            m_game_object_id: None,
            m_father_id: None,
            is_stripped: false,
            source_prefab_guid: None,
            transform_parent_id: None,
            prefab_instance_id: None,
            m_layer: None,
            m_tag_string: None,
            m_static_editor_flags: None,
            m_is_active: None,
            m_enabled: None,
            transform_root_order: None,
            transform_children: Vec::new(),
            m_script_guid: None,
            doc_index,
        }
    }

    #[test]
    fn parse_importer_subassets_precheck_skips_plain_metas_and_parses_tables() {
        // No marker key anywhere → the precheck must return without ever
        // running serde_yaml.
        let plain: &[u8] = b"fileFormatVersion: 2\nguid: deadbeefdeadbeefdeadbeefdeadbeef\nNativeFormatImporter:\n  mainObjectFileID: 100100000\n";
        assert!(parse_importer_subassets(plain).is_empty());

        let with_table: &[u8] = b"fileFormatVersion: 2\nguid: deadbeefdeadbeefdeadbeefdeadbeef\nTextureImporter:\n  internalIDToNameTable:\n  - first:\n      213: 21300000\n    second: Button\n  spriteSheet:\n    sprites: []\n";
        let entries = parse_importer_subassets(with_table);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_id, 21300000);
        assert_eq!(entries[0].name, "Button");
        assert_eq!(entries[0].class_id, Some(213));
    }

    #[test]
    fn importer_subassets_follow_parent_asset_searchability() {
        let entries = vec![ImporterSubAsset {
            file_id: 21300000,
            class_id: Some(213),
            name: "Button".to_string(),
            order_index: 0,
        }];

        let present = build_importer_sub_asset_objects(&texture_node(true), &entries);
        assert_eq!(present.len(), 1);
        assert!(present[0].searchable);

        let stale_meta = build_importer_sub_asset_objects(&texture_node(false), &entries);
        assert_eq!(stale_meta.len(), 1);
        assert!(!stale_meta[0].searchable);
    }

    #[test]
    fn audio_mixer_group_yaml_docs_are_searchable_subassets() {
        let asset = asset_node(
            "Assets/Settings/Audio/DefaultAudioMixer.mixer",
            AssetKind::OtherYaml,
            true,
        );
        let docs = vec![
            yaml_doc(
                24100000,
                241,
                "AudioMixerController",
                Some("DefaultAudioMixer"),
                0,
            ),
            yaml_doc(
                -2919845427630868010,
                243,
                "AudioMixerGroupController",
                Some("Music"),
                1,
            ),
            yaml_doc(
                -3791635409326915812,
                245,
                "AudioMixerSnapshotController",
                Some("Snapshot"),
                2,
            ),
            yaml_doc(
                -4629685397704031051,
                244,
                "AudioMixerEffectController",
                Some("Attenuation"),
                3,
            ),
        ];

        let objects = build_yaml_asset_objects(&asset, &docs, |_| None);
        // Main doc + group + snapshot. The effect controller (class 244) is
        // not addressable from outside the mixer, so it is not persisted.
        assert_eq!(objects.len(), 3);
        assert!(!objects[0].searchable);
        assert!(objects[1].searchable);
        assert!(objects[2].searchable);
        assert!(objects[1].is_sub_asset);
        assert_eq!(objects[1].name, "Music");
        assert_eq!(objects[1].type_name, "AudioMixerGroupController");
        assert!(objects[1].type_search.contains("audiomixergroup"));
        assert!(objects[2].type_search.contains("audiomixersnapshot"));
        assert_eq!(
            objects[1].target_id.as_deref(),
            Some("doc:-2919845427630868010")
        );
    }

    #[test]
    fn scene_and_prefab_yaml_subdocs_are_not_indexed_as_asset_objects() {
        let scene = asset_node("Assets/Scenes/Main.unity", AssetKind::Scene, true);
        let scene_docs = vec![
            yaml_doc(1660057539, 29, "OcclusionCullingSettings", None, 0),
            yaml_doc(963194225, 1, "GameObject", Some("Player"), 1),
            yaml_doc(963194228, 4, "Transform", None, 2),
            yaml_doc(963194226, 114, "MonoBehaviour", None, 3),
            yaml_doc(1426667978, 1001, "PrefabInstance", None, 4),
        ];

        let objects = build_yaml_asset_objects(&scene, &scene_docs, |_| None);
        assert_eq!(objects.len(), 1);
        assert!(objects[0].is_main);
        assert!(!objects[0].is_sub_asset);

        let prefab = asset_node("Assets/Prefabs/Enemy.prefab", AssetKind::Prefab, true);
        let prefab_docs = vec![
            yaml_doc(100000, 1, "GameObject", Some("Enemy"), 0),
            yaml_doc(400000, 4, "Transform", None, 1),
            yaml_doc(11400000, 114, "MonoBehaviour", None, 2),
        ];

        let objects = build_yaml_asset_objects(&prefab, &prefab_docs, |_| None);
        assert_eq!(objects.len(), 1);
        assert!(objects[0].is_main);
        assert!(!objects[0].is_sub_asset);
        assert_eq!(objects[0].name, "Enemy");
    }

    #[test]
    fn timeline_yaml_subdocs_are_not_indexed_as_asset_objects() {
        let asset = asset_node(
            "Assets/Timelines/FarmingAnimation.playable",
            AssetKind::OtherYaml,
            true,
        );
        let docs = vec![
            yaml_doc(11400000, 114, "MonoBehaviour", Some("FarmingAnimation"), 0),
            yaml_doc(
                -571814945566941427,
                114,
                "MonoBehaviour",
                Some("Animation Track"),
                1,
            ),
            yaml_doc(
                -8784712527854523363,
                114,
                "MonoBehaviour",
                Some("AnimationPlayableAsset"),
                2,
            ),
        ];

        let objects = build_yaml_asset_objects(&asset, &docs, |_| None);
        assert_eq!(objects.len(), 1);
        assert!(objects[0].is_main);
        assert!(!objects[0].is_sub_asset);
        assert_eq!(objects[0].name, "FarmingAnimation");
    }
}
