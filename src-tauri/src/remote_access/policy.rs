//! Which Tauri commands a paired device may call, and how.
//!
//! Every command the desktop frontend can invoke was classified for remote
//! use (table: `Locus/knowledge/plan/移动端远程命令覆盖.csv`):
//! - [`Tier::View`]: any paired device. Reads and chat-level actions (start or
//!   stop a run, answer a question, organize sessions).
//! - [`Tier::Control`]: devices with full control. Anything that changes
//!   files, git, knowledge or settings, or executes code.
//! - not listed: desktop only (windows, OS integration, desktop file
//!   dialogs, plain-text credentials, remote access administration). Unknown
//!   and new commands are therefore denied until classified here.
//!
//! The desktop allows some reads of any absolute path (it opens files the
//! agent mentions anywhere). For view-tier devices the listed path
//! arguments must stay inside an open workspace.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    View,
    Control,
}

#[derive(Debug)]
pub struct Rule {
    pub command: &'static str,
    pub tier: Tier,
    /// Argument selectors checked for view-tier devices: `name`,
    /// `object.field`, or `name[]` for an array of strings.
    pub paths: &'static [&'static str],
}

const fn rule(command: &'static str, tier: Tier, paths: &'static [&'static str]) -> Rule {
    Rule { command, tier, paths }
}

/// Sorted by command name (checked by a test).
const RULES: &[Rule] = &[
    rule("acquire_unity_project_slot", Tier::Control, &[]),
    rule("answer_question", Tier::View, &[]),
    rule("anthropic_rate_limits", Tier::View, &[]),
    rule("apply_knowledge_proposal", Tier::Control, &[]),
    rule("archive_session", Tier::View, &[]),
    rule("asset_db_light_status", Tier::View, &[]),
    rule("asset_db_overview", Tier::View, &[]),
    rule("asset_risk_report", Tier::View, &[]),
    rule("auth_logout", Tier::Control, &[]),
    rule("authorize_claude_ssh", Tier::Control, &[]),
    rule("backup_claude_native_session", Tier::Control, &[]),
    rule("bind_session_worktree", Tier::Control, &[]),
    rule("blender_add_installation", Tier::Control, &[]),
    rule("blender_background_instances", Tier::View, &[]),
    rule("blender_close_background", Tier::Control, &[]),
    rule("blender_install_addon", Tier::Control, &[]),
    rule("blender_launch", Tier::Control, &[]),
    rule("blender_list_installations", Tier::View, &[]),
    rule("blender_refresh_installations", Tier::Control, &[]),
    rule("blender_reload_hosts", Tier::Control, &[]),
    rule("blender_remove_installation", Tier::Control, &[]),
    rule("blender_set_default_installation", Tier::Control, &[]),
    rule("blender_set_session_target", Tier::Control, &[]),
    rule("blender_set_workspace_version", Tier::Control, &[]),
    rule("blender_unhide_installation", Tier::Control, &[]),
    rule("blender_uninstall_addon", Tier::Control, &[]),
    rule("blender_workspace_status", Tier::View, &[]),
    rule("cancel_background_task", Tier::View, &[]),
    rule("cancel_chat", Tier::View, &[]),
    rule("chat", Tier::View, &["knowledgeDocPath"]),
    rule("check_claude_session_transfer", Tier::View, &[]),
    rule("check_unity_connection", Tier::View, &[]),
    rule("check_unity_connection_status", Tier::View, &[]),
    rule("check_unity_plugin", Tier::View, &[]),
    rule("check_unity_plugin_install_plan", Tier::View, &[]),
    rule("clear_api_key", Tier::Control, &[]),
    rule("clear_app_temp_dir", Tier::Control, &[]),
    rule("clear_claude_ssh_unlock_password", Tier::Control, &[]),
    rule("clear_log_entries", Tier::Control, &[]),
    rule("close_headless_unity_project", Tier::Control, &[]),
    rule("code_analysis_tools_get_config", Tier::View, &[]),
    rule("code_analysis_tools_set_config", Tier::Control, &[]),
    rule("codex_consume_rate_limit_reset_credit", Tier::Control, &[]),
    rule("codex_logout", Tier::Control, &[]),
    rule("codex_poll_login", Tier::Control, &[]),
    rule("codex_rate_limits", Tier::View, &[]),
    rule("codex_retry_auth", Tier::Control, &[]),
    rule("codex_start_login", Tier::Control, &[]),
    rule("codex_status", Tier::View, &[]),
    rule("connect_claude_session", Tier::View, &[]),
    rule("create_session", Tier::View, &[]),
    rule("create_skill_scaffold", Tier::Control, &[]),
    rule("create_worktree", Tier::Control, &[]),
    rule("csharp_lsp_get_status", Tier::View, &[]),
    rule("csharp_lsp_restart", Tier::Control, &[]),
    rule("csharp_lsp_set_enabled", Tier::Control, &[]),
    rule("csv_file_relocate", Tier::Control, &[]),
    rule("csv_view_read", Tier::View, &["filePath"]),
    rule("csv_view_write", Tier::Control, &[]),
    rule("delete_pending_chat_input", Tier::View, &[]),
    rule("delete_provider_key", Tier::Control, &[]),
    rule("delete_rule", Tier::Control, &[]),
    rule("delete_session", Tier::Control, &[]),
    rule("delete_skill_package", Tier::Control, &[]),
    rule("diff_semantic_target", Tier::View, &[]),
    rule("diff_single_file", Tier::View, &["request.filePath", "request.oldPath"]),
    rule("diff_strings", Tier::View, &[]),
    rule("diff_text_for_large", Tier::View, &["request.filePath", "request.oldPath"]),
    rule("discover_worktrees", Tier::View, &["sourceRoot"]),
    rule("exchange_auth_code", Tier::Control, &[]),
    rule("explorer_file_action", Tier::Control, &[]),
    rule("extra_workdirs_get", Tier::View, &[]),
    rule("extra_workdirs_map", Tier::View, &[]),
    rule("extra_workdirs_set", Tier::Control, &[]),
    rule("fetch_app_update_manifest", Tier::View, &[]),
    rule("file_history", Tier::View, &["path"]),
    rule("file_history_preview", Tier::View, &["path"]),
    rule("file_history_restore", Tier::Control, &[]),
    rule("fork_session", Tier::View, &[]),
    rule("fork_session_from_message", Tier::View, &[]),
    rule("garbage_collection", Tier::Control, &[]),
    rule("get_agent_env_template", Tier::View, &[]),
    rule("get_agent_model_preferences", Tier::View, &[]),
    rule("get_agent_rendered_env_prompt", Tier::View, &[]),
    rule("get_agent_system_prompt", Tier::View, &[]),
    rule("get_agent_system_prompt_stats", Tier::View, &[]),
    rule("get_all_skill_configs", Tier::View, &[]),
    rule("get_all_worktree_pool_usage", Tier::View, &[]),
    rule("get_anthropic_native_lazy_enabled", Tier::View, &[]),
    rule("get_app_storage_info", Tier::View, &[]),
    rule("get_app_temp_info", Tier::View, &[]),
    rule("get_archived_checkout_storage_bytes", Tier::View, &[]),
    rule("get_auth_status", Tier::View, &[]),
    rule("get_auth_url", Tier::Control, &[]),
    rule("get_claude_agent_routes", Tier::View, &[]),
    rule("get_claude_code_cli_enabled", Tier::View, &[]),
    rule("get_claude_code_models", Tier::View, &[]),
    rule("get_claude_connection", Tier::View, &[]),
    rule("get_claude_native_agents_support", Tier::View, &[]),
    rule("get_claude_native_archive", Tier::View, &[]),
    rule("get_claude_session_connection", Tier::View, &["workingDir"]),
    rule("get_codex_available_models", Tier::View, &[]),
    rule("get_codex_fast_mode", Tier::View, &[]),
    rule("get_codex_model_config", Tier::View, &[]),
    rule("get_compacted_context_output", Tier::View, &[]),
    rule("get_config_registry", Tier::View, &[]),
    rule("get_custom_provider_catalog", Tier::View, &[]),
    rule("get_debug_mode", Tier::View, &[]),
    rule("get_default_skill_package_namespace", Tier::View, &[]),
    rule("get_dynamic_tool_loading_mode", Tier::View, &[]),
    rule("get_file_tool_workspace_boundary", Tier::View, &[]),
    rule("get_last_effort", Tier::View, &[]),
    rule("get_last_model", Tier::View, &[]),
    rule("get_llm_retry_max_attempts", Tier::View, &[]),
    rule("get_log_entries", Tier::Control, &[]),
    rule("get_model_catalog", Tier::View, &[]),
    rule("get_model_defaults", Tier::View, &[]),
    rule("get_model_usage_stats", Tier::View, &[]),
    rule("get_plan_file_content", Tier::View, &[]),
    rule("get_providers", Tier::View, &[]),
    rule("get_proxy_status", Tier::View, &[]),
    rule("get_python_runtime_state", Tier::View, &[]),
    rule("get_running_task_count", Tier::View, &[]),
    rule("get_service_switches", Tier::View, &[]),
    rule("get_session_active_run", Tier::View, &[]),
    rule("get_session_attention", Tier::View, &[]),
    rule("get_session_context_usage_report", Tier::View, &[]),
    rule("get_session_maintenance_status", Tier::View, &[]),
    rule("get_session_plan_state", Tier::View, &[]),
    rule("get_session_resume_available", Tier::View, &[]),
    rule("get_session_undo_enabled", Tier::View, &[]),
    rule("get_session_usage", Tier::View, &[]),
    rule("get_skill_config", Tier::View, &["relPath"]),
    rule("get_skill_unity_install_status", Tier::View, &[]),
    rule("get_subagent_max_concurrent", Tier::View, &[]),
    rule("get_subagent_max_depth", Tier::View, &[]),
    rule("get_system_locale", Tier::View, &[]),
    rule("get_todos", Tier::View, &[]),
    rule("get_tool_failure_log_enabled", Tier::View, &[]),
    rule("get_tool_permission_mode", Tier::View, &[]),
    rule("get_tool_permissions", Tier::View, &[]),
    rule("get_unity_background_hook_enabled", Tier::View, &[]),
    rule("get_unity_background_hook_status", Tier::View, &[]),
    rule("get_unity_console_text", Tier::View, &[]),
    rule("get_unity_editor_resources", Tier::View, &[]),
    rule("get_unity_embed_enabled", Tier::View, &[]),
    rule("get_unity_external_editor_default_enabled", Tier::View, &[]),
    rule("get_unity_multi_agent_editor_enabled", Tier::View, &[]),
    rule("get_unity_native_bridge_enabled", Tier::View, &[]),
    rule("get_unity_native_broker_status", Tier::View, &[]),
    rule("get_unity_semantic_state", Tier::View, &[]),
    rule("get_unity_state_probe_enabled", Tier::View, &[]),
    rule("get_unity_state_probe_status", Tier::View, &[]),
    rule("get_unity_test_tools_workspace_status", Tier::View, &[]),
    rule("get_watcher_tuning", Tier::View, &[]),
    rule("get_workspace_agent_env_template", Tier::View, &[]),
    rule("get_workspace_agent_rendered_env_prompt", Tier::View, &[]),
    rule("get_workspace_agent_system_prompt", Tier::View, &[]),
    rule("get_workspace_agent_system_prompt_stats", Tier::View, &[]),
    rule("get_workspace_config_registry", Tier::View, &[]),
    rule("get_workspace_selection", Tier::View, &[]),
    rule("get_workspace_service_resource_limits", Tier::View, &[]),
    rule("get_workspace_service_resource_metrics", Tier::View, &[]),
    rule("get_workspace_service_states", Tier::View, &[]),
    rule("get_worktree_defaults", Tier::View, &["sourceRoot"]),
    rule("get_worktree_operations", Tier::View, &["sourceRoot"]),
    rule("get_worktree_pool_usage", Tier::View, &["sourceRoot"]),
    rule("get_worktree_root_settings", Tier::View, &[]),
    rule("git_branch_action", Tier::Control, &[]),
    rule("git_branches", Tier::View, &[]),
    rule("git_check_user_config", Tier::View, &[]),
    rule("git_clear_override", Tier::Control, &[]),
    rule("git_commit", Tier::Control, &[]),
    rule("git_commit_action", Tier::Control, &[]),
    rule("git_commit_body", Tier::View, &[]),
    rule("git_commit_files", Tier::View, &[]),
    rule("git_compare_files", Tier::View, &[]),
    rule("git_config_snapshot", Tier::View, &[]),
    rule("git_discard_file", Tier::Control, &[]),
    rule("git_generate_commit_message", Tier::View, &[]),
    rule("git_head_hash", Tier::View, &[]),
    rule("git_history_search", Tier::View, &[]),
    rule("git_history_snapshot", Tier::View, &[]),
    rule("git_init_unity", Tier::Control, &[]),
    rule("git_install_help", Tier::View, &[]),
    rule("git_log", Tier::View, &[]),
    rule("git_merge_action", Tier::Control, &[]),
    rule("git_merge_apply", Tier::Control, &[]),
    rule("git_merge_file", Tier::Control, &[]),
    rule("git_merge_semantic_apply", Tier::Control, &[]),
    rule("git_merge_semantic_session", Tier::Control, &[]),
    rule("git_merge_semantic_target", Tier::Control, &[]),
    rule("git_merge_semantic_validate", Tier::Control, &[]),
    rule("git_probe", Tier::View, &[]),
    rule("git_runtime_state", Tier::View, &[]),
    rule("git_save_config", Tier::Control, &[]),
    rule("git_save_runtime_selection", Tier::Control, &[]),
    rule("git_set_user_config", Tier::Control, &[]),
    rule("git_stage", Tier::Control, &[]),
    rule("git_stage_all", Tier::Control, &[]),
    rule("git_stage_paths", Tier::Control, &[]),
    rule("git_stash_action", Tier::Control, &[]),
    rule("git_stashes", Tier::View, &[]),
    rule("git_status", Tier::View, &[]),
    rule("git_submodules", Tier::View, &[]),
    rule("git_unstage", Tier::Control, &[]),
    rule("git_unstage_all", Tier::Control, &[]),
    rule("git_unstage_paths", Tier::Control, &[]),
    rule("global_search", Tier::View, &[]),
    rule("ignore_knowledge_proposal", Tier::View, &[]),
    rule("import_claude_code_oauth", Tier::Control, &[]),
    rule("import_codex_cli", Tier::Control, &[]),
    rule("import_worktree", Tier::Control, &[]),
    rule("insert_pending_chat_input", Tier::View, &[]),
    rule("inspect_claude_ssh", Tier::Control, &[]),
    rule("install_skill_unity_files", Tier::Control, &[]),
    rule("install_unity_plugin", Tier::Control, &[]),
    rule("knowledge_activate_embedding", Tier::Control, &[]),
    rule("knowledge_cancel_feishu_reference_import", Tier::Control, &[]),
    rule("knowledge_cancel_local_embedding_model_download", Tier::Control, &[]),
    rule("knowledge_cancel_local_reference_import", Tier::Control, &[]),
    rule("knowledge_cancel_unity_reference_import", Tier::Control, &[]),
    rule("knowledge_create", Tier::Control, &[]),
    rule("knowledge_deactivate_embedding", Tier::Control, &[]),
    rule("knowledge_delete", Tier::Control, &[]),
    rule("knowledge_delete_external_reference_directory", Tier::Control, &[]),
    rule("knowledge_delete_feishu_reference_docs", Tier::Control, &[]),
    rule("knowledge_delete_local_reference_docs", Tier::Control, &[]),
    rule("knowledge_delete_unity_reference_docs", Tier::Control, &[]),
    rule("knowledge_document_source", Tier::View, &[]),
    rule("knowledge_download_local_embedding_model", Tier::Control, &[]),
    rule("knowledge_edit", Tier::Control, &[]),
    rule("knowledge_find_unity_reference_directory", Tier::View, &[]),
    rule("knowledge_get_embedding_config", Tier::View, &[]),
    rule("knowledge_get_embedding_status", Tier::View, &[]),
    rule("knowledge_get_feishu_reference_import_status", Tier::View, &["targetPath"]),
    rule("knowledge_get_general_config", Tier::View, &[]),
    rule("knowledge_get_lexical_rebuild_status", Tier::View, &[]),
    rule("knowledge_get_local_embedding_model_catalog", Tier::View, &[]),
    rule("knowledge_get_local_reference_import_status", Tier::View, &["targetPath"]),
    rule("knowledge_get_overview", Tier::View, &[]),
    rule("knowledge_get_unity_reference_import_status", Tier::View, &["targetPath"]),
    rule("knowledge_import_feishu_reference_docs", Tier::Control, &[]),
    rule("knowledge_import_unity_reference_docs", Tier::Control, &[]),
    rule("knowledge_list", Tier::View, &["pathPrefix"]),
    rule("knowledge_list_directories", Tier::View, &[]),
    rule("knowledge_list_directories_scoped", Tier::View, &[]),
    rule("knowledge_list_directory_documents", Tier::View, &["path"]),
    rule("knowledge_list_directory_documents_page", Tier::View, &["path"]),
    rule("knowledge_list_external_reference_directories", Tier::View, &[]),
    rule("knowledge_list_feishu_reference_space_nodes", Tier::View, &[]),
    rule("knowledge_list_page", Tier::View, &["pathPrefix"]),
    rule("knowledge_list_page_scoped", Tier::View, &["pathPrefix"]),
    rule("knowledge_list_scoped", Tier::View, &["pathPrefix"]),
    rule("knowledge_list_unity_managed_directory_stats", Tier::View, &[]),
    rule("knowledge_move", Tier::Control, &[]),
    rule("knowledge_query", Tier::View, &["pathPrefix"]),
    rule("knowledge_read", Tier::View, &[]),
    rule("knowledge_read_scoped", Tier::View, &[]),
    rule("knowledge_rebuild_lexical_index", Tier::Control, &[]),
    rule("knowledge_save_embedding_config", Tier::Control, &[]),
    rule("knowledge_save_feishu_reference_config", Tier::Control, &[]),
    rule("knowledge_save_general_config", Tier::Control, &[]),
    rule("knowledge_sync_local_reference_docs", Tier::Control, &[]),
    rule("knowledge_test_embedding_runtime", Tier::Control, &[]),
    rule("knowledge_test_feishu_reference_connection", Tier::Control, &[]),
    rule("launch_unity_project", Tier::Control, &[]),
    rule("list_agent_injected_items", Tier::View, &[]),
    rule("list_agents", Tier::View, &[]),
    rule("list_app_rules", Tier::View, &[]),
    rule("list_app_tasks", Tier::View, &[]),
    rule("list_archived_checkout_sessions", Tier::View, &[]),
    rule("list_archived_sessions", Tier::View, &[]),
    rule("list_background_tasks", Tier::View, &[]),
    rule("list_checkout_sessions", Tier::View, &[]),
    rule("list_dir_entries", Tier::View, &["subPath"]),
    rule("list_dir_entries_page", Tier::View, &["subPath"]),
    rule("list_managed_worktrees", Tier::View, &["sourceRoot"]),
    rule("list_project_contexts", Tier::View, &[]),
    rule("list_project_sessions", Tier::View, &[]),
    rule("list_recent_dirs", Tier::View, &[]),
    rule("list_rules", Tier::View, &[]),
    rule("list_session_events", Tier::View, &[]),
    rule("list_sessions", Tier::View, &[]),
    rule("list_skills", Tier::View, &[]),
    rule("list_subagent_defs", Tier::View, &[]),
    rule("list_workspace_agent_injected_items", Tier::View, &[]),
    rule("list_workspace_agents", Tier::View, &[]),
    rule("list_workspace_runtimes", Tier::View, &[]),
    rule("list_workspace_subagent_defs", Tier::View, &[]),
    rule("list_worktree_branches", Tier::View, &[]),
    rule("load_session", Tier::View, &[]),
    rule("load_session_message_images", Tier::View, &[]),
    rule("load_session_message_page", Tier::View, &[]),
    rule("load_session_turn_preview", Tier::View, &[]),
    rule("load_session_view", Tier::View, &[]),
    rule("mark_session_attention_read", Tier::View, &[]),
    rule("markdown_image_target", Tier::View, &["contentPath"]),
    rule("markdown_import_image", Tier::Control, &[]),
    rule("markdown_relative_path", Tier::View, &["contentPath", "targetPath"]),
    rule("markdown_rewrite_image_links", Tier::View, &["contentPath", "oldPath", "newPath"]),
    rule("mcp_get_status", Tier::View, &[]),
    rule("mcp_server_integration_apply", Tier::Control, &[]),
    rule("mcp_server_integration_remove", Tier::Control, &[]),
    rule("mcp_server_integrations", Tier::View, &[]),
    rule("mcp_server_set_enabled", Tier::Control, &[]),
    rule("mcp_server_tool_inventory", Tier::View, &[]),
    rule("mcp_server_tools_inventory", Tier::View, &[]),
    rule("mcp_server_update_settings", Tier::Control, &[]),
    rule("mcp_server_wire_tools", Tier::Control, &[]),
    rule("merge_job_execute", Tier::Control, &[]),
    rule("merge_job_prepare", Tier::Control, &[]),
    rule("plugin_github_auth_logout", Tier::Control, &[]),
    rule("plugin_github_auth_save_token", Tier::Control, &[]),
    rule("plugin_github_auth_status", Tier::View, &[]),
    rule("plugin_github_oauth_poll", Tier::Control, &[]),
    rule("plugin_github_oauth_start", Tier::Control, &[]),
    rule("plugin_github_repo_set_starred", Tier::Control, &[]),
    rule("plugin_github_repo_star_status", Tier::View, &[]),
    rule("plugin_inspector_drawer_packages", Tier::View, &[]),
    rule("plugin_install_from_registry", Tier::Control, &[]),
    rule("plugin_install_from_source", Tier::Control, &[]),
    rule("plugin_list_installed", Tier::View, &[]),
    rule("plugin_registry_fetch_description", Tier::View, &[]),
    rule("plugin_registry_fetch_manifest", Tier::View, &[]),
    rule("plugin_registry_fetch_plugin", Tier::View, &[]),
    rule("plugin_registry_fetch_search_index", Tier::View, &[]),
    rule("plugin_registry_fetch_shard", Tier::View, &[]),
    rule("plugin_registry_sources_get", Tier::View, &[]),
    rule("plugin_registry_sources_set", Tier::Control, &[]),
    rule("plugin_set_enabled", Tier::Control, &[]),
    rule("plugin_uninstall", Tier::Control, &[]),
    rule("preview_workspace_asset", Tier::View, &["filePath"]),
    rule("preview_workspace_asset_target", Tier::View, &[]),
    rule("preview_workspace_asset_thumbnail", Tier::View, &["filePath"]),
    rule("preview_workspace_file", Tier::View, &["filePath"]),
    rule("prewarm_claude_session", Tier::View, &[]),
    rule("project_collaboration_snapshot", Tier::View, &[]),
    rule("project_explorer_apply_operations", Tier::Control, &[]),
    rule("project_explorer_create_preset", Tier::Control, &[]),
    rule("project_explorer_delete_preset", Tier::Control, &[]),
    rule("project_explorer_file_revision", Tier::View, &["path"]),
    rule("project_explorer_list_mount", Tier::View, &[]),
    rule("project_explorer_list_presets", Tier::View, &[]),
    rule("project_explorer_preview_file", Tier::View, &["path"]),
    rule("project_explorer_rename_preset", Tier::Control, &[]),
    rule("project_explorer_snapshot", Tier::View, &[]),
    rule("project_explorer_switch_preset", Tier::Control, &[]),
    rule("project_explorer_write_file", Tier::Control, &[]),
    rule("project_knowledge_list", Tier::View, &["pathPrefix"]),
    rule("queue_chat_input", Tier::View, &[]),
    rule("queue_session_compact", Tier::View, &[]),
    rule("read_app_rule", Tier::View, &["fileName"]),
    rule("read_rule", Tier::View, &["fileName"]),
    rule("read_skill_manifest", Tier::View, &["dirName"]),
    rule("read_workspace_agent_document", Tier::View, &[]),
    rule("read_workspace_asset_preview_frame_cache", Tier::View, &["filePath"]),
    rule("reading_position_get", Tier::View, &[]),
    rule("reading_position_put", Tier::View, &[]),
    rule("ref_graph_deps", Tier::View, &[]),
    rule("ref_graph_refs", Tier::View, &[]),
    rule("ref_graph_resolve_guid", Tier::View, &["path"]),
    rule("ref_graph_resolve_path", Tier::View, &[]),
    rule("ref_graph_scan", Tier::Control, &[]),
    rule("ref_graph_scan_start", Tier::Control, &[]),
    rule("ref_graph_status", Tier::View, &[]),
    rule("ref_graph_walk_deps", Tier::View, &[]),
    rule("ref_graph_walk_refs", Tier::View, &[]),
    rule("refresh_external_skills", Tier::View, &[]),
    rule("refresh_model_catalog", Tier::View, &[]),
    rule("release_unity_project_slot", Tier::Control, &[]),
    rule("remove_managed_worktree", Tier::Control, &[]),
    rule("remove_recent_dir", Tier::Control, &[]),
    rule("remove_skill_unity_files", Tier::Control, &[]),
    rule("remove_workspace", Tier::Control, &[]),
    rule("rename_session", Tier::View, &[]),
    rule("resolve_markdown_image", Tier::View, &["source", "contentPath"]),
    rule("restart_unresponsive_unity_project", Tier::Control, &[]),
    rule("restore_claude_native_session", Tier::Control, &[]),
    rule("rollback_session_to_message", Tier::Control, &[]),
    rule("run_command", Tier::Control, &[]),
    rule("save_agent_model_preference", Tier::View, &[]),
    rule("save_api_key", Tier::Control, &[]),
    rule("save_codex_fast_mode", Tier::View, &[]),
    rule("save_codex_model_config", Tier::Control, &[]),
    rule("save_last_effort", Tier::View, &[]),
    rule("save_last_model", Tier::View, &[]),
    rule("save_model_defaults", Tier::Control, &[]),
    rule("save_provider_key", Tier::Control, &[]),
    rule("save_proxy_config", Tier::Control, &[]),
    rule("save_python_runtime_selection", Tier::Control, &[]),
    rule("save_rule", Tier::Control, &[]),
    rule("save_session_execution_state", Tier::View, &[]),
    rule("save_tool_permission_mode", Tier::Control, &[]),
    rule("save_tool_permissions", Tier::Control, &[]),
    rule("save_workspace_agent_document", Tier::Control, &[]),
    rule("search_assets", Tier::View, &[]),
    rule("search_session_titles", Tier::View, &[]),
    rule("search_workspace_assets", Tier::View, &["roots[]"]),
    rule("search_workspace_entries", Tier::View, &[]),
    rule("search_workspace_scene_objects", Tier::View, &["scenePath"]),
    rule("send_unity_log", Tier::Control, &[]),
    rule("server_client_cancel_login", Tier::Control, &[]),
    rule("server_client_login", Tier::Control, &[]),
    rule("server_client_logout", Tier::Control, &[]),
    rule("server_client_models", Tier::View, &[]),
    rule("server_client_status", Tier::View, &[]),
    rule("server_client_usage", Tier::View, &[]),
    rule("set_agent_injection_enabled", Tier::Control, &[]),
    rule("set_agent_tool_direct_load", Tier::Control, &[]),
    rule("set_agent_tool_enabled", Tier::Control, &[]),
    rule("set_anthropic_native_lazy_enabled", Tier::Control, &[]),
    rule("set_claude_code_cli_enabled", Tier::Control, &[]),
    rule("set_claude_native_agents", Tier::Control, &[]),
    rule("set_claude_permission_mode", Tier::Control, &[]),
    rule("set_claude_prewarm", Tier::Control, &[]),
    rule("set_claude_runtime_settings", Tier::Control, &[]),
    rule("set_default_skill_package_namespace", Tier::Control, &[]),
    rule("set_dynamic_tool_loading_mode", Tier::Control, &[]),
    rule("set_file_tool_workspace_boundary", Tier::Control, &[]),
    rule("set_llm_retry_max_attempts", Tier::Control, &[]),
    rule("set_rule_enabled", Tier::Control, &[]),
    rule("set_rule_order", Tier::Control, &[]),
    rule("set_service_enabled", Tier::Control, &[]),
    rule("set_session_plan_mode", Tier::View, &[]),
    rule("set_session_undo_enabled", Tier::Control, &[]),
    rule("set_skill_config", Tier::Control, &[]),
    rule("set_subagent_max_concurrent", Tier::Control, &[]),
    rule("set_subagent_max_depth", Tier::Control, &[]),
    rule("set_tool_failure_log_enabled", Tier::Control, &[]),
    rule("set_unity_background_hook_enabled", Tier::Control, &[]),
    rule("set_unity_embed_enabled", Tier::Control, &[]),
    rule("set_unity_external_editor_default_enabled", Tier::Control, &[]),
    rule("set_unity_multi_agent_editor_enabled", Tier::Control, &[]),
    rule("set_unity_native_bridge_enabled", Tier::Control, &[]),
    rule("set_unity_state_probe_enabled", Tier::Control, &[]),
    rule("set_unity_test_tools_workspace_enabled", Tier::Control, &[]),
    rule("set_watcher_tuning", Tier::Control, &[]),
    rule("set_workspace_default_agent", Tier::Control, &[]),
    rule("set_workspace_service_resource_limits", Tier::Control, &[]),
    rule("stale_knowledge_proposals", Tier::View, &[]),
    rule("start_workspace_service", Tier::Control, &[]),
    rule("start_workspace_unity_service", Tier::Control, &[]),
    rule("stat_workspace_entries", Tier::View, &["paths[]"]),
    rule("sync_claude_remote_session", Tier::View, &[]),
    rule("synchronize_session_remote_history", Tier::View, &[]),
    rule("test_claude_code_cli", Tier::Control, &[]),
    rule("test_custom_endpoint", Tier::Control, &[]),
    rule("unarchive_session", Tier::View, &[]),
    rule("undo_check_conflicts", Tier::View, &[]),
    rule("undo_check_dirty", Tier::View, &[]),
    rule("undo_latest_conversation_turn", Tier::Control, &[]),
    rule("undo_list", Tier::View, &[]),
    rule("undo_perform", Tier::Control, &[]),
    rule("undo_perform_to_message", Tier::Control, &[]),
    rule("undo_preview", Tier::View, &[]),
    rule("undo_revert_file", Tier::Control, &[]),
    rule("unity_assets_execute", Tier::Control, &[]),
    rule("unity_embed_status", Tier::View, &[]),
    rule("unity_execute_snippet_run", Tier::Control, &[]),
    rule("unity_hot_reload_preflight", Tier::View, &[]),
    rule("unity_hot_reload_set_code_optimization", Tier::Control, &[]),
    rule("unity_hot_reload_set_code_optimization_debug", Tier::Control, &[]),
    rule("unity_hot_reload_set_enabled", Tier::Control, &[]),
    rule("unity_hot_reload_set_play_mode_reload", Tier::Control, &[]),
    rule("unity_in_process_compile_fallback_get_enabled", Tier::View, &[]),
    rule("unity_in_process_compile_fallback_set_enabled", Tier::Control, &[]),
    rule("unity_inline_force_evaluate_set_enabled", Tier::Control, &[]),
    rule("unity_non_public_access_set_enabled", Tier::Control, &[]),
    rule("unity_property_objects", Tier::View, &[]),
    rule("unity_recompile_probe_run", Tier::Control, &[]),
    rule("unity_recompile_run", Tier::Control, &[]),
    rule("unity_serialized_property_apply", Tier::Control, &[]),
    rule("unity_serialized_property_discover", Tier::View, &[]),
    rule("unity_serialized_property_read", Tier::View, &[]),
    rule("unity_serialized_property_read_many", Tier::View, &[]),
    rule("unity_serialized_property_write", Tier::Control, &[]),
    rule("unity_sidecar_compiler_get_status", Tier::View, &[]),
    rule("unity_sidecar_compiler_set_enabled", Tier::Control, &[]),
    rule("use_local_claude", Tier::Control, &[]),
    rule("validate_unity_scene_object", Tier::View, &["scenePath"]),
    rule("view_create", Tier::Control, &[]),
    rule("view_create_folder", Tier::Control, &[]),
    rule("view_delete_entry", Tier::Control, &[]),
    rule("view_document_read", Tier::View, &["reference.path"]),
    rule("view_document_write", Tier::Control, &[]),
    rule("view_list", Tier::View, &[]),
    rule("view_move_entry", Tier::Control, &[]),
    rule("view_read", Tier::View, &[]),
    rule("view_rename_entry", Tier::Control, &[]),
    rule("view_tree", Tier::View, &[]),
    rule("view_update_metadata", Tier::Control, &[]),
    rule("workspace_create_document", Tier::Control, &[]),
    rule("workspace_create_folder", Tier::Control, &[]),
    rule("workspace_file_order", Tier::View, &[]),
    rule("workspace_file_order_save", Tier::Control, &[]),
    rule("workspace_file_preview", Tier::View, &["filePath"]),
    rule("workspace_file_revision", Tier::View, &["filePath"]),
    rule("workspace_file_write", Tier::Control, &[]),
    rule("workspace_move_entry", Tier::Control, &[]),
];

/// Methods handled by the WebSocket layer itself, available to every device.
pub const SESSION_METHODS: &[&str] = &[
    locus_remote_protocol::methods::SET_INTERESTS,
    locus_remote_protocol::methods::SET_FILE_WATCHES,
    locus_remote_protocol::methods::GET_DEVICE_INFO,
];

pub fn rule_for(command: &str) -> Option<&'static Rule> {
    RULES
        .binary_search_by(|rule| rule.command.cmp(command))
        .ok()
        .map(|index| &RULES[index])
}

