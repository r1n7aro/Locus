//! Unity-aware code analysis tools bridging C# symbols to serialized asset
//! data (scenes / prefabs / ScriptableObjects) and project settings.
//!
//! Unlike the `code_*` family these do not need the Roslyn server: they work
//! off the asset reference database (`crate::asset_db`), the Unity YAML
//! parser (`crate::unity_yaml`) and `ProjectSettings/` files, so they answer
//! the questions semantic C# analysis cannot — "where is this script
//! attached", "which Button.onClick binds this method", "does this tag
//! actually exist".

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::code::file_path_arg;
use super::code_unity_managed::{self, ManagedSnapshot};
use super::code_unity_plain;
use super::{make_exec, ToolDef, ToolResult};
use crate::asset_db::types::{guid_to_hex, AssetKind, Guid};
use crate::unity_project_config::{load_tag_manager, BUILTIN_TAGS};

fn err(message: impl Into<String>) -> ToolResult {
    ToolResult {
        output: message.into(),
        is_error: true,
    }
}

fn ok(output: String) -> ToolResult {
    ToolResult {
        output,
        is_error: false,
    }
}

fn resolve_in_workspace(root: &Path, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    dunce::simplified(&absolute).to_path_buf()
}

fn rel_display(root: &Path, path: &Path) -> String {
    let root_text = root.to_string_lossy().to_lowercase();
    let path_text = path.to_string_lossy().to_string();
    if path_text.to_lowercase().starts_with(&root_text) {
        let stripped = path_text[root_text.len()..].trim_start_matches(['\\', '/']);
        if !stripped.is_empty() {
            return stripped.replace('\\', "/");
        }
    }
    path_text.replace('\\', "/")
}

// ─── unity_code_usages ──────────────────────────────────────────────────────

const MAX_CANDIDATE_FILES: usize = 500;

pub(super) fn unity_code_usages() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::UNITY_CODE_USAGES);
    ToolDef {
        name: "unity_code_usages".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let Some(execution) = ctx.execution.as_ref() else {
                    return err(
                        "Tool 'unity_code_usages' requires a checkout-scoped ToolExecutionContext.",
                    );
                };
                let workspace = execution.root().to_string_lossy().to_string();
                let file_path = match file_path_arg(&args, &ctx) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let member = args
                    .get("member")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string);
                let max_results = args
                    .get("max_results")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.clamp(1, 400) as usize)
                    .unwrap_or(100);

                execution
                    .workspace
                    .core()
                    .refresh_asset_db_if_missing(execution.root());
                let asset_db = execution.workspace.core().asset_db();
                let task = tokio::task::spawn_blocking(move || {
                    run_code_usages(
                        asset_db,
                        &workspace,
                        &file_path,
                        member.as_deref(),
                        max_results,
                    )
                })
                .await;
                match task {
                    Ok(result) => result,
                    Err(error) => err(format!("unity_code_usages task failed: {error}")),
                }
            })
        }),
    }
}

