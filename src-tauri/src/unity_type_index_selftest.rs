use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::view::{
    UnitySerializedPropertyDiscoverResult, UnitySerializedPropertyReadResult,
    UnitySerializedPropertySnapshot, UnitySerializedPropertyTarget,
};

const SAMPLE_TARGETS: usize = 32;
const MAX_DISCOVER_TYPES_PER_TARGET: usize = 6;
const MAX_DIFFS: usize = 40;
const READ_MAX_DEPTH: i32 = 8;
const READ_MAX_ARRAY_ITEMS: i32 = 16;
const DISCOVER_MAX_RESULTS: i32 = 500;
const DISCOVER_INCLUDE_ALL_MAX_RESULTS: i32 = 50_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeIndexSelfTestSummary {
    pub passed: u32,
    pub failed: u32,
    pub checked_targets: u32,
    pub checked_properties: u32,
    pub checked_discover_filters: u32,
    pub skipped_targets: u32,
    pub lines: Vec<String>,
    pub diffs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TypeIndexSampleMode {
    Sample32,
    All,
}

impl TypeIndexSampleMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "sample" | "sample32" | "sample-32" | "32" => Ok(Self::Sample32),
            "all" | "full" | "exhaustive" => Ok(Self::All),
            other => Err(format!(
                "Unknown type-index sample mode '{other}'. Use sample32 or all."
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sample32 => "sample32",
            Self::All => "all",
        }
    }

    fn max_targets(self) -> usize {
        match self {
            Self::Sample32 => SAMPLE_TARGETS,
            Self::All => usize::MAX,
        }
    }

    fn unity_target_limit(self) -> usize {
        match self {
            Self::Sample32 => SAMPLE_TARGETS,
            Self::All => 0,
        }
    }
}

impl Default for TypeIndexSampleMode {
    fn default() -> Self {
        Self::Sample32
    }
}

impl TypeIndexSelfTestSummary {
    fn new() -> Self {
        Self {
            passed: 0,
            failed: 0,
            checked_targets: 0,
            checked_properties: 0,
            checked_discover_filters: 0,
            skipped_targets: 0,
            lines: Vec::new(),
            diffs: Vec::new(),
        }
    }

    fn pass(&mut self) {
        self.passed = self.passed.saturating_add(1);
    }

    fn diff(&mut self, label: impl Into<String>) {
        self.failed = self.failed.saturating_add(1);
        if self.diffs.len() < MAX_DIFFS {
            self.diffs.push(label.into());
        }
    }
}

/// Progress snapshot emitted while the self-test walks its sampled targets.
/// `percent` is the share of targets processed so far (0-100).
#[derive(Debug, Clone, Copy)]
pub struct TypeIndexProgress {
    pub processed_targets: u32,
    pub total_targets: u32,
    pub percent: u32,
    pub checked_properties: u32,
}

/// Returns the new whole-percent milestone when finishing `processed` of
/// `total` targets crosses past `last_percent`, otherwise `None`. With fewer
/// than 100 targets each one advances at least a full percent, so every target
/// reports; with larger totals this throttles progress to ~1% steps.
fn next_progress_percent(processed: u32, total: u32, last_percent: u32) -> Option<u32> {
    if total == 0 {
        return None;
    }
    let percent = (u64::from(processed) * 100 / u64::from(total)) as u32;
    (percent > last_percent).then_some(percent)
}

