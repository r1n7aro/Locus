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
    type_parameters: Vec<String>,
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
    element_type_full_name: String,
    element_type_assembly: String,
}

/// A fully-concrete type argument lifted out of a constructed generic name such
/// as `Foo`1[[System.Int32, mscorlib, Version=…]]`. `full_name`/`assembly` are
/// the bare name and simple assembly name used when a type parameter is the
/// whole field type; `aqn` is the verbatim assembly-qualified text spliced back
/// in when the parameter appears nested inside another generic argument, so the
/// rendered string matches the compiler's output byte-for-byte.
#[derive(Debug, Clone)]
struct ConcreteArg {
    full_name: String,
    assembly: String,
    aqn: String,
}

#[derive(Debug, Clone)]
struct TypeCursor {
    concrete_full_name: String,
    assembly: String,
    open_full_name: String,
    args: Vec<ConcreteArg>,
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
        if payload.schema_version > 2 {
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
        } else if snapshot.enum_options.is_empty() {
            // The declared field type may be a type parameter that substitutes to
            // an enum (e.g. ToggleProperty<SomeEnum>.data); the field carries no
            // enum metadata, but the resolved concrete type does.
            if let Some((options, is_flags)) = self.type_enum_options(
                &resolved.field_type_full_name,
                &resolved.field_type_assembly,
            ) {
                snapshot.enum_options = options;
                snapshot.is_flags_enum = is_flags;
            }
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
                let rf = self.resolve_field(&current_full_name, &current_assembly, member_name)?;
                current_full_name = rf.field_type_full_name.clone();
                current_assembly = rf.field_type_assembly.clone();
                resolved = Some(rf);
            }