pub(super) fn run_code_usages(
    asset_db: std::sync::Arc<std::sync::Mutex<Option<crate::asset_db::AssetDb>>>,
    workspace: &str,
    file_path: &str,
    member: Option<&str>,
    max_results: usize,
) -> ToolResult {
    let root = dunce::simplified(Path::new(workspace)).to_path_buf();
    let script_abs = resolve_in_workspace(&root, file_path);
    if !script_abs.is_file() {
        return err(format!("File not found: {}", script_abs.display()));
    }
    let is_cs = script_abs
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("cs"))
        .unwrap_or(false);
    if !is_cs {
        return err("unity_code_usages expects a .cs script file.");
    }
    let class_name = script_abs
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let meta_path = PathBuf::from(format!("{}.meta", script_abs.display()));
    let Ok(meta_bytes) = std::fs::read(&meta_path) else {
        return err(format!(
            "No .meta file next to {} — the script must live inside the Unity project (Assets/ or Packages/).",
            rel_display(&root, &script_abs)
        ));
    };
    let Some(guid) = crate::asset_db::meta_parser::extract_guid(&meta_bytes) else {
        return err("Could not read a GUID from the script's .meta file.");
    };

    // Assets that reference the script GUID, via the asset reference graph,
    // plus (in member mode) the indexed serialized member bindings and the
    // managed-reference snapshot. The lock is released before any file is
    // read or parsed.
    let (candidates, member_hits, snapshot): (
        Vec<String>,
        Vec<crate::asset_db::MemberBindingHit>,
        ManagedSnapshot,
    ) = {
        let guard = match asset_db.lock() {
            Ok(guard) => guard,
            Err(error) => return err(format!("Failed to lock AssetDb: {error}")),
        };
        let Some(db) = guard.as_ref() else {
            return err(
                "AssetDb not initialized. Run a reference graph scan first (scan button in the UI).",
            );
        };
        let edges = match db.get_direct_refs(&guid) {
            Ok(edges) => edges,
            Err(error) => return err(format!("Failed to query reference graph: {error}")),
        };
        let mut seen: HashSet<Guid> = HashSet::new();
        let mut paths: Vec<String> = Vec::new();
        for edge in &edges {
            if !seen.insert(edge.src_guid) {
                continue;
            }
            let Ok(Some((path, kind))) = db.resolve_path_and_kind_by_guid(&edge.src_guid) else {
                continue;
            };
            // m_Script / member references only live in text-serialized YAML.
            if matches!(
                kind,
                AssetKind::Scene
                    | AssetKind::Prefab
                    | AssetKind::GenericAsset
                    | AssetKind::Controller
                    | AssetKind::OtherYaml
            ) {
                paths.push(path);
            }
        }
        paths.sort();

        let member_hits = match member {
            Some(member) => match db.get_member_bindings(member, &class_name.to_lowercase(), &guid)
            {
                Ok(hits) => hits,
                Err(error) => return err(format!("Failed to query member bindings: {error}")),
            },
            None => Vec::new(),
        };
        let snapshot = match ManagedSnapshot::load(db) {
            Ok(snapshot) => snapshot,
            Err(error) => return err(format!("Failed to read managed reference index: {error}")),
        };
        (paths, member_hits, snapshot)
    };

    // `[SerializeReference]` instances carry `{class, ns, asm}` instead of the
    // script GUID: plan which serialized identities belong to the file's
    // types (and their subclasses), then fetch exactly those rows.
    let script_rel = snapshot
        .script_path(&guid)
        .map(str::to_string)
        .unwrap_or_else(|| rel_display(&root, &script_abs));
    // One catalog (parsed scripts, one-time source scan) serves both the
    // `[SerializeReference]` planner and the plain `[Serializable]` routes.
    let mut catalog = code_unity_managed::TypeCatalog::new(&root, &snapshot);
    let plan = code_unity_managed::plan_usage_query(&mut catalog, &script_rel, &snapshot, member);
    let plain_plan = match code_unity_plain::plan_plain_routes(
        &asset_db,
        &mut catalog,
        &snapshot,
        &script_rel,
        member,
    ) {
        Ok(plain_plan) => plain_plan,
        Err(error) => return err(format!("Failed to plan [Serializable] routes: {error}")),
    };
    drop(catalog);
    // Registry rows plus prefab-instance overrides (type replacements and, in
    // member mode, `managedReferences[R].<member>` values) — DB lookups only.
    let (hits, overrides) = if plan.relations.is_empty() {
        (Vec::new(), code_unity_managed::OverrideFacts::default())
    } else {
        let guard = match asset_db.lock() {
            Ok(guard) => guard,
            Err(error) => return err(format!("Failed to lock AssetDb: {error}")),
        };
        match guard.as_ref() {
            Some(db) => {
                let hits = match db.get_managed_refs_by_class_keys(&plan.class_keys()) {
                    Ok(hits) => hits,
                    Err(error) => {
                        return err(format!("Failed to query managed references: {error}"))
                    }
                };
                let overrides =
                    match code_unity_managed::collect_override_facts(db, &plan, &snapshot) {
                        Ok(overrides) => overrides,
                        Err(error) => {
                            return err(format!("Failed to resolve prefab overrides: {error}"))
                        }
                    };
                (hits, overrides)
            }
            None => (Vec::new(), code_unity_managed::OverrideFacts::default()),
        }
    };
    let managed =
        code_unity_managed::build_usage_report(&root, &plan, &hits, &overrides, &snapshot);
    let plain_report =
        match code_unity_plain::search_plain_values(&asset_db, &root, &plain_plan, &snapshot) {
            Ok(report) => report,
            Err(error) => return err(format!("Failed to search [Serializable] values: {error}")),
        };
    let managed_ctx = ManagedContext {
        script_rel: &script_rel,
        plan: &plan,
        report: &managed,
        plain: &plain_plan,
        plain_report: &plain_report,
    };

    match member {
        None => format_attach_points(
            &root,
            &candidates,
            &guid,
            &class_name,
            &managed_ctx,
            max_results,
        ),
        Some(member) => format_member_usages(
            &root,
            &candidates,
            &guid,
            &class_name,
            member,
            &member_hits,
            &managed_ctx,
            max_results,
        ),
    }
}

/// Managed-reference results threaded into the two output formatters.
struct ManagedContext<'a> {
    script_rel: &'a str,
    plan: &'a code_unity_managed::UsagePlan,
    report: &'a code_unity_managed::UsageReport,
    plain: &'a code_unity_plain::PlainPlan,
    plain_report: &'a code_unity_plain::PlainReport,
}

const COVERAGE_NOTE: &str = "Covered: m_Script attach points, [SerializeReference] instances (including prefab-instance type replacements and managedReferences[...] overrides), and plain [Serializable] values reached through serialized fields (T, T[], List<T>, project generics) including their prefab overrides, in text-serialized Assets/ and Packages/ assets. Not covered: binary-serialized assets, Presets, custom serialization (ISerializationCallbackReceiver, dictionaries), and string/reflection-based use.";

/// One line describing the plain `[Serializable]` search for empty results.
fn plain_summary(ctx: &ManagedContext) -> String {
    let plain = ctx.plain;
    if plain.targets.is_empty() {
        if plain.not_serializable.is_empty() {
            return String::new();
        }
        return format!(
            " {} lack [Serializable], so Unity never serializes them as plain field values.",
            capped_list(&plain.not_serializable, 4)
        );
    }
    let routes = plain.route_labels();
    if routes.is_empty() {
        return format!(
            " No serialized field of a MonoBehaviour / ScriptableObject / [SerializeReference] type holds {} (directly, in arrays/lists, or nested in other [Serializable] types).",
            capped_list(&plain.targets, 4)
        );
    }
    let shown: Vec<&str> = routes.iter().take(6).map(String::as_str).collect();
    format!(
        " No assets hold {} values along {} field path{} ({}{}).",
        capped_list(&plain.targets, 4),
        routes.len(),
        if routes.len() == 1 { "" } else { "s" },
        shown.join(", "),
        if routes.len() > shown.len() {
            ", …"
        } else {
            ""
        }
    )
}