pub fn allows(rule: &Rule, full_control: bool) -> bool {
    full_control || rule.tier == Tier::View
}

/// Every method a device may call, for `get_device_info`.
pub fn allowed_methods(full_control: bool) -> &'static [String] {
    static VIEW: OnceLock<Vec<String>> = OnceLock::new();
    static CONTROL: OnceLock<Vec<String>> = OnceLock::new();
    let build = |full: bool| {
        SESSION_METHODS
            .iter()
            .copied()
            .chain(RULES.iter().filter(|rule| allows(rule, full)).map(|rule| rule.command))
            .map(str::to_string)
            .collect()
    };
    if full_control {
        CONTROL.get_or_init(|| build(true))
    } else {
        VIEW.get_or_init(|| build(false))
    }
}

// ----- path guard ------------------------------------------------------------

fn collect_strings<'a>(value: &'a Value, selector: &str, out: &mut Vec<&'a str>) {
    let (head, rest) = match selector.split_once('.') {
        Some((head, rest)) => (head, Some(rest)),
        None => (selector, None),
    };
    let (key, array) = match head.strip_suffix("[]") {
        Some(key) => (key, true),
        None => (head, false),
    };
    let Some(field) = value.get(key) else {
        return;
    };
    let items: Vec<&Value> = if array {
        field.as_array().map(|items| items.iter().collect()).unwrap_or_default()
    } else {
        vec![field]
    };
    for item in items {
        match rest {
            Some(rest) => collect_strings(item, rest, out),
            None => {
                if let Some(text) = item.as_str() {
                    out.push(text);
                }
            }
        }
    }
}

