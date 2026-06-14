use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::view::{
    UnityEnumOption, UnityManagedReferenceTypeOption, UnitySerializedPropertyAttributeInfo,
    UnitySerializedPropertyDiscoverMatch, UnitySerializedPropertyDiscoverResult,
    UnitySerializedPropertyReadResult, UnitySerializedPropertySnapshot,
    UnitySerializedPropertyTarget,
};

const SCHEMA_CURRENCY_TTL: Duration = Duration::from_millis(300);

#[derive(Clone)]
struct CacheEntry {
    fingerprint: String,
    schema: Arc<SerializedSchemaIndex>,
    verified_at: Instant,
}

fn cache() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn build_locks() -> &'static Mutex<HashMap<String, Arc<Mutex<()>>>> {
    static BUILD_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    BUILD_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn project_key(project_path: &str) -> String {
    project_path
        .strip_prefix(r"\\?\")
        .unwrap_or(project_path)
        .trim()
        .replace('\\', "/")
        .to_ascii_lowercase()
}

pub async fn load_current_schema(project_path: &str) -> Result<Arc<SerializedSchemaIndex>, String> {
    let key = project_key(project_path);

    if let Some(schema) = fresh_cached_schema(&key).await {
        return Ok(schema);
    }

    let params = crate::csharp_compile::params::get_params(project_path).await?;

    if let Some(schema) = cached_schema_for_fingerprint(&key, &params.fingerprint).await {
        return Ok(schema);
    }

    let build_lock = {
        let mut locks = build_locks().lock().await;
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    let _guard = build_lock.lock().await;

    if let Some(schema) = cached_schema_for_fingerprint(&key, &params.fingerprint).await {
        return Ok(schema);
    }

    let payload = crate::csharp_compile::index_serialized_schema(&params).await?;
    let payload: SerializedSchemaPayload = serde_json::from_value(payload)
        .map_err(|error| format!("malformed index/schema response: {error}"))?;
    let schema = Arc::new(SerializedSchemaIndex::new(payload));

    cache().lock().await.insert(
        key,
        CacheEntry {
            fingerprint: params.fingerprint,
            schema: schema.clone(),
            verified_at: Instant::now(),
        },
    );
    Ok(schema)
}

async fn fresh_cached_schema(key: &str) -> Option<Arc<SerializedSchemaIndex>> {
    let now = Instant::now();
    cache()
        .lock()
        .await
        .get(key)
        .filter(|entry| cache_entry_is_fresh(entry, now))
        .map(|entry| entry.schema.clone())
}

async fn cached_schema_for_fingerprint(
    key: &str,
    fingerprint: &str,
) -> Option<Arc<SerializedSchemaIndex>> {
    let mut cache = cache().lock().await;
    let entry = cache.get_mut(key)?;
    if entry.fingerprint != fingerprint {
        return None;
    }

    entry.verified_at = Instant::now();
    Some(entry.schema.clone())
}

fn cache_entry_is_fresh(entry: &CacheEntry, now: Instant) -> bool {
    now.saturating_duration_since(entry.verified_at) < SCHEMA_CURRENCY_TTL
}

pub async fn invalidate(project_path: &str) {
    let key = project_key(project_path);
    cache().lock().await.remove(&key);
}

pub async fn try_load_current_schema(project_path: &str) -> Option<Arc<SerializedSchemaIndex>> {
    match load_current_schema(project_path).await {
        Ok(schema) => Some(schema),
        Err(error) => {
            eprintln!("[UnitySerializedSchema] sidecar schema unavailable: {error}");
            None
        }
    }
}

pub fn target_is_static_schema_excluded(target: &UnitySerializedPropertyTarget) -> bool {
    let full_name = target_type_full_name(target);
    if full_name.trim().is_empty() {
        return true;
    }

    let assembly = target
        .target_type_assembly
        .as_deref()
        .unwrap_or_default()
        .trim();
    is_transient_schema_assembly(assembly)
        || is_unity_builtin_schema_assembly(assembly)
        || is_unity_builtin_type_name(&full_name)
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SerializedSchemaPayload {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    types: Vec<SchemaType>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SchemaTypeRef {
    #[serde(default)]
    full_name: String,
    #[serde(default)]
    assembly: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SchemaType {
    #[serde(default)]
    full_name: String,
    #[serde(default)]
    assembly: String,
    #[serde(default)]
    base_type_full_name: String,
    #[serde(default)]
    base_type_assembly: String,
    #[serde(default)]
    interfaces: Vec<SchemaTypeRef>,
    #[serde(default)]
    is_serializable: bool,
    #[serde(default)]
    is_abstract: bool,
    #[serde(default)]
    is_interface: bool,
    #[serde(default)]
    is_generic_type_definition: bool,
    #[serde(default)]
    contains_generic_parameters: bool,
    #[serde(default)]
    is_unity_object: bool,
    #[serde(default)]
    is_flags_enum: bool,
    #[serde(default)]
    enum_options: Vec<UnityEnumOption>,
    #[serde(default)]
    fields: Vec<SchemaField>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct SchemaField {
    #[serde(default)]
    name: String,
    #[serde(default)]
    field_type_full_name: String,
    #[serde(default)]
    field_type_assembly: String,
    #[serde(default)]
    element_type_full_name: String,
    #[serde(default)]
    element_type_assembly: String,
    #[serde(default)]
    is_array: bool,
    #[serde(default)]
    is_list: bool,
    #[serde(default)]
    has_serialize_reference: bool,
    #[serde(default)]
    is_flags_enum: bool,
    #[serde(default)]
    enum_options: Vec<UnityEnumOption>,
    #[serde(default)]
    tooltip: String,
    #[serde(default)]
    header: String,
    #[serde(default)]
    has_range: bool,
    #[serde(default)]
    range_min: f32,
    #[serde(default)]
    range_max: f32,
    #[serde(default)]
    multiline: bool,
    #[serde(default)]
    min_lines: i32,
    #[serde(default)]
    max_lines: i32,
    #[serde(default)]
    attributes: Vec<UnitySerializedPropertyAttributeInfo>,
}

#[derive(Debug, Clone)]
struct ResolvedFieldSchema {
    field: SchemaField,
    field_type_full_name: String,
    field_type_assembly: String,
}

#[derive(Debug)]
pub struct SerializedSchemaIndex {
    types_by_key: HashMap<String, SchemaType>,
    keys_by_full_name: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct DiscoverFilters {
    pub query: Option<String>,
    pub field_name: Option<String>,
    pub field_type: Option<String>,
    pub max_results: Option<i32>,
}

impl SerializedSchemaIndex {
    fn new(payload: SerializedSchemaPayload) -> Self {
        if payload.schema_version > 1 {
            eprintln!(
                "[UnitySerializedSchema] newer schema version {}; attempting best-effort parse",
                payload.schema_version
            );
        }

        let mut types_by_key = HashMap::new();
        let mut keys_by_full_name: HashMap<String, Vec<String>> = HashMap::new();
        for schema_type in payload.types {
            if schema_type.full_name.trim().is_empty() {
                continue;
            }
            let key = type_key(&schema_type.full_name, &schema_type.assembly);
            keys_by_full_name
                .entry(schema_type.full_name.clone())
                .or_default()
                .push(key.clone());
            types_by_key.insert(key, schema_type);
        }

        Self {
            types_by_key,
            keys_by_full_name,
        }
    }

    pub fn can_enrich_target(&self, target: &UnitySerializedPropertyTarget) -> bool {
        let full_name = target_type_full_name(target);
        if full_name.trim().is_empty() {
            return false;
        }

        if target_is_static_schema_excluded(target) {
            return false;
        }

        let assembly = target
            .target_type_assembly
            .as_deref()
            .unwrap_or_default()
            .trim();
        let Some(schema_type) = self.find_type(&full_name, assembly) else {
            return false;
        };

        !self.type_or_base_has_serialize_reference(schema_type)
    }

    pub fn enrich_read_result(&self, result: &mut UnitySerializedPropertyReadResult) {
        let target = result.target.clone();
        self.enrich_snapshot_tree(&target, &mut result.property);
        for property in &mut result.properties {
            self.enrich_snapshot_tree(&target, property);
        }
    }

    pub fn enrich_discover_result(
        &self,
        result: &mut UnitySerializedPropertyDiscoverResult,
        filters: &DiscoverFilters,
    ) {
        let target = result.target.clone();
        for entry in &mut result.matches {
            self.enrich_discover_match(&target, entry);
        }

        result
            .matches
            .retain(|entry| self.matches_discover_filters(&target, entry, filters));

        if let Some(max_results) = filters.max_results {
            if max_results > 0 && result.matches.len() > max_results as usize {
                result.matches.truncate(max_results as usize);
            }
        }

        if result.matches.is_empty() {
            result.message = "No matching properties.".to_string();
        } else {
            result.message = "ok".to_string();
        }
    }

    fn enrich_snapshot_tree(
        &self,
        result_target: &UnitySerializedPropertyTarget,
        snapshot: &mut UnitySerializedPropertySnapshot,
    ) {
        self.enrich_snapshot(result_target, snapshot);
        let child_target = snapshot
            .binding_target
            .as_ref()
            .unwrap_or(result_target)
            .clone();
        for child in &mut snapshot.children {
            self.enrich_snapshot_tree(&child_target, child);
        }
    }

    fn enrich_snapshot(
        &self,
        result_target: &UnitySerializedPropertyTarget,
        snapshot: &mut UnitySerializedPropertySnapshot,
    ) {
        let target = snapshot.binding_target.as_ref().unwrap_or(result_target);
        let Some(resolved) = self.resolve_property_schema(target, &snapshot.property_path) else {
            if snapshot.number_step <= 0.0 && snapshot.property_type == "Integer" {
                snapshot.number_step = 1.0;
            }
            return;
        };

        snapshot.field_type_full_name = resolved.field_type_full_name.clone();
        snapshot.field_type_assembly = resolved.field_type_assembly.clone();
        snapshot.tooltip = resolved.field.tooltip.clone();
        snapshot.header = resolved.field.header.clone();
        snapshot.has_range = resolved.field.has_range;
        snapshot.range_min = resolved.field.range_min;
        snapshot.range_max = resolved.field.range_max;
        snapshot.multiline = resolved.field.multiline;
        snapshot.min_lines = resolved.field.min_lines;
        snapshot.max_lines = resolved.field.max_lines;
        snapshot.attributes = resolved.field.attributes.clone();
        snapshot.is_flags_enum = resolved.field.is_flags_enum;
        if snapshot.number_step <= 0.0 && snapshot.property_type == "Integer" {
            snapshot.number_step = 1.0;
        }
        if !resolved.field.enum_options.is_empty() {
            snapshot.enum_options = resolved.field.enum_options.clone();
        }
        if snapshot.property_type == "ObjectReference" {
            snapshot.reference_type_full_name = resolved.field_type_full_name.clone();
            snapshot.reference_type_assembly = resolved.field_type_assembly.clone();
        }
        if snapshot.is_managed_reference || snapshot.property_type == "ManagedReference" {
            if snapshot.managed_reference_field_typename.trim().is_empty() {
                snapshot.managed_reference_field_typename = managed_reference_type_name(
                    &resolved.field_type_assembly,
                    &resolved.field_type_full_name,
                );
            }
            if snapshot.managed_reference_display_name.trim().is_empty() {
                snapshot.managed_reference_display_name =
                    managed_reference_display_name(&snapshot.managed_reference_full_typename);
            }
            snapshot.managed_reference_types = self.managed_reference_options(
                &snapshot.managed_reference_field_typename,
                &snapshot.managed_reference_full_typename,
            );
        }

        if let Some(value) = snapshot.value.as_object_mut() {
            if snapshot.property_type == "Enum" {
                value.insert(
                    "isFlags".to_string(),
                    serde_json::Value::Bool(snapshot.is_flags_enum),
                );
            }
        }
    }

    fn enrich_discover_match(
        &self,
        target: &UnitySerializedPropertyTarget,
        entry: &mut UnitySerializedPropertyDiscoverMatch,
    ) {
        let Some(resolved) = self.resolve_property_schema(target, &entry.property_path) else {
            return;
        };
        entry.field_type_full_name = resolved.field_type_full_name;
        entry.field_type_assembly = resolved.field_type_assembly;
    }

    fn matches_discover_filters(
        &self,
        target: &UnitySerializedPropertyTarget,
        entry: &UnitySerializedPropertyDiscoverMatch,
        filters: &DiscoverFilters,
    ) -> bool {
        if !matches_field_name(entry, filters.field_name.as_deref().unwrap_or_default()) {
            return false;
        }

        let query = normalize(filters.query.as_deref().unwrap_or_default());
        if !query.is_empty()
            && !contains_normalized(&entry.property_path, &query)
            && !contains_normalized(&entry.display_name, &query)
            && !contains_normalized(&entry.name, &query)
            && !contains_normalized(&entry.property_type, &query)
            && !contains_normalized(&entry.field_type_full_name, &query)
            && !contains_normalized(&entry.field_type_assembly, &query)
        {
            return false;
        }

        let field_type = filters.field_type.as_deref().unwrap_or_default().trim();
        if field_type.is_empty() {
            return true;
        }

        if let Some(resolved) = self.resolve_property_schema(target, &entry.property_path) {
            return self.type_matches(
                &resolved.field_type_full_name,
                &resolved.field_type_assembly,
                field_type,
            );
        }
        false
    }

    fn resolve_property_schema(
        &self,
        target: &UnitySerializedPropertyTarget,
        property_path: &str,
    ) -> Option<ResolvedFieldSchema> {
        let target_full_name = target
            .target_type_full_name
            .as_deref()
            .or(target.component_type.as_deref())
            .unwrap_or_default();
        if target_full_name.trim().is_empty() || property_path.trim().is_empty() {
            return None;
        }
        if is_serialized_array_size_path(property_path) {
            return None;
        }

        let mut current_full_name = target_full_name.to_string();
        let mut current_assembly = target
            .target_type_assembly
            .as_deref()
            .unwrap_or_default()
            .to_string();
        let mut resolved: Option<ResolvedFieldSchema> = None;

        let normalized_path = property_path.replace(".Array.data[", "[");
        for part in normalized_path.split('.') {
            if part.is_empty() || part == "Array" || part == "size" {
                continue;
            }

            let bracket = part.find('[');
            let member_name = bracket.map_or(part, |idx| &part[..idx]);
            if !member_name.is_empty() {
                let field = self.find_field(&current_full_name, &current_assembly, member_name)?;
                current_full_name = field.field_type_full_name.clone();
                current_assembly = field.field_type_assembly.clone();
                resolved = Some(ResolvedFieldSchema {
                    field,
                    field_type_full_name: current_full_name.clone(),
                    field_type_assembly: current_assembly.clone(),
                });
            }

            if part.contains('[') {
                let base = resolved.as_ref()?;
                let generic_element = generic_first_argument_type_ref(&base.field_type_full_name);
                let element_full_name = if !base.field.element_type_full_name.trim().is_empty() {
                    base.field.element_type_full_name.clone()
                } else if let Some((full_name, _)) = &generic_element {
                    full_name.clone()
                } else {
                    strip_array_suffix(&base.field_type_full_name)
                };
                let element_assembly = if !base.field.element_type_assembly.trim().is_empty() {
                    base.field.element_type_assembly.clone()
                } else if let Some((_, assembly)) = &generic_element {
                    assembly.clone()
                } else {
                    base.field_type_assembly.clone()
                };
                if element_full_name.trim().is_empty() {
                    return None;
                }
                current_full_name = element_full_name;
                current_assembly = element_assembly;
                if let Some(prev) = &mut resolved {
                    prev.field_type_full_name = current_full_name.clone();
                    prev.field_type_assembly = current_assembly.clone();
                }
            }
        }

        resolved
    }

    fn find_field(
        &self,
        owner_full_name: &str,
        owner_assembly: &str,
        field_name: &str,
    ) -> Option<SchemaField> {
        let mut current = self.find_type(owner_full_name, owner_assembly)?;
        loop {
            if let Some(field) = current.fields.iter().find(|field| field.name == field_name) {
                return Some(field.clone());
            }

            if current.base_type_full_name.trim().is_empty() {
                return None;
            }
            current = self.find_type(&current.base_type_full_name, &current.base_type_assembly)?;
        }
    }

    fn find_type(&self, full_name: &str, assembly: &str) -> Option<&SchemaType> {
        if !assembly.trim().is_empty() {
            if let Some(schema_type) = self.types_by_key.get(&type_key(full_name, assembly)) {
                return Some(schema_type);
            }
        }

        let keys = self.keys_by_full_name.get(full_name)?;
        keys.iter().find_map(|key| self.types_by_key.get(key))
    }

    fn type_or_base_has_serialize_reference(&self, schema_type: &SchemaType) -> bool {
        let mut seen = HashSet::new();
        let mut current = Some(schema_type);
        while let Some(schema_type) = current {
            let key = type_key(&schema_type.full_name, &schema_type.assembly);
            if !seen.insert(key) {
                break;
            }
            if schema_type
                .fields
                .iter()
                .any(|field| field.has_serialize_reference)
            {
                return true;
            }
            current = self.find_type(
                &schema_type.base_type_full_name,
                &schema_type.base_type_assembly,
            );
        }
        false
    }

    fn type_matches(&self, full_name: &str, assembly: &str, expected: &str) -> bool {
        let expected = expected.trim();
        if expected.is_empty() {
            return true;
        }

        if name_matches(full_name, expected) {
            return true;
        }

        let mut seen = HashSet::new();
        let mut current = self.find_type(full_name, assembly);
        while let Some(schema_type) = current {
            let key = type_key(&schema_type.full_name, &schema_type.assembly);
            if !seen.insert(key) {
                break;
            }
            if name_matches(&schema_type.full_name, expected) {
                return true;
            }
            if schema_type
                .interfaces
                .iter()
                .any(|iface| name_matches(&iface.full_name, expected))
            {
                return true;
            }
            current = self.find_type(
                &schema_type.base_type_full_name,
                &schema_type.base_type_assembly,
            );
        }
        false
    }

    fn managed_reference_options(
        &self,
        field_typename: &str,
        current_typename: &str,
    ) -> Vec<UnityManagedReferenceTypeOption> {
        let (base_assembly, base_full_name) = split_managed_reference_type_name(field_typename);
        if base_full_name.trim().is_empty() {
            return Vec::new();
        }

        let mut options: Vec<UnityManagedReferenceTypeOption> = self
            .types_by_key
            .values()
            .filter(|schema_type| self.is_managed_reference_candidate(schema_type))
            .filter(|schema_type| {
                base_full_name == "System.Object"
                    || self.type_is_assignable_to(
                        &schema_type.full_name,
                        &schema_type.assembly,
                        &base_full_name,
                        &base_assembly,
                    )
            })
            .map(|schema_type| UnityManagedReferenceTypeOption {
                label: schema_type.full_name.clone(),
                value: managed_reference_type_name(&schema_type.assembly, &schema_type.full_name),
                full_name: schema_type.full_name.clone(),
                assembly: schema_type.assembly.clone(),
            })
            .collect();

        options.sort_by(|a, b| {
            a.full_name
                .cmp(&b.full_name)
                .then(a.assembly.cmp(&b.assembly))
        });
        options.truncate(200);
        if let Some(current) = managed_reference_current_option(current_typename) {
            options.retain(|option| option.value != current.value);
            options.insert(0, current);
        }
        options
    }

    fn is_managed_reference_candidate(&self, schema_type: &SchemaType) -> bool {
        schema_type.is_serializable
            && !schema_type.is_abstract
            && !schema_type.is_interface
            && !schema_type.is_generic_type_definition
            && !schema_type.contains_generic_parameters
            && !schema_type.is_unity_object
    }

    fn type_is_assignable_to(
        &self,
        full_name: &str,
        assembly: &str,
        base_full_name: &str,
        base_assembly: &str,
    ) -> bool {
        if same_type(full_name, assembly, base_full_name, base_assembly) {
            return true;
        }

        let Some(schema_type) = self.find_type(full_name, assembly) else {
            return false;
        };

        if schema_type.interfaces.iter().any(|iface| {
            same_type(
                &iface.full_name,
                &iface.assembly,
                base_full_name,
                base_assembly,
            )
        }) {
            return true;
        }

        if schema_type.base_type_full_name.trim().is_empty() {
            return false;
        }

        self.type_is_assignable_to(
            &schema_type.base_type_full_name,
            &schema_type.base_type_assembly,
            base_full_name,
            base_assembly,
        )
    }
}

fn target_type_full_name(target: &UnitySerializedPropertyTarget) -> String {
    target
        .target_type_full_name
        .as_deref()
        .or(target.component_type.as_deref())
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn is_transient_schema_assembly(assembly: &str) -> bool {
    assembly.starts_with("__LocusSkillPackage_") || assembly.starts_with("__LocusHotPatch_")
}

fn is_unity_builtin_schema_assembly(assembly: &str) -> bool {
    assembly == "UnityEngine"
        || assembly.starts_with("UnityEngine.")
        || assembly == "UnityEditor"
        || assembly.starts_with("UnityEditor.")
}

fn is_unity_builtin_type_name(full_name: &str) -> bool {
    full_name.starts_with("UnityEngine.") || full_name.starts_with("UnityEditor.")
}

fn managed_reference_current_option(type_name: &str) -> Option<UnityManagedReferenceTypeOption> {
    let (assembly, full_name) = split_managed_reference_type_name(type_name);
    if full_name.trim().is_empty() {
        return None;
    }

    Some(UnityManagedReferenceTypeOption {
        label: full_name.clone(),
        value: managed_reference_type_name(&assembly, &full_name),
        full_name,
        assembly,
    })
}

fn type_key(full_name: &str, assembly: &str) -> String {
    format!("{}|{}", assembly.trim(), full_name.trim())
}

fn same_type(
    full_name: &str,
    assembly: &str,
    expected_full_name: &str,
    expected_assembly: &str,
) -> bool {
    full_name == expected_full_name
        && (expected_assembly.trim().is_empty() || assembly == expected_assembly)
}

fn name_matches(full_name: &str, expected: &str) -> bool {
    full_name == expected || simple_type_name(full_name) == expected
}

fn simple_type_name(full_name: &str) -> &str {
    let after_nested = full_name.rsplit('+').next().unwrap_or(full_name);
    after_nested.rsplit('.').next().unwrap_or(after_nested)
}

fn strip_array_suffix(value: &str) -> String {
    value.strip_suffix("[]").unwrap_or(value).to_string()
}

fn is_serialized_array_size_path(property_path: &str) -> bool {
    let path = property_path.trim();
    path == "Array.size" || path.ends_with(".Array.size")
}

fn generic_first_argument_type_ref(full_name: &str) -> Option<(String, String)> {
    let start = full_name.find("[[")? + 2;
    let rest = &full_name[start..];
    let mut depth = 0usize;
    let mut end = None;
    for (idx, ch) in rest.char_indices() {
        match ch {
            '[' => depth = depth.saturating_add(1),
            ']' if depth == 0 => {
                end = Some(idx);
                break;
            }
            ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let arg = rest[..end?].trim();
    if arg.is_empty() {
        return None;
    }

    let split = split_generic_argument_type_and_assembly(arg);
    let type_name = split.0.trim();
    if type_name.is_empty() {
        return None;
    }
    Some((type_name.to_string(), split.1.trim().to_string()))
}

fn split_generic_argument_type_and_assembly(value: &str) -> (&str, &str) {
    let mut depth = 0usize;
    for (idx, ch) in value.char_indices() {
        match ch {
            '[' => depth = depth.saturating_add(1),
            ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let assembly = value[idx + 1..]
                    .split(',')
                    .next()
                    .unwrap_or_default()
                    .trim();
                return (&value[..idx], assembly);
            }
            _ => {}
        }
    }
    (value, "")
}

fn managed_reference_type_name(assembly: &str, full_name: &str) -> String {
    let assembly = assembly.trim();
    let full_name = full_name.trim();
    if full_name.is_empty() {
        String::new()
    } else if assembly.is_empty() {
        full_name.to_string()
    } else {
        format!("{assembly} {full_name}")
    }
}

fn split_managed_reference_type_name(type_name: &str) -> (String, String) {
    let type_name = type_name.trim();
    if type_name.is_empty() {
        return (String::new(), String::new());
    }

    if let Some((assembly, full_name)) = type_name.split_once(' ') {
        (assembly.trim().to_string(), full_name.trim().to_string())
    } else {
        (String::new(), type_name.to_string())
    }
}

fn managed_reference_display_name(type_name: &str) -> String {
    let (_, full_name) = split_managed_reference_type_name(type_name);
    simple_type_name(&full_name).to_string()
}

fn matches_field_name(entry: &UnitySerializedPropertyDiscoverMatch, field_name: &str) -> bool {
    let expected = field_name.trim();
    if expected.is_empty() {
        return true;
    }

    entry.name.eq_ignore_ascii_case(expected)
        || entry.display_name.eq_ignore_ascii_case(expected)
        || serialized_property_leaf_name(&entry.property_path).eq_ignore_ascii_case(expected)
        || entry
            .property_path
            .to_ascii_lowercase()
            .ends_with(&format!(".{}", expected.to_ascii_lowercase()))
}

fn serialized_property_leaf_name(property_path: &str) -> &str {
    property_path.rsplit('.').next().unwrap_or(property_path)
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn contains_normalized(source: &str, query: &str) -> bool {
    !source.is_empty() && source.to_ascii_lowercase().contains(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_index() -> SerializedSchemaIndex {
        SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 1,
            types: vec![
                SchemaType {
                    full_name: "Game.Inventory".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "items".to_string(),
                        field_type_full_name: "System.Collections.Generic.List`1".to_string(),
                        field_type_assembly: "mscorlib".to_string(),
                        element_type_full_name: "Game.Item".to_string(),
                        element_type_assembly: "Assembly-CSharp".to_string(),
                        is_list: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                SchemaType {
                    full_name: "Game.Item".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "label".to_string(),
                        field_type_full_name: "System.String".to_string(),
                        field_type_assembly: "mscorlib".to_string(),
                        tooltip: "Shown in UI".to_string(),
                        attributes: vec![UnitySerializedPropertyAttributeInfo {
                            r#type: "UnityEngine.TooltipAttribute".to_string(),
                            display_name: "TooltipAttribute".to_string(),
                            value: "Shown in UI".to_string(),
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        })
    }

    #[test]
    fn cache_entry_freshness_expires_after_short_ttl() {
        let schema = Arc::new(test_index());
        let now = Instant::now();
        let fresh = CacheEntry {
            fingerprint: "fp".to_string(),
            schema: schema.clone(),
            verified_at: now - (SCHEMA_CURRENCY_TTL / 2),
        };
        let stale = CacheEntry {
            fingerprint: "fp".to_string(),
            schema,
            verified_at: now - SCHEMA_CURRENCY_TTL - std::time::Duration::from_millis(1),
        };

        assert!(cache_entry_is_fresh(&fresh, now));
        assert!(!cache_entry_is_fresh(&stale, now));
    }

    #[test]
    fn resolves_nested_array_element_field_schema() {
        let index = test_index();
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("Game.Inventory".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };

        let resolved = index
            .resolve_property_schema(&target, "items.Array.data[0].label")
            .expect("schema");

        assert_eq!(resolved.field_type_full_name, "System.String");
        assert_eq!(resolved.field.tooltip, "Shown in UI");
    }

    #[test]
    fn resolves_generic_element_schema_from_assembly_qualified_field_type() {
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 1,
            types: vec![
                SchemaType {
                    full_name: "Game.Inventory".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "items".to_string(),
                        field_type_full_name: "System.Collections.Generic.List`1[[Game.Item, Assembly-CSharp, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null]]".to_string(),
                        field_type_assembly: "mscorlib".to_string(),
                        is_list: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                SchemaType {
                    full_name: "Game.Item".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "label".to_string(),
                        field_type_full_name: "System.String".to_string(),
                        field_type_assembly: "mscorlib".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        });
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("Game.Inventory".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };

        let element = index
            .resolve_property_schema(&target, "items.Array.data[0]")
            .expect("element schema");
        let child = index
            .resolve_property_schema(&target, "items.Array.data[0].label")
            .expect("child schema");

        assert_eq!(element.field_type_full_name, "Game.Item");
        assert_eq!(element.field_type_assembly, "Assembly-CSharp");
        assert_eq!(child.field_type_full_name, "System.String");
    }

    #[test]
    fn array_size_paths_do_not_match_collection_field_type_filters() {
        let index = test_index();
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("Game.Inventory".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };

        assert!(index
            .resolve_property_schema(&target, "items.Array.size")
            .is_none());

        let mut result = UnitySerializedPropertyDiscoverResult {
            ok: true,
            binding_id: None,
            message: "ok".to_string(),
            target,
            matches: vec![UnitySerializedPropertyDiscoverMatch {
                property_path: "items.Array.size".to_string(),
                name: "size".to_string(),
                display_name: "Size".to_string(),
                property_type: "ArraySize".to_string(),
                value_type: "ArraySize".to_string(),
                field_type_full_name: String::new(),
                field_type_assembly: String::new(),
                display_value: String::new(),
                editable: true,
                has_children: false,
                is_array: false,
                is_managed_reference: false,
                depth: 2,
            }],
        };

        index.enrich_discover_result(
            &mut result,
            &DiscoverFilters {
                field_type: Some("System.Collections.Generic.List`1".to_string()),
                ..Default::default()
            },
        );

        assert!(result.matches.is_empty());
    }

    #[test]
    fn discover_filter_matches_field_type_by_simple_name() {
        let index = test_index();
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("Game.Inventory".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };
        let mut result = UnitySerializedPropertyDiscoverResult {
            ok: true,
            binding_id: None,
            message: "ok".to_string(),
            target,
            matches: vec![UnitySerializedPropertyDiscoverMatch {
                property_path: "items.Array.data[0].label".to_string(),
                name: "label".to_string(),
                display_name: "Label".to_string(),
                property_type: "String".to_string(),
                value_type: "String".to_string(),
                field_type_full_name: String::new(),
                field_type_assembly: String::new(),
                display_value: String::new(),
                editable: true,
                has_children: false,
                is_array: false,
                is_managed_reference: false,
                depth: 3,
            }],
        };

        index.enrich_discover_result(
            &mut result,
            &DiscoverFilters {
                field_type: Some("String".to_string()),
                ..Default::default()
            },
        );

        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].field_type_full_name, "System.String");
    }

    #[test]
    fn can_enrich_target_rejects_runtime_and_builtin_assemblies() {
        let index = test_index();

        let project_target = UnitySerializedPropertyTarget {
            kind: "component".to_string(),
            target_type_full_name: Some("Game.Inventory".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };
        assert!(index.can_enrich_target(&project_target));

        let skill_target = UnitySerializedPropertyTarget {
            kind: "component".to_string(),
            target_type_full_name: Some("Tool.Widget".to_string()),
            target_type_assembly: Some("__LocusSkillPackage_tool_hash".to_string()),
            ..Default::default()
        };
        assert!(!index.can_enrich_target(&skill_target));

        let hot_patch_target = UnitySerializedPropertyTarget {
            kind: "component".to_string(),
            target_type_full_name: Some("Patch.Widget".to_string()),
            target_type_assembly: Some("__LocusHotPatch_00000000_00000001".to_string()),
            ..Default::default()
        };
        assert!(!index.can_enrich_target(&hot_patch_target));

        let transform_target = UnitySerializedPropertyTarget {
            kind: "component".to_string(),
            target_type_full_name: Some("UnityEngine.Transform".to_string()),
            target_type_assembly: Some("UnityEngine.CoreModule".to_string()),
            ..Default::default()
        };
        assert!(!index.can_enrich_target(&transform_target));
    }

    #[test]
    fn can_enrich_target_rejects_serialize_reference_owners() {
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 1,
            types: vec![SchemaType {
                full_name: "Game.Actor".to_string(),
                assembly: "Assembly-CSharp".to_string(),
                is_serializable: true,
                fields: vec![SchemaField {
                    name: "state".to_string(),
                    field_type_full_name: "Game.State".to_string(),
                    field_type_assembly: "Assembly-CSharp".to_string(),
                    has_serialize_reference: true,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        });
        let target = UnitySerializedPropertyTarget {
            kind: "component".to_string(),
            target_type_full_name: Some("Game.Actor".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };

        assert!(!index.can_enrich_target(&target));
    }

    #[test]
    fn managed_reference_options_prepends_current_type() {
        let index = test_index();

        let options = index.managed_reference_options(
            "Assembly-CSharp Game.Item",
            "__LocusSkillPackage_tool_hash Tool.DynamicItem",
        );

        assert_eq!(options[0].full_name, "Tool.DynamicItem");
        assert_eq!(options[0].assembly, "__LocusSkillPackage_tool_hash");
        assert_eq!(
            options[0].value,
            "__LocusSkillPackage_tool_hash Tool.DynamicItem"
        );
    }
}
