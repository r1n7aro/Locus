//! Configuration registry – exposes every Locus setting with its description,
//! storage location, and current value so that an agent tool (`config_query`)
//! can explain Locus configuration to the user.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tauri::Manager;

use crate::error::AppError;
use crate::keychain;
use crate::workspace::Workspace;

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

pub fn collect_all(app_handle: &tauri::AppHandle) -> Result<Vec<ConfigEntry>, AppError> {
    let mut entries = Vec::with_capacity(40);

    collect_general(&mut entries);
    collect_display(app_handle, &mut entries);
    collect_notifications(&mut entries);
    collect_api(&mut entries);
    collect_models(&mut entries);
    collect_permissions(app_handle, &mut entries);
    collect_knowledge(app_handle, &mut entries);

    Ok(entries)
}

pub fn collect_by_category(
    app_handle: &tauri::AppHandle,
    category: &str,
) -> Result<Vec<ConfigEntry>, AppError> {
    let mut entries = Vec::new();
    match category {
        "general" => collect_general(&mut entries),
        "display" => collect_display(app_handle, &mut entries),
        "notifications" => collect_notifications(&mut entries),
        "api" => collect_api(&mut entries),
        "models" => collect_models(&mut entries),
        "permissions" => collect_permissions(app_handle, &mut entries),
        "knowledge" => collect_knowledge(app_handle, &mut entries),
        _ => {
            return Err(AppError::new(
                "invalid_category",
                format!(
                    "Unknown category '{}'. Valid: general, display, notifications, api, models, permissions, knowledge",
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
        key: "general.language".into(),
        category: "general".into(),
        label: "Interface Language".into(),
        description:
            "UI language (zh / en). Stored in the WebView; not accessible from Rust or agent code."
                .into(),
        storage: "localStorage: locus-locale".into(),
        current_value: "(frontend-only)".into(),
    });
    out.push(ConfigEntry {
        key: "general.onboarding_completed".into(),
        category: "general".into(),
        label: "Onboarding Completed".into(),
        description: "Whether the first-run onboarding wizard has been completed.".into(),
        storage: "localStorage: locus-onboarding-completed".into(),
        current_value: "(frontend-only)".into(),
    });
}

// ── display ──────────────────────────────────────────────────────────────────

fn collect_display(app_handle: &tauri::AppHandle, out: &mut Vec<ConfigEntry>) {
    out.push(ConfigEntry {
        key: "display.theme.main_window".into(),
        category: "display".into(),
        label: "Main Window Theme".into(),
        description: "Color theme for the main window: system, light, or dark.".into(),
        storage: "localStorage: locus-theme-preference".into(),
        current_value: "(frontend-only)".into(),
    });
    out.push(ConfigEntry {
        key: "display.theme.unity_embed_window".into(),
        category: "display".into(),
        label: "Unity Embedded Window Theme".into(),
        description:
            "Color theme for the Unity embedded window: system, light, or dark. Default is dark."
                .into(),
        storage: "localStorage: locus-unity-embed-theme-preference".into(),
        current_value: "(frontend-only)".into(),
    });
    let view_windows_above_main = app_handle
        .try_state::<Arc<crate::config::AppConfig>>()
        .map(|config| config.view_windows_above_main_enabled())
        .unwrap_or(false);
    out.push(ConfigEntry {
        key: "display.view_windows_above_main".into(),
        category: "display".into(),
        label: "Keep View Windows Above Main Window".into(),
        description: "Open View windows as owned windows so they stay above the Locus main window."
            .into(),
        storage: "persistent_config_dir/config.json → view_windows_above_main".into(),
        current_value: view_windows_above_main.to_string(),
    });
    let view_open_in_existing_window = app_handle
        .try_state::<Arc<crate::config::AppConfig>>()
        .map(|config| config.view_open_in_existing_window_enabled())
        .unwrap_or(true);
    out.push(ConfigEntry {
        key: "display.view_open_in_existing_window".into(),
        category: "display".into(),
        label: "Open New Views In Existing Window".into(),
        description: "Open a new View as a tab in an existing View window when one is available."
            .into(),
        storage: "persistent_config_dir/config.json → view_open_in_existing_window".into(),
        current_value: view_open_in_existing_window.to_string(),
    });

    for (key_suffix, label, desc) in [
        (
            "todo_auto_open",
            "Auto-open TODO Panel",
            "Automatically open the TODO panel when new items appear.",
        ),
        (
            "changes_auto_open",
            "Auto-open Changes Panel",
            "Automatically open the file-changes panel when files change.",
        ),
        (
            "changes_auto_close",
            "Auto-close Changes Panel",
            "Automatically close the file-changes panel when a new tool-call round starts.",
        ),
    ] {
        out.push(ConfigEntry {
            key: format!("display.{}", key_suffix),
            category: "display".into(),
            label: label.into(),
            description: desc.into(),
            storage: "localStorage: locus-display-settings".into(),
            current_value: "(frontend-only)".into(),
        });
    }

    for (slot, label) in [
        ("ui", "UI Font"),
        ("prose", "Prose Font"),
        ("mono_inline", "Mono Inline Font"),
        ("mono_block", "Mono Block Font"),
        ("mono_editor", "Mono Editor Font"),
    ] {
        out.push(ConfigEntry {
            key: format!("display.font.{}", slot),
            category: "display".into(),
            label: label.into(),
            description: format!(
                "Custom font family for the {} slot. Empty string means system default.",
                slot
            ),
            storage: "localStorage: locus-display-settings → fonts".into(),
            current_value: "(frontend-only)".into(),
        });
    }
}

// ── notifications ────────────────────────────────────────────────────────────

fn collect_notifications(out: &mut Vec<ConfigEntry>) {
    for (key_suffix, label, desc) in [
        (
            "system_notifications_enabled",
            "Background System Notifications",
            "Enable desktop notifications for key chat events while the app is unfocused.",
        ),
        (
            "notify_on_chat_done",
            "Notify on Chat Complete",
            "Send a desktop notification when a chat run completes.",
        ),
        (
            "notify_on_subagent_done",
            "Notify on Sub-agent Complete",
            "Send a desktop notification when a sub-agent run completes.",
        ),
        (
            "notify_on_ask_user",
            "Notify on User Input Request",
            "Send a desktop notification when the agent asks the user for input.",
        ),
        (
            "notify_on_chat_error",
            "Notify on Chat Error",
            "Send a desktop notification when a chat run fails.",
        ),
        (
            "notify_on_tool_confirm",
            "Notify on Tool Approval",
            "Send a desktop notification when a tool action waits for approval.",
        ),
        (
            "sound_alerts_enabled",
            "Sound Alerts",
            "Enable sound alerts for key chat events.",
        ),
        (
            "sound_alert_mode",
            "Sound Alert Mode",
            "Sound profile used for sound alerts: soft, bright, or urgent.",
        ),
        (
            "sound_alert_source",
            "Sound Alert Source",
            "Sound source used for sound alerts: built-in or custom.",
        ),
        (
            "sound_alert_custom_file_path",
            "Custom Sound File",
            "Audio file path used when custom sound alerts are selected.",
        ),
        (
            "sound_alert_volume",
            "Sound Alert Volume",
            "Volume used for built-in and custom sound alerts.",
        ),
        (
            "sound_on_chat_done",
            "Sound on Chat Complete",
            "Play a sound when a chat run completes.",
        ),
        (
            "sound_on_subagent_done",
            "Sound on Sub-agent Complete",
            "Play a sound when a sub-agent run completes.",
        ),
        (
            "sound_on_ask_user",
            "Sound on User Input Request",
            "Play a sound when the agent asks the user for input.",
        ),
        (
            "sound_on_chat_error",
            "Sound on Chat Error",
            "Play a sound when a chat run fails.",
        ),
        (
            "sound_on_tool_confirm",
            "Sound on Tool Approval",
            "Play a sound when a tool action waits for approval.",
        ),
    ] {
        out.push(ConfigEntry {
            key: format!("notifications.{}", key_suffix),
            category: "notifications".into(),
            label: label.into(),
            description: desc.into(),
            storage: "localStorage: locus-display-settings".into(),
            current_value: "(frontend-only)".into(),
        });
    }
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

    let codex_transport = crate::commands::load_codex_model_config()
        .map(|config| match config.transport {
            crate::commands::CodexTransportMode::Http => "http",
            crate::commands::CodexTransportMode::Websocket => "websocket",
        })
        .unwrap_or("http");

    out.push(ConfigEntry {
        key: "api.codex_transport".into(),
        category: "api".into(),
        label: "Codex Transport".into(),
        description:
            "Transport used for ChatGPT Codex models: current HTTP SSE route or OpenAI Responses websocket route."
                .into(),
        storage: "persistent_config_dir/codex_model_config.json → transport".into(),
        current_value: codex_transport.into(),
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

    let defaults: Option<serde_json::Value> = dir.as_ref().and_then(|d| {
        std::fs::read_to_string(d.join("model_defaults.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    });

    let main_model = defaults
        .as_ref()
        .and_then(|v| v.get("mainModel"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let plan_model = defaults
        .as_ref()
        .and_then(|v| v.get("planModel"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let subagent_models = defaults
        .as_ref()
        .and_then(|v| v.get("subagentModels"))
        .cloned()
        .unwrap_or(serde_json::Value::Object(Default::default()));

    out.push(ConfigEntry {
        key: "models.main_model".into(),
        category: "models".into(),
        label: "Main Model".into(),
        description:
            "Default model for the main chat session. Empty means reuse the last selected model, or fall back to the first available model."
                .into(),
        storage: "persistent_config_dir/model_defaults.json → mainModel".into(),
        current_value: if main_model.is_empty() {
            "(default)".into()
        } else {
            main_model
        },
    });

    out.push(ConfigEntry {
        key: "models.plan_model".into(),
        category: "models".into(),
        label: "Plan Model".into(),
        description: "Default model used during plan mode. Empty means use the main model.".into(),
        storage: "persistent_config_dir/model_defaults.json → planModel".into(),
        current_value: if plan_model.is_empty() {
            "(default)".into()
        } else {
            plan_model
        },
    });

    out.push(ConfigEntry {
        key: "models.subagent_models".into(),
        category: "models".into(),
        label: "Sub-agent Model Overrides".into(),
        description: "Per-agent model overrides. Keys are agent IDs, values are model IDs. Empty object means sub-agents inherit the current session model.".into(),
        storage: "persistent_config_dir/model_defaults.json → subagentModels".into(),
        current_value: serde_json::to_string(&subagent_models).unwrap_or_default(),
    });

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
        description: "The model ID selected in the most recent chat session.".into(),
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

fn collect_permissions(app_handle: &tauri::AppHandle, out: &mut Vec<ConfigEntry>) {
    // Read tool_permissions.json from the active app storage directory.
    let perms: HashMap<String, String> = crate::commands::resolve_runtime_storage_dir(app_handle)
        .ok()
        .and_then(|dir| std::fs::read_to_string(dir.join("tool_permissions.json")).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let tool_list = [
        ("read", "File reading"),
        ("grep", "Content search"),
        ("list", "Directory listing"),
        ("task", "Sub-agent delegation"),
        ("todowrite", "TODO list management"),
        ("ask_user_question", "Ask user a question"),
        ("sheet", "Confirm a multi-field form"),
        ("graph_view", "Show or edit an interactive graph"),
        ("write", "File creation (new files only)"),
        ("edit", "File editing (partial)"),
        ("bash", "Shell command execution"),
        ("web_fetch", "HTTP fetch from the web"),
        ("unity_execute", "Execute C# code in Unity"),
        ("unity_run_states", "Run Unity state-machine debugging flow"),
        ("unity_recompile", "Trigger Unity recompilation"),
        ("unity_ref_search", "Unity reference graph search"),
        ("code_find_references", "C# find references (Roslyn)"),
        ("code_goto_definition", "C# go to definition (Roslyn)"),
        ("code_symbol_search", "C# symbol search (Roslyn)"),
        ("unity_asset_search", "Unity asset search"),
        ("unity_yaml_list", "List Unity YAML hierarchy"),
        ("unity_yaml_search", "Search Unity YAML hierarchy"),
        ("unity_yaml_read", "Read Unity YAML detail"),
        ("knowledge_list", "Unified knowledge document listing"),
        ("knowledge_query", "Unified knowledge document search"),
        ("knowledge_read", "Unified knowledge entry read"),
        ("knowledge_create", "Unified knowledge entry create"),
        ("knowledge_delete", "Unified knowledge entry delete"),
        ("knowledge_move", "Unified knowledge entry move"),
        ("knowledge_edit", "Unified knowledge entry edit"),
    ];

    for (name, desc) in tool_list {
        let default_mode = match name {
            "write" | "edit" | "bash" | "web_fetch" | "unity_execute" | "unity_run_states" => "ask",
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
            storage: "app_storage_dir/tool_permissions.json".into(),
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
    ];

    for (name, label, desc) in behavior_list {
        let current = perms.get(name).map(|s| s.as_str()).unwrap_or("ask");

        out.push(ConfigEntry {
            key: format!("permissions.{}", name),
            category: "permissions".into(),
            label: label.into(),
            description: format!(
                "{} 'auto' = execute without confirmation, 'ask' = require user approval.",
                desc
            ),
            storage: "app_storage_dir/tool_permissions.json".into(),
            current_value: current.into(),
        });
    }
}

// ── knowledge ────────────────────────────────────────────────────────────────

fn collect_knowledge(app_handle: &tauri::AppHandle, out: &mut Vec<ConfigEntry>) {
    // Get project working dir for project-level configs
    let workspace: Option<tauri::State<Arc<Workspace>>> = app_handle.try_state();
    let working_dir = workspace.and_then(|_ws| {
        // We can't .await in sync context, so read the file directly
        crate::commands::resolve_runtime_storage_dir(app_handle)
            .ok()
            .and_then(|dir| std::fs::read_to_string(dir.join("working_dir.txt")).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    });

    let knowledge_root = working_dir
        .as_ref()
        .map(|wd| std::path::Path::new(wd).join("Locus").join("knowledge"));
    let knowledge_root_value = knowledge_root
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "(not set)".into());
    let knowledge_doc_count = working_dir
        .as_ref()
        .and_then(|wd| crate::knowledge_store::list_documents(wd, None, None).ok())
        .map(|docs| docs.len())
        .unwrap_or(0);
    let default_skill_package_namespace = app_handle
        .try_state::<Arc<crate::config::AppConfig>>()
        .map(|config| config.default_skill_package_namespace())
        .unwrap_or_default();

    out.push(ConfigEntry {
        key: "knowledge.documents.root".into(),
        category: "knowledge".into(),
        label: "Knowledge Root".into(),
        description: "Unified knowledge root for design, memory, skill, and reference documents."
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

    out.push(ConfigEntry {
        key: "knowledge.skill.default_package_namespace".into(),
        category: "knowledge".into(),
        label: "Default Skill Package Namespace".into(),
        description:
            "Optional author namespace for manually assigned published or shared Skill package IDs."
                .into(),
        storage: "persistent_config_dir/config.json → default_skill_package_namespace".into(),
        current_value: if default_skill_package_namespace.is_empty() {
            "(empty)".into()
        } else {
            default_skill_package_namespace
        },
    });
}
