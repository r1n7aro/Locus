use serde::{Deserialize, Serialize};
use tauri::Manager;

use super::{make_exec, ToolDef, ToolResult};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PluginListToolArgs {
    #[serde(default)]
    working_dir: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginListToolOutput {
    working_dir: String,
    count: usize,
    plugins: Vec<crate::plugin::InstalledPluginSummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginSearchToolArgs {
    query: String,
    #[serde(default)]
    registry_base_url: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginSearchShardError {
    bucket: String,
    error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginSearchToolOutput {
    query: String,
    registry_base_url: String,
    registry_version: u32,
    updated_at: String,
    scanned_buckets: usize,
    search_index_used: bool,
    failed_buckets: Vec<PluginSearchShardError>,
    result_count: usize,
    results: Vec<crate::commands::PluginRegistrySummary>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallToolArgs {
    #[serde(default)]
    plugin_id: Option<String>,
    #[serde(default)]
    registry_base_url: Option<String>,
    #[serde(default)]
    expected_plugin_id: Option<String>,
    #[serde(default)]
    scope: Option<crate::plugin::PluginInstallScope>,
    #[serde(default)]
    source: Option<crate::commands::PluginDownloadSource>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default, rename = "ref")]
    ref_name: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    commit: Option<String>,
    #[serde(default)]
    asset: Option<String>,
    #[serde(default)]
    asset_pattern: Option<String>,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginInstallToolOutput {
    source: String,
    registry_base_url: Option<String>,
    installed: crate::plugin::InstalledPluginSummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginUninstallToolArgs {
    plugin_id: String,
    #[serde(default)]
    scope: Option<crate::plugin::PluginInstallScope>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginUninstallToolOutput {
    working_dir: String,
    plugin_id: String,
    scope: crate::plugin::PluginInstallScope,
    removed: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginSetEnabledToolArgs {
    plugin_id: String,
    #[serde(default)]
    scope: Option<crate::plugin::PluginInstallScope>,
    enabled: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginSetEnabledToolOutput {
    working_dir: String,
    plugin_id: String,
    scope: crate::plugin::PluginInstallScope,
    enabled: bool,
    plugin: crate::plugin::InstalledPluginSummary,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginExportToolArgs {
    id: String,
    name: String,
    version: String,
    file_path: String,
    #[serde(default)]
    skill_package_ids: Vec<String>,
    #[serde(default)]
    view_ids: Vec<String>,
    #[serde(default)]
    rule_files: Vec<crate::commands::PluginExportRuleFile>,
    #[serde(default)]
    project_dependencies: Vec<crate::plugin::LocusPluginProjectDependency>,
    #[serde(default = "default_plugin_export_install_after")]
    install_after_export: bool,
    #[serde(default)]
    install_scope: Option<crate::plugin::PluginInstallScope>,
    #[serde(default = "default_plugin_export_transfer_ownership")]
    transfer_ownership: bool,
    audit_summary: String,
    structure_plan: String,
    user_approval: String,
}

fn default_plugin_export_install_after() -> bool {
    true
}

fn default_plugin_export_transfer_ownership() -> bool {
    true
}

fn require_detail(label: &str, value: &str) -> Result<(), ToolResult> {
    if value.trim().chars().count() < 20 {
        return Err(ToolResult {
            output: format!(
                "plugin_export requires a detailed {}. Run the /plugin audit and present the structure plan before exporting.",
                label
            ),
            is_error: true,
        });
    }
    Ok(())
}

fn parse_args(args: serde_json::Value) -> Result<PluginExportToolArgs, ToolResult> {
    let parsed =
        serde_json::from_value::<PluginExportToolArgs>(args).map_err(|error| ToolResult {
            output: format!("Error parsing plugin_export arguments: {}", error),
            is_error: true,
        })?;
    if parsed.skill_package_ids.is_empty()
        && parsed.view_ids.is_empty()
        && parsed.rule_files.is_empty()
    {
        return Err(ToolResult {
            output: "plugin_export requires at least one Skill package id, View id, or Rule file."
                .to_string(),
            is_error: true,
        });
    }
    require_detail("auditSummary", &parsed.audit_summary)?;
    require_detail("structurePlan", &parsed.structure_plan)?;
    require_detail("userApproval", &parsed.user_approval)?;
    if parsed.transfer_ownership && !parsed.install_after_export {
        return Err(ToolResult {
            output: "plugin_export transferOwnership requires installAfterExport.".to_string(),
            is_error: true,
        });
    }
    Ok(parsed)
}

fn tool_error(message: impl Into<String>) -> ToolResult {
    ToolResult {
        output: message.into(),
        is_error: true,
    }
}

fn json_output<T: Serialize>(tool_name: &str, value: &T) -> ToolResult {
    match serde_json::to_string_pretty(value) {
        Ok(output) => ToolResult {
            output,
            is_error: false,
        },
        Err(error) => tool_error(format!(
            "Failed to serialize {} result: {}",
            tool_name, error
        )),
    }
}

fn context_working_dir(
    ctx: &crate::tool::ToolExecutionContext,
    override_value: Option<String>,
) -> String {
    override_value
        .or_else(|| ctx.working_dir.clone())
        .unwrap_or_default()
}

fn plugin_summary_matches(summary: &crate::commands::PluginRegistrySummary, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return false;
    }
    let mut haystack = vec![
        summary.id.to_ascii_lowercase(),
        summary.name.to_ascii_lowercase(),
        summary.summary.to_ascii_lowercase(),
        summary.author.to_ascii_lowercase(),
    ];
    haystack.extend(summary.tags.iter().map(|tag| tag.to_ascii_lowercase()));
    haystack.extend(
        summary
            .summary_i18n
            .values()
            .map(|value| value.to_ascii_lowercase()),
    );
    haystack.iter().any(|value| value.contains(&query))
}

fn plugin_summary_score(summary: &crate::commands::PluginRegistrySummary, query: &str) -> u8 {
    let query = query.trim().to_ascii_lowercase();
    if summary.id.eq_ignore_ascii_case(&query) {
        return 5;
    }
    if summary.name.eq_ignore_ascii_case(&query) {
        return 4;
    }
    if summary.id.to_ascii_lowercase().contains(&query) {
        return 3;
    }
    if summary.name.to_ascii_lowercase().contains(&query) {
        return 2;
    }
    1
}

fn registry_entry_to_summary(
    entry: crate::commands::PluginRegistryEntry,
) -> crate::commands::PluginRegistrySummary {
    crate::commands::PluginRegistrySummary {
        id: entry.id,
        name: entry.name,
        summary: entry.summary,
        summary_i18n: entry.summary_i18n,
        author: entry.author,
        tags: entry.tags,
        latest_version: entry.latest_version,
        updated_at: entry.updated_at,
        icon: entry.icon,
        stats: entry.stats,
        compatibility: entry.compatibility,
    }
}

fn source_from_install_args(
    parsed: &PluginInstallToolArgs,
) -> Option<crate::commands::PluginDownloadSource> {
    if let Some(source) = parsed.source.clone() {
        let mut source = source;
        if source.id.trim().is_empty() {
            source.id = parsed.expected_plugin_id.clone().unwrap_or_default();
        }
        if source.version.trim().is_empty() {
            source.version = parsed.version.clone().unwrap_or_default();
        }
        return Some(source);
    }
    let expected_plugin_id = parsed.expected_plugin_id.clone().unwrap_or_default();
    if let Some(path) = parsed
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(crate::commands::PluginDownloadSource {
            kind: "local".to_string(),
            id: expected_plugin_id.clone(),
            input: path.to_string(),
            version: parsed.version.clone().unwrap_or_default(),
            ..Default::default()
        });
    }
    if let Some(url) = parsed
        .url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(crate::commands::PluginDownloadSource {
            kind: "url".to_string(),
            id: expected_plugin_id.clone(),
            url: url.to_string(),
            sha256: parsed.sha256.clone().unwrap_or_default(),
            version: parsed.version.clone().unwrap_or_default(),
            ..Default::default()
        });
    }
    if let Some(repo) = parsed
        .repo
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let mut source = crate::commands::PluginDownloadSource {
            kind: "repo".to_string(),
            id: expected_plugin_id,
            repo: repo.to_string(),
            ref_name: parsed.ref_name.clone().unwrap_or_default(),
            branch: parsed.branch.clone().unwrap_or_default(),
            tag: parsed.tag.clone().unwrap_or_default(),
            commit: parsed.commit.clone().unwrap_or_default(),
            asset: parsed.asset.clone().unwrap_or_default(),
            asset_pattern: parsed.asset_pattern.clone().unwrap_or_default(),
            sha256: parsed.sha256.clone().unwrap_or_default(),
            version: parsed.version.clone().unwrap_or_default(),
            ..Default::default()
        };
        if !source.asset.trim().is_empty() || !source.asset_pattern.trim().is_empty() {
            source.kind = "release".to_string();
        } else if !source.branch.trim().is_empty() {
            source.kind = "branch".to_string();
        } else if !source.tag.trim().is_empty() {
            source.kind = "tag".to_string();
        } else if !source.commit.trim().is_empty() {
            source.kind = "commit".to_string();
        }
        return Some(source);
    }
    None
}

async fn reload_plugin_registries_after_install(
    ctx: &crate::tool::ToolExecutionContext,
    working_dir: &str,
    source: &str,
    tool_name: &str,
) -> Result<(), ToolResult> {
    let app_handle = ctx
        .app_handle
        .clone()
        .ok_or_else(|| tool_error(format!("{} requires an application context", tool_name)))?;
    let registry = app_handle.state::<crate::AgentDefRegistryState>();
    let app_agent_dir = app_handle.state::<crate::AppAgentDir>();
    crate::commands::reload_agent_registry(&registry, &app_agent_dir, working_dir).await;
    crate::commands::emit_plugins_changed(&app_handle, working_dir, source);
    Ok(())
}

fn resolve_installed_plugin_scope(
    tool_name: &str,
    working_dir: &str,
    plugin_id: &str,
    requested_scope: Option<crate::plugin::PluginInstallScope>,
) -> Result<crate::plugin::PluginInstallScope, ToolResult> {
    if let Some(scope) = requested_scope {
        return Ok(scope);
    }

    let normalized_id = crate::plugin::normalize_plugin_id(plugin_id).map_err(tool_error)?;
    let mut scopes = crate::plugin::list_installed_plugin_summaries(working_dir)
        .into_iter()
        .filter(|plugin| plugin.id == normalized_id)
        .map(|plugin| plugin.scope)
        .collect::<Vec<_>>();
    scopes.sort();
    scopes.dedup();

    match scopes.as_slice() {
        [scope] => Ok(*scope),
        [] => Err(tool_error(format!(
            "{} could not find installed plugin: {}",
            tool_name, normalized_id
        ))),
        _ => Err(tool_error(format!(
            "{} requires scope because '{}' is installed in multiple scopes",
            tool_name, normalized_id
        ))),
    }
}

pub(super) fn plugin_list() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_LIST);
    ToolDef {
        name: "plugin_list".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let parsed = match serde_json::from_value::<PluginListToolArgs>(args) {
                    Ok(value) => value,
                    Err(error) => {
                        return tool_error(format!(
                            "Error parsing plugin_list arguments: {}",
                            error
                        ));
                    }
                };
                let working_dir = context_working_dir(&ctx, parsed.working_dir);
                let plugins = crate::plugin::list_installed_plugin_summaries(&working_dir);
                json_output(
                    "plugin_list",
                    &PluginListToolOutput {
                        working_dir,
                        count: plugins.len(),
                        plugins,
                    },
                )
            })
        }),
    }
}