pub async fn run(
    project_path: &str,
    sample_mode: TypeIndexSampleMode,
    on_progress: &mut (dyn FnMut(TypeIndexProgress) + Send),
) -> Result<TypeIndexSelfTestSummary, String> {
    let schema = crate::unity_serialized_schema::load_current_schema(project_path).await?;
    let targets = discover_candidate_targets(project_path, sample_mode).await?;
    let mut summary = TypeIndexSelfTestSummary::new();

    if targets.is_empty() {
        summary.diff("no custom ScriptableObject or prefab component targets found");
        summary.lines.push(format!(
            "sample mode: {}, candidate targets: 0",
            sample_mode.as_str()
        ));
        return Ok(summary);
    }

    let candidate_count = targets.len();
    let targets: Vec<_> = targets
        .into_iter()
        .take(sample_mode.max_targets())
        .collect();
    let total_targets = targets.len() as u32;
    let mut processed_targets: u32 = 0;
    let mut last_percent: u32 = 0;
    for target in targets {
        processed_targets += 1;
        let label = target_label(&target);
        if crate::unity_serialized_schema::target_is_static_schema_excluded(&target)
            || !schema.can_enrich_target(&target)
        {
            summary.skipped_targets = summary.skipped_targets.saturating_add(1);
        } else {
            summary.checked_targets = summary.checked_targets.saturating_add(1);
            let full = read_target(project_path, &target, "full").await?;
            let mut dynamic = read_target(project_path, &target, "dynamic").await?;
            schema.enrich_read_result(&mut dynamic);
            compare_read_result(&label, &full, &dynamic, &mut summary);

            let field_types = discover_field_type_samples(&full);
            for field_type in field_types.into_iter().take(MAX_DISCOVER_TYPES_PER_TARGET) {
                let full_discover =
                    discover_target(project_path, &target, "full", false, &field_type).await?;
                let mut dynamic_discover =
                    discover_target(project_path, &target, "dynamic", true, "").await?;
                schema.enrich_discover_result(
                    &mut dynamic_discover,
                    &crate::unity_serialized_schema::DiscoverFilters {
                        field_type: Some(field_type.clone()),
                        max_results: Some(DISCOVER_MAX_RESULTS),
                        ..Default::default()
                    },
                );
                compare_discover_result(
                    &format!("{label} fieldType={field_type}"),
                    &full_discover,
                    &dynamic_discover,
                    &mut summary,
                );
            }
        }

        // Surface progress on every ~1% of targets processed; when the total is
        // under 100 each target advances at least 1% so every one is reported.
        if let Some(percent) = next_progress_percent(processed_targets, total_targets, last_percent)
        {
            last_percent = percent;
            on_progress(TypeIndexProgress {
                processed_targets,
                total_targets,
                percent,
                checked_properties: summary.checked_properties,
            });
        }
    }

    if summary.checked_targets == 0 {
        summary.diff("no target was eligible for static schema enrichment");
    }
    if summary.failed > 0 {
        summary.lines.push(format!(
            "sample mode: {}, candidate targets: {}, checked targets: {}, properties: {}, discover filters: {}, skipped: {}",
            sample_mode.as_str(),
            candidate_count,
            summary.checked_targets,
            summary.checked_properties,
            summary.checked_discover_filters,
            summary.skipped_targets
        ));
    }
    Ok(summary)
}

async fn discover_candidate_targets(
    project_path: &str,
    sample_mode: TypeIndexSampleMode,
) -> Result<Vec<UnitySerializedPropertyTarget>, String> {
    let snippet = DISCOVER_TARGETS_SNIPPET.replace(
        "__LOCUS_TYPE_INDEX_TARGET_LIMIT__",
        &sample_mode.unity_target_limit().to_string(),
    );
    let output = crate::unity_bridge::unity_execute_code(project_path, &snippet)
        .await
        .map_err(|error| format!("type-index target discovery failed: {error}"))?;
    let json_text = extract_json_array(&output)
        .ok_or_else(|| format!("type-index target discovery returned no JSON array: {output}"))?;
    serde_json::from_str(json_text)
        .map_err(|error| format!("type-index target discovery parse failed: {error}: {json_text}"))
}

async fn read_target(
    project_path: &str,
    target: &UnitySerializedPropertyTarget,
    schema_mode: &str,
) -> Result<UnitySerializedPropertyReadResult, String> {
    let payload = serde_json::json!({
        "target": target,
        "maxDepth": READ_MAX_DEPTH,
        "maxArrayItems": READ_MAX_ARRAY_ITEMS,
        "schemaMode": schema_mode,
    });
    let raw = crate::unity_bridge::view_binding_read(project_path, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("{schema_mode} read response parse failed: {error}"))
}