/// `a, b, c, d, +N more`.
fn capped_list(items: &[String], cap: usize) -> String {
    let shown: Vec<&str> = items.iter().take(cap).map(String::as_str).collect();
    if items.len() > shown.len() {
        format!("{}, +{} more", shown.join(", "), items.len() - shown.len())
    } else {
        shown.join(", ")
    }
}

/// Plain `[Serializable]` value listing, `None` when there are none.
fn plain_value_section(ctx: &ManagedContext, max_results: usize) -> Option<String> {
    let entries = &ctx.plain_report.entries;
    if entries.is_empty() {
        return None;
    }
    let routes = ctx.plain.route_labels();
    let shown: Vec<&str> = routes.iter().take(6).map(String::as_str).collect();
    let mut output = format!(
        "{} [Serializable] value{} of {} in {} asset{} via {} field path{} ({}{})\n\n",
        entries.len(),
        if entries.len() == 1 { "" } else { "s" },
        capped_list(&ctx.plain_report.targets, 4),
        ctx.plain_report.asset_count,
        if ctx.plain_report.asset_count == 1 {
            ""
        } else {
            "s"
        },
        routes.len(),
        if routes.len() == 1 { "" } else { "s" },
        shown.join(", "),
        if routes.len() > shown.len() {
            ", …"
        } else {
            ""
        }
    );
    push_grouped_entries(&mut output, entries, max_results);
    Some(output)
}

fn declared_types_label(ctx: &ManagedContext) -> String {
    let types = &ctx.plan.declared_types;
    if types.is_empty() {
        return "it declares no class/struct/interface".to_string();
    }
    let shown: Vec<&str> = types.iter().take(6).map(String::as_str).collect();
    let more = if types.len() > shown.len() {
        format!(", +{} more", types.len() - shown.len())
    } else {
        String::new()
    };
    format!(
        "{} declared type{} ({}{more}){}",
        types.len(),
        if types.len() == 1 { "" } else { "s" },
        shown.join(", "),
        ctx.plan
            .file_assembly
            .as_deref()
            .map(|asm| format!(" in assembly '{asm}'"))
            .unwrap_or_default()
    )
}

/// `[SerializeReference]` instance listing, `None` when there are none.
fn managed_instance_section(ctx: &ManagedContext, max_results: usize) -> Option<String> {
    let instances = &ctx.report.instances;
    if instances.is_empty() {
        return None;
    }
    let mut output = format!(
        "{} [SerializeReference] instance{} of {} serialized type{} in {} asset{} — {}, incl. subclasses/implementers\n\n",
        instances.len(),
        if instances.len() == 1 { "" } else { "s" },
        ctx.report.type_count,
        if ctx.report.type_count == 1 { "" } else { "s" },
        ctx.report.asset_count,
        if ctx.report.asset_count == 1 { "" } else { "s" },
        declared_types_label(ctx),
    );
    push_grouped_entries(&mut output, instances, max_results);
    Some(output)
}

/// Component / asset-object blocks whose `m_Script` points at the script,
/// followed by `[SerializeReference]` instances of the types it declares.
fn format_attach_points(
    root: &Path,
    candidates: &[String],
    guid: &Guid,
    class_name: &str,
    managed: &ManagedContext,
    max_results: usize,
) -> ToolResult {
    let guid_section = guid_attach_section(root, candidates, guid, class_name, max_results);
    let managed_section = managed_instance_section(managed, max_results);
    let plain_section = plain_value_section(managed, max_results);
    if guid_section.is_none() && managed_section.is_none() && plain_section.is_none() {
        return ok(format!(
            "No serialized usages of '{}': no asset references script '{class_name}' (GUID {}), and no [SerializeReference] instances of its {} or their subclasses/implementers exist in indexed assets.{}\n{COVERAGE_NOTE}",
            managed.script_rel,
            guid_to_hex(guid),
            declared_types_label(managed),
            plain_summary(managed),
        ));
    }
    let mut sections: Vec<String> = Vec::new();
    match guid_section {
        Some(section) => sections.push(section),
        None => sections.push(format!(
            "No asset references script '{class_name}' by GUID (not attached as a component / ScriptableObject).\n"
        )),
    }
    sections.extend(managed_section);
    sections.extend(plain_section);
    ok(sections.join("\n"))
}

