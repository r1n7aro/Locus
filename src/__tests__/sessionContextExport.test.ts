import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

function read(path: string): string {
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}

describe("structured session context export", () => {
  it("uses schema v44 with session multi agent selection and prior context migrations", () => {
    const store = read("src-tauri/src/session/store.rs");

    expect(store).toContain("const SCHEMA_VERSION: i32 = 44;");
    expect(store).toContain('36,\n                "persist project contexts, shared sessions, and scoped runs"');
    expect(store).toContain('37,\n                "backfill unambiguous legacy session checkout bindings"');
    expect(store).toContain('38,\n                "persist explicit citation arrays on assistant text render parts"');
    expect(store).toContain('Self::migrate(conn, 40, "persist workspace tree visibility"');
    expect(store).toContain("v36_database_backfills_only_unambiguous_legacy_session_checkouts");
    expect(store).toContain("v37_database_migrates_text_render_parts_with_explicit_empty_citations");
    expect(store).toContain("default_checkout_id TEXT REFERENCES workspace_checkouts(checkout_id)");
    expect(store).toContain("git_branch_ref TEXT");
    expect(store).toContain("git_head_oid TEXT");
    expect(store).toContain("CREATE TABLE IF NOT EXISTS project_explorer_nodes");
    expect(store).toContain('Self::migrate(conn, 26, "persist session context attempts"');
    expect(store).toContain('27,\n                "persist structured conversation checkpoints"');
    expect(store).toContain("migrate_conversation_checkpoints");
    expect(store).toContain('28,\n                "repair empty prompt windows created by historical forks"');
    expect(store).toContain("migrate_empty_prompt_windows");
    expect(store).toContain("v27_migration_repairs_sessions_with_visible_but_empty_prompt_history");
    expect(store).toContain('29,\n                "repair terminal tool rounds missing persisted outputs"');
    expect(store).toContain("migrate_terminal_tool_round_outputs");
    expect(store).toContain("v28_migration_repairs_terminal_tool_round_and_keeps_context_exportable");
    expect(store).toContain('Self::migrate(conn, 32, "persist prompt cache checks"');
    expect(store).toContain('33,\n                "use server usage baselines for prompt cache checks"');
    expect(store).toContain('34,\n                "detect prompt cache invalidation from server input growth"');
    expect(store).toContain('Self::migrate(conn, 35, "persist the latest session Fast mode"');
    expect(store).toContain("v34_database_migrates_session_fast_mode_and_exports_legacy_value_as_empty");
    expect(store).toContain("baseline_tokens INTEGER NOT NULL");
    expect(store).toContain("input_tokens INTEGER NOT NULL");
    expect(store).toContain("excess_input_tokens INTEGER NOT NULL");
    expect(store).toContain("reason TEXT NOT NULL");
    expect(store).toContain("v33_cache_checks_migrate_to_server_input_growth_and_keep_sessions_exportable");
    expect(store).toContain("CREATE TABLE IF NOT EXISTS session_prompt_cache_checks");
    expect(store).toContain("conversation_checkpoint: Option<compact::ConversationCheckpoint>");
    expect(store).toContain("CREATE TABLE IF NOT EXISTS session_context_attempts");
    expect(store).toContain("request_gzip BLOB NOT NULL");
    expect(store).toContain("response_gzip BLOB NOT NULL");
    expect(store).toContain("CREATE TABLE IF NOT EXISTS session_context_capture_gaps");
    expect(store).toContain("INSERT OR IGNORE INTO session_context_capture_gaps");
    expect(store).toContain("v25_database_migrates_context_attempts_and_old_session_exports_with_explicit_empty");
    expect(store).toContain("v26_database_migrates_structured_checkpoints_and_exports_legacy_empty");
  });

  it("exports one versioned YAML audit document and removes the Markdown exporter", () => {
    const exporter = read("src-tauri/src/session/context_export.rs");
    const commands = read("src-tauri/src/commands/session.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(exporter).toContain('const EXPORT_FORMAT: &str = "locus.context_review";');
    expect(exporter).toContain("const EXPORT_FORMAT_VERSION: u32 = 9;");
    expect(exporter).toContain('"defaultCheckoutId"');
    expect(exporter).toContain('"branchRef"');
    expect(exporter).toContain('"headOid"');
    expect(exporter).toContain("cache_invalidations: Value");
    expect(exporter).toContain("serde_yaml::to_string");
    expect(exporter).toContain("content_hash");
    expect(exporter).toContain("session_tree_ids");
    expect(exporter).toContain("context_attempts: Value");
    expect(exporter).toContain("compactions: Value");
    expect(exporter).toContain("context_budget: AttemptContextBudgetExport");
    expect(exporter).toContain('unit: "serialized_json_characters_proxy"');
    expect(exporter).toContain('const EMPTY: &str = "empty";');
    expect(commands).toContain("pub async fn export_session_context");
    expect(lib).toContain("commands::export_session_context");
    expect(commands).not.toContain("format_rounds_as_markdown");
    expect(commands).not.toContain("save_raw_context");
    expect(lib).not.toContain("commands::save_raw_context");
  });

  it("persists and restores model, effort, and Fast mode as one session execution state", () => {
    const store = read("src-tauri/src/session/store.rs");
    const commands = read("src-tauri/src/commands/session.rs");
    const service = read("src/services/session.ts");
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const chatStore = read("src/stores/chat.ts");

    expect(store).toContain("pub fn set_session_execution_state(");
    expect(commands).toContain("pub async fn save_session_execution_state(");
    expect(service).toContain('ipcInvoke("save_session_execution_state"');
    expect(workspace).toContain("saveSessionExecutionState(");
    expect(workspace).toContain("function selectWorkspaceFastMode(enabled: boolean)");
    expect(chatStore).toContain("detail.lastFastMode ?? modelStore.defaultCodexFastMode");
  });

  it("ships the context review workflow as a builtin skill", () => {
    const skill = read("knowledge/skill/review-context.md");
    const zh = JSON.parse(read("src/language/zh.json"));
    const en = JSON.parse(read("src/language/en.json"));

    expect(skill).toContain("# Review Context");
    expect(skill).toContain("id: kd_skill_review_context");
    expect(skill).toContain("injectMode: excerpt");
    expect(skill).toContain("skillEnabled: true");
    expect(skill).toContain("skillSurface: command");
    expect(skill).toContain("commandTrigger: /review-context");
    expect(skill).toContain("tools:\n  - read\n  - grep");
    expect(skill).not.toContain("\ntitle:");
    expect(skill).not.toContain("\npath:");
    expect(skill).not.toContain("\ncommandEnabled:");
    expect(skill).toContain("Tool-result prompt share");
    expect(skill).toContain("character-share proxy");
    expect(skill).toContain("## Instructions");
    expect(skill).toContain("### Evaluate compaction continuity");
    expect(skill).toContain("Tool-result audit");
    expect(zh["chat.contextReviewPrompt"]).toBe("请review这个context");
    expect(en["chat.contextReviewPrompt"]).toBe("Please review this context");
    expect(zh["chat.contextReviewPrompt"]).not.toContain("完整分析轨迹");
    expect(en["chat.contextReviewPrompt"]).not.toContain("trajectory review");
  });

  it("copies running sessions through an online database snapshot and a separate runtime sample", () => {
    const exporter = read("src-tauri/src/session/context_export.rs");
    const commands = read("src-tauri/src/commands/session.rs");
    const store = read("src-tauri/src/session/store.rs");

    expect(store).toContain("pub fn create_export_snapshot(&self)");
    expect(store).toContain("rusqlite::backup::Backup::new");
    expect(store).toContain("self.event_writer.flush()?");
    expect(exporter).toContain('database_copy: if store.export_snapshot_created_at().is_some()');
    expect(exporter).toContain('"sqlite_online_backup"');
    expect(exporter).toContain("runtime_snapshot_at");
    expect(commands).toContain("capture_context_export_live_snapshot");
    expect(commands).toContain("fork_session_from_export_snapshot");
    expect(commands).toContain("partial_was_copied");
  });

  it("captures every provider attempt status for trajectory review", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const claude = read("src-tauri/src/agent/instance/claude_code_cli.rs");

    for (const status of ["completed", "failed", "invalid", "cancelled"]) {
      expect(agent).toContain(`"${status}"`);
    }
    expect(agent).toContain("record_context_attempt");
    expect(agent).toContain("raw_request");
    expect(claude).toContain("record_captured_attempt");
  });

});
