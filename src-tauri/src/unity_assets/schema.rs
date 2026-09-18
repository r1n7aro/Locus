//! Offline type evidence for the shared asset API. Reads project source and
//! meta files only; it neither starts Unity nor uses stale Editor assemblies.
//! Unknown, conditional and ambiguous declarations produce explicit warnings.
//! Proven declarations enforce the same primitive ranges as SerializedProperty.

use crate::unity_asset_core::{self as core, AssetOperation, Diagnostic, Node, Severity, Span};
use crate::unity_csharp::{parse_cs_script, ScriptFieldMeta};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
#[path = "schema/native.rs"]
mod native;

#[derive(Clone, Debug)]
struct Declaration {
    name: String,
    namespace: String,
    wire_class: String,
    source: PathBuf,
    fields: Arc<Vec<ScriptFieldMeta>>,
    field_index: Arc<HashMap<String, usize>>,
    base: Option<String>,
    parents: Vec<String>,
    enum_base: Option<String>,
    uncertain: bool,
    managed_constructible: bool,
}

#[derive(Clone)]
struct FieldType {
    name: String,
    context: String,
    managed: bool,
}

static PARSED: LazyLock<Mutex<HashMap<String, Vec<Declaration>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Load once per transaction and reuse across its assets. Source bytes are
/// captured lazily and remain immutable for this instance's validation run.
pub struct ProjectSchema {
    guid_paths: BTreeMap<String, Vec<PathBuf>>,
    source_names: BTreeMap<String, Vec<PathBuf>>,
    loaded: BTreeSet<PathBuf>,
    declarations: BTreeMap<String, Vec<Declaration>>,
    frozen: Option<BTreeMap<PathBuf, Vec<u8>>>,
    captured: BTreeMap<PathBuf, Vec<u8>>,
    indexed_sources: BTreeMap<PathBuf, Vec<u8>>,
    script_types: HashMap<String, Option<Declaration>>,
    // Preserve the metadata bytes used to resolve GUID -> source. Reading them
    // again after indexing could otherwise certify a different script identity.
    indexed_source_metas: BTreeMap<PathBuf, Vec<u8>>,
}

impl ProjectSchema {
    pub fn captured_sources(&self) -> &BTreeMap<PathBuf, Vec<u8>> {
        &self.captured
    }
    /// Logical property writes require proven types. The raw asset API retains
    /// its explicitly structural contract and returns diagnostics instead.
    pub fn validate_properties(
        &mut self,
        bytes: &[u8],
        operations: &[AssetOperation],
    ) -> Result<(), String> {
        let diagnostics = self.validate(bytes, operations)?;
        if let Some(diagnostic) = diagnostics.first() {
            return Err(format!(
                "property.schema_unverified: {}: {}",
                diagnostic.property_path.as_deref().unwrap_or(""),
                diagnostic.message
            ));
        }
        Ok(())
    }
    pub fn guid_index(&self, root: &Path) -> BTreeMap<String, Vec<String>> {
        self.guid_paths
            .iter()
            .map(|(guid, paths)| {
                (
                    guid.clone(),
                    paths
                        .iter()
                        .filter_map(|p| {
                            p.strip_prefix(root)
                                .ok()
                                .map(|p| p.to_string_lossy().replace('\\', "/"))
                        })
                        .collect(),
                )
            })
            .collect()
    }
    pub fn indexed_meta(&self, root: &Path, relative: &str) -> Option<&[u8]> {
        let path = root.join(format!("{relative}.meta"));
        self.indexed_source_metas
            .get(&path)
            .or_else(|| {
                if cfg!(windows) {
                    self.indexed_source_metas
                        .iter()
                        .find(|(p, _)| {
                            p.to_string_lossy()
                                .replace('\\', "/")
                                .eq_ignore_ascii_case(&path.to_string_lossy().replace('\\', "/"))
                        })
                        .map(|(_, b)| b)
                } else {
                    None
                }
            })
            .map(Vec::as_slice)
    }
    pub fn validate_created_objects(&mut self, bytes: &[u8], ids: &[String]) -> Result<(), String> {
        let asset = core::parse(bytes).map_err(|e| e.to_string())?;
        let model =
            core::semantic::SemanticAsset::new(core::inspect(bytes).map_err(|e| e.to_string())?)?;
        native::validate_templates(&model, ids)?;
        for id in ids {
            let object = model.object(id)?;
            let value = model.resolve(id, "", false)?.value.unwrap();
            if object.root_type == "MonoBehaviour" {
                let ty = self
                    .object_type(&asset, id)
                    .ok_or("property.creation_script_schema_required")?;
                let declaration = self
                    .find_type(&ty, "")
                    .ok_or("property.creation_script_schema_required")?;
                let owner = core::authoring::decimal(&value["m_GameObject"]["fileID"])
                    .ok_or("property.creation_owner_required")?;
                let expected = if owner == "0" {
                    "ScriptableObject"
                } else {
                    "MonoBehaviour"
                };
                let operation = AssetOperation::Set {
                    object_id: id.clone(),
                    property_path: "/MonoBehaviour/m_Script".into(),
                    value: value["m_Script"].clone(),
                };
                let mut diagnostics = Vec::new();
                self.check_assignable(expected, "", &ty, &operation, &mut diagnostics)?;
                if !diagnostics.is_empty() {
                    return Err("property.creation_script_base_unverified".into());
                }
                let fields = self
                    .all_fields(&declaration, 0)
                    .ok_or("property.creation_schema_unverified")?;
                for name in fields.keys() {
                    if !value.as_object().unwrap().contains_key(name) {
                        return Err(format!("property.explicit_data_required: {name}"));
                    }
                }
            }
            for (key, value) in value.as_object().unwrap() {
                let operation = AssetOperation::Set {
                    object_id: id.clone(),
                    property_path: format!(
                        "/{}/{}",
                        object.root_type,
                        core::semantic::escape_pointer(key)
                    ),
                    value: value.clone(),
                };
                let diagnostics = self.validate(bytes, &[operation])?;
                if object.root_type == "MonoBehaviour"
                    && !key.starts_with("m_")
                    && key != "references"
                    && !diagnostics.is_empty()
                {
                    return Err(format!("property.creation_schema_unverified: {key}"));
                }
            }
        }
        Ok(())
    }
    /// Creation requires complete source evidence, including assembly identity.
    /// This is deliberately stricter than editing an existing unknown field.
    pub fn validate_creation(
        &mut self,
        root: &Path,
        bytes: &[u8],
        host: &str,
        slot: &str,
        created: &[String],
    ) -> Result<(), String> {
        let sources = self
            .source_names
            .values()
            .flatten()
            .cloned()
            .collect::<Vec<_>>();
        for path in sources {
            self.load_source(&path);
        }
        let asset = core::parse(bytes).map_err(|e| e.to_string())?;
        let model =
            core::semantic::SemanticAsset::new(core::inspect(bytes).map_err(|e| e.to_string())?)?;
        let resolved = model.resolve(host, slot, false)?;
        let operation = AssetOperation::Set {
            object_id: host.into(),
            property_path: resolved.pointer,
            value: resolved.value.unwrap().clone(),
        };
        if !self.validate(bytes, &[operation.clone()])?.is_empty() {
            return Err("property.creation_schema_unverified: slot type".into());
        }
        let data = model.resolve(host, "", false)?.value.unwrap();
        let entries = data
            .pointer("/references/RefIds")
            .and_then(Value::as_array)
            .ok_or("property.invalid_registry")?;
        for entry in entries.iter().filter(|entry| {
            core::authoring::decimal(&entry["rid"]).is_some_and(|rid| created.contains(&rid))
        }) {
            let class = entry["type"]["class"]
                .as_str()
                .ok_or("property.creation_type_missing")?;
            let ns = entry["type"]["ns"].as_str().unwrap_or("");
            let full = if ns.is_empty() {
                class.to_string()
            } else {
                format!("{ns}.{class}")
            };
            let declaration = self
                .find_type(&full, "")
                .filter(|d| {
                    d.wire_class == class
                        && d.namespace == ns
                        && !d.uncertain
                        && d.managed_constructible
                })
                .ok_or_else(|| format!("property.creation_schema_unverified: {full}"))?;
            let mut parent = declaration.source.parent();
            let mut assembly = "Assembly-CSharp".to_string();
            while let Some(dir) = parent.filter(|d| d.starts_with(root) && *d != root) {
                let definitions = std::fs::read_dir(dir)
                    .map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().and_then(|v| v.to_str()) == Some("asmdef"))
                    .collect::<Vec<_>>();
                if definitions.len() > 1 {
                    return Err("property.creation_ambiguous_assembly".into());
                }
                if let Some(path) = definitions.first() {
                    let bytes = self
                        .source_bytes(path)
                        .ok_or("property.creation_assembly_missing")?;
                    assembly = serde_json::from_slice::<Value>(&bytes)
                        .map_err(|e| e.to_string())?["name"]
                        .as_str()
                        .ok_or("property.creation_assembly_missing")?
                        .into();
                    self.captured.insert(path.clone(), bytes);
                    break;
                }
                if dir.file_name().and_then(|n| n.to_str()) == Some("Editor") {
                    return Err("property.creation_editor_type".into());
                }
                parent = dir.parent();
            }
            if entry["type"]["asm"] != assembly {
                return Err(format!("property.creation_assembly_mismatch: {full}"));
            }
            let mut diagnostics = vec![];
            self.check_value(
                &asset,
                &FieldType {
                    name: declaration.name,
                    context: declaration.namespace,
                    managed: false,
                },
                &entry["data"],
                &operation,
                &mut diagnostics,
                0,
            )?;
            if !diagnostics.is_empty() {
                return Err("property.creation_schema_unverified: template data".into());
            }
        }
        Ok(())
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        let mut schema = Self {
            guid_paths: BTreeMap::new(),
            source_names: BTreeMap::new(),
            loaded: BTreeSet::new(),
            declarations: BTreeMap::new(),
            frozen: None,
            captured: BTreeMap::new(),
            indexed_sources: BTreeMap::new(),
            script_types: HashMap::new(),
            indexed_source_metas: BTreeMap::new(),
        };
        for folder in ["Assets", "Packages"] {
            for entry in walkdir::WalkDir::new(root.join(folder))
                .follow_links(false)
                .into_iter()
                .filter_map(Result::ok)
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if path.extension().and_then(|v| v.to_str()) == Some("cs") {
                    if let Some(name) = path.file_stem().and_then(|v| v.to_str()) {
                        schema
                            .source_names
                            .entry(name.into())
                            .or_default()
                            .push(path.to_path_buf());
                    }
                } else if path.extension().and_then(|v| v.to_str()) == Some("meta") {
                    let Ok(text) = std::fs::read_to_string(path) else {
                        continue;
                    };
                    schema
                        .indexed_source_metas
                        .insert(path.to_path_buf(), text.as_bytes().to_vec());
                    if let Some(guid) = text
                        .lines()
                        .find_map(|line| line.strip_prefix("guid:").map(str::trim))
                        .filter(|guid| {
                            guid.len() == 32 && guid.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                    {
                        schema
                            .guid_paths
                            .entry(guid.to_ascii_lowercase())
                            .or_default()
                            .push(path.with_extension(""));
                    }
                }
            }
        }
        Ok(schema)
    }