/// GUID attach-point listing, `None` when no asset references the GUID.
fn guid_attach_section(
    root: &Path,
    candidates: &[String],
    guid: &Guid,
    class_name: &str,
    max_results: usize,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }

    let mut entries: Vec<(String, u32, String)> = Vec::new();
    let mut files_scanned = 0usize;
    let mut files_truncated = false;
    for path in candidates {
        if files_scanned >= MAX_CANDIDATE_FILES {
            files_truncated = true;
            break;
        }
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            continue;
        };
        files_scanned += 1;
        let docs = crate::unity_yaml::parse_yaml_docs(&bytes);
        let mut go_names: HashMap<i64, &str> = HashMap::new();
        for doc in &docs {
            if doc.class_id == 1 {
                if let Some(name) = doc.m_name.as_deref() {
                    go_names.insert(doc.file_id, name);
                }
            }
        }
        for doc in &docs {
            if doc.m_script_guid.as_ref() != Some(guid) {
                continue;
            }
            let desc = match doc.m_game_object_id.and_then(|id| go_names.get(&id)) {
                Some(name) => format!("component on GameObject '{name}'"),
                None => match doc.m_name.as_deref() {
                    Some(name) if !name.is_empty() => format!("asset object '{name}'"),
                    _ => "component (GameObject defined in source prefab)".to_string(),
                },
            };
            entries.push((path.clone(), doc.line_start as u32 + 1, desc));
        }
    }

    if entries.is_empty() {
        return Some(format!(
            "{} asset(s) reference script '{class_name}' but contain no component block for it (indirect references only, e.g. via prefab overrides). Candidates:\n{}\n",
            candidates.len(),
            candidates
                .iter()
                .take(20)
                .map(|p| format!("  {p}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let total = entries.len();
    let mut output = format!(
        "{total} attach point{} of '{class_name}'\n\n",
        if total == 1 { "" } else { "s" }
    );
    push_grouped_entries(&mut output, &entries, max_results);
    if files_truncated {
        output.push_str(&format!(
            "\n(Only the first {MAX_CANDIDATE_FILES} referencing assets were scanned.)\n"
        ));
    }
    Some(output)
}

/// Serialized member usages: UnityEvent bindings, serialized field values and
/// AnimationEvent function names. UnityEvent/anim bindings come pre-resolved
/// from the indexed `member_bindings` table (`member_hits`, already ordered by
/// path then line); only the serialized-field-value match still scans the
/// referencing assets directly, since field values aren't indexed.
fn format_member_usages(
    root: &Path,
    candidates: &[String],
    guid: &Guid,
    class_name: &str,
    member: &str,
    member_hits: &[crate::asset_db::MemberBindingHit],
    managed: &ManagedContext,
    max_results: usize,
) -> ToolResult {
    let mut entries: Vec<(String, u32, String)> = Vec::new();
    let mut files_scanned = 0usize;
    let mut files_truncated = false;

    // Serialized field values (`member: value` inside one of this script's own
    // component blocks) are not indexed, so scan the referencing assets for
    // them. UnityEvent bindings and AnimationEvents are handled by the DB below.
    for path in candidates {
        if files_scanned >= MAX_CANDIDATE_FILES {
            files_truncated = true;
            break;
        }
        let Ok(bytes) = std::fs::read(root.join(path)) else {
            continue;
        };
        files_scanned += 1;
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let docs = crate::unity_yaml::parse_yaml_docs(&bytes);

        let mut go_names: HashMap<i64, &str> = HashMap::new();
        for doc in &docs {
            if doc.class_id == 1 {
                if let Some(name) = doc.m_name.as_deref() {
                    go_names.insert(doc.file_id, name);
                }
            }
        }
        // Line ranges of this script's own component blocks (for field hits).
        let our_docs: Vec<(usize, usize, Option<String>)> = docs
            .iter()
            .filter(|doc| doc.m_script_guid.as_ref() == Some(guid))
            .map(|doc| {
                let go = doc
                    .m_game_object_id
                    .and_then(|id| go_names.get(&id))
                    .map(|name| name.to_string());
                (doc.line_start, doc.line_end, go)
            })
            .collect();

        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim_start();
            let item = trimmed.strip_prefix("- ").unwrap_or(trimmed);

            // Serialized field inside one of this script's component blocks.
            if let Some(rest) = item.strip_prefix(member) {
                if let Some(value) = rest.strip_prefix(':') {
                    if let Some((_, _, go)) = our_docs
                        .iter()
                        .find(|(start, end, _)| index >= *start && index <= *end)
                    {
                        let mut preview = value.trim().to_string();
                        if preview.chars().count() > 60 {
                            preview = preview.chars().take(60).collect::<String>() + "…";
                        }
                        let location = go
                            .as_deref()
                            .map(|name| format!(" — GameObject '{name}'"))
                            .unwrap_or_default();
                        entries.push((
                            path.clone(),
                            index as u32 + 1,
                            format!("[serialized field] {member}: {preview}{location}"),
                        ));
                    }
                }
            }
        }
    }

    // UnityEvent persistent calls + AnimationEvents, resolved once from the
    // indexed `member_bindings` table. Covers broken/unbound bindings named
    // only by type string (no GUID, so the reference graph never sees them) and
    // animation events project-wide — replacing the old per-query anim scan and
    // scene/prefab/asset sweep.
    for hit in member_hits {
        let desc = if hit.binding_kind == 1 {
            format!("[AnimationEvent] functionName: {member} (receiver resolved at runtime)")
        } else if hit.target_is_ours {
            let go = if hit.target_go_name.is_empty() {
                String::new()
            } else {
                format!(", GameObject '{}'", hit.target_go_name)
            };
            format!("[UnityEvent] m_MethodName: {member} (target: this script{go})")
        } else if matches!(hit.target_file_id, None | Some(0)) {
            format!(
                "[UnityEvent] m_MethodName: {member} (target: '{}' by type name, m_Target unset — broken/unbound binding)",
                hit.target_type_full
            )
        } else {
            format!(
                "[UnityEvent] m_MethodName: {member} (target: '{}' by type name)",
                hit.target_type_full
            )
        };
        entries.push((hit.path.clone(), hit.line, desc));
    }

    // Field values inside `[SerializeReference]` instance `data:` blocks of
    // the file's types and their subclasses (incl. [FormerlySerializedAs]
    // names of the declaring field).
    entries.extend(managed.report.members.iter().cloned());
    // Member values inside plain `[Serializable]` values along every route.
    entries.extend(managed.plain_report.entries.iter().cloned());

    if entries.is_empty() {
        return ok(format!(
            "No serialized usages of '{class_name}.{member}' found in {} referencing asset(s), indexed UnityEvent/AnimationEvent bindings, or the data of {} [SerializeReference] instance(s) of its {}.{} Code references are not covered here — use code_find_references for those.\n{COVERAGE_NOTE}",
            candidates.len().min(files_scanned),
            managed.report.instances.len(),
            declared_types_label(managed),
            plain_summary(managed),
        ));
    }

    // Field-value entries (scan order) and binding entries (SQL order) are
    // interleaved by path; sort by (path, line) for stable grouped output.
    entries.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    let total = entries.len();
    let mut output = format!(
        "{total} serialized usage{} of '{class_name}.{member}'\n\n",
        if total == 1 { "" } else { "s" }
    );
    push_grouped_entries(&mut output, &entries, max_results);
    if files_truncated {
        output.push_str(&format!(
            "\n(Only the first {MAX_CANDIDATE_FILES} referencing assets were scanned.)\n"
        ));
    }
    ok(output)
}

fn push_grouped_entries(output: &mut String, entries: &[(String, u32, String)], cap: usize) {
    let shown = entries.len().min(cap);
    let mut current_file = "";
    for (path, line, desc) in entries.iter().take(cap) {
        if path != current_file {
            if !current_file.is_empty() {
                output.push('\n');
            }
            output.push_str(path);
            output.push('\n');
            current_file = path;
        }
        output.push_str(&format!("  {line}: {desc}\n"));
    }
    if shown < entries.len() {
        output.push_str(&format!(
            "\n(Showing first {shown} of {}.)\n",
            entries.len()
        ));
    }
}

// ─── project string-ref validation (folded into code_diagnostics) ──────────

const MAX_SOURCE_FILES: usize = 5000;
const MAX_RESOURCE_ENTRIES: usize = 20000;

struct ProjectRefs {
    tags: Vec<String>,
    layers: Vec<String>,
    /// Enabled scene paths, in build order ("Assets/Scenes/Main.unity").
    scenes_enabled: Vec<String>,
    scenes_disabled: Vec<String>,
    has_build_settings: bool,
    axes: Vec<String>,
    /// `activeInputHandler` from ProjectSettings.asset:
    /// 0 = legacy Input Manager, 1 = new Input System only, 2 = both.
    /// None when the field is absent (old Unity = legacy).
    active_input_handler: Option<u8>,
    /// Lowercased Resources-relative path without extension -> original.
    resources: HashMap<String, String>,
}

fn load_project_refs(root: &Path) -> ProjectRefs {
    let (tags, layers) = parse_tag_manager(root);
    let (scenes_enabled, scenes_disabled, has_build_settings) = parse_build_settings(root);
    let axes = parse_input_axes(root);
    let active_input_handler = parse_active_input_handler(root);
    let resources = collect_resource_paths(root);
    ProjectRefs {
        tags,
        layers,
        scenes_enabled,
        scenes_disabled,
        has_build_settings,
        axes,
        active_input_handler,
        resources,
    }
}

/// `activeInputHandler` from `ProjectSettings/ProjectSettings.asset`.
fn parse_active_input_handler(root: &Path) -> Option<u8> {
    let path = root.join("ProjectSettings").join("ProjectSettings.asset");
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        if let Some(value) = line.trim().strip_prefix("activeInputHandler:") {
            return value.trim().parse().ok();
        }
    }
    None
}

/// Tags (built-in + custom) and named layers from
/// `ProjectSettings/TagManager.asset`.
fn parse_tag_manager(root: &Path) -> (Vec<String>, Vec<String>) {
    let mut tags: Vec<String> = BUILTIN_TAGS.iter().map(|t| t.to_string()).collect();
    let Ok(config) = load_tag_manager(root) else {
        return (tags, Vec::new());
    };

    let layers = config
        .named_layers()
        .map(|(_, name)| name.to_string())
        .collect();
    tags.extend(config.custom_tags);
    (tags, layers)
}

/// (enabled scene paths in build order, disabled scene paths, file existed).
fn parse_build_settings(root: &Path) -> (Vec<String>, Vec<String>, bool) {
    let path = root
        .join("ProjectSettings")
        .join("EditorBuildSettings.asset");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return (Vec::new(), Vec::new(), false);
    };
    let mut enabled = Vec::new();
    let mut disabled = Vec::new();
    let mut in_scenes = false;
    let mut current_enabled = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "m_Scenes:" {
            in_scenes = true;
            continue;
        }
        if trimmed == "m_Scenes: []" {
            in_scenes = false;
            continue;
        }
        if in_scenes {
            if !line.starts_with(' ') && trimmed.contains(':') {
                in_scenes = false;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("- enabled: ") {
                current_enabled = value.trim() == "1";
            } else if let Some(value) = trimmed.strip_prefix("path: ") {
                let scene_path = value.trim().to_string();
                if scene_path.is_empty() {
                    continue;
                }
                if current_enabled {
                    enabled.push(scene_path);
                } else {
                    disabled.push(scene_path);
                }
            }
        }
    }
    (enabled, disabled, true)
}

/// Legacy input axis names from `ProjectSettings/InputManager.asset`.
fn parse_input_axes(root: &Path) -> Vec<String> {
    let path = root.join("ProjectSettings").join("InputManager.asset");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut axes = Vec::new();
    let mut in_axes = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "m_Axes:" {
            in_axes = true;
            continue;
        }
        if in_axes {
            if !line.starts_with(' ') && trimmed.contains(':') {
                in_axes = false;
                continue;
            }
            let item = trimmed.strip_prefix("- ").unwrap_or(trimmed);
            if let Some(value) = item.strip_prefix("m_Name: ") {
                let name = value.trim().to_string();
                if !name.is_empty() && !axes.contains(&name) {
                    axes.push(name);
                }
            }
        }
    }
    axes
}