/// `scheme:` with a scheme longer than one letter (so not `C:`).
fn url_scheme(value: &str) -> Option<&str> {
    let (scheme, _) = value.split_once(':')?;
    let valid = scheme.len() > 1
        && scheme.as_bytes()[0].is_ascii_alphabetic()
        && scheme.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'));
    valid.then_some(scheme)
}

fn lexical_normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

fn normalize_case(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/').to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn inside(path: &Path, root: &Path) -> bool {
    let path = normalize_case(path);
    let root = normalize_case(root);
    path == root || path.starts_with(&format!("{root}/"))
}

/// Whether a view-tier device may name this path. Relative paths are
/// resolved by the command against its workspace and may not climb out;
/// absolute paths must lie inside an open workspace (links resolved).
pub fn path_allowed(raw: &str, roots: &[PathBuf]) -> bool {
    let value = raw.trim();
    if value.is_empty() {
        return true;
    }
    if let Some(scheme) = url_scheme(value) {
        return !scheme.eq_ignore_ascii_case("file");
    }
    let path = Path::new(value);
    let rooted = path.has_root() || path.components().any(|component| matches!(component, Component::Prefix(_)));
    if !rooted {
        return path.components().all(|component| matches!(component, Component::Normal(_) | Component::CurDir));
    }
    let resolved = match dunce::canonicalize(path) {
        Ok(canonical) => canonical,
        Err(_) => match lexical_normalize(path) {
            Some(normalized) => normalized,
            None => return false,
        },
    };
    roots.iter().any(|root| inside(&resolved, root))
}