pub(super) fn plugin_search() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_SEARCH);
    ToolDef {
        name: "plugin_search".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: false,
        execute: make_exec(|args, _ctx| {
            Box::pin(async move {
                let parsed = match serde_json::from_value::<PluginSearchToolArgs>(args) {
                    Ok(value) => value,
                    Err(error) => {
                        return tool_error(format!(
                            "Error parsing plugin_search arguments: {}",
                            error
                        ));
                    }
                };
                let query = parsed.query.trim().to_string();
                if query.is_empty() {
                    return tool_error("plugin_search requires a non-empty query");
                }
                let limit = parsed.limit.unwrap_or(20).clamp(1, 50);
                let manifest_result = match crate::commands::plugin_registry_fetch_manifest(
                    parsed.registry_base_url,
                    None,
                )
                .await
                {
                    Ok(value) => value,
                    Err(error) => {
                        return tool_error(format!(
                            "Failed to fetch plugin registry manifest: {}",
                            error
                        ));
                    }
                };
                let base_url = manifest_result.base_url.clone();
                let summary_base_path = manifest_result.manifest.summary_base_path.clone();
                let entry_base_path = manifest_result.manifest.entry_base_path.clone();
                let search_index_path = manifest_result.manifest.search_index_path.clone();
                let mut results = Vec::new();
                let mut failed_buckets = Vec::new();
                let mut search_index_used = false;

                if let Ok(index) = crate::commands::plugin_registry_fetch_search_index(
                    Some(base_url.clone()),
                    Some(search_index_path),
                    None,
                )
                .await
                {
                    search_index_used = true;
                    results.extend(
                        index
                            .plugins
                            .into_iter()
                            .filter(|summary| plugin_summary_matches(summary, &query)),
                    );
                } else {
                    for bucket in &manifest_result.manifest.available_buckets {
                        match crate::commands::plugin_registry_fetch_shard(
                            Some(base_url.clone()),
                            Some(summary_base_path.clone()),
                            bucket.clone(),
                            None,
                        )
                        .await
                        {
                            Ok(shard) => {
                                results.extend(
                                    shard
                                        .plugins
                                        .into_iter()
                                        .filter(|summary| plugin_summary_matches(summary, &query)),
                                );
                            }
                            Err(error) => failed_buckets.push(PluginSearchShardError {
                                bucket: bucket.clone(),
                                error: error.to_string(),
                            }),
                        }
                    }
                }

                if results.is_empty() {
                    if let Ok(entry) = crate::commands::plugin_registry_fetch_plugin(
                        Some(base_url.clone()),
                        Some(entry_base_path),
                        query.clone(),
                        None,
                    )
                    .await
                    {
                        results.push(registry_entry_to_summary(entry));
                    }
                }

                results.sort_by(|a, b| {
                    plugin_summary_score(b, &query)
                        .cmp(&plugin_summary_score(a, &query))
                        .then_with(|| a.id.cmp(&b.id))
                });
                results.dedup_by(|a, b| a.id == b.id);
                results.truncate(limit);

                json_output(
                    "plugin_search",
                    &PluginSearchToolOutput {
                        query,
                        registry_base_url: base_url,
                        registry_version: manifest_result.manifest.registry_version,
                        updated_at: manifest_result.manifest.updated_at,
                        scanned_buckets: if search_index_used {
                            0
                        } else {
                            manifest_result.manifest.available_buckets.len()
                        },
                        search_index_used,
                        failed_buckets,
                        result_count: results.len(),
                        results,
                    },
                )
            })
        }),
    }
}

