import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Codex extended context setting", () => {
  it("persists an opt-in switch and applies Codex-compatible context limits", () => {
    const rustConfig = read("src-tauri/src/commands/workspace.rs");
    const rustModels = read("src-tauri/src/llm/codex_models.rs");
    const rustCompact = read("src-tauri/src/compact.rs");
    const rustAgent = read("src-tauri/src/agent/instance/mod.rs");
    const settingsState = read("src/composables/useSettingsState.ts");
    const settingsView = read("src/components/SettingsView.vue");
    const apiProviders = read("src/components/settings/ApiProviders.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(rustConfig).toContain("pub extended_context: bool");
    expect(rustConfig).toContain("#[serde(default)]");
    expect(rustModels).toContain("pub fn resolve_context_limits(");
    expect(rustModels).toContain("CODEX_STANDARD_CONTEXT_WINDOW: u32 = 272_000");
    expect(rustModels).toContain("CODEX_EXTENDED_CONTEXT_WINDOW: u32 = 372_000");
    expect(rustModels).toContain("context_limits_with_trusted_override");
    expect(rustModels).toContain("intentionally bypasses the remote catalog's 272K");
    expect(rustCompact).toContain("pub fn codex_auto_compact_token_limit(");
    expect(rustAgent).toContain("config.extended_context");
    expect(rustAgent).toContain("limits.auto_compact_token_limit");

    expect(settingsState).toContain("setCodexExtendedContext");
    expect(settingsView).toContain(":codex-extended-context=");
    expect(settingsView).toContain("@update:codex-extended-context=");
    expect(apiProviders).toContain("update:codexExtendedContext");
    expect(apiProviders).toContain("<BaseSwitch");
    expect(zh).toContain('"settings.codex.extendedContextTitle": "扩展上下文"');
    expect(en).toContain('"settings.codex.extendedContextTitle": "Extended context"');
  });

  it("keeps Codex compaction on the dedicated endpoint", () => {
    const codex = read("src-tauri/src/llm/codex.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");

    expect(codex).toContain('format!("{}/compact", codex_responses_endpoint(base_url))');
    expect(agent).toContain("execute_codex_remote_compact");
  });
});
