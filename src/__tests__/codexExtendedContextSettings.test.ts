import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Codex context window setting", () => {
  it("persists a custom window with a 272K default and Codex-compatible limits", () => {
    const rustConfig = read("src-tauri/src/commands/workspace.rs");
    const rustModels = read("src-tauri/src/llm/codex_models.rs");
    const rustCompact = read("src-tauri/src/compact.rs");
    const rustAgent = read("src-tauri/src/agent/instance/mod.rs");
    const settingsState = read("src/composables/useSettingsState.ts");
    const settingsView = read("src/components/SettingsView.vue");
    const apiProviders = read("src/components/settings/ApiProviders.vue");
    const contextConfig = read("src/config/codexContext.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(rustConfig).toContain("pub context_window: Option<u32>");
    expect(rustConfig).toContain("DEFAULT_CODEX_CONTEXT_WINDOW: u32 = 272_000");
    expect(rustConfig).toContain("MAX_CODEX_CONTEXT_WINDOW: u32 = 1_000_000");
    expect(rustConfig).toContain("LEGACY_CODEX_EXTENDED_CONTEXT_WINDOW");
    expect(rustConfig).toContain("#[serde(default)]");
    expect(rustModels).toContain("pub fn resolve_context_limits(");
    expect(rustModels).toContain("CODEX_MAX_CONTEXT_WINDOW: u32 = 1_000_000");
    expect(rustModels).toContain("context_limits_with_trusted_override");
    expect(rustModels).toContain("exceed an older remote catalog maximum");
    expect(rustCompact).toContain("pub fn codex_auto_compact_token_limit(");
    expect(rustAgent).toContain("config.resolved_context_window()");
    expect(rustAgent).toContain("limits.auto_compact_token_limit");

    expect(contextConfig).toContain("CODEX_DEFAULT_CONTEXT_WINDOW = 272_000");
    expect(contextConfig).toContain("CODEX_MAX_CONTEXT_WINDOW = 1_000_000");
    expect(settingsState).toContain("setCodexContextWindow");
    expect(settingsView).toContain(":codex-context-window=");
    expect(settingsView).toContain("@update:codex-context-window=");
    expect(apiProviders).toContain("update:codexContextWindow");
    expect(apiProviders).toContain('type="number"');
    expect(zh).toContain('"settings.codex.contextWindowTitle": "上下文窗口"');
    expect(en).toContain('"settings.codex.contextWindowTitle": "Context window"');
  });

  it("keeps Codex compaction on the dedicated endpoint", () => {
    const codex = read("src-tauri/src/llm/codex.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");

    expect(codex).toContain('format!("{}/compact", codex_responses_endpoint(base_url))');
    expect(agent).toContain("execute_codex_remote_compact");
  });
});