pub(super) fn plugin_install() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_INSTALL);
    ToolDef {
        name: "plugin_install".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Project-scope installs write plugin files into the workspace.
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let parsed = match serde_json::from_value::<PluginInstallToolArgs>(args) {
                    Ok(value) => value,
                    Err(error) => {
                        return tool_error(format!(
                            "Error parsing plugin_install arguments: {}",
                            error
                        ));
                    }
                };
                let scope = parsed
                    .scope
                    .unwrap_or(crate::plugin::PluginInstallScope::App);
                let working_dir = ctx.working_dir.clone().unwrap_or_default();
                if scope == crate::plugin::PluginInstallScope::Project
                    && working_dir.trim().is_empty()
                {
                    return tool_error("plugin_install requires a workspace for project scope");
                }

                let plugin_id = parsed
                    .plugin_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);
                let source = source_from_install_args(&parsed);
                if plugin_id.is_some() && source.is_some() {
                    return tool_error(
                        "plugin_install accepts either pluginId or source fields, not both",
                    );
                }

                let (source_label, registry_base_url, installed) =
                    if let Some(plugin_id) = plugin_id {
                        let manifest_result = match crate::commands::plugin_registry_fetch_manifest(
                            parsed.registry_base_url.clone(),
                            None,
                        )
                        .await
                        {
                            Ok(value) => value,
                            Err(error) => {
                                return tool_error(format!(
                                    "Failed to fetch plugin registry manifest: {}",
                                    error
                                ));
                            }
                        };
                        let entry = match crate::commands::plugin_registry_fetch_plugin(
                            Some(manifest_result.base_url.clone()),
                            Some(manifest_result.manifest.entry_base_path.clone()),
                            plugin_id.clone(),
                            None,
                        )
                        .await
                        {
                            Ok(value) => value,
                            Err(error) => {
                                return tool_error(format!(
                                    "Failed to fetch plugin registry entry '{}': {}",
                                    plugin_id, error
                                ));
                            }
                        };
                        let request = crate::commands::PluginRegistryInstallRequest {
                            id: entry.id.clone(),
                            latest_version: entry.latest_version,
                            download: entry.download,
                            download_source: entry.download_source,
                        };
                        let installed = match crate::commands::install_plugin_from_registry_request(
                            &working_dir,
                            request,
                            scope,
                        )
                        .await
                        {
                            Ok(value) => value,
                            Err(error) => return tool_error(error),
                        };
                        (
                            format!("registry:{}", entry.id),
                            Some(manifest_result.base_url),
                            installed,
                        )
                    } else if let Some(source) = source {
                        let installed = match crate::commands::install_plugin_from_download_source(
                            &working_dir,
                            source,
                            scope,
                        )
                        .await
                        {
                            Ok(value) => value,
                            Err(error) => return tool_error(error),
                        };
                        ("source".to_string(), None, installed)
                    } else {
                        return tool_error(
                            "plugin_install requires pluginId, source, path, url, or repo",
                        );
                    };

                if let Err(result) = reload_plugin_registries_after_install(
                    &ctx,
                    &working_dir,
                    "plugin_tool_install",
                    "plugin_install",
                )
                .await
                {
                    return result;
                }

                json_output(
                    "plugin_install",
                    &PluginInstallToolOutput {
                        source: source_label,
                        registry_base_url,
                        installed,
                    },
                )
            })
        }),
    }
}