/// All loadable paths under `Resources/` folders (Assets + Packages), keyed
/// by lowercased Resources-relative path without extension.
fn collect_resource_paths(root: &Path) -> HashMap<String, String> {
    let mut resources = HashMap::new();
    for base in ["Assets", "Packages"] {
        let base_dir = root.join(base);
        if !base_dir.is_dir() {
            continue;
        }
        let mut stack = vec![base_dir];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if name == "Resources" {
                    collect_resources_under(&path, &path, &mut resources);
                    if resources.len() >= MAX_RESOURCE_ENTRIES {
                        return resources;
                    }
                } else {
                    stack.push(path);
                }
            }
        }
    }
    resources
}

fn collect_resources_under(resources_root: &Path, dir: &Path, out: &mut HashMap<String, String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_resources_under(resources_root, &path, out);
            continue;
        }
        if out.len() >= MAX_RESOURCE_ENTRIES {
            return;
        }
        let is_meta = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("meta"))
            .unwrap_or(false);
        if is_meta {
            continue;
        }
        let Ok(relative) = path.strip_prefix(resources_root) else {
            continue;
        };
        let without_ext = relative.with_extension("");
        let display = without_ext.to_string_lossy().replace('\\', "/");
        out.insert(display.to_lowercase(), display);
    }
}

