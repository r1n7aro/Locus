//! Configuration registry – exposes every Locus setting with its description,
//! storage location, and current value so that an agent tool (`config_query`)
//! can explain Locus configuration to the user.

use std::collections::HashMap;

use serde::Serialize;

use crate::error::AppError;
use crate::keychain;

// ── ConfigEntry ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigEntry {
    /// Dot-separated key, e.g. `"models.main_model"`
    pub key: String,
    /// Category for filtering
    pub category: String,
    /// Human-readable label
    pub label: String,
    /// What this setting controls
    pub description: String,
    /// Where the value is persisted
    pub storage: String,
    /// Current value (JSON string, or status like `"configured"` / `"not set"`)
    pub current_value: String,
}

// ── collect ──────────────────────────────────────────────────────────────────

pub fn collect_all(
    workspace_scope: Option<&crate::workspace_service::ResolvedWorkspaceScope>,
) -> Result<Vec<ConfigEntry>, AppError> {
    let mut entries = Vec::with_capacity(40);

    collect_general(&mut entries);
    collect_api(&mut entries);
    collect_models(&mut entries);
    collect_permissions(&mut entries);
    if let Some(scope) = workspace_scope {
        collect_knowledge(scope, &mut entries);
    }
    collect_mcp(&mut entries);

    Ok(entries)
}

pub fn collect_by_category(
    category: &str,
    workspace_scope: Option<&crate::workspace_service::ResolvedWorkspaceScope>,
) -> Result<Vec<ConfigEntry>, AppError> {
    let mut entries = Vec::new();
    match category {
        "general" => collect_general(&mut entries),
        "api" => collect_api(&mut entries),
        "models" => collect_models(&mut entries),
        "permissions" => collect_permissions(&mut entries),
        "knowledge" => {
            let scope = workspace_scope.ok_or_else(|| {
                AppError::new(
                    "workspace.scope_required",
                    "Knowledge configuration inspection requires a checkout scope.",
                )
            })?;
            collect_knowledge(scope, &mut entries)
        }
        "mcp" => collect_mcp(&mut entries),
        _ => {
            return Err(AppError::new(
                "invalid_category",
                format!(
                    "Unknown category '{}'. Valid: general, api, models, permissions, knowledge, mcp",
                    category
                ),
            ));
        }
    }
    Ok(entries)
}

// ── general ──────────────────────────────────────────────────────────────────

fn collect_general(out: &mut Vec<ConfigEntry>) {
    out.push(ConfigEntry {
        key: "general.onboarding_completed".into(),
        category: "general".into(),
        label: "Onboarding Completed".into(),
        description: "Whether the first-run onboarding wizard has been completed.".into(),
        storage: "localStorage: locus-onboarding-completed".into(),
        current_value: "(frontend-only)".into(),
    });
}

// ── api ──────────────────────────────────────────────────────────────────────

fn collect_api(out: &mut Vec<ConfigEntry>) {
    let or_status = keychain::get_secret(keychain::KEY_OPENROUTER)
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(|_| "configured")
        .unwrap_or("not set");

    out.push(ConfigEntry {
        key: "api.openrouter".into(),
        category: "api".into(),
        label: "OpenRouter API Key".into(),
        description: "API key for OpenRouter. Stored securely in the OS keychain.".into(),
        storage: "keychain: openrouter_api_key".into(),
        current_value: or_status.into(),
    });

    let anthropic_status = keychain::get_secret("claude_tokens")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(|_| "configured")
        .unwrap_or("not set");

    out.push(ConfigEntry {
        key: "api.anthropic_oauth".into(),
        category: "api".into(),
        label: "Anthropic OAuth".into(),
        description: "Anthropic account linked via OAuth device flow. Token stored in OS keychain."
            .into(),
        storage: "keychain: claude_tokens".into(),
        current_value: anthropic_status.into(),
    });

    let codex_status = keychain::get_secret("codex_tokens")
        .ok()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(|_| "configured")
        .unwrap_or("not set");

    out.push(ConfigEntry {
        key: "api.codex".into(),
        category: "api".into(),
        label: "Codex Auth".into(),
        description:
            "Codex account linked via device authorization flow. Token stored in OS keychain."
                .into(),
        storage: "keychain: codex_tokens".into(),
        current_value: codex_status.into(),
    });

    // Custom endpoints – count only, no secrets
    let ep_count = crate::commands::persistent_config_dir()
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join("custom_endpoints.json")).ok())
        .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(&s).ok())
        .map(|v| v.len())
        .unwrap_or(0);

    out.push(ConfigEntry {
        key: "api.custom_endpoints".into(),
        category: "api".into(),
        label: "Custom Endpoints".into(),
        description: "User-defined API endpoints (OpenAI-compatible or Anthropic). API keys stored in OS keychain; endpoint metadata in persistent config dir.".into(),
        storage: "persistent_config_dir/custom_endpoints.json + keychain: endpoint/{id}".into(),
        current_value: format!("{} endpoint(s)", ep_count),
    });
}