pub(super) fn plugin_set_enabled() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_SET_ENABLED);
    ToolDef {
        name: "plugin_set_enabled".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let parsed = match serde_json::from_value::<PluginSetEnabledToolArgs>(args) {
                    Ok(value) => value,
                    Err(error) => {
                        return tool_error(format!(
                            "Error parsing plugin_set_enabled arguments: {}",
                            error
                        ));
                    }
                };
                let plugin_id = parsed.plugin_id.trim().to_string();
                if plugin_id.is_empty() {
                    return tool_error("plugin_set_enabled requires a non-empty pluginId");
                }
                let working_dir = ctx.working_dir.clone().unwrap_or_default();
                let scope = match resolve_installed_plugin_scope(
                    "plugin_set_enabled",
                    &working_dir,
                    &plugin_id,
                    parsed.scope,
                ) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                if scope == crate::plugin::PluginInstallScope::Project
                    && working_dir.trim().is_empty()
                {
                    return tool_error("plugin_set_enabled requires a workspace for project scope");
                }
                if ctx.app_handle.is_none() {
                    return tool_error("plugin_set_enabled requires an application context");
                }

                let plugin = match crate::plugin::set_plugin_enabled_sync(
                    &working_dir,
                    &plugin_id,
                    scope,
                    parsed.enabled,
                ) {
                    Ok(value) => value,
                    Err(error) => return tool_error(error),
                };

                if let Err(result) = reload_plugin_registries_after_install(
                    &ctx,
                    &working_dir,
                    if parsed.enabled {
                        "plugin_tool_enable"
                    } else {
                        "plugin_tool_disable"
                    },
                    "plugin_set_enabled",
                )
                .await
                {
                    return result;
                }

                json_output(
                    "plugin_set_enabled",
                    &PluginSetEnabledToolOutput {
                        working_dir,
                        plugin_id,
                        scope,
                        enabled: parsed.enabled,
                        plugin,
                    },
                )
            })
        }),
    }
}