            if part.contains('[') {
                let base = resolved.as_ref()?;
                // Prefer the already-substituted element type recorded on the
                // field; otherwise lift the element out of the (concrete)
                // collection name, falling back to stripping an array suffix.
                let (element_full_name, element_assembly) =
                    if !base.element_type_full_name.trim().is_empty() {
                        (
                            base.element_type_full_name.clone(),
                            base.element_type_assembly.clone(),
                        )
                    } else if let Some((_, args)) = parse_constructed_name(&current_full_name) {
                        match args.into_iter().next() {
                            Some(arg) => (arg.full_name, arg.assembly),
                            None => (String::new(), String::new()),
                        }
                    } else {
                        (
                            strip_array_suffix(&current_full_name),
                            current_assembly.clone(),
                        )
                    };
                if element_full_name.trim().is_empty() {
                    return None;
                }
                current_full_name = element_full_name;
                current_assembly = element_assembly;
                if let Some(prev) = &mut resolved {
                    prev.field_type_full_name = current_full_name.clone();
                    prev.field_type_assembly = current_assembly.clone();
                    // The element has been drilled into; any further index step
                    // must re-derive from the (new) concrete collection name.
                    prev.element_type_full_name = String::new();
                    prev.element_type_assembly = String::new();
                }
            }
        }

        resolved
    }

    /// Resolves a member on `owner_full_name`, parsing a constructed generic
    /// owner into its open definition plus arguments and substituting any type
    /// parameters in the field's declared type against those arguments.
    fn resolve_field(
        &self,
        owner_full_name: &str,
        owner_assembly: &str,
        member_name: &str,
    ) -> Option<ResolvedFieldSchema> {
        let (open_name, args) = match parse_constructed_name(owner_full_name) {
            Some((open, args)) => (open, args),
            None => (open_definition_name(owner_full_name), Vec::new()),
        };
        let (field, ctx) =
            self.find_field_substituted(&open_name, owner_assembly, &args, member_name)?;
        let (field_type_full_name, field_type_assembly) = substitute_type_ref(
            &field.field_type_full_name,
            &field.field_type_assembly,
            &ctx,
        );
        let (element_type_full_name, element_type_assembly) =
            if field.element_type_full_name.trim().is_empty() {
                (String::new(), String::new())
            } else {
                substitute_type_ref(
                    &field.element_type_full_name,
                    &field.element_type_assembly,
                    &ctx,
                )
            };
        Some(ResolvedFieldSchema {
            field,
            field_type_full_name,
            field_type_assembly,
            element_type_full_name,
            element_type_assembly,
        })
    }

    /// Walks the field's owner and its base chain, threading a substitution
    /// context (the constructed type arguments) so a base reached through a
    /// constructed generic such as `Base`1[[!0]]` is resolved with the derived
    /// type's arguments applied. Returns the matching field together with the
    /// argument context of the type that declared it.
    fn find_field_substituted(
        &self,
        open_full_name: &str,
        assembly: &str,
        args: &[ConcreteArg],
        field_name: &str,
    ) -> Option<(SchemaField, Vec<ConcreteArg>)> {
        let mut current_full_name = open_full_name.to_string();
        let mut current_assembly = assembly.to_string();
        let mut current_args = args.to_vec();
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(type_key(&current_full_name, &current_assembly)) {
                return None;
            }
            let schema_type = self.find_type(&current_full_name, &current_assembly)?;
            if let Some(field) = schema_type.fields.iter().find(|f| f.name == field_name) {
                return Some((field.clone(), current_args));
            }
            if schema_type.base_type_full_name.trim().is_empty() {
                return None;
            }
            let base_template = schema_type.base_type_full_name.clone();
            let base_assembly_stored = schema_type.base_type_assembly.clone();
            let (base_concrete, base_assembly) =
                substitute_type_ref(&base_template, &base_assembly_stored, &current_args);
            match parse_constructed_name(&base_concrete) {
                Some((open, base_args)) => {
                    current_full_name = open;
                    current_assembly = base_assembly;
                    current_args = base_args;
                }
                None => {
                    current_full_name = open_definition_name(&base_concrete);
                    current_assembly = base_assembly;
                    current_args = Vec::new();
                }
            }
        }
    }

    /// Returns enum metadata for a concrete type if the schema knows it as an
    /// enum, used to recover enum options for fields whose type parameter
    /// substituted to an enum.
    fn type_enum_options(
        &self,
        full_name: &str,
        assembly: &str,
    ) -> Option<(Vec<UnityEnumOption>, bool)> {
        let schema_type = self.find_type(full_name, assembly)?;
        if schema_type.enum_options.is_empty() {
            return None;
        }
        Some((schema_type.enum_options.clone(), schema_type.is_flags_enum))
    }

    fn find_type(&self, full_name: &str, assembly: &str) -> Option<&SchemaType> {
        if !assembly.trim().is_empty() {
            if let Some(schema_type) = self.types_by_key.get(&type_key(full_name, assembly)) {
                return Some(schema_type);
            }
        }

        if let Some(keys) = self.keys_by_full_name.get(full_name) {
            if let Some(schema_type) = keys.iter().find_map(|key| self.types_by_key.get(key)) {
                return Some(schema_type);
            }
        }

        // Constructed generic reference: fall back to the open generic
        // definition (e.g. `Foo`1[[System.Int32, …]]` -> `Foo`1`).
        // The schema stores generic types under their open-definition name; the
        // caller supplies a constructed argument context for substitution.
        let open = open_definition_name(full_name);
        if open != full_name {
            if !assembly.trim().is_empty() {
                if let Some(schema_type) = self.types_by_key.get(&type_key(&open, assembly)) {
                    return Some(schema_type);
                }
            }
            if let Some(keys) = self.keys_by_full_name.get(&open) {
                return keys.iter().find_map(|key| self.types_by_key.get(key));
            }
        }
        None
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

        let mut current = TypeCursor::new(full_name, assembly);
        let mut seen = HashSet::new();
        loop {
            let key = type_key(&current.concrete_full_name, &current.assembly);
            if !seen.insert(key) {
                break;
            }
            let Some(schema_type) = self.find_type(&current.open_full_name, &current.assembly)
            else {
                break;
            };
            if name_matches(&current.concrete_full_name, expected)
                || name_matches(&schema_type.full_name, expected)
            {
                return true;
            }
            for iface in &schema_type.interfaces {
                let (iface_full_name, _) =
                    substitute_type_ref(&iface.full_name, &iface.assembly, &current.args);
                if name_matches(&iface_full_name, expected) {
                    return true;
                }
            }
            let Some(next) = current.substituted_base(schema_type) else {
                break;
            };
            current = next;
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

        let mut current = TypeCursor::new(full_name, assembly);
        let mut seen = HashSet::new();
        loop {
            let key = type_key(&current.concrete_full_name, &current.assembly);
            if !seen.insert(key) {
                return false;
            }
            if same_type(
                &current.concrete_full_name,
                &current.assembly,
                base_full_name,
                base_assembly,
            ) {
                return true;
            }

            let Some(schema_type) = self.find_type(&current.open_full_name, &current.assembly)
            else {
                return false;
            };
            for iface in &schema_type.interfaces {
                let (iface_full_name, iface_assembly) =
                    substitute_type_ref(&iface.full_name, &iface.assembly, &current.args);
                if same_type(
                    &iface_full_name,
                    &iface_assembly,
                    base_full_name,
                    base_assembly,
                ) {
                    return true;
                }
            }

            let Some(next) = current.substituted_base(schema_type) else {
                return false;
            };
            current = next;
        }
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
    let type_name = full_name.split("[[").next().unwrap_or(full_name);
    let after_nested = type_name.rsplit('+').next().unwrap_or(type_name);
    after_nested.rsplit('.').next().unwrap_or(after_nested)
}

fn strip_array_suffix(value: &str) -> String {
    value.strip_suffix("[]").unwrap_or(value).to_string()
}

fn is_serialized_array_size_path(property_path: &str) -> bool {
    let path = property_path.trim();
    path == "Array.size" || path.ends_with(".Array.size")
}

impl TypeCursor {
    fn new(full_name: &str, assembly: &str) -> Self {
        match parse_constructed_name(full_name) {
            Some((open_full_name, args)) => Self {
                concrete_full_name: full_name.trim().to_string(),
                assembly: assembly.trim().to_string(),
                open_full_name,
                args,
            },
            None => {
                let full_name = full_name.trim().to_string();
                Self {
                    concrete_full_name: full_name.clone(),
                    assembly: assembly.trim().to_string(),
                    open_full_name: full_name,
                    args: Vec::new(),
                }
            }
        }
    }

    fn substituted_base(&self, schema_type: &SchemaType) -> Option<Self> {
        if schema_type.base_type_full_name.trim().is_empty() {
            return None;
        }
        let (base_full_name, base_assembly) = substitute_type_ref(
            &schema_type.base_type_full_name,
            &schema_type.base_type_assembly,
            &self.args,
        );
        if base_full_name.trim().is_empty() {
            None
        } else {
            Some(Self::new(&base_full_name, &base_assembly))
        }
    }
}

/// Strips a constructed generic instantiation down to the open generic
/// definition name: `Foo`1[[System.Int32, …]]` -> `Foo`1`. Non-generic names,
/// including arrays, are returned unchanged.
fn open_definition_name(full_name: &str) -> String {
    let trimmed = full_name.trim();
    match parse_constructed_name(trimmed) {
        Some((open, _)) => open,
        None => trimmed.to_string(),
    }
}

/// Parses a constructed generic name into its open definition and the concrete
/// type arguments, e.g.
/// `Dictionary`2[[System.String, mscorlib, …],[Game.Handle, Asm, …]]`
/// -> ("System.Collections.Generic.Dictionary`2", [String, Game.Handle]).
/// Returns `None` when the name is not a constructed generic.
fn parse_constructed_name(full_name: &str) -> Option<(String, Vec<ConcreteArg>)> {
    let trimmed = full_name.trim();
    let open_end = trimmed.find("[[")?;
    let open = trimmed[..open_end].trim().to_string();
    if open.is_empty() {
        return None;
    }
    // Remainder is `[[arg0],[arg1],…]`; strip the single outer bracket pair.
    let remainder = &trimmed[open_end..];
    if !remainder.starts_with('[') || !remainder.ends_with(']') || remainder.len() < 2 {
        return None;
    }
    let inner = &remainder[1..remainder.len() - 1];
    let groups = split_top_level_groups(inner)?;
    let args = groups
        .iter()
        .map(|group| parse_concrete_arg(group))
        .collect();
    Some((open, args))
}

/// Splits `[a],[b],[c]` into the inner contents `[a, b, c]`, respecting nested
/// brackets so an argument that is itself a constructed generic stays intact.
fn split_top_level_groups(inner: &str) -> Option<Vec<String>> {
    let mut groups = Vec::new();
    let mut depth = 0usize;
    let mut start: Option<usize> = None;
    for (idx, ch) in inner.char_indices() {
        match ch {
            '[' => {
                if depth == 0 {
                    start = Some(idx + 1);
                }
                depth += 1;
            }
            ']' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    let begin = start.take()?;
                    groups.push(inner[begin..idx].to_string());
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    Some(groups)
}

fn parse_concrete_arg(content: &str) -> ConcreteArg {
    let content = content.trim();
    let (full_name, assembly) = split_generic_argument_type_and_assembly(content);
    ConcreteArg {
        full_name: full_name.trim().to_string(),
        assembly: assembly.trim().to_string(),
        aqn: content.to_string(),
    }
}

/// `!0` -> `Some(0)`; anything else -> `None`.
fn parse_param_token(value: &str) -> Option<usize> {
    let digits = value.strip_prefix('!')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Substitutes type-parameter placeholders in a stored type reference against a
/// constructed type's arguments. A bare `!n` (optionally `!n[]`) takes both its
/// name and assembly from the argument; a `!n` nested inside another generic is
/// spliced back assembly-qualified so the rendered name matches the compiler's
/// output. References without placeholders are returned unchanged.
fn substitute_type_ref(
    stored_full: &str,
    stored_assembly: &str,
    ctx: &[ConcreteArg],
) -> (String, String) {
    let trimmed = stored_full.trim();
    if trimmed.is_empty() {
        return (String::new(), stored_assembly.to_string());
    }
    let (core, array_suffix) = match trimmed.strip_suffix("[]") {
        Some(core) => (core, "[]"),
        None => (trimmed, ""),
    };

    if let Some(ordinal) = parse_param_token(core) {
        if let Some(arg) = ctx.get(ordinal) {
            return (
                format!("{}{}", arg.full_name, array_suffix),
                arg.assembly.clone(),
            );
        }
        return (format!("{core}{array_suffix}"), stored_assembly.to_string());
    }

    if !core.contains('!') {
        return (format!("{core}{array_suffix}"), stored_assembly.to_string());
    }

    let rendered = render_type_template(core, ctx);
    (
        format!("{rendered}{array_suffix}"),
        stored_assembly.to_string(),
    )
}

/// Replaces every `[!n]` argument group in a stored generic template with the
/// assembly-qualified text of `ctx[n]`, leaving concrete arguments untouched.
/// Type parameters only ever appear as a complete argument group, so this purely
/// textual replacement is exact and handles arbitrary nesting.
fn render_type_template(core: &str, ctx: &[ConcreteArg]) -> String {
    let mut result = String::with_capacity(core.len() + 32);
    let mut rest = core;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("[!") {
            let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            let tail = &after[digits.len()..];
            if !digits.is_empty() && tail.starts_with(']') {
                match digits.parse::<usize>().ok().and_then(|ord| ctx.get(ord)) {
                    Some(arg) => result.push_str(&format!("[{}]", arg.aqn)),
                    None => result.push_str(&format!("[!{digits}]")),
                }
                rest = &tail[1..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        result.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    result
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

    #[test]
    fn resolves_constructed_generic_field_child() {
        // ToggleProperty<int>.data : T -> System.Int32 and .enable : bool.
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 2,
            types: vec![
                SchemaType {
                    full_name: "ProjectB.EntityAction".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "minFps".to_string(),
                        field_type_full_name: "ProjectB.ToggleProperty`1[[System.Int32, mscorlib, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089]]".to_string(),
                        field_type_assembly: "Assembly-CSharp".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                SchemaType {
                    full_name: "ProjectB.ToggleProperty`1".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    type_parameters: vec!["T".to_string()],
                    fields: vec![
                        SchemaField {
                            name: "enable".to_string(),
                            field_type_full_name: "System.Boolean".to_string(),
                            field_type_assembly: "mscorlib".to_string(),
                            ..Default::default()
                        },
                        SchemaField {
                            name: "data".to_string(),
                            field_type_full_name: "!0".to_string(),
                            // Deliberately wrong; a bare parameter must take both
                            // its name and assembly from the constructed argument.
                            field_type_assembly: "Assembly-CSharp".to_string(),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
            ],
        });
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("ProjectB.EntityAction".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };

        let data = index
            .resolve_property_schema(&target, "minFps.data")
            .expect("data schema");
        assert_eq!(data.field_type_full_name, "System.Int32");
        assert_eq!(data.field_type_assembly, "mscorlib");

        let enable = index
            .resolve_property_schema(&target, "minFps.enable")
            .expect("enable schema");
        assert_eq!(enable.field_type_full_name, "System.Boolean");
        assert_eq!(enable.field_type_assembly, "mscorlib");
    }

    #[test]
    fn rejects_serialize_reference_inherited_through_constructed_generic_base() {
        // TransitionAsset : TransitionAsset<ITransitionDetailed>, where the base
        // declares a [SerializeReference] field. The owner must still be excluded
        // from static enrichment even though the SerializeReference is inherited
        // through a constructed generic base.
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 2,
            types: vec![
                SchemaType {
                    full_name: "Animancer.TransitionAsset".to_string(),
                    assembly: "Kybernetik.Animancer".to_string(),
                    base_type_full_name: "Animancer.TransitionAsset`1[[Animancer.ITransitionDetailed, Kybernetik.Animancer, Version=8.0.0.0, Culture=neutral, PublicKeyToken=null]]".to_string(),
                    base_type_assembly: "Kybernetik.Animancer".to_string(),
                    is_unity_object: true,
                    ..Default::default()
                },
                SchemaType {
                    full_name: "Animancer.TransitionAsset`1".to_string(),
                    assembly: "Kybernetik.Animancer".to_string(),
                    type_parameters: vec!["TTransition".to_string()],
                    is_unity_object: true,
                    fields: vec![SchemaField {
                        name: "_Transition".to_string(),
                        field_type_full_name: "!0".to_string(),
                        field_type_assembly: "Kybernetik.Animancer".to_string(),
                        has_serialize_reference: true,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        });
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("Animancer.TransitionAsset".to_string()),
            target_type_assembly: Some("Kybernetik.Animancer".to_string()),
            ..Default::default()
        };

        assert!(!index.can_enrich_target(&target));

        // And were it ever resolved, the inherited managed-reference field type
        // substitutes from the constructed base argument.
        let resolved = index
            .resolve_property_schema(&target, "_Transition")
            .expect("inherited field schema");
        assert_eq!(
            resolved.field_type_full_name,
            "Animancer.ITransitionDetailed"
        );
        assert_eq!(resolved.field_type_assembly, "Kybernetik.Animancer");
    }

    #[test]
    fn resolves_nested_generic_field_and_array_element() {
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 2,
            types: vec![
                SchemaType {
                    full_name: "Game.Owner".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "holder".to_string(),
                        field_type_full_name: "Game.Holder`1[[Game.Item, Assembly-CSharp, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null]]".to_string(),
                        field_type_assembly: "Assembly-CSharp".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                SchemaType {
                    full_name: "Game.Holder`1".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    type_parameters: vec!["T".to_string()],
                    fields: vec![SchemaField {
                        name: "items".to_string(),
                        field_type_full_name: "System.Collections.Generic.List`1[[!0]]".to_string(),
                        field_type_assembly: "mscorlib".to_string(),
                        element_type_full_name: "!0".to_string(),
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
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            ],
        });
        let target = UnitySerializedPropertyTarget {
            kind: "asset".to_string(),
            target_type_full_name: Some("Game.Owner".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };

        // The nested List<T> renders with the substituted, assembly-qualified arg
        // spliced in verbatim.
        let items = index
            .resolve_property_schema(&target, "holder.items")
            .expect("items schema");
        assert_eq!(
            items.field_type_full_name,
            "System.Collections.Generic.List`1[[Game.Item, Assembly-CSharp, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null]]"
        );

        let element = index
            .resolve_property_schema(&target, "holder.items.Array.data[0]")
            .expect("element schema");
        assert_eq!(element.field_type_full_name, "Game.Item");
        assert_eq!(element.field_type_assembly, "Assembly-CSharp");

        let label = index
            .resolve_property_schema(&target, "holder.items.Array.data[0].label")
            .expect("label schema");
        assert_eq!(label.field_type_full_name, "System.String");
    }

    #[test]
    fn discover_field_type_filter_does_not_match_array_parent_as_element_type() {
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 2,
            types: vec![
                SchemaType {
                    full_name: "Game.Owner".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    fields: vec![SchemaField {
                        name: "items".to_string(),
                        field_type_full_name: "Game.Item[]".to_string(),
                        field_type_assembly: "Assembly-CSharp".to_string(),
                        element_type_full_name: "Game.Item".to_string(),
                        element_type_assembly: "Assembly-CSharp".to_string(),
                        is_array: true,
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
            target_type_full_name: Some("Game.Owner".to_string()),
            target_type_assembly: Some("Assembly-CSharp".to_string()),
            ..Default::default()
        };
        let mut result = UnitySerializedPropertyDiscoverResult {
            ok: true,
            binding_id: None,
            message: "ok".to_string(),
            target,
            matches: vec![
                UnitySerializedPropertyDiscoverMatch {
                    property_path: "items".to_string(),
                    name: "items".to_string(),
                    display_name: "Items".to_string(),
                    property_type: "Array".to_string(),
                    value_type: "Array".to_string(),
                    field_type_full_name: String::new(),
                    field_type_assembly: String::new(),
                    display_value: String::new(),
                    editable: true,
                    has_children: true,
                    is_array: true,
                    is_managed_reference: false,
                    depth: 1,
                },
                UnitySerializedPropertyDiscoverMatch {
                    property_path: "items.Array.data[0]".to_string(),
                    name: "data".to_string(),
                    display_name: "Element 0".to_string(),
                    property_type: "Generic".to_string(),
                    value_type: "Generic".to_string(),
                    field_type_full_name: String::new(),
                    field_type_assembly: String::new(),
                    display_value: String::new(),
                    editable: true,
                    has_children: true,
                    is_array: false,
                    is_managed_reference: false,
                    depth: 2,
                },
            ],
        };

        index.enrich_discover_result(
            &mut result,
            &DiscoverFilters {
                field_type: Some("Game.Item".to_string()),
                ..Default::default()
            },
        );

        let paths = result
            .matches
            .iter()
            .map(|entry| entry.property_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["items.Array.data[0]"]);
    }

    #[test]
    fn type_matching_substitutes_generic_base_and_interface_arguments() {
        let int_arg = "System.Int32, mscorlib, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089";
        let string_arg = "System.String, mscorlib, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089";
        let base_int = format!("Game.Base`1[[{int_arg}]]");
        let base_string = format!("Game.Base`1[[{string_arg}]]");
        let iface_int = format!("Game.IBox`1[[{int_arg}]]");
        let iface_string = format!("Game.IBox`1[[{string_arg}]]");
        let middle_int = format!("Game.Middle`1[[{int_arg}]]");
        let index = SerializedSchemaIndex::new(SerializedSchemaPayload {
            schema_version: 2,
            types: vec![
                SchemaType {
                    full_name: "Game.Concrete".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    base_type_full_name: middle_int,
                    base_type_assembly: "Assembly-CSharp".to_string(),
                    is_serializable: true,
                    ..Default::default()
                },
                SchemaType {
                    full_name: "Game.Middle`1".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    base_type_full_name: "Game.Base`1[[!0]]".to_string(),
                    base_type_assembly: "Assembly-CSharp".to_string(),
                    interfaces: vec![SchemaTypeRef {
                        full_name: "Game.IBox`1[[!0]]".to_string(),
                        assembly: "Assembly-CSharp".to_string(),
                    }],
                    type_parameters: vec!["T".to_string()],
                    is_serializable: true,
                    ..Default::default()
                },
                SchemaType {
                    full_name: "Game.Base`1".to_string(),
                    assembly: "Assembly-CSharp".to_string(),
                    type_parameters: vec!["T".to_string()],
                    is_serializable: true,
                    ..Default::default()
                },
            ],
        });

        assert!(index.type_matches("Game.Concrete", "Assembly-CSharp", &base_int));
        assert!(!index.type_matches("Game.Concrete", "Assembly-CSharp", &base_string));
        assert!(index.type_matches("Game.Concrete", "Assembly-CSharp", &iface_int));
        assert!(!index.type_matches("Game.Concrete", "Assembly-CSharp", &iface_string));
        assert!(index.type_is_assignable_to(
            "Game.Concrete",
            "Assembly-CSharp",
            &iface_int,
            "Assembly-CSharp"
        ));
        assert!(!index.type_is_assignable_to(
            "Game.Concrete",
            "Assembly-CSharp",
            &iface_string,
            "Assembly-CSharp"
        ));
    }

    #[test]
    fn parses_assembly_qualified_constructed_name() {
        let (open, args) = parse_constructed_name(
            "Game.DictionaryList`2[[System.String, mscorlib, Version=4.0.0.0, Culture=neutral, PublicKeyToken=b77a5c561934e089],[Game.Handle, Assembly-CSharp, Version=0.0.0.0, Culture=neutral, PublicKeyToken=null]]",
        )
        .expect("constructed name");
        assert_eq!(open, "Game.DictionaryList`2");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].full_name, "System.String");
        assert_eq!(args[0].assembly, "mscorlib");
        assert_eq!(args[1].full_name, "Game.Handle");
        assert_eq!(args[1].assembly, "Assembly-CSharp");

        // A nested generic argument is kept intact as a single argument.
        let (open_nested, nested_args) = parse_constructed_name(
            "System.Collections.Generic.List`1[[System.Collections.Generic.List`1[[System.Int32, mscorlib]], mscorlib]]",
        )
        .expect("nested constructed name");
        assert_eq!(open_nested, "System.Collections.Generic.List`1");
        assert_eq!(nested_args.len(), 1);
        assert_eq!(
            nested_args[0].full_name,
            "System.Collections.Generic.List`1[[System.Int32, mscorlib]]"
        );
        assert_eq!(nested_args[0].assembly, "mscorlib");

        assert_eq!(
            open_definition_name("Game.Holder`1[[Game.Item, Asm]]"),
            "Game.Holder`1"
        );
        assert_eq!(open_definition_name("Game.Holder`1[]"), "Game.Holder`1[]");
        assert_eq!(open_definition_name("System.Int32"), "System.Int32");
        assert!(parse_constructed_name("System.Int32").is_none());
    }
}
