import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();
const read = (path: string) => readFileSync(resolve(root, path), "utf8");

describe("session file undo setting", () => {
  it("persists a default-on setting and exposes it in general settings", () => {
    const rustConfig = read("src-tauri/src/config.rs");
    const rustCommands = read("src-tauri/src/commands/workspace.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const service = read("src/services/system.ts");
    const settings = read("src/components/settings/GeneralSettings.vue");
    const sharedSettings = read("src/composables/useSessionUndoSettings.ts");
    const chatView = read("src/components/ChatView.vue");

    expect(rustConfig).toContain("fn default_session_undo_enabled()");
    expect(rustConfig).toContain("pub fn session_undo_enabled(&self) -> bool");
    expect(rustConfig).toContain("pub fn set_session_undo_enabled(&self, value: bool)");
    expect(rustCommands).toContain("pub async fn get_session_undo_enabled");
    expect(rustCommands).toContain("pub async fn set_session_undo_enabled");
    expect(rustCommands).toContain("SESSION_UNDO_ENABLED_CHANGED_EVENT");
    expect(rustApp).toContain("commands::get_session_undo_enabled");
    expect(rustApp).toContain("commands::set_session_undo_enabled");
    expect(service).toContain('ipcInvoke<boolean>("get_session_undo_enabled")');
    expect(service).toContain('ipcInvoke<void>("set_session_undo_enabled", { value })');
    expect(settings).toContain('t("settings.general.sessionUndo")');
    expect(settings).toContain(':model-value="sessionUndoSettings.enabled"');
    expect(sharedSettings).toContain('"session-undo-enabled-changed"');
    expect(chatView).toContain("sessionUndoSettings.enabled");
    expect(chatView).toContain("chatChangesStore.hasChangesForSession(props.activeSessionId)");
  });

  it("separates targeted file locks from Unity execution when undo is disabled", () => {
    const session = read("src-tauri/src/commands/session.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const cli = read("src-tauri/src/agent/instance/claude_code_cli.rs");
    const policy = read("src-tauri/src/agent/tool_execution_policy.rs");

    expect(session).toContain("instance.set_session_undo_enabled(config.session_undo_enabled())");
    expect(agent).toMatch(/self\.session_undo_enabled\s*&& target_name != "execute_typescript"\s*&& self\.tool_call_needs_undo_tracking\(name, args\)/);
    expect(policy).toContain('matches!(name, "write" | "edit")');
    expect(policy).toContain("if !session_undo_enabled");
    expect(agent).toContain("crate::agent::tool_execution_policy::workspace_request(");
    expect(agent).toContain("crate::agent::unity_execution_scope::run(");
    expect(agent).toContain("let _file_guard = if !self.session_undo_enabled");
    expect(agent).toContain("if !self.session_undo_enabled || !is_active(tc)");
    expect(cli).toContain("if !self.agent.session_undo_enabled");
    expect(agent).toContain("if has_unity_asset_writes && !self.session_undo_enabled");
    expect(cli).toContain("begin_edit_session_in_background");
    expect(cli).toContain("self.agent.should_track_session_undo(tool_name, args)");
  });

  it("refreshes scoped workbench changes when an undoable write arrives", () => {
    const editor = read("src/components/workbench/WorkbenchSessionEditor.vue");

    expect(editor).toContain("() => undoableMessageIds.value.size");
    expect(editor).toContain("undoableCount === previousUndoableCount");
    expect(editor).toContain("chatChangesStore.refresh(targetSessionId)");
  });
});
