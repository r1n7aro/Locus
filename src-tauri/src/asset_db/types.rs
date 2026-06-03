use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub type Guid = [u8; 16];

pub fn parse_guid_hex(hex: &str) -> Option<Guid> {
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut guid = [0u8; 16];
    for i in 0..16 {
        guid[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(guid)
}

pub fn guid_to_hex(guid: &Guid) -> String {
    guid.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn asset_object_key(guid: &Guid, file_id: Option<i64>) -> String {
    let guid_hex = guid_to_hex(guid);
    match file_id {
        Some(file_id) => format!("{}:{}", guid_hex, file_id),
        None => guid_hex,
    }
}

pub fn hash128(data: &[u8]) -> [u8; 16] {
    let h = blake3::hash(data);
    let mut out = [0u8; 16];
    out.copy_from_slice(&h.as_bytes()[..16]);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum AssetKind {
    Scene = 0,
    Prefab = 1,
    GenericAsset = 2,
    Material = 3,
    Animation = 4,
    Controller = 5,
    OtherYaml = 6,
    MetaOnly = 7,
    Script = 8,
    Texture = 9,
    Audio = 10,
    Shader = 11,
    Model = 12,
}

impl AssetKind {
    pub fn from_ext(ext: &str) -> Self {
        match ext {
            "unity" => Self::Scene,
            "prefab" => Self::Prefab,
            "asset" => Self::GenericAsset,
            "mat" => Self::Material,
            "anim" => Self::Animation,
            "controller" => Self::Controller,
            "cs" => Self::Script,
            "png" | "jpg" | "jpeg" | "tga" | "psd" | "tif" | "tiff" | "bmp" | "gif" | "exr"
            | "hdr" => Self::Texture,
            "wav" | "mp3" | "ogg" | "aif" | "aiff" => Self::Audio,
            "shader" | "cginc" | "hlsl" | "glsl" | "compute" => Self::Shader,
            "fbx" | "obj" | "blend" | "dae" | "3ds" | "max" => Self::Model,
            _ => Self::OtherYaml,
        }
    }

    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::Scene,
            1 => Self::Prefab,
            2 => Self::GenericAsset,
            3 => Self::Material,
            4 => Self::Animation,
            5 => Self::Controller,
            6 => Self::OtherYaml,
            7 => Self::MetaOnly,
            8 => Self::Script,
            9 => Self::Texture,
            10 => Self::Audio,
            11 => Self::Shader,
            12 => Self::Model,
            _ => Self::OtherYaml,
        }
    }

    /// Stable camelCase string identifier used in IPC payloads and search
    /// result rows. Single source of truth — the inline match in `db.rs` and
    /// the helper in `commands/asset.rs` both delegate here.
    pub fn camel_str(&self) -> &'static str {
        match self {
            Self::Scene => "scene",
            Self::Prefab => "prefab",
            Self::GenericAsset => "genericAsset",
            Self::Material => "material",
            Self::Animation => "animation",
            Self::Controller => "animatorController",
            Self::OtherYaml => "otherYaml",
            Self::MetaOnly => "metaOnly",
            Self::Script => "script",
            Self::Texture => "texture",
            Self::Audio => "audio",
            Self::Shader => "shader",
            Self::Model => "model",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum FileRole {
    Meta = 0,
    YamlAsset = 1,
}

impl FileRole {
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => Self::Meta,
            1 => Self::YamlAsset,
            _ => Self::YamlAsset,
        }
    }
}

/// Low-level root partition for the `assets` table. Lives here so the `db`
/// layer never needs to reach into `commands::asset`. The command-layer
/// `AssetSearchRoot` is a thin shell around this enum and provides 1:1
/// translation via `From`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AssetRoot {
    Assets = 0,
    Packages = 1,
    ProjectSettings = 2,
    Other = 3,
}

impl AssetRoot {
    /// Classify a workspace-relative path (forward slashes) into a root
    /// partition. Anything that isn't under one of the three known top-level
    /// directories falls into `Other` (which is still indexed but never
    /// matches a `RootIn` filter).
    pub fn from_rel_path(path: &str) -> Self {
        if let Some(rest) = path.strip_prefix("Assets") {
            if rest.is_empty() || rest.starts_with('/') {
                return Self::Assets;
            }
        }
        if let Some(rest) = path.strip_prefix("Packages") {
            if rest.is_empty() || rest.starts_with('/') {
                return Self::Packages;
            }
        }
        if let Some(rest) = path.strip_prefix("ProjectSettings") {
            if rest.is_empty() || rest.starts_with('/') {
                return Self::ProjectSettings;
            }
        }
        Self::Other
    }

    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::Assets,
            1 => Self::Packages,
            2 => Self::ProjectSettings,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AssetNode {
    pub guid: Guid,
    pub path: String,
    pub ext: String,
    pub kind: AssetKind,
    pub exists_on_disk: bool,
    pub mtime_ns: u64,
    pub size: u64,
    pub content_hash: [u8; 16],
    pub meta_hash: [u8; 16],
    pub parser_version: u32,
    pub script_class_name: Option<String>,
    pub script_class_lower: String,
    pub script_namespace_lower: String,
    pub script_full_name_lower: String,
    pub script_type_search: String,
    pub script_inheritance_search: String,
}

#[derive(Debug, Clone)]
pub struct AssetObject {
    pub object_key: String,
    pub asset_guid: Guid,
    pub file_id: Option<i64>,
    pub path: String,
    pub kind: AssetKind,
    pub root: AssetRoot,
    pub path_lower: String,
    pub file_name_lower: String,
    pub name: String,
    pub name_lower: String,
    pub type_name: String,
    pub type_lower: String,
    pub type_search: String,
    pub script_class_name: Option<String>,
    pub script_class_lower: String,
    pub is_main: bool,
    pub is_sub_asset: bool,
    pub searchable: bool,
    pub target_id: Option<String>,
    pub sort_index: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefEdge {
    pub src_guid: Guid,
    pub src_file_id: Option<i64>,
    pub dst_guid: Guid,
    pub dst_file_id: Option<i64>,
    pub class_id_hint: Option<i32>,
    pub field_hint: Option<String>,
    pub ref_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExtractedRef {
    pub src_file_id: Option<i64>,
    pub dst_guid: Guid,
    pub dst_file_id: Option<i64>,
    pub class_id_hint: Option<i32>,
    pub field_hint: Option<String>,
    pub ref_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrefabSourceRef {
    pub guid: Guid,
    pub source_file_id: i64,
    pub type_id: i32,
}

#[derive(Debug, Clone)]
pub struct PropertyOverride {
    pub target: PrefabSourceRef,
    pub property_path: String,
    pub value: Option<String>,
    pub object_ref: Option<PrefabSourceRef>,
}

#[derive(Debug, Clone)]
pub struct RemovedComponent {
    pub target: PrefabSourceRef,
}

#[derive(Debug, Clone)]
pub struct PrefabInstanceIR {
    pub local_file_id: i64,
    pub source_prefab_guid: Guid,
    pub source_prefab_file_id: i64,
    pub transform_parent: Option<i64>,
    pub instance_name: Option<String>,
    pub property_overrides: Vec<PropertyOverride>,
    pub removed_components: Vec<RemovedComponent>,
    pub line_start: usize,
    pub line_end: usize,
}

#[derive(Debug, Clone)]
pub struct StrippedMapping {
    pub local_file_id: i64,
    pub class_id: i32,
    pub type_name: String,
    pub source: PrefabSourceRef,
    pub prefab_instance_id: i64,
}

#[derive(Debug, Clone)]
pub struct TransformOverrideSummary {
    pub target: PrefabSourceRef,
    pub label: Option<String>,
    pub position: Option<[Option<String>; 3]>,
    pub rotation: Option<[Option<String>; 4]>,
    pub scale: Option<[Option<String>; 3]>,
    pub euler_hint: Option<[Option<String>; 3]>,
}

#[derive(Debug, Clone)]
pub struct BulkPropertyOverride {
    pub property_path: String,
    pub value: String,
    pub target_count: usize,
    pub target_source_file_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
pub struct RendererOverrideSummary {
    pub target: PrefabSourceRef,
    pub label: Option<String>,
    pub overrides: Vec<(String, String)>, // (property_path, value)
}

#[derive(Debug, Clone)]
pub struct KeyOverride {
    pub target: PrefabSourceRef,
    pub label: Option<String>,
    pub property_path: String,
    pub value: Option<String>,
    pub object_ref_desc: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OverrideSummary {
    pub instance_name: String,
    pub source_prefab_guid: Guid,
    pub source_prefab_path: Option<String>,
    pub total_override_count: usize,
    pub stripped_ref_count: usize,
    pub removed_component_count: usize,
    pub transform_overrides: Vec<TransformOverrideSummary>,
    pub bulk_overrides: Vec<BulkPropertyOverride>,
    pub renderer_overrides: Vec<RendererOverrideSummary>,
    pub key_overrides: Vec<KeyOverride>,
    pub child_prefab_names: Vec<String>,
    pub detail_file_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum ScanPhase {
    #[serde(rename_all = "camelCase")]
    DirScan,
    #[serde(rename_all = "camelCase")]
    MetaParse { total: u64, completed: u64 },
    #[serde(rename_all = "camelCase")]
    YamlParse { total: u64, completed: u64 },
    #[serde(rename_all = "camelCase")]
    DbWrite,
    #[serde(rename_all = "camelCase")]
    Reconcile {
        verify_hashes: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stage: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completed: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queued: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failed: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    ReconcileDone,
    #[serde(rename_all = "camelCase")]
    Done { stats: ScanStats },
    #[serde(rename_all = "camelCase")]
    Error { error: AppError },
}

impl ScanPhase {
    pub fn reconcile_started(verify_hashes: bool) -> Self {
        Self::Reconcile {
            verify_hashes,
            stage: Some("scanning".to_string()),
            total: None,
            completed: None,
            queued: None,
            failed: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGuidOverview {
    /// Number of GUID groups with at least two colliding `.meta` files.
    pub group_count: u64,
    /// Total number of `.meta` file paths that participate in collisions.
    pub path_count: u64,
    /// Collisions whose duplicated paths all live under `Assets/`.
    pub assets_only_groups: u64,
    /// Collisions whose duplicated paths all live under `Packages/`.
    pub packages_only_groups: u64,
    /// Collisions spanning multiple roots (for example `Assets/` + `Packages/`).
    pub cross_root_groups: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedAssetRoot {
    pub link_rel_path: String,
    pub target_path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStats {
    pub dirs_scanned: u64,
    pub meta_files_found: u64,
    pub yaml_assets_found: u64,
    pub nodes_added: u64,
    pub edges_added: u64,
    pub nodes_updated: u64,
    pub nodes_deleted: u64,
    pub parse_failures: u64,
    pub elapsed_ms: u64,
    pub duplicate_guids: DuplicateGuidOverview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AssetRiskKind {
    BrokenReferences,
    MissingScripts,
    ParseFailures,
    DuplicateGuids,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetRiskEntry {
    pub kind: AssetRiskKind,
    pub count: u64,
}