// ── models ───────────────────────────────────────────────────────────────────

fn collect_models(out: &mut Vec<ConfigEntry>) {
    let dir = crate::commands::persistent_config_dir().ok();

    let last_model = dir
        .as_ref()
        .and_then(|d| std::fs::read_to_string(d.join("last_model.txt")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    out.push(ConfigEntry {
        key: "models.last_model".into(),
        category: "models".into(),
        label: "Last Used Model".into(),
        description: "The model ID most recently selected in any workspace; used for workspaces without a remembered choice.".into(),
        storage: "persistent_config_dir/last_model.txt".into(),
        current_value: if last_model.is_empty() {
            "(none)".into()
        } else {
            last_model
        },
    });

    let last_effort = dir
        .as_ref()
        .and_then(|d| std::fs::read_to_string(d.join("last_effort.txt")).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    out.push(ConfigEntry {
        key: "models.last_effort".into(),
        category: "models".into(),
        label: "Last Used Reasoning Effort".into(),
        description: "The reasoning effort selected in the most recent chat session.".into(),
        storage: "persistent_config_dir/last_effort.txt".into(),
        current_value: if last_effort.is_empty() {
            "(none)".into()
        } else {
            last_effort
        },
    });
}

// ── permissions ──────────────────────────────────────────────────────────────

/// Effective approval per tool and behavior, including defaults that
/// `permissions.tools` (overrides only) does not list.
fn collect_permissions(out: &mut Vec<ConfigEntry>) {
    let perms: HashMap<String, String> =
        crate::commands::tool_permissions_setting(crate::settings::global::global());

    let tool_list = [
        ("read", "File reading"),
        ("grep", "Content search"),
        ("list", "Directory listing"),
        ("subagent", "Sub-agent delegation"),
        ("todowrite", "TODO list management"),
        ("ask_user_question", "Ask user a question"),
        ("write", "File creation (new files only)"),
        ("edit", "File editing (partial)"),
        ("bash", "Shell command execution"),
        ("python", "Checkout-scoped Python and Locus SDK execution"),
        ("web_fetch", "HTTP fetch from the web"),
        (
            "unity_execute",
            "Execute C# in Unity with direct private/internal access",
        ),
        ("unity_set_play_mode", "Start or stop Unity Play Mode"),
        ("unity_test_list", "List tests from Unity Test Framework"),
        ("unity_test_run", "Run tests through Unity Test Framework"),
        (
            "unity_run_states",
            "Run Unity state-machine debugging with direct private/internal access",
        ),
        ("unity_recompile", "Trigger Unity recompilation"),
        ("unity_hot_reload", "Hot-patch C# method edits into Unity"),
        ("unity_ref_search", "Unity reference graph search"),
        ("code_find_references", "C# find references (Roslyn)"),
        ("code_goto_definition", "C# go to definition (Roslyn)"),
        ("code_symbol_search", "C# symbol search (Roslyn)"),
        ("code_diagnostics", "C# diagnostics (Roslyn)"),
        ("code_hover", "C# hover info (Roslyn)"),
        ("unity_code_usages", "Script usages in Unity assets"),
        ("unity_asset_search", "Unity asset search"),
        ("unity_yaml_search", "Search Unity Property Tree"),
        ("unity_yaml_read", "Progressively read Unity Property Tree"),
        ("knowledge_query", "Unified knowledge document search"),
    ];

    for (name, desc) in tool_list {
        let default_mode = match name {
            "write" | "edit" | "bash" | "python" | "web_fetch" | "unity_execute"
            | "unity_run_states" | "unity_test_run" | "subagent" => "ask",
            _ => "auto",
        };
        let current = perms.get(name).map(|s| s.as_str()).unwrap_or(default_mode);

        out.push(ConfigEntry {
            key: format!("permissions.{}", name),
            category: "permissions".into(),
            label: format!("Tool: {}", name),
            description: format!(
                "Permission mode for the '{}' tool ({}). 'auto' = execute without confirmation, 'ask' = require user approval.",
                name, desc
            ),
            storage: "settings.json → permissions.tools (setting key permissions.tools)".into(),
            current_value: current.into(),
        });
    }

    let behavior_list = [
        (
            "behavior.unity_editor_status_change",
            "Switch Unity Editor status",
            "Approval mode for changing Unity Editor status during tool execution.",
        ),
        (
            "behavior.knowledge_governance",
            "Edit protected knowledge",
            "Approval mode for protected knowledge changes, including Design, Skill, Reference, and approval-gated folders.",
        ),
        (
            crate::settings::agent::BEHAVIOR_SETTINGS_CHANGE,
            "Change settings",
            "Approval mode for config_set changes to normal Locus settings.",
        ),
        (
            crate::settings::agent::BEHAVIOR_SENSITIVE_SETTINGS_CHANGE,
            "Change sensitive settings",
            "Approval mode for config_set changes to sensitive Locus settings (debug, workspace boundary, compiler access, worktree root). Global auto mode does not skip this confirmation.",
        ),
    ];

    for (name, label, desc) in behavior_list {
        let default_mode = if name == crate::settings::agent::BEHAVIOR_SENSITIVE_SETTINGS_CHANGE {
            "ask"
        } else {
            "auto"
        };
        let current = perms.get(name).map(|s| s.as_str()).unwrap_or(default_mode);

        out.push(ConfigEntry {
            key: format!("permissions.{}", name),
            category: "permissions".into(),
            label: label.into(),
            description: format!(
                "{} 'auto' = execute without confirmation, 'ask' = require user approval.",
                desc
            ),
            storage: "settings.json → permissions.tools (setting key permissions.tools)".into(),
            current_value: current.into(),
        });
    }
}

// ── knowledge ────────────────────────────────────────────────────────────────

fn collect_knowledge(
    workspace_scope: &crate::workspace_service::ResolvedWorkspaceScope,
    out: &mut Vec<ConfigEntry>,
) {
    let working_dir = workspace_scope.runtime().root().to_string_lossy();
    let knowledge_root = workspace_scope
        .runtime()
        .root()
        .join("Locus")
        .join("knowledge");
    let knowledge_root_value = knowledge_root.display().to_string();
    let knowledge_doc_count = crate::knowledge_store::list_documents(&working_dir, None, None)
        .ok()
        .map(|docs| docs.len())
        .unwrap_or(0);

    out.push(ConfigEntry {
        key: "knowledge.documents.root".into(),
        category: "knowledge".into(),
        label: "Knowledge Root".into(),
        description:
            "Unified knowledge root for design, plan, memory, skill, and reference documents."
                .into(),
        storage: "<project>/Locus/knowledge".into(),
        current_value: knowledge_root_value,
    });

    out.push(ConfigEntry {
        key: "knowledge.documents.count".into(),
        category: "knowledge".into(),
        label: "Knowledge Documents".into(),
        description: "Number of knowledge documents currently indexed in this workspace.".into(),
        storage: "<project>/Locus/knowledge/**/*.md".into(),
        current_value: knowledge_doc_count.to_string(),
    });
}

// ── mcp ──────────────────────────────────────────────────────────────────────

fn collect_mcp(out: &mut Vec<ConfigEntry>) {
    let servers = crate::mcp::config::load_servers();
    let enabled = servers.iter().filter(|s| s.enabled).count();
    out.push(ConfigEntry {
        key: "mcp.servers".into(),
        category: "mcp".into(),
        label: "MCP Servers".into(),
        description:
            "External MCP servers whose tools are exposed to agents as mcp__<server>__<tool>. \
             Managed in Settings → MCP Servers; agents can edit the file via bash and apply it \
             with the mcp_reload tool."
                .into(),
        storage: "persistent_config_dir/mcp_servers.json".into(),
        current_value: format!("{} configured, {} enabled", servers.len(), enabled),
    });
    for server in servers {
        let connection = match server.transport {
            crate::mcp::config::McpTransport::Stdio => {
                let mut command = server.command.clone();
                if !server.args.is_empty() {
                    command.push(' ');
                    command.push_str(&server.args.join(" "));
                }
                format!("stdio: {command}")
            }
            crate::mcp::config::McpTransport::Http => format!("http: {}", server.url),
        };
        let mut facts = vec![
            connection,
            format!(
                "load mode {}",
                match server.load_mode {
                    crate::mcp::config::McpLoadMode::Lazy => "lazy",
                    crate::mcp::config::McpLoadMode::Direct => "direct",
                }
            ),
            format!("timeout {}s", server.call_timeout_ms / 1000),
        ];
        if server.auto_restart {
            facts.push("auto-restart on".into());
        }
        if !server.tool_allowlist.is_empty() {
            facts.push(format!("allowlist {} tools", server.tool_allowlist.len()));
        }
        if !server.tool_denylist.is_empty() {
            facts.push(format!("denylist {} tools", server.tool_denylist.len()));
        }
        out.push(ConfigEntry {
            key: format!("mcp.servers.{}", server.id),
            category: "mcp".into(),
            label: server.name.clone(),
            description: facts.join("; "),
            storage: "persistent_config_dir/mcp_servers.json".into(),
            current_value: if server.enabled {
                "enabled".into()
            } else {
                "disabled".into()
            },
        });
    }
}