async fn discover_target(
    project_path: &str,
    target: &UnitySerializedPropertyTarget,
    schema_mode: &str,
    include_all: bool,
    field_type: &str,
) -> Result<UnitySerializedPropertyDiscoverResult, String> {
    let payload = serde_json::json!({
        "target": target,
        "query": "",
        "fieldName": "",
        "fieldType": field_type,
        "maxDepth": READ_MAX_DEPTH,
        "maxResults": if include_all { DISCOVER_INCLUDE_ALL_MAX_RESULTS } else { DISCOVER_MAX_RESULTS },
        "includeAll": include_all,
        "schemaMode": schema_mode,
    });
    let raw = crate::unity_bridge::view_binding_discover(project_path, &payload).await?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("{schema_mode} discover response parse failed: {error}"))
}

fn compare_read_result(
    label: &str,
    full: &UnitySerializedPropertyReadResult,
    dynamic: &UnitySerializedPropertyReadResult,
    summary: &mut TypeIndexSelfTestSummary,
) {
    let full_map = flatten_property_map(&full.property);
    let dynamic_map = flatten_property_map(&dynamic.property);
    for (path, full_node) in full_map {
        if path.is_empty() {
            continue;
        }
        summary.checked_properties = summary.checked_properties.saturating_add(1);
        let Some(dynamic_node) = dynamic_map.get(&path) else {
            summary.diff(format!("{label} {path}: dynamic snapshot missing property"));
            continue;
        };
        compare_snapshot_node(label, &path, full_node, dynamic_node, summary);
    }
}

fn compare_snapshot_node(
    label: &str,
    path: &str,
    full: &UnitySerializedPropertySnapshot,
    dynamic: &UnitySerializedPropertySnapshot,
    summary: &mut TypeIndexSelfTestSummary,
) {
    compare_nonempty(
        summary,
        label,
        path,
        "fieldTypeFullName",
        &full.field_type_full_name,
        &dynamic.field_type_full_name,
    );
    compare_nonempty(
        summary,
        label,
        path,
        "fieldTypeAssembly",
        &full.field_type_assembly,
        &dynamic.field_type_assembly,
    );
    compare_nonempty(
        summary,
        label,
        path,
        "tooltip",
        &full.tooltip,
        &dynamic.tooltip,
    );
    compare_nonempty(
        summary,
        label,
        path,
        "header",
        &full.header,
        &dynamic.header,
    );
    compare_nonempty(
        summary,
        label,
        path,
        "referenceTypeFullName",
        &full.reference_type_full_name,
        &dynamic.reference_type_full_name,
    );
    compare_nonempty(
        summary,
        label,
        path,
        "referenceTypeAssembly",
        &full.reference_type_assembly,
        &dynamic.reference_type_assembly,
    );
    compare_nonempty(
        summary,
        label,
        path,
        "managedReferenceFieldTypename",
        &full.managed_reference_field_typename,
        &dynamic.managed_reference_field_typename,
    );
    if full.is_flags_enum != dynamic.is_flags_enum {
        summary.diff(format!(
            "{label} {path}: isFlagsEnum full={} dynamic={}",
            full.is_flags_enum, dynamic.is_flags_enum
        ));
    }
    if !full.enum_options.is_empty() {
        let full_options = full
            .enum_options
            .iter()
            .map(|option| format!("{}:{}", option.name, option.numeric_value))
            .collect::<Vec<_>>();
        let dynamic_options = dynamic
            .enum_options
            .iter()
            .map(|option| format!("{}:{}", option.name, option.numeric_value))
            .collect::<Vec<_>>();
        if full_options != dynamic_options {
            summary.diff(format!(
                "{label} {path}: enumOptions full={} dynamic={}",
                full_options.len(),
                dynamic_options.len()
            ));
        }
    }
    if !full.managed_reference_types.is_empty() {
        let full_options = full
            .managed_reference_types
            .iter()
            .map(|option| format!("{}|{}", option.assembly, option.full_name))
            .collect::<Vec<_>>();
        let dynamic_options = dynamic
            .managed_reference_types
            .iter()
            .map(|option| format!("{}|{}", option.assembly, option.full_name))
            .collect::<Vec<_>>();
        if full_options != dynamic_options {
            summary.diff(format!(
                "{label} {path}: managedReferenceTypes full={} dynamic={}",
                full_options.len(),
                dynamic_options.len()
            ));
        }
    }
}

