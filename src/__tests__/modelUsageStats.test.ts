import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("model usage statistics", () => {
  it("records call-level usage and exposes an aggregated command", () => {
    const store = read("src-tauri/src/session/store.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const claudeCode = read("src-tauri/src/agent/instance/claude_code_cli.rs");
    const title = read("src-tauri/src/session/title.rs");
    const commands = read("src-tauri/src/commands/session.rs");
    const lib = read("src-tauri/src/lib.rs");

    expect(store).toContain('const SCHEMA_VERSION: i32 = 27;');
    expect(store).toContain("CREATE TABLE IF NOT EXISTS model_usage_events");
    expect(store).toContain("pub fn record_model_usage(");
    expect(store).toContain("pub fn record_model_usage_event(");
    expect(store).toContain("pub fn get_model_usage_report(");
    expect(agent).toContain("store.record_model_usage(");
    expect(agent).toContain('"compaction"');
    expect(agent).toContain('"completion"');
    expect(claudeCode).toContain("store.record_model_usage(");
    expect(title).toContain("store.record_model_usage_event(");
    expect(title).toContain('"session_title"');
    expect(commands).toContain("pub async fn get_model_usage_stats(");
    expect(lib).toContain("commands::get_model_usage_stats");
  });

  it("adds a restrained settings table with four token categories", () => {
    const settings = read("src/components/SettingsView.vue");
    const panel = read("src/components/settings/ModelUsageStats.vue");
    const service = read("src/services/session.ts");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(settings).toContain('activeCategory === \'modelUsage\'');
    expect(settings).toContain("<ModelUsageStats />");
    expect(panel).toContain("<BaseSegmented");
    expect(panel).toContain('class="model-usage-summary"');
    expect(panel).toContain('class="model-usage-table"');
    expect(panel).toContain("usage.inputTokens");
    expect(panel).toContain("usage.outputTokens");
    expect(panel).toContain("usage.cacheReadTokens");
    expect(panel).toContain("usage.cacheWriteTokens");
    expect(panel).not.toMatch(/#[0-9a-fA-F]{3,8}/);
    expect(service).toContain('ipcInvoke<ModelUsageReport>("get_model_usage_stats"');
    expect(zh).toContain('"settings.tab.modelUsage": "调用统计"');
    expect(en).toContain('"settings.tab.modelUsage": "Usage"');
  });
});