    /// Merge callers provide their captured target/result tree, so schema and
    /// reference checks cannot accidentally consult subsequently edited files.
    pub fn from_frozen(files: BTreeMap<String, Vec<u8>>) -> Self {
        let frozen: BTreeMap<PathBuf, Vec<u8>> = files
            .into_iter()
            .map(|(path, bytes)| (PathBuf::from(path), bytes))
            .collect();
        let mut schema = Self {
            guid_paths: BTreeMap::new(),
            source_names: BTreeMap::new(),
            loaded: BTreeSet::new(),
            declarations: BTreeMap::new(),
            frozen: None,
            captured: BTreeMap::new(),
            indexed_sources: BTreeMap::new(),
            script_types: HashMap::new(),
            indexed_source_metas: BTreeMap::new(),
        };
        for (path, bytes) in &frozen {
            if path.extension().and_then(|extension| extension.to_str()) == Some("cs") {
                if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
                    schema
                        .source_names
                        .entry(name.into())
                        .or_default()
                        .push(path.clone());
                }
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("meta") {
                if let Ok(text) = std::str::from_utf8(bytes) {
                    if let Some(guid) = text
                        .lines()
                        .find_map(|line| line.strip_prefix("guid:").map(str::trim))
                        .filter(|guid| {
                            guid.len() == 32 && guid.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                    {
                        schema
                            .guid_paths
                            .entry(guid.to_ascii_lowercase())
                            .or_default()
                            .push(path.with_extension(""));
                    }
                }
            }
        }
        schema.frozen = Some(frozen);
        schema
    }

    fn source_bytes(&self, path: &Path) -> Option<Vec<u8>> {
        if let Some(bytes) = self.captured.get(path) {
            return Some(bytes.clone());
        }
        if let Some(bytes) = self.indexed_sources.get(path) {
            return Some(bytes.clone());
        }
        if let Some(frozen) = &self.frozen {
            frozen.get(path).cloned()
        } else {
            std::fs::read(path).ok()
        }
    }

    fn load_source(&mut self, path: &Path) {
        if !self.loaded.insert(path.to_path_buf()) {
            return;
        }
        let Some(bytes) = self.source_bytes(path) else {
            return;
        };
        self.indexed_sources
            .insert(path.to_path_buf(), bytes.clone());
        let Ok(content) = std::str::from_utf8(&bytes) else {
            return;
        };
        let key = blake3::hash(&bytes).to_hex().to_string();
        let cached = PARSED
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key)
            .cloned();
        let parsed = cached.unwrap_or_else(|| {
            let parsed = declarations(content);
            let mut cache = PARSED.lock().unwrap_or_else(|error| error.into_inner());
            if cache.len() >= 8192 {
                cache.clear();
            }
            cache.insert(key, parsed.clone());
            parsed
        });
        for mut declaration in parsed {
            declaration.source = path.to_path_buf();
            self.declarations
                .entry(declaration.name.clone())
                .or_default()
                .push(declaration);
        }
    }

    fn use_declaration(&mut self, declaration: Declaration) -> Declaration {
        let path = &declaration.source;
        if self.captured.contains_key(path) {
            return declaration;
        }
        if let Some(bytes) = self.indexed_sources.get(path) {
            self.captured.insert(path.clone(), bytes.clone());
        }
        let meta = PathBuf::from(format!("{}.meta", path.display()));
        if let Some(bytes) = self
            .indexed_source_metas
            .get(&meta)
            .cloned()
            .or_else(|| self.source_bytes(&meta))
        {
            self.captured.insert(meta, bytes);
        }
        declaration
    }

    fn script_type(&mut self, guid: &str) -> Option<Declaration> {
        let key = guid.to_ascii_lowercase();
        if let Some(cached) = self.script_types.get(&key) {
            return cached.clone();
        }
        let paths = self.guid_paths.get(&guid.to_ascii_lowercase())?;
        if paths.len() != 1 || paths[0].extension().and_then(|v| v.to_str()) != Some("cs") {
            return None;
        }
        let path = paths[0].clone();
        self.load_source(&path);
        let name = path.file_stem()?.to_str()?;
        let matches: Vec<_> = self
            .declarations
            .values()
            .flatten()
            .filter(|declaration| {
                declaration.source == path && short_name(&declaration.name) == name
            })
            .cloned()
            .collect();
        let result = if matches.len() == 1 {
            Some(self.use_declaration(matches[0].clone()))
        } else {
            None
        };
        self.script_types.insert(key, result.clone());
        result
    }

    fn find_type(&mut self, name: &str, context: &str) -> Option<Declaration> {
        let name = normalized_type(name);
        let short = short_name(&name);
        if let Some(paths) = self.source_names.get(short).cloned() {
            for path in paths {
                self.load_source(&path);
            }
        }
        let mut candidates = Vec::new();
        if name.contains('.') {
            candidates.push(name.clone());
        }
        let mut enclosing = context;
        loop {
            candidates.push(if enclosing.is_empty() {
                name.clone()
            } else {
                format!("{enclosing}.{name}")
            });
            let Some((parent, _)) = enclosing.rsplit_once('.') else {
                break;
            };
            enclosing = parent;
        }
        candidates.push(name.clone());
        for candidate in candidates {
            if let Some(found) = self.declarations.get(&candidate) {
                let declaration = (found.len() == 1).then(|| found[0].clone());
                return declaration.map(|d| self.use_declaration(d));
            }
        }
        let found: Vec<_> = self
            .declarations
            .values()
            .flatten()
            .filter(|declaration| short_name(&declaration.name) == short)
            .cloned()
            .collect();
        (found.len() == 1)
            .then(|| found[0].clone())
            .map(|d| self.use_declaration(d))
    }

    fn field(&mut self, declaration: &Declaration, name: &str, depth: usize) -> Option<FieldType> {
        if depth > 32 || declaration.uncertain {
            return None;
        }
        let field = declaration
            .field_index
            .get(name)
            .and_then(|index| declaration.fields.get(*index));
        if let Some(field) = field {
            return Some(FieldType {
                name: field.field_type.clone(),
                context: declaration.name.clone(),
                managed: field.serialize_reference,
            });
        }
        let base = self.find_type(declaration.base.as_deref()?, &declaration.namespace)?;
        self.field(&base, name, depth + 1)
    }

    fn all_fields(
        &mut self,
        declaration: &Declaration,
        depth: usize,
    ) -> Option<BTreeMap<String, FieldType>> {
        if depth > 32 || declaration.uncertain {
            return None;
        }
        let mut fields = BTreeMap::new();
        if let Some(base_name) = declaration.base.as_deref() {
            if let Some(base) = self.find_type(base_name, &declaration.namespace) {
                fields.extend(self.all_fields(&base, depth + 1)?);
            } else if !matches!(
                short_name(base_name),
                "Object" | "ValueType" | "MonoBehaviour" | "ScriptableObject"
            ) {
                return None;
            }
        }
        for field in declaration.fields.iter() {
            fields.insert(
                field.name.clone(),
                FieldType {
                    name: field.field_type.clone(),
                    context: declaration.name.clone(),
                    managed: field.serialize_reference,
                },
            );
        }
        Some(fields)
    }

    fn object_type(&mut self, asset: &core::Asset, id: &str) -> Option<String> {
        let document = asset.document(id)?;
        let root = document.root.entries()?.first()?;
        if root.key != "MonoBehaviour" {
            return Some(root.key.clone());
        }
        let guid = raw(asset, root.value.get("m_Script")?.get("guid")?)?;
        self.script_type(&guid).map(|declaration| declaration.name)
    }

    fn type_at(&mut self, asset: &core::Asset, operation: &AssetOperation) -> Option<FieldType> {
        let tokens = tokens(operation.property_path())?;
        let document = asset.document(operation.object_id())?;
        let root = document.root.entries()?.first()?;
        if tokens.len() < 2 || tokens[0] != root.key {
            return None;
        }
        // A registry entry often names a nested class in its host script, so
        // load the m_Script declaration before resolving its concrete type.
        let host_declaration = root
            .value
            .get("m_Script")
            .and_then(|node| node.get("guid"))
            .and_then(|node| raw(asset, node))
            .and_then(|guid| self.script_type(&guid));
        let mut start = 2;
        let mut ty = if tokens.len() >= 6
            && tokens[1] == "references"
            && tokens[2] == "RefIds"
            && tokens[4] == "data"
        {
            let registry = root.value.get("references")?.get("RefIds")?.items()?;
            let entry = if let Some(rid) = tokens[3].strip_prefix("@rid=") {
                registry.iter().find(|entry| {
                    entry
                        .value
                        .get("rid")
                        .and_then(|node| raw(asset, node))
                        .as_deref()
                        == Some(rid)
                })?
            } else {
                registry.get(tokens[3].parse::<usize>().ok()?)?
            };
            let identity = entry.value.get("type")?;
            let class = raw(asset, identity.get("class")?)?;
            let namespace = raw(asset, identity.get("ns")?).unwrap_or_default();
            let declaration = self.find_type(
                &if namespace.is_empty() {
                    class
                } else {
                    format!("{namespace}.{class}")
                },
                "",
            )?;
            start = 6;
            self.field(&declaration, &tokens[5], 0)?
        } else if let Some(known) = builtin_field(&root.key, &tokens[1]) {
            known
        } else {
            let declaration = host_declaration?;
            self.field(&declaration, &tokens[1], 0)?
        };
        for token in &tokens[start..] {
            if let Some(element) = element_type(&ty.name) {
                if token.parse::<usize>().is_err() && !token.starts_with('@') {
                    return None;
                }
                ty.name = element;
            } else if let Some(member) = builtin_member(&ty.name, token) {
                ty = FieldType {
                    name: member.into(),
                    context: ty.context.clone(),
                    managed: false,
                };
            } else {
                let declaration = self.find_type(&ty.name, &ty.context)?;
                ty = self.field(&declaration, token, 0)?;
            }
        }
        Some(ty)
    }

    /// Unknown schemas are visible warnings, while proven incompatibilities are
    /// hard errors. Callers can choose to require an empty diagnostics list.
    pub fn validate(
        &mut self,
        bytes: &[u8],
        operations: &[AssetOperation],
    ) -> Result<Vec<Diagnostic>, String> {
        let asset = core::parse(bytes).map_err(|error| format!("assets.{error}"))?;
        let mut diagnostics = Vec::new();
        for (index, operation) in operations.iter().enumerate() {
            let Some(mut ty) = self.type_at(&asset, operation) else {
                warn(
                    &mut diagnostics,
                    operation,
                    "No unambiguous source or built-in schema proves this field's type",
                );
                continue;
            };
            let value = match operation {
                AssetOperation::Set { value, .. } => Some(value),
                AssetOperation::ArrayInsert { value, .. } => {
                    let Some(element) = element_type(&ty.name) else {
                        return Err(mismatch(
                            index,
                            operation,
                            "array operation requires a declared array or List<T>",
                        ));
                    };
                    ty.name = element;
                    Some(value)
                }
                AssetOperation::ArrayResize { value, .. } => {
                    let Some(element) = element_type(&ty.name) else {
                        return Err(mismatch(
                            index,
                            operation,
                            "array operation requires a declared array or List<T>",
                        ));
                    };
                    ty.name = element;
                    value.as_ref()
                }
                AssetOperation::ArrayMove { .. } | AssetOperation::ArrayRemove { .. } => {
                    if element_type(&ty.name).is_none() {
                        return Err(mismatch(
                            index,
                            operation,
                            "array operation requires a declared array or List<T>",
                        ));
                    }
                    None
                }
            };
            if let Some(value) = value {
                self.check_value(&asset, &ty, value, operation, &mut diagnostics, 0)
                    .map_err(|message| mismatch(index, operation, &message))?;
            }
        }
        Ok(diagnostics)
    }

    /// Prove scalar interpretations separately from validation so reads and
    /// array rewrites preserve numeric-looking strings without broad coercion.
    pub fn scalar_hints(&mut self, bytes: &[u8]) -> Result<core::ScalarHints, String> {
        let asset = core::parse(bytes).map_err(|error| format!("assets.{error}"))?;
        let snapshot = core::inspect(bytes).map_err(|error| format!("assets.{error}"))?;
        let mut hints = core::ScalarHints::new();
        for object in snapshot.objects {
            for field in object.fields {
                if !matches!(
                    field.kind.as_str(),
                    "integer" | "number" | "string" | "null" | "array"
                ) {
                    continue;
                }
                let operation = AssetOperation::Set {
                    object_id: object.object_id.clone(),
                    property_path: field.property_path.clone(),
                    value: Value::Null,
                };
                let Some(ty) = self.type_at(&asset, &operation) else {
                    continue;
                };
                let name = normalized_type(&ty.name);
                let packed = element_type(&name).and_then(|element| {
                    packed_element(&element).or_else(|| {
                        self.find_type(&element, &ty.context)
                            .filter(|declaration| !declaration.uncertain)
                            .and_then(|declaration| declaration.enum_base)
                            .filter(|base| matches!(short_name(base), "int" | "Int32"))
                            .map(|_| core::PackedElement::I32)
                    })
                });
                let hint = if let Some(element) = packed {
                    Some(core::ScalarHint::PackedArray(element))
                } else if integer_range(&name).is_some() {
                    Some(core::ScalarHint::Integer)
                } else {
                    match short_name(&name) {
                        "string" | "String" => Some(core::ScalarHint::String),
                        "bool" | "Boolean" => Some(core::ScalarHint::Boolean),
                        "float" | "Single" | "double" | "Double" => Some(core::ScalarHint::Float),
                        _ => self
                            .find_type(&name, &ty.context)
                            .filter(|declaration| {
                                !declaration.uncertain && declaration.enum_base.is_some()
                            })
                            .map(|_| core::ScalarHint::Integer),
                    }
                };
                if let Some(hint) = hint {
                    hints
                        .entry(object.object_id.clone())
                        .or_default()
                        .insert(field.property_path, hint);
                }
            }
        }
        Ok(hints)
    }

    /// Empty inherited arrays have no YAML element from which to infer shape.
    /// Prove serialized zero values from source; never run constructors or guess
    /// an unknown native layout. Nested empty arrays receive their own prototypes.
    pub fn prefab_array_templates(
        &mut self,
        bytes: &[u8],
        file: &core::authoring::AuthoringAsset,
    ) -> Result<BTreeMap<String, BTreeMap<String, Value>>, String> {
        fn zero(
            schema: &mut ProjectSchema,
            ty: &FieldType,
            path: &str,
            templates: &mut BTreeMap<String, Value>,
            depth: usize,
        ) -> Option<Value> {
            if depth > 32 {
                return None;
            }
            if let Some(element) = element_type(&ty.name) {
                let value = zero(
                    schema,
                    &FieldType {
                        name: element,
                        ..ty.clone()
                    },
                    &format!("{path}/*"),
                    templates,
                    depth + 1,
                )?;
                templates.insert(path.into(), value);
                return Some(serde_json::json!([]));
            }
            if ty.managed {
                return Some(serde_json::json!({"rid":"-2"}));
            }
            let name = normalized_type(&ty.name);
            if integer_range(&name).is_some() {
                return Some(serde_json::json!(0));
            }
            match short_name(&name) {
                "bool" | "Boolean" => return Some(serde_json::json!(false)),
                "float" | "Single" | "double" | "Double" => return Some(serde_json::json!(0.0)),
                "string" | "String" => return Some(serde_json::json!("")),
                _ => {}
            }
            if schema.is_object_type(&name, &ty.context, 0) == Some(true) {
                return Some(serde_json::json!({"fileID":"0"}));
            }
            if let Some(members) = builtin_members(&name) {
                let mut fields = serde_json::Map::new();
                for (key, name) in members {
                    fields.insert(
                        key.into(),
                        zero(
                            schema,
                            &FieldType {
                                name: name.into(),
                                context: ty.context.clone(),
                                managed: false,
                            },
                            &format!("{path}/{key}"),
                            templates,
                            depth + 1,
                        )?,
                    );
                }
                return Some(Value::Object(fields));
            }
            let declaration = schema.find_type(&name, &ty.context)?;
            if declaration.uncertain {
                return None;
            }
            if declaration.enum_base.is_some() {
                return Some(serde_json::json!(0));
            }
            if !declaration.managed_constructible {
                return None;
            }
            let fields = schema.all_fields(&declaration, 0)?;
            let mut value = serde_json::Map::new();
            for (key, field) in fields {
                value.insert(
                    key.clone(),
                    zero(
                        schema,
                        &field,
                        &format!("{path}/{}", core::semantic::escape_pointer(&key)),
                        templates,
                        depth + 1,
                    )?,
                );
            }
            Some(Value::Object(value))
        }
        let empty = file
            .semantic()?
            .snapshot
            .objects
            .into_iter()
            .flat_map(|o| {
                o.fields
                    .into_iter()
                    .filter(|f| f.value.as_array().is_some_and(Vec::is_empty))
                    .map(move |f| (o.object_id.clone(), f.property_path))
            })
            .collect::<Vec<_>>();
        if empty.is_empty() {
            return Ok(BTreeMap::new());
        }
        let asset = core::parse(bytes).map_err(|e| e.to_string())?;
        let mut result = BTreeMap::new();
        for (id, path) in empty {
            let op = AssetOperation::Set {
                object_id: id.clone(),
                property_path: path.clone(),
                value: Value::Null,
            };
            if let Some(ty) = self.type_at(&asset, &op) {
                let pattern = core::prefab::array_hint_pattern(&path);
                let mut templates = BTreeMap::new();
                if zero(self, &ty, &pattern, &mut templates, 0).is_some() {
                    result
                        .entry(id)
                        .or_insert_with(BTreeMap::new)
                        .extend(templates);
                }
            }
        }
        Ok(result)
    }

    fn check_value(
        &mut self,
        asset: &core::Asset,
        ty: &FieldType,
        value: &Value,
        operation: &AssetOperation,
        diagnostics: &mut Vec<Diagnostic>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 96 {
            return Err("serialized type nesting exceeds 96".into());
        }
        if let Some(element) = element_type(&ty.name) {
            let items = value.as_array().ok_or("expected array")?;
            let element = FieldType {
                name: element,
                ..ty.clone()
            };
            for item in items {
                self.check_value(asset, &element, item, operation, diagnostics, depth + 1)?;
            }
            return Ok(());
        }
        if ty.managed {
            core::decode_asset_value(value).map_err(|error| error.to_string())?;
            let rid = value
                .as_object()
                .filter(|map| map.len() == 1)
                .and_then(|map| map.get("rid"))
                .and_then(Value::as_str)
                .ok_or("expected managed reference {rid: decimal string}")?;
            if rid == "-2" {
                return Ok(());
            }
            let host = asset
                .document(operation.object_id())
                .and_then(|document| document.root.entries())
                .and_then(|roots| roots.first())
                .ok_or("managed reference host is missing")?;
            let registry = host
                .value
                .get("references")
                .and_then(|node| node.get("RefIds"))
                .and_then(Node::items)
                .ok_or("managed reference registry is missing")?;
            let entry = registry
                .iter()
                .find(|entry| {
                    entry
                        .value
                        .get("rid")
                        .and_then(|node| raw(asset, node))
                        .as_deref()
                        == Some(rid)
                })
                .ok_or("managed reference ID is absent from the host registry")?;
            let identity = entry
                .value
                .get("type")
                .ok_or("managed reference type is missing")?;
            let class = identity
                .get("class")
                .and_then(|node| raw(asset, node))
                .ok_or("managed reference class is missing")?;
            let ns = identity
                .get("ns")
                .and_then(|node| raw(asset, node))
                .unwrap_or_default();
            let actual = if ns.is_empty() {
                class
            } else {
                format!("{ns}.{class}")
            };
            return self.check_assignable(&ty.name, &ty.context, &actual, operation, diagnostics);
        }
        let name = normalized_type(&ty.name);
        if native::check_value(&name, value)? {
            return Ok(());
        }
        if let Some((minimum, maximum)) = integer_range(&name) {
            let number = core::decode_asset_value(value).map_err(|error| error.to_string())?;
            let integer = number
                .as_i64()
                .map(i128::from)
                .or_else(|| number.as_u64().map(i128::from))
                .ok_or("expected an exact integer, not a float or boolean")?;
            if integer < minimum || integer > maximum {
                return Err(format!(
                    "integer is outside {} range [{minimum}, {maximum}]",
                    ty.name
                ));
            }
            return Ok(());
        }
        match short_name(&name) {
            "bool" | "Boolean" => {
                return if value.is_boolean() || matches!(value.as_i64(), Some(0 | 1)) {
                    Ok(())
                } else {
                    Err("expected boolean or integer 0/1".into())
                }
            }
            "string" | "String" => {
                return if value.is_string() {
                    Ok(())
                } else {
                    Err("expected string; null and numeric values are not strings".into())
                }
            }
            "float" | "Single" | "double" | "Double" => {
                let number = value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or("expected finite numeric value")?;
                if matches!(short_name(&name), "float" | "Single") && number.abs() > f32::MAX as f64
                {
                    return Err("number exceeds single precision range".into());
                }
                return Ok(());
            }
            _ => {}
        }
        if let Some(members) = builtin_members(&name) {
            let map = value.as_object().ok_or("expected value-type object")?;
            if map.len() != members.len() || members.iter().any(|(key, _)| !map.contains_key(*key))
            {
                return Err(format!(
                    "{} requires exactly its serialized members",
                    ty.name
                ));
            }
            for (key, name) in members {
                self.check_value(
                    asset,
                    &FieldType {
                        name: name.into(),
                        context: ty.context.clone(),
                        managed: false,
                    },
                    &map[key],
                    operation,
                    diagnostics,
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        if self.is_object_type(&name, &ty.context, 0) == Some(true) {
            core::decode_asset_value(value).map_err(|error| error.to_string())?;
            let reference = value
                .as_object()
                .filter(|map| map.contains_key("fileID"))
                .ok_or("expected Unity object reference")?;
            let id = reference["fileID"]
                .as_str()
                .ok_or("reference fileID must be a decimal string")?;
            if id == "0" {
                return Ok(());
            }
            let actual = if let Some(guid) = reference.get("guid").and_then(Value::as_str) {
                self.reference_type(guid, id)
            } else {
                self.object_type(asset, id)
            };
            if let Some(actual) = actual {
                self.check_assignable(&name, &ty.context, &actual, operation, diagnostics)?;
            } else {
                warn(diagnostics, operation, "Referenced asset type cannot be proven from source or serialized project files");
            }
            return Ok(());
        }
        let Some(declaration) = self.find_type(&name, &ty.context) else {
            warn(
                diagnostics,
                operation,
                &format!("No source schema proves type {}", ty.name),
            );
            return Ok(());
        };
        if declaration.uncertain {
            warn(
                diagnostics,
                operation,
                "Conditional, partial or malformed declarations require a compiled schema",
            );
            return Ok(());
        }
        if let Some(base) = declaration.enum_base {
            return self.check_value(
                asset,
                &FieldType {
                    name: base,
                    context: ty.context.clone(),
                    managed: false,
                },
                value,
                operation,
                diagnostics,
                depth + 1,
            );
        }
        let Some(fields) = self.all_fields(&declaration, 0) else {
            warn(
                diagnostics,
                operation,
                "Inherited or conditional type members cannot be completely proven",
            );
            return Ok(());
        };
        let map = value
            .as_object()
            .ok_or("expected serialized struct or class object")?;
        for (name, value) in map {
            let field = fields
                .get(name)
                .or_else(|| {
                    name.strip_prefix('<')
                        .and_then(|name| name.strip_suffix(">k__BackingField"))
                        .and_then(|name| fields.get(name))
                })
                .ok_or_else(|| format!("{} has no serialized member {name}", declaration.name))?;
            self.check_value(asset, field, value, operation, diagnostics, depth + 1)?;
        }
        if map.len() != fields.len() {
            return Err(format!(
                "{} assignment must provide every serialized member",
                declaration.name
            ));
        }
        Ok(())
    }

    fn is_object_type(&mut self, name: &str, context: &str, depth: usize) -> Option<bool> {
        if depth > 32 {
            return None;
        }
        if builtin_object(name) {
            return Some(true);
        }
        let declaration = self.find_type(name, context)?;
        if declaration.uncertain {
            return None;
        }
        let Some(base) = declaration.base else {
            return Some(false);
        };
        self.is_object_type(&base, &declaration.namespace, depth + 1)
    }

    fn reference_type(&mut self, guid: &str, id: &str) -> Option<String> {
        let paths = self.guid_paths.get(&guid.to_ascii_lowercase())?;
        if paths.len() != 1 {
            return None;
        }
        let path = paths[0].clone();
        match path.extension().and_then(|value| value.to_str())? {
            "cs" => return Some("MonoScript".into()),
            "png" | "jpg" | "jpeg" | "tga" | "psd" | "exr" => return None, // Sprite subassets share texture GUIDs.
            _ => {}
        }
        let bytes = self.source_bytes(&path)?;
        let asset = core::parse(&bytes).ok()?;
        self.object_type(&asset, id)
    }

    fn check_assignable(
        &mut self,
        expected: &str,
        context: &str,
        actual: &str,
        operation: &AssetOperation,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> Result<(), String> {
        if matches!(expected, "object" | "System.Object") {
            return Ok(());
        }
        let expected = self
            .find_type(expected, context)
            .map(|declaration| declaration.name)
            .unwrap_or_else(|| normalized_type(expected));
        let original_actual = normalized_type(actual);
        let mut pending = vec![original_actual.clone()];
        let mut visited = BTreeSet::new();
        let mut unknown = false;
        while let Some(actual) = pending.pop() {
            if !visited.insert(actual.clone()) {
                continue;
            }
            if visited.len() > 128 {
                return Err("reference inheritance exceeds supported depth".into());
            }
            if expected == actual
                || (builtin_object(&expected) && short_name(&expected) == short_name(&actual))
            {
                return Ok(());
            }
            if matches!(short_name(&expected), "Object") && builtin_object(&actual) {
                return Ok(());
            }
            if builtin_object(&actual) {
                if let Some(parent) = builtin_parent(&actual) {
                    pending.push(parent.into());
                }
                continue;
            }
            let Some(declaration) = self.find_type(&actual, "") else {
                unknown = true;
                continue;
            };
            if declaration.uncertain {
                unknown = true;
                continue;
            }
            for base in declaration.parents {
                let resolved = self
                    .find_type(&base, &declaration.namespace)
                    .map(|declaration| declaration.name)
                    .unwrap_or(base);
                pending.push(resolved);
            }
        }
        if unknown {
            warn(diagnostics, operation, "Reference assignability requires an unavailable, conditional or partial type schema");
            Ok(())
        } else {
            Err(format!(
                "reference type {original_actual} is incompatible with {expected}"
            ))
        }
    }
}

pub fn validate(root: &Path, bytes: &[u8], operations: &[AssetOperation]) -> Result<(), String> {
    validate_with_diagnostics(root, bytes, operations).map(|_| ())
}

pub fn validate_with_diagnostics(
    root: &Path,
    bytes: &[u8],
    operations: &[AssetOperation],
) -> Result<Vec<Diagnostic>, String> {
    ProjectSchema::load(root)?.validate(bytes, operations)
}

fn mismatch(index: usize, operation: &AssetOperation, message: &str) -> String {
    format!(
        "assets.schema_type_mismatch: operation {index}, object {}, {}: {message}",
        operation.object_id(),
        operation.property_path()
    )
}

fn warn(diagnostics: &mut Vec<Diagnostic>, operation: &AssetOperation, message: &str) {
    if diagnostics.iter().any(|diagnostic| {
        diagnostic.object_id.as_deref() == Some(operation.object_id())
            && diagnostic.property_path.as_deref() == Some(operation.property_path())
    }) {
        return;
    }
    diagnostics.push(Diagnostic {
        code: "schema_unverified".into(),
        message: message.into(),
        severity: Severity::Warning,
        span: Span::default(),
        object_id: Some(operation.object_id().into()),
        property_path: Some(operation.property_path().into()),
    });
}

fn raw(asset: &core::Asset, node: &Node) -> Option<String> {
    node.scalar(asset)
        .map(|text| text.trim().trim_matches('"').trim_matches('\'').to_string())
}

fn tokens(path: &str) -> Option<Vec<String>> {
    if !path.starts_with('/') {
        return None;
    }
    path[1..]
        .split('/')
        .map(|token| {
            let mut out = String::new();
            let mut chars = token.chars();
            while let Some(character) = chars.next() {
                if character != '~' {
                    out.push(character);
                    continue;
                }
                match chars.next()? {
                    '0' => out.push('~'),
                    '1' => out.push('/'),
                    _ => return None,
                }
            }
            Some(out)
        })
        .collect()
}

fn normalized_type(name: &str) -> String {
    name.replace("global::", "")
        .replace([' ', '\t', '\r', '\n'], "")
        .replace(['+', '/'], ".")
}
fn short_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}
fn element_type(name: &str) -> Option<String> {
    let name = normalized_type(name);
    if let Some(element) = name.strip_suffix("[]") {
        return Some(element.into());
    }
    for prefix in ["List<", "System.Collections.Generic.List<"] {
        if let Some(element) = name
            .strip_prefix(prefix)
            .and_then(|name| name.strip_suffix('>'))
        {
            return Some(element.into());
        }
    }
    None
}

fn integer_range(name: &str) -> Option<(i128, i128)> {
    Some(match short_name(name) {
        "sbyte" | "SByte" => (i8::MIN as i128, i8::MAX as i128),
        "byte" | "Byte" => (0, u8::MAX as i128),
        "short" | "Int16" => (i16::MIN as i128, i16::MAX as i128),
        "ushort" | "UInt16" | "char" | "Char" => (0, u16::MAX as i128),
        "int" | "Int32" | "LayerMask" => (i32::MIN as i128, i32::MAX as i128),
        "uint" | "UInt32" => (0, u32::MAX as i128),
        "long" | "Int64" => (i64::MIN as i128, i64::MAX as i128),
        "ulong" | "UInt64" => (0, u64::MAX as i128),
        _ => return None,
    })
}

fn packed_element(name: &str) -> Option<core::PackedElement> {
    use core::PackedElement::*;
    Some(match short_name(name) {
        "bool" | "Boolean" => Bool,
        "sbyte" | "SByte" => I8,
        "byte" | "Byte" => U8,
        "short" | "Int16" => I16,
        "ushort" | "UInt16" | "char" | "Char" => U16,
        "int" | "Int32" => I32,
        "uint" | "UInt32" => U32,
        "long" | "Int64" => I64,
        "ulong" | "UInt64" => U64,
        "float" | "Single" => F32,
        "double" | "Double" => F64,
        _ => return None,
    })
}

fn builtin_members(name: &str) -> Option<Vec<(&'static str, &'static str)>> {
    Some(match short_name(name) {
        "Vector2" => vec![("x", "float"), ("y", "float")],
        "Vector3" => vec![("x", "float"), ("y", "float"), ("z", "float")],
        "Vector4" | "Quaternion" => vec![
            ("x", "float"),
            ("y", "float"),
            ("z", "float"),
            ("w", "float"),
        ],
        "Vector2Int" => vec![("x", "int"), ("y", "int")],
        "Vector3Int" => vec![("x", "int"), ("y", "int"), ("z", "int")],
        "Color" => vec![
            ("r", "float"),
            ("g", "float"),
            ("b", "float"),
            ("a", "float"),
        ],
        "Color32" => vec![("r", "byte"), ("g", "byte"), ("b", "byte"), ("a", "byte")],
        "Rect" => vec![
            ("x", "float"),
            ("y", "float"),
            ("width", "float"),
            ("height", "float"),
        ],
        _ => return None,
    })
}
fn builtin_member(name: &str, member: &str) -> Option<&'static str> {
    builtin_members(name)?
        .into_iter()
        .find(|(name, _)| *name == member)
        .map(|(_, ty)| ty)
}
fn builtin_field(root: &str, field: &str) -> Option<FieldType> {
    let name = match field {
        "serializedVersion" => "int",
        "m_Name" | "m_EditorClassIdentifier" => "string",
        "m_ObjectHideFlags" | "m_EditorHideFlags" => "uint",
        "m_CorrespondingSourceObject" | "m_PrefabInstance" | "m_PrefabAsset" => {
            "UnityEngine.Object"
        }
        "m_Script" if root == "MonoBehaviour" => "MonoScript",
        "m_GameObject" => "GameObject",
        "m_Enabled" if root == "MonoBehaviour" => "bool",
        "m_Layer" if root == "GameObject" => "int",
        "m_IsActive" if root == "GameObject" => "bool",
        "m_TagString" if root == "GameObject" => "string",
        "m_Icon" if root == "GameObject" => "UnityEngine.Object",
        "m_Component" if root == "GameObject" => "__GameObjectComponents",
        "m_NavMeshLayer" if root == "GameObject" => "int",
        "m_StaticEditorFlags" if root == "GameObject" => "uint",
        "m_RootOrder" if matches!(root, "Transform" | "RectTransform") => "int",
        "m_LocalPosition" | "m_LocalScale" | "m_LocalEulerAnglesHint"
            if matches!(root, "Transform" | "RectTransform") =>
        {
            "Vector3"
        }
        "m_LocalRotation" if matches!(root, "Transform" | "RectTransform") => "Quaternion",
        "m_ConstrainProportionsScale" if matches!(root, "Transform" | "RectTransform") => "bool",
        "m_Father" if matches!(root, "Transform" | "RectTransform") => "Transform",
        "m_Children" if matches!(root, "Transform" | "RectTransform") => "Transform[]",
        "m_AnchorMin" | "m_AnchorMax" | "m_AnchoredPosition" | "m_SizeDelta" | "m_Pivot"
            if root == "RectTransform" =>
        {
            "Vector2"
        }
        _ => return None,
    };
    Some(FieldType {
        name: name.into(),
        context: String::new(),
        managed: false,
    })
}
fn builtin_object(name: &str) -> bool {
    matches!(
        short_name(name),
        "Object"
            | "GameObject"
            | "Component"
            | "Behaviour"
            | "MonoBehaviour"
            | "ScriptableObject"
            | "Transform"
            | "RectTransform"
            | "Material"
            | "Shader"
            | "Texture"
            | "Texture2D"
            | "Texture3D"
            | "RenderTexture"
            | "Sprite"
            | "Mesh"
            | "AudioClip"
            | "AnimationClip"
            | "RuntimeAnimatorController"
            | "AnimatorController"
            | "AnimatorOverrideController"
            | "MonoScript"
            | "TextAsset"
    )
}
fn builtin_parent(name: &str) -> Option<&'static str> {
    Some(match short_name(name) {
        "Object" => return None,
        "MonoBehaviour" => "Behaviour",
        "Behaviour" => "Component",
        "Component" => "Object",
        "RectTransform" => "Transform",
        "Transform" => "Component",
        "Texture2D" | "Texture3D" | "RenderTexture" => "Texture",
        "AnimatorController" | "AnimatorOverrideController" => "RuntimeAnimatorController",
        _ if builtin_object(name) => "Object",
        _ => return None,
    })
}

fn declarations(content: &str) -> Vec<Declaration> {
    fn constructible(node: tree_sitter::Node<'_>, content: &str) -> bool {
        if node.kind() != "class_declaration" {
            return false;
        }
        let mut serializable = false;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "type_parameter_list" {
                return false;
            }
            if child.kind() == "modifier"
                && matches!(
                    child.utf8_text(content.as_bytes()).ok(),
                    Some("abstract" | "static")
                )
            {
                return false;
            }
            if child.kind() == "attribute_list" {
                let mut attributes = child.walk();
                for attr in child.named_children(&mut attributes) {
                    if attr.kind() == "attribute"
                        && attr
                            .child_by_field_name("name")
                            .and_then(|n| n.utf8_text(content.as_bytes()).ok())
                            .is_some_and(|name| {
                                matches!(
                                    name.trim_start_matches("global::"),
                                    "Serializable"
                                        | "SerializableAttribute"
                                        | "System.Serializable"
                                        | "System.SerializableAttribute"
                                )
                            })
                    {
                        serializable = true;
                    }
                }
            }
        }
        serializable
    }
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .is_err()
    {
        return vec![];
    }
    let Some(tree) = parser.parse(content, None) else {
        return vec![];
    };
    let mut out = Vec::new();
    fn walk(
        content: &str,
        node: tree_sitter::Node<'_>,
        namespace: &str,
        enclosing: &str,
        conditional: bool,
        out: &mut Vec<Declaration>,
    ) {
        let mut local_namespace = namespace.to_string();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            let kind = child.kind();
            if matches!(
                kind,
                "namespace_declaration" | "file_scoped_namespace_declaration"
            ) {
                let name = child
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(content.as_bytes()).ok())
                    .unwrap_or("");
                let next = if local_namespace.is_empty() {
                    name.into()
                } else {
                    format!("{local_namespace}.{name}")
                };
                if kind == "file_scoped_namespace_declaration" {
                    local_namespace = next;
                } else {
                    walk(content, child, &next, enclosing, conditional, out);
                }
            } else if matches!(
                kind,
                "class_declaration"
                    | "struct_declaration"
                    | "interface_declaration"
                    | "enum_declaration"
            ) {
                let Some(name) = child
                    .child_by_field_name("name")
                    .and_then(|name| name.utf8_text(content.as_bytes()).ok())
                else {
                    continue;
                };
                let raw = child.utf8_text(content.as_bytes()).unwrap_or("");
                let prefix = if enclosing.is_empty() {
                    local_namespace.as_str()
                } else {
                    enclosing
                };
                let full = if prefix.is_empty() {
                    name.into()
                } else {
                    format!("{prefix}.{name}")
                };
                let wire_class = if enclosing.is_empty() {
                    name.to_string()
                } else {
                    let parent = if local_namespace.is_empty() {
                        enclosing
                    } else {
                        enclosing
                            .strip_prefix(&format!("{local_namespace}."))
                            .unwrap_or(enclosing)
                    };
                    format!("{}/{name}", parent.replace('.', "/"))
                };
                let uncertain = conditional
                    || child.has_error()
                    || raw.contains("#if")
                    || raw.contains("partial ");
                if kind == "enum_declaration" {
                    let mut children = child.walk();
                    let base = child
                        .named_children(&mut children)
                        .find(|node| node.kind() == "base_list")
                        .and_then(|node| node.utf8_text(content.as_bytes()).ok())
                        .map(|text| text.trim().trim_start_matches(':').trim().to_string())
                        .unwrap_or_else(|| "int".into());
                    out.push(Declaration {
                        name: full.clone(),
                        namespace: local_namespace.clone(),
                        wire_class: wire_class.clone(),
                        source: PathBuf::new(),
                        fields: Arc::new(vec![]),
                        field_index: Default::default(),
                        base: None,
                        parents: vec![],
                        enum_base: Some(base),
                        uncertain,
                        managed_constructible: false,
                    });
                } else if let Some(parsed) = parse_cs_script(raw, Some(name)) {
                    let field_index = parsed
                        .serialized_fields
                        .iter()
                        .enumerate()
                        .flat_map(|(index, field)| {
                            let mut names = vec![
                                field.name.clone(),
                                format!("<{}>k__BackingField", field.name),
                            ];
                            names.extend(field.former_names.iter().cloned());
                            names.into_iter().map(move |name| (name, index))
                        })
                        .collect();
                    let mut children = child.walk();
                    let parents = child
                        .named_children(&mut children)
                        .find(|node| node.kind() == "base_list")
                        .and_then(|node| node.utf8_text(content.as_bytes()).ok())
                        .map(|text| {
                            text.trim()
                                .trim_start_matches(':')
                                .split(',')
                                .map(|name| name.trim().to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    out.push(Declaration {
                        name: full.clone(),
                        namespace: local_namespace.clone(),
                        wire_class,
                        source: PathBuf::new(),
                        fields: Arc::new(parsed.serialized_fields),
                        field_index: Arc::new(field_index),
                        base: parsed.base_type,
                        parents,
                        enum_base: None,
                        uncertain,
                        managed_constructible: constructible(child, content),
                    });
                }
                if let Some(body) = child.child_by_field_name("body") {
                    walk(content, body, &local_namespace, &full, conditional, out);
                }
            } else {
                walk(
                    content,
                    child,
                    &local_namespace,
                    enclosing,
                    conditional || kind.starts_with("preproc_"),
                    out,
                );
            }
        }
    }
    walk(content, tree.root_node(), "", "", false, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const GUID: &str = "aabbccdd00112233445566778899aabb";
    const SOURCE: &str = r#"using UnityEngine;
namespace Demo {
public class Config : ScriptableObject {
 public int amount; public float speed; public bool enabled; public string note;
 public long large; public uint unsigned; public ulong wide; public Mode mode;
 public Row[] rows; public System.Collections.Generic.List<int> numbers;
 public GameObject target; public Vector3 vector;
 [SerializeReference] public Node root;
 [System.Serializable] public struct Row { public string name; public int amount; }
 [System.Serializable] public class Node { public int amount; public float speed; }
 public enum Mode : byte { A, B }
}
}"#;
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Assets")).unwrap();
        std::fs::write(dir.path().join("Assets/Config.cs"), SOURCE).unwrap();
        std::fs::write(
            dir.path().join("Assets/Config.cs.meta"),
            format!("fileFormatVersion: 2\nguid: {GUID}\n"),
        )
        .unwrap();
        dir
    }
    fn asset() -> Vec<u8> {
        format!("--- !u!114 &11400000\nMonoBehaviour:\n  m_Script: {{fileID: 11500000, guid: {GUID}, type: 3}}\n  amount: 1\n  speed: 1\n  enabled: 0\n  note:\n  large: 0\n  unsigned: 0\n  wide: 0\n  mode: 0\n  rows: []\n  numbers: []\n  target: {{fileID: 0}}\n  vector: {{x: 1, y: 2, z: 3}}\n  root: {{rid: 7}}\n  references:\n    version: 2\n    RefIds:\n    - rid: 7\n      type: {{class: Config/Node, ns: Demo, asm: Assembly-CSharp}}\n      data:\n        amount: 1\n        speed: 1\n").into_bytes()
    }
    fn set(path: &str, value: Value) -> AssetOperation {
        AssetOperation::Set {
            object_id: "11400000".into(),
            property_path: format!("/MonoBehaviour/{path}"),
            value,
        }
    }

    #[test]
    fn indexed_script_meta_identity_is_preserved_until_transaction_validation() {
        let project = project();
        let meta = project.path().join("Assets/Config.cs.meta");
        let original_meta = std::fs::read(&meta).unwrap();
        let mut schema = ProjectSchema::load(project.path()).unwrap();
        std::fs::write(&meta, "guid: ffffffffffffffffffffffffffffffff\n").unwrap();
        let bytes = asset();
        std::fs::write(project.path().join("Assets/Data.asset"), &bytes).unwrap();
        schema.scalar_hints(&bytes).unwrap();
        assert_eq!(schema.captured_sources()[&meta], original_meta);
        let output = core::edit(&bytes, &[set("amount", json!(2))]).unwrap();
        let candidates = (
            vec![("Assets/Data.asset".into(), bytes.clone(), output)],
            BTreeMap::from([(
                "Assets/Config.cs.meta".into(),
                schema.captured_sources()[&meta].clone(),
            )]),
        );
        let error = crate::unity_assets::storage::execute_candidates(
            project.path(),
            &json!({"action":"apply_batch"}),
            Some(candidates),
        )
        .unwrap_err();
        assert!(error.contains("stale_dependency"), "{error}");
        assert_eq!(
            std::fs::read(project.path().join("Assets/Data.asset")).unwrap(),
            bytes
        );
    }

    #[test]
    fn source_guid_distinguishes_identical_integer_and_float_yaml_encodings() {
        let project = project();
        assert!(
            validate(project.path(), &asset(), &[set("amount", json!(1.5))])
                .unwrap_err()
                .contains("exact integer")
        );
        assert!(validate_with_diagnostics(
            project.path(),
            &asset(),
            &[set("speed", json!(1.5)), set("amount", json!(2))]
        )
        .unwrap()
        .is_empty());
        assert!(validate(project.path(), &asset(), &[set("amount", json!(true))]).is_err());
        assert!(validate(project.path(), &asset(), &[set("enabled", json!(2))]).is_err());
        assert!(validate(
            project.path(),
            &asset(),
            &[set("enabled", json!(true)), set("note", json!("hello"))]
        )
        .is_ok());
        assert!(validate(project.path(), &asset(), &[set("note", Value::Null)]).is_err());
    }

    #[test]
    fn exact_numeric_ranges_enums_and_nested_array_members_are_checked() {
        let project = project();
        for (path, value) in [
            ("amount", json!(2147483648_i64)),
            ("unsigned", json!(-1)),
            (
                "large",
                json!({"kind":"int64","value":"9223372036854775808"}),
            ),
            ("mode", json!(256)),
            ("rows", json!([{"name":"one","amount":1.5}])),
        ] {
            assert!(
                validate(project.path(), &asset(), &[set(path, value)]).is_err(),
                "{path}"
            );
        }
        let operations = vec![
            set(
                "large",
                json!({"kind":"int64","value":"9223372036854775807"}),
            ),
            set(
                "wide",
                json!({"kind":"uint64","value":"18446744073709551615"}),
            ),
            set("rows", json!([{"name":"one","amount":1}])),
            set("vector/x", json!(1.25)),
        ];
        assert!(
            validate_with_diagnostics(project.path(), &asset(), &operations)
                .unwrap()
                .is_empty()
        );
        let insert = AssetOperation::ArrayInsert {
            object_id: "11400000".into(),
            property_path: "/MonoBehaviour/numbers".into(),
            index: 0,
            value: json!(1.5),
        };
        assert!(validate(project.path(), &asset(), &[insert]).is_err());
    }

    #[test]
    fn managed_registry_concrete_types_and_reference_shapes_are_checked() {
        let project = project();
        assert!(validate(
            project.path(),
            &asset(),
            &[set("references/RefIds/@rid=7/data/amount", json!(1.5))]
        )
        .is_err());
        assert!(validate_with_diagnostics(
            project.path(),
            &asset(),
            &[set("references/RefIds/@rid=7/data/speed", json!(1.5))]
        )
        .unwrap()
        .is_empty());
        assert!(validate(
            project.path(),
            &asset(),
            &[set("target", json!({"fileID":0}))]
        )
        .is_err());
        assert!(validate(
            project.path(),
            &asset(),
            &[set("target", json!({"fileID":"0"}))]
        )
        .is_ok());
    }

    #[test]
    fn unknown_and_conditional_schemas_are_explicit_warnings_without_blocking_builtins() {
        let project = project();
        std::fs::write(
            project.path().join("Assets/Config.cs"),
            format!("#if FEATURE\n{SOURCE}\n#endif"),
        )
        .unwrap();
        let warnings =
            validate_with_diagnostics(project.path(), &asset(), &[set("amount", json!(1.5))])
                .unwrap();
        assert_eq!(warnings[0].code, "schema_unverified");
        assert!(
            validate_with_diagnostics(project.path(), &asset(), &[set("m_Enabled", json!(1))])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn referenced_asset_source_type_rejects_wrong_unity_object_class() {
        let project = project();
        let guid = "11223344556677889900aabbccddeeff";
        std::fs::write(project.path().join("Assets/Other.asset"), asset()).unwrap();
        std::fs::write(
            project.path().join("Assets/Other.asset.meta"),
            format!("guid: {guid}\n"),
        )
        .unwrap();
        let operation = set("target", json!({"fileID":"11400000","guid":guid,"type":2}));
        let error = validate(project.path(), &asset(), &[operation.clone()]).unwrap_err();
        assert!(error.contains("incompatible"), "{error}");
        std::fs::write(
            project.path().join("Assets/Config.cs"),
            SOURCE.replace(
                "public GameObject target;",
                "public UnityEngine.Object target;",
            ),
        )
        .unwrap();
        assert!(
            validate_with_diagnostics(project.path(), &asset(), &[operation])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn managed_reference_assignability_follows_declared_interfaces() {
        let project = project();
        let source = SOURCE
            .replace("public Node root;", "public INode root;")
            .replace(
                "public class Node {",
                "public interface INode {} public class Node : INode {",
            );
        std::fs::write(project.path().join("Assets/Config.cs"), source).unwrap();
        assert!(validate_with_diagnostics(
            project.path(),
            &asset(),
            &[set("root", json!({"rid":"7"}))]
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn frozen_schema_uses_captured_source_instead_of_working_tree() {
        let files = BTreeMap::from([
            ("Assets/Config.cs".into(), SOURCE.as_bytes().to_vec()),
            (
                "Assets/Config.cs.meta".into(),
                format!("guid: {GUID}\n").into_bytes(),
            ),
        ]);
        let mut schema = ProjectSchema::from_frozen(files);
        assert!(schema
            .validate(&asset(), &[set("amount", json!(1.5))])
            .is_err());
        assert!(schema
            .validate(&asset(), &[set("speed", json!(1.5))])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn proven_string_hints_preserve_numeric_null_and_empty_spellings() {
        let project = project();
        let mut schema = ProjectSchema::load(project.path()).unwrap();
        for lexical in ["123", "null", "true", ""] {
            let bytes = String::from_utf8(asset())
                .unwrap()
                .replace("  note:\n", &format!("  note: {lexical}\n"))
                .into_bytes();
            let hints = schema.scalar_hints(&bytes).unwrap();
            let snapshot = core::inspect_with_hints(&bytes, &hints).unwrap();
            let field = snapshot.objects[0]
                .fields
                .iter()
                .find(|field| field.property_path == "/MonoBehaviour/note")
                .unwrap();
            assert_eq!(field.value, json!(lexical));
            let output =
                core::edit_with_hints(&bytes, &[set("note", json!("456"))], &hints).unwrap();
            assert!(String::from_utf8(output.bytes)
                .unwrap()
                .contains("note: \"456\""));
        }
    }

    #[test]
    fn source_int_list_proves_compact_primitive_array_codec() {
        let project = project();
        let bytes = String::from_utf8(asset())
            .unwrap()
            .replace("numbers: []", "numbers: 010000000200000003000000")
            .into_bytes();
        let mut schema = ProjectSchema::load(project.path()).unwrap();
        let hints = schema.scalar_hints(&bytes).unwrap();
        let snapshot = core::inspect_with_hints(&bytes, &hints).unwrap();
        let field = snapshot.objects[0]
            .fields
            .iter()
            .find(|field| field.property_path == "/MonoBehaviour/numbers")
            .unwrap();
        assert_eq!(field.value, json!([1, 2, 3]));
        let op = AssetOperation::ArrayInsert {
            object_id: "11400000".into(),
            property_path: "/MonoBehaviour/numbers".into(),
            index: 1,
            value: json!(9),
        };
        assert!(schema.validate(&bytes, &[op.clone()]).unwrap().is_empty());
        let output = core::edit_with_hints(&bytes, &[op], &hints).unwrap();
        assert!(String::from_utf8(output.bytes)
            .unwrap()
            .contains("numbers: 01000000090000000200000003000000"));
    }
}