fn compare_nonempty(
    summary: &mut TypeIndexSelfTestSummary,
    label: &str,
    path: &str,
    field: &str,
    full: &str,
    dynamic: &str,
) {
    if full.trim().is_empty() {
        return;
    }
    if full != dynamic {
        summary.diff(format!(
            "{label} {path}: {field} full='{full}' dynamic='{dynamic}'"
        ));
    }
}

fn compare_discover_result(
    label: &str,
    full: &UnitySerializedPropertyDiscoverResult,
    dynamic: &UnitySerializedPropertyDiscoverResult,
    summary: &mut TypeIndexSelfTestSummary,
) {
    summary.checked_discover_filters = summary.checked_discover_filters.saturating_add(1);
    let full_paths = full
        .matches
        .iter()
        .map(|entry| entry.property_path.clone())
        .collect::<BTreeSet<_>>();
    let dynamic_paths = dynamic
        .matches
        .iter()
        .map(|entry| entry.property_path.clone())
        .collect::<BTreeSet<_>>();
    if full_paths == dynamic_paths {
        summary.pass();
        return;
    }

    let missing = full_paths
        .difference(&dynamic_paths)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    let extra = dynamic_paths
        .difference(&full_paths)
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    summary.diff(format!(
        "{label}: discover paths differ; missing={:?}; extra={:?}",
        missing, extra
    ));
}

fn flatten_property_map(
    root: &UnitySerializedPropertySnapshot,
) -> BTreeMap<String, &UnitySerializedPropertySnapshot> {
    let mut map = BTreeMap::new();
    push_snapshot(root, &mut map);
    map
}

fn push_snapshot<'a>(
    node: &'a UnitySerializedPropertySnapshot,
    map: &mut BTreeMap<String, &'a UnitySerializedPropertySnapshot>,
) {
    map.insert(node.property_path.clone(), node);
    for child in &node.children {
        push_snapshot(child, map);
    }
}

fn discover_field_type_samples(result: &UnitySerializedPropertyReadResult) -> Vec<String> {
    let mut map = BTreeMap::<String, usize>::new();
    for node in flatten_property_map(&result.property).into_values() {
        let field_type = node.field_type_full_name.trim();
        if field_type.is_empty() || node.property_path.trim().is_empty() {
            continue;
        }
        *map.entry(field_type.to_string()).or_default() += 1;
    }

    let mut entries = map.into_iter().collect::<Vec<_>>();
    entries.sort_by(|(left_name, left_count), (right_name, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_name.cmp(right_name))
    });
    entries
        .into_iter()
        .map(|(field_type, _)| field_type)
        .collect()
}

fn extract_json_array(output: &str) -> Option<&str> {
    let start = output.find('[')?;
    let end = output.rfind(']')?;
    (start <= end).then_some(&output[start..=end])
}

fn target_label(target: &UnitySerializedPropertyTarget) -> String {
    let path = target.path.as_deref().unwrap_or("<selection>");
    let type_name = target
        .target_type_full_name
        .as_deref()
        .or(target.component_type.as_deref())
        .unwrap_or("<unknown>");
    match target.kind.as_str() {
        "component" => format!(
            "{} {}[{}]",
            path,
            type_name,
            target.component_index.unwrap_or(0)
        ),
        _ => format!("{path} {type_name}"),
    }
}

const DISCOVER_TARGETS_SNIPPET: &str = r#"
string Esc(string value)
{
    if (value == null) return "";
    return value
        .Replace("\\", "\\\\")
        .Replace("\"", "\\\"")
        .Replace("\r", "\\r")
        .Replace("\n", "\\n");
}

bool IsProjectType(System.Type type)
{
    if (type == null || string.IsNullOrWhiteSpace(type.FullName)) return false;
    string assembly = type.Assembly != null ? type.Assembly.GetName().Name ?? "" : "";
    if (assembly.StartsWith("UnityEngine") || assembly.StartsWith("UnityEditor")) return false;
    if (assembly.StartsWith("System") || assembly.StartsWith("Microsoft") || assembly.StartsWith("Mono")) return false;
    if (assembly.StartsWith("__Locus")) return false;
    return true;
}