pub(super) struct RefProblem {
    pub(super) path: String,
    pub(super) line: u32,
    pub(super) message: String,
}

/// Result of scanning C# sources for project string references (consumed by
/// `code_diagnostics`, which folds it into its output).
pub(super) struct ProjectRefsReport {
    pub(super) files_checked: usize,
    pub(super) files_capped: bool,
    /// Checked reference counts: tag, layer, scene, resource, input.
    pub(super) counts: [usize; 5],
    /// Legacy Input refs that could not be validated (no axes defined).
    pub(super) unvalidated_axis_refs: usize,
    pub(super) problems: Vec<RefProblem>,
}

/// Validate tag/layer/scene/Resources/Input string literals in C# code
/// against the actual project configuration. `target_path` limits the scan
/// to a file or directory; `None` scans the whole Assets folder.
pub(super) fn scan_project_refs(
    workspace: &str,
    target_path: Option<&str>,
) -> Result<ProjectRefsReport, String> {
    let root = dunce::simplified(Path::new(workspace)).to_path_buf();
    let target = match target_path {
        Some(path) => {
            let resolved = resolve_in_workspace(&root, path);
            if !resolved.exists() {
                return Err(format!("Path not found: {}", resolved.display()));
            }
            resolved
        }
        None => {
            let assets = root.join("Assets");
            if !assets.is_dir() {
                return Err("No Assets folder in the workspace".to_string());
            }
            assets
        }
    };

    let refs = load_project_refs(&root);

    let files: Vec<PathBuf> = if target.is_file() {
        vec![target.clone()]
    } else {
        let mut files = Vec::new();
        let mut stack = vec![target.clone()];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if path.is_dir() {
                    if !name.starts_with('.') {
                        stack.push(path);
                    }
                    continue;
                }
                let is_cs = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("cs"))
                    .unwrap_or(false);
                if is_cs && files.len() < MAX_SOURCE_FILES {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    };
    if files.is_empty() {
        return Ok(ProjectRefsReport {
            files_checked: 0,
            files_capped: false,
            counts: [0; 5],
            unvalidated_axis_refs: 0,
            problems: Vec::new(),
        });
    }

    // String-literal reference sites. (?s) is unnecessary: scanning per line.
    let tag_res = [
        regex::Regex::new(r#"(?:CompareTag|FindWithTag|FindGameObjectsWithTag)\s*\(\s*"([^"]*)""#)
            .unwrap(),
        regex::Regex::new(r#"\.tag\s*[!=]=\s*"([^"]*)""#).unwrap(),
        regex::Regex::new(r#"\.tag\s*=\s*"([^"]*)""#).unwrap(),
    ];
    let layer_re = regex::Regex::new(r#"NameToLayer\s*\(\s*"([^"]*)""#).unwrap();
    let layer_mask_re = regex::Regex::new(r#"LayerMask\.GetMask\s*\(([^)]*)\)"#).unwrap();
    let string_literal_re = regex::Regex::new(r#""([^"]*)""#).unwrap();
    let scene_re = regex::Regex::new(
        r#"(?:LoadScene|LoadSceneAsync|UnloadSceneAsync|GetSceneByName)\s*\(\s*"([^"]*)""#,
    )
    .unwrap();
    let scene_index_re =
        regex::Regex::new(r#"(?:LoadScene|LoadSceneAsync)\s*\(\s*(\d+)\s*[,)]"#).unwrap();
    let resources_re =
        regex::Regex::new(r#"Resources\.(?:Load|LoadAll|LoadAsync)(?:<[^>(]*>)?\s*\(\s*"([^"]*)""#)
            .unwrap();
    let input_re = regex::Regex::new(
        r#"Input\.(?:GetAxis|GetAxisRaw|GetButton|GetButtonDown|GetButtonUp)\s*\(\s*"([^"]*)""#,
    )
    .unwrap();

    let mut problems: Vec<RefProblem> = Vec::new();
    let mut counts = [0usize; 5]; // tags, layers, scenes, resources, axes
    let mut axis_refs_seen = 0usize;

    let scene_stems_enabled: Vec<String> = refs
        .scenes_enabled
        .iter()
        .filter_map(|p| scene_stem(p))
        .collect();
    let scene_stems_disabled: Vec<String> = refs
        .scenes_disabled
        .iter()
        .filter_map(|p| scene_stem(p))
        .collect();

    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let display = rel_display(&root, file);
        let mut in_block_comment = false;
        for (index, raw_line) in text.lines().enumerate() {
            let line_no = index as u32 + 1;
            let mut line = raw_line;
            if in_block_comment {
                match line.find("*/") {
                    Some(end) => {
                        line = &line[end + 2..];
                        in_block_comment = false;
                    }
                    None => continue,
                }
            }
            if let Some(start) = line.find("/*") {
                if !line[start..].contains("*/") {
                    in_block_comment = true;
                    line = &line[..start];
                }
            }
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }

            // Tags.
            for tag_re in &tag_res {
                for caps in tag_re.captures_iter(line) {
                    let value = &caps[1];
                    counts[0] += 1;
                    if !refs.tags.iter().any(|t| t == value) {
                        let suggestion = closest_match(&refs.tags, value);
                        problems.push(RefProblem {
                            path: display.clone(),
                            line: line_no,
                            message: format!(
                                "unknown tag \"{value}\"{}",
                                suggestion
                                    .map(|s| format!(" (did you mean \"{s}\"?)"))
                                    .unwrap_or_default()
                            ),
                        });
                    }
                }
            }

            // Layers.
            for caps in layer_re.captures_iter(line) {
                let value = &caps[1];
                counts[1] += 1;
                if !refs.layers.iter().any(|l| l == value) {
                    let suggestion = closest_match(&refs.layers, value);
                    problems.push(RefProblem {
                        path: display.clone(),
                        line: line_no,
                        message: format!(
                            "unknown layer \"{value}\"{}",
                            suggestion
                                .map(|s| format!(" (did you mean \"{s}\"?)"))
                                .unwrap_or_default()
                        ),
                    });
                }
            }
            for caps in layer_mask_re.captures_iter(line) {
                for literal in string_literal_re.captures_iter(&caps[1]) {
                    let value = literal[1].to_string();
                    counts[1] += 1;
                    if !refs.layers.iter().any(|l| *l == value) {
                        let suggestion = closest_match(&refs.layers, &value);
                        problems.push(RefProblem {
                            path: display.clone(),
                            line: line_no,
                            message: format!(
                                "unknown layer \"{value}\" in GetMask{}",
                                suggestion
                                    .map(|s| format!(" (did you mean \"{s}\"?)"))
                                    .unwrap_or_default()
                            ),
                        });
                    }
                }
            }

            // Scenes by name/path.
            for caps in scene_re.captures_iter(line) {
                let value = &caps[1];
                counts[2] += 1;
                if let Some(message) = check_scene_ref(
                    value,
                    &refs.scenes_enabled,
                    &scene_stems_enabled,
                    &refs.scenes_disabled,
                    &scene_stems_disabled,
                    refs.has_build_settings,
                ) {
                    problems.push(RefProblem {
                        path: display.clone(),
                        line: line_no,
                        message,
                    });
                }
            }
            // Scenes by build index.
            if refs.has_build_settings {
                for caps in scene_index_re.captures_iter(line) {
                    counts[2] += 1;
                    if let Ok(scene_index) = caps[1].parse::<usize>() {
                        if scene_index >= refs.scenes_enabled.len() {
                            problems.push(RefProblem {
                                path: display.clone(),
                                line: line_no,
                                message: format!(
                                    "scene index {scene_index} out of range ({} enabled scene{} in build settings)",
                                    refs.scenes_enabled.len(),
                                    if refs.scenes_enabled.len() == 1 { "" } else { "s" }
                                ),
                            });
                        }
                    }
                }
            }

            // Resources paths.
            for caps in resources_re.captures_iter(line) {
                let value = caps[1].replace('\\', "/");
                let value = value.trim_matches('/');
                if value.is_empty() {
                    continue; // Resources.LoadAll("") loads everything — valid.
                }
                counts[3] += 1;
                let key = value.to_lowercase();
                match refs.resources.get(&key) {
                    Some(original) if original != value => {
                        problems.push(RefProblem {
                            path: display.clone(),
                            line: line_no,
                            message: format!(
                                "resource path \"{value}\" case-mismatches \"{original}\" (breaks on case-sensitive platforms)"
                            ),
                        });
                    }
                    Some(_) => {}
                    None => {
                        // LoadAll on a folder: accept when any entry is under it.
                        let prefix = format!("{key}/");
                        let is_folder = refs.resources.keys().any(|k| k.starts_with(&prefix));
                        if !is_folder {
                            let query_stem = value.rsplit('/').next().unwrap_or(value);
                            let suggestion = refs
                                .resources
                                .values()
                                .find(|original| {
                                    original
                                        .rsplit('/')
                                        .next()
                                        .map(|stem| stem.eq_ignore_ascii_case(query_stem))
                                        .unwrap_or(false)
                                })
                                .cloned()
                                .or_else(|| {
                                    let stems: Vec<String> = refs
                                        .resources
                                        .values()
                                        .filter_map(|original| {
                                            original.rsplit('/').next().map(str::to_string)
                                        })
                                        .collect();
                                    closest_match(&stems, query_stem).and_then(|stem| {
                                        refs.resources
                                            .values()
                                            .find(|original| {
                                                original.rsplit('/').next() == Some(stem.as_str())
                                            })
                                            .cloned()
                                    })
                                });
                            problems.push(RefProblem {
                                path: display.clone(),
                                line: line_no,
                                message: format!(
                                    "no asset at Resources path \"{value}\"{}",
                                    suggestion
                                        .map(|s| format!(" (closest: \"{s}\")"))
                                        .unwrap_or_default()
                                ),
                            });
                        }
                    }
                }
            }

            // Legacy input axes.
            for caps in input_re.captures_iter(line) {
                let value = &caps[1];
                axis_refs_seen += 1;
                counts[4] += 1;
                // With the new Input System enabled exclusively, every legacy
                // Input.* call is a guaranteed runtime exception — flag the
                // call itself instead of validating the axis name.
                if refs.active_input_handler == Some(1) {
                    problems.push(RefProblem {
                        path: display.clone(),
                        line: line_no,
                        message: format!(
                            "legacy Input API call with \"{value}\" — this project enables the new Input System only (activeInputHandler: 1), so Input.GetAxis/GetButton throws InvalidOperationException at runtime"
                        ),
                    });
                    continue;
                }
                if refs.axes.is_empty() {
                    continue; // no axes defined — nothing to validate against.
                }
                if !refs.axes.iter().any(|a| a == value) {
                    let suggestion = closest_match(&refs.axes, value);
                    problems.push(RefProblem {
                        path: display.clone(),
                        line: line_no,
                        message: format!(
                            "unknown input axis \"{value}\"{}",
                            suggestion
                                .map(|s| format!(" (did you mean \"{s}\"?)"))
                                .unwrap_or_default()
                        ),
                    });
                }
            }
        }
    }

    let unvalidated_axis_refs =
        if refs.active_input_handler != Some(1) && refs.axes.is_empty() && axis_refs_seen > 0 {
            axis_refs_seen
        } else {
            0
        };

    Ok(ProjectRefsReport {
        files_checked: files.len(),
        files_capped: files.len() >= MAX_SOURCE_FILES,
        counts,
        unvalidated_axis_refs,
        problems,
    })
}

fn scene_stem(path: &str) -> Option<String> {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, char_a) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, char_b) in b.iter().enumerate() {
            let cost = usize::from(char_a != char_b);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

/// Closest known name for a did-you-mean suggestion: exact case-insensitive
/// match first (case typos), then smallest edit distance — ≤2 for names of 5+
/// chars, ≤1 for shorter ones, enough for "Enemey"→"Enemy" without dragging
/// in unrelated names.
fn closest_match(known: &[String], value: &str) -> Option<String> {
    if let Some(exact) = known
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(value) && candidate.as_str() != value)
    {
        return Some(exact.clone());
    }
    let max_distance = if value.chars().count() >= 5 { 2 } else { 1 };
    let value_lower = value.to_lowercase();
    known
        .iter()
        .filter(|candidate| candidate.as_str() != value)
        .map(|candidate| {
            (
                levenshtein(&candidate.to_lowercase(), &value_lower),
                candidate,
            )
        })
        .filter(|(distance, _)| *distance <= max_distance)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate.clone())
}

/// Validate a scene string (name, full path, or extension-less partial path)
/// against the build list. Returns a problem message, or None when fine.
fn check_scene_ref(
    value: &str,
    enabled_paths: &[String],
    enabled_stems: &[String],
    disabled_paths: &[String],
    disabled_stems: &[String],
    has_build_settings: bool,
) -> Option<String> {
    if !has_build_settings {
        return None;
    }
    let normalized = value.replace('\\', "/");
    let matches_list = |paths: &[String], stems: &[String]| -> bool {
        stems.iter().any(|stem| stem == &normalized)
            || paths.iter().any(|path| {
                path == &normalized
                    || path
                        .strip_suffix(".unity")
                        .map(|stripped| {
                            stripped == normalized || stripped.ends_with(&format!("/{normalized}"))
                        })
                        .unwrap_or(false)
            })
    };
    if matches_list(enabled_paths, enabled_stems) {
        return None;
    }
    if matches_list(disabled_paths, disabled_stems) {
        return Some(format!(
            "scene \"{value}\" is in build settings but disabled"
        ));
    }
    // Case-insensitive rescue, then edit-distance, for a suggestion.
    let tail = normalized.rsplit('/').next().unwrap_or(&normalized);
    let suggestion = enabled_stems
        .iter()
        .chain(enabled_paths.iter())
        .find(|known| known.eq_ignore_ascii_case(&normalized))
        .cloned()
        .or_else(|| {
            enabled_stems
                .iter()
                .find(|stem| stem.eq_ignore_ascii_case(tail))
                .cloned()
        })
        .or_else(|| closest_match(enabled_stems, tail));
    let available = if suggestion.is_none() && !enabled_stems.is_empty() && enabled_stems.len() <= 8
    {
        format!(" (scenes in build: {})", enabled_stems.join(", "))
    } else {
        String::new()
    };
    Some(format!(
        "scene \"{value}\" not in build settings{}{available}",
        suggestion
            .map(|s| format!(" (did you mean \"{s}\"?)"))
            .unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::unity_code_usages;

    #[tokio::test]
    async fn code_usage_search_requires_checkout_execution_scope() {
        let tool = unity_code_usages();
        let result = (tool.execute)(
            serde_json::json!({ "file_path": "Assets/Scripts/Test.cs" }),
            crate::tool::ToolExecutionContext::default(),
        )
        .await;

        assert!(result.is_error);
        assert!(result
            .output
            .contains("checkout-scoped ToolExecutionContext"));
    }
}