pub(super) fn plugin_uninstall() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_UNINSTALL);
    ToolDef {
        name: "plugin_uninstall".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let parsed = match serde_json::from_value::<PluginUninstallToolArgs>(args) {
                    Ok(value) => value,
                    Err(error) => {
                        return tool_error(format!(
                            "Error parsing plugin_uninstall arguments: {}",
                            error
                        ));
                    }
                };
                let plugin_id = parsed.plugin_id.trim().to_string();
                if plugin_id.is_empty() {
                    return tool_error("plugin_uninstall requires a non-empty pluginId");
                }
                let working_dir = ctx.working_dir.clone().unwrap_or_default();
                let scope = match resolve_installed_plugin_scope(
                    "plugin_uninstall",
                    &working_dir,
                    &plugin_id,
                    parsed.scope,
                ) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                if scope == crate::plugin::PluginInstallScope::Project
                    && working_dir.trim().is_empty()
                {
                    return tool_error("plugin_uninstall requires a workspace for project scope");
                }
                if ctx.app_handle.is_none() {
                    return tool_error("plugin_uninstall requires an application context");
                }

                let removed =
                    match crate::plugin::uninstall_plugin_sync(&working_dir, &plugin_id, scope) {
                        Ok(value) => value,
                        Err(error) => return tool_error(error),
                    };

                if let Err(result) = reload_plugin_registries_after_install(
                    &ctx,
                    &working_dir,
                    "plugin_tool_uninstall",
                    "plugin_uninstall",
                )
                .await
                {
                    return result;
                }

                json_output(
                    "plugin_uninstall",
                    &PluginUninstallToolOutput {
                        working_dir,
                        plugin_id,
                        scope,
                        removed,
                    },
                )
            })
        }),
    }
}

