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

  it("gates automatic undo tracking while retaining targeted and Unity locks", () => {
    const session = read("src-tauri/src/commands/session.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const cli = read("src-tauri/src/agent/instance/claude_code_cli.rs");

    expect(session).toContain("instance.set_session_undo_enabled(config.session_undo_enabled())");
    expect(agent).toContain("self.session_undo_enabled && self.tool_call_needs_undo_tracking");
    expect(agent).toContain("&& self.bash_needs_primary_workspace_tracking(&target_args)");
    expect(agent).toContain('matches!(target_name.as_str(), "write" | "edit")');
    expect(agent).toContain("Self::is_unity_execution_barrier_tool(&target_name)");
    expect(cli).toContain("self.agent.should_track_session_undo(tool_name, args)");
  });

  it("refreshes scoped workbench changes when an undoable write arrives", () => {
    const editor = read("src/components/workbench/WorkbenchSessionEditor.vue");

    expect(editor).toContain("() => undoableMessageIds.value.size");
    expect(editor).toContain("undoableCount === previousUndoableCount");
    expect(editor).toContain("chatChangesStore.refresh(targetSessionId)");
  });
});