/// First argument of `rule` that a view-tier device may not use.
pub fn path_violation<'a>(rule: &Rule, params: &'a Value, roots: &[PathBuf]) -> Option<&'a str> {
    let mut values = Vec::new();
    for selector in rule.paths {
        collect_strings(params, selector, &mut values);
    }
    values.into_iter().find(|value| !path_allowed(value, roots))
}

/// Roots of the open workspaces, canonicalized.
pub fn workspace_roots(registry: &crate::workspace_service::ProjectRegistry) -> Vec<PathBuf> {
    registry
        .runtimes()
        .iter()
        .map(|runtime| dunce::canonicalize(runtime.root()).unwrap_or_else(|_| runtime.root().to_path_buf()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules_are_sorted_unique_and_registered() {
        for pair in RULES.windows(2) {
            assert!(pair[0].command < pair[1].command, "{} / {}", pair[0].command, pair[1].command);
        }
        let lib = include_str!("../lib.rs");
        let start = lib.find("tauri::generate_handler![").expect("handler list");
        let handlers = &lib[start..];
        let handlers = &handlers[..handlers.find("])").expect("end of handler list")];
        for rule in RULES {
            let registered = handlers
                .lines()
                .map(|line| line.trim().trim_end_matches(','))
                .any(|line| line.rsplit("::").next() == Some(rule.command));
            assert!(registered, "{} is not a registered command", rule.command);
        }
    }

    #[test]
    fn desktop_only_commands_are_absent() {
        for command in [
            "remote_access_get_state",
            "remote_access_set_device_permission",
            "get_custom_providers",
            "mcp_servers_get",
            "open_file_external",
            "request_app_exit",
            "set_workspace_event_interests",
            "sub_window_open",
            "view_fs_write_file",
        ] {
            assert!(rule_for(command).is_none(), "{command} must stay desktop only");
        }
    }

    #[test]
    fn tiers_gate_changes() {
        let read = rule_for("workspace_file_preview").unwrap();
        let write = rule_for("workspace_file_write").unwrap();
        assert!(allows(read, false) && allows(read, true));
        assert!(!allows(write, false) && allows(write, true));
        assert!(rule_for("chat").is_some_and(|rule| rule.tier == Tier::View));
        assert!(rule_for("run_command").is_some_and(|rule| rule.tier == Tier::Control));
        assert!(allowed_methods(false).iter().any(|name| name == "get_device_info"));
        assert!(!allowed_methods(false).iter().any(|name| name == "git_commit"));
        assert!(allowed_methods(true).iter().any(|name| name == "git_commit"));
    }

    #[test]
    fn path_guard_keeps_view_devices_inside_workspaces() {
        let workspace = std::env::temp_dir().join(format!("locus-policy-{}", std::process::id()));
        std::fs::create_dir_all(workspace.join("docs")).unwrap();
        std::fs::write(workspace.join("docs").join("a.md"), "x").unwrap();
        let roots = vec![dunce::canonicalize(&workspace).unwrap()];
        let inside_file = workspace.join("docs").join("a.md");
        assert!(path_allowed("docs/a.md", &roots));
        assert!(path_allowed("./docs", &roots));
        assert!(path_allowed("", &roots));
        assert!(path_allowed("https://example.com/x.png", &roots));
        assert!(path_allowed(inside_file.to_str().unwrap(), &roots));
        assert!(!path_allowed("../secrets.txt", &roots));
        assert!(!path_allowed("docs/../../x", &roots));
        assert!(!path_allowed("file:///etc/passwd", &roots));
        let outside = std::env::temp_dir().join("locus-policy-outside.txt");
        assert!(!path_allowed(outside.to_str().unwrap(), &roots));
        let sneaky = format!("{}/../locus-policy-outside.txt", workspace.display());
        assert!(!path_allowed(&sneaky, &roots));
        if cfg!(windows) {
            assert!(!path_allowed("C:\\Windows\\win.ini", &roots));
            assert!(!path_allowed("\\\\server\\share\\x", &roots));
            assert!(!path_allowed("\\Windows", &roots));
            assert!(path_allowed(&inside_file.to_string_lossy().to_uppercase(), &roots));
        }

        let preview = rule_for("diff_single_file").unwrap();
        let ok = json!({"request": {"filePath": "docs/a.md", "oldPath": null}});
        let bad = json!({"request": {"filePath": "docs/a.md", "oldPath": "../../x"}});
        assert_eq!(path_violation(preview, &ok, &roots), None);
        assert_eq!(path_violation(preview, &bad, &roots), Some("../../x"));
        let stat = rule_for("stat_workspace_entries").unwrap();
        assert_eq!(path_violation(stat, &json!({"paths": ["docs", "/etc"]}), &roots), Some("/etc"));
        let _ = std::fs::remove_dir_all(workspace);
    }
}