pub(super) fn plugin_export() -> ToolDef {
    let prompt = crate::prompt::parse_tool_prompt(crate::prompt::tools::PLUGIN_EXPORT);
    ToolDef {
        name: "plugin_export".to_string(),
        description: prompt.description,
        parameters: prompt.parameters,
        // Ownership transfer and install-after-export rewrite workspace files.
        mutates_workspace: true,
        execute: make_exec(|args, ctx| {
            Box::pin(async move {
                let parsed = match parse_args(args) {
                    Ok(value) => value,
                    Err(result) => return result,
                };
                let request = crate::commands::PluginExportRequest {
                    id: parsed.id,
                    name: parsed.name,
                    version: parsed.version,
                    file_path: parsed.file_path,
                    skill_package_ids: parsed.skill_package_ids,
                    view_ids: parsed.view_ids,
                    rule_files: parsed.rule_files,
                    project_dependencies: parsed.project_dependencies,
                    install_after_export: parsed.install_after_export,
                    install_scope: parsed.install_scope,
                    transfer_ownership: parsed.transfer_ownership,
                };
                let working_dir = ctx.working_dir.clone().unwrap_or_default();
                match crate::commands::export_plugin_archive_sync(&working_dir, request) {
                    Ok(result) => match serde_json::to_string_pretty(&result) {
                        Ok(output) => {
                            if result.installed_plugin.is_some()
                                || !result.transferred_components.is_empty()
                            {
                                if let Err(refresh_result) = reload_plugin_registries_after_install(
                                    &ctx,
                                    &working_dir,
                                    "plugin_export",
                                    "plugin_export",
                                )
                                .await
                                {
                                    return refresh_result;
                                }
                            }
                            ToolResult {
                                output,
                                is_error: false,
                            }
                        }
                        Err(error) => ToolResult {
                            output: format!("Failed to serialize plugin_export result: {}", error),
                            is_error: true,
                        },
                    },
                    Err(error) => ToolResult {
                        output: error,
                        is_error: true,
                    },
                }
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool::{ToolExecutionContext, ToolRegistry};

    fn write_plugin_manifest(root: &std::path::Path, id: &str) {
        std::fs::create_dir_all(root).expect("create plugin root");
        std::fs::write(
            root.join(crate::plugin::PLUGIN_MANIFEST_FILE_NAME),
            serde_json::json!({
                "schemaVersion": 1,
                "id": id,
                "name": id,
                "version": "1.0.0",
                "components": {
                    "agents": [],
                    "rules": [],
                    "skills": [],
                    "views": []
                }
            })
            .to_string(),
        )
        .expect("write plugin manifest");
    }

    #[test]
    fn plugin_tools_are_skill_loaded() {
        let registry = ToolRegistry::with_builtins();
        for tool_name in [
            "plugin_list",
            "plugin_search",
            "plugin_install",
            "plugin_set_enabled",
            "plugin_uninstall",
            "plugin_export",
        ] {
            assert_eq!(
                registry.default_load_mode(tool_name),
                crate::tool::ToolLoadMode::Skill
            );
        }
    }

    #[test]
    fn plugin_uninstall_scope_uses_requested_scope() {
        let scope = resolve_installed_plugin_scope(
            "plugin_uninstall",
            "",
            "scope-demo",
            Some(crate::plugin::PluginInstallScope::App),
        )
        .expect("resolve requested scope");

        assert_eq!(scope, crate::plugin::PluginInstallScope::App);
    }

    #[test]
    fn plugin_uninstall_scope_infers_single_installed_scope() {
        let workspace = tempfile::tempdir().expect("workspace");
        let plugin_root = workspace
            .path()
            .join(crate::plugin::PROJECT_PLUGINS_RELATIVE)
            .join("scope-demo");
        write_plugin_manifest(&plugin_root, "scope-demo");

        let scope = resolve_installed_plugin_scope(
            "plugin_uninstall",
            &workspace.path().to_string_lossy(),
            "scope-demo",
            None,
        )
        .expect("resolve installed scope");

        assert_eq!(scope, crate::plugin::PluginInstallScope::Project);
    }

    #[tokio::test]
    async fn plugin_export_tool_requires_audit_structure_and_approval() {
        let tool = plugin_export();
        let result = (tool.execute)(
            serde_json::json!({
                "id": "com.example.demo",
                "name": "Demo",
                "version": "0.1.0",
                "filePath": "demo.zip",
                "skillPackageIds": [],
                "viewIds": ["demo-view"],
                "projectDependencies": [],
                "auditSummary": "short",
                "structurePlan": "short",
                "userApproval": "short"
            }),
            ToolExecutionContext::default(),
        )
        .await;
        assert!(result.is_error);
        assert!(result.output.contains("auditSummary"));
    }
}