var entries = new System.Collections.Generic.List<string>();
int targetLimit = __LOCUS_TYPE_INDEX_TARGET_LIMIT__;
bool HasCapacity()
{
    return targetLimit <= 0 || entries.Count < targetLimit;
}

void AddTarget(string kind, string path, string componentType, int componentIndex, System.Type type)
{
    if (!HasCapacity()) return;
    if (!IsProjectType(type)) return;
    string assembly = type.Assembly != null ? type.Assembly.GetName().Name ?? "" : "";
    entries.Add("{"
        + "\"kind\":\"" + Esc(kind) + "\","
        + "\"path\":\"" + Esc(path) + "\","
        + "\"componentType\":\"" + Esc(componentType) + "\","
        + "\"componentIndex\":" + componentIndex.ToString(System.Globalization.CultureInfo.InvariantCulture) + ","
        + "\"targetTypeFullName\":\"" + Esc(type.FullName ?? type.Name ?? "") + "\","
        + "\"targetTypeAssembly\":\"" + Esc(assembly) + "\","
        + "\"targetTypeName\":\"" + Esc(type.Name ?? "") + "\""
        + "}");
}

foreach (string guid in AssetDatabase.FindAssets("t:ScriptableObject", new[] { "Assets" }))
{
    if (!HasCapacity()) break;
    string path = AssetDatabase.GUIDToAssetPath(guid);
    var obj = AssetDatabase.LoadMainAssetAtPath(path);
    if (obj == null) continue;
    AddTarget("asset", path, "", 0, obj.GetType());
}

foreach (string guid in AssetDatabase.FindAssets("t:Prefab", new[] { "Assets" }))
{
    if (!HasCapacity()) break;
    string path = AssetDatabase.GUIDToAssetPath(guid);
    var root = AssetDatabase.LoadAssetAtPath<GameObject>(path);
    if (root == null) continue;
    var seen = new System.Collections.Generic.Dictionary<string, int>();
    foreach (var component in root.GetComponents<Component>())
    {
        if (!HasCapacity()) break;
        if (component == null) continue;
        var type = component.GetType();
        string typeName = type.FullName ?? type.Name ?? "";
        int index = 0;
        seen.TryGetValue(typeName, out index);
        seen[typeName] = index + 1;
        AddTarget("component", path, typeName, index, type);
    }
}

print("[" + string.Join(",", entries.ToArray()) + "]");
"#;

#[cfg(test)]
mod tests {
    use super::next_progress_percent;

    /// Replays the per-target cadence and returns (lines emitted, first
    /// target that emitted, last percent reported).
    fn replay(total: u32) -> (u32, Option<u32>, u32) {
        let mut last = 0u32;
        let mut emits = 0u32;
        let mut first_emit_at = None;
        for processed in 1..=total {
            if let Some(percent) = next_progress_percent(processed, total, last) {
                first_emit_at.get_or_insert(processed);
                last = percent;
                emits += 1;
            }
        }
        (emits, first_emit_at, last)
    }

    #[test]
    fn reports_every_target_when_total_under_100() {
        let (emits, first, last) = replay(6);
        assert_eq!(emits, 6, "each of the 6 targets should report progress");
        assert_eq!(first, Some(1), "the very first target reports");
        assert_eq!(last, 100, "the run ends at 100%");
    }

    #[test]
    fn reports_every_target_at_exactly_100() {
        let (emits, first, last) = replay(100);
        assert_eq!(emits, 100);
        assert_eq!(first, Some(1));
        assert_eq!(last, 100);
    }

    #[test]
    fn throttles_to_whole_percent_steps_for_large_totals() {
        // 1000 targets => one line per 10 targets (1%); the first nine stay
        // below 1% and emit nothing.
        let (emits, first, last) = replay(1000);
        assert_eq!(emits, 100);
        assert_eq!(first, Some(10));
        assert_eq!(last, 100);
    }

    #[test]
    fn no_progress_when_total_zero() {
        assert_eq!(next_progress_percent(0, 0, 0), None);
        assert_eq!(next_progress_percent(1, 0, 0), None);
    }
}
