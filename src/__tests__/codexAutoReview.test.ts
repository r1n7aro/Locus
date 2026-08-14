import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Codex automatic approval review", () => {
  it("keeps local dangerous-command interception enabled by default", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const detector = read("src-tauri/src/agent/instance/dangerous_command.rs");
    const settingsState = read("src/composables/useSettingsState.ts");

    expect(settingsState).toMatch(
      /name:\s*"behavior\.local_dangerous_commands",[\s\S]*defaultMode:\s*"ask"/,
    );
    expect(agent).toContain("PERMISSION_BEHAVIOR_LOCAL_DANGEROUS_COMMANDS");
    expect(agent).toContain("dangerous_command::dangerous_command_match(command)");
    expect(agent).toMatch(
      /PERMISSION_BEHAVIOR_LOCAL_DANGEROUS_COMMANDS[\s\S]*true,/,
    );
    expect(detector).toContain("tree_sitter_bash::LANGUAGE as BASH");
    expect(detector).toContain("PowerShellForceDelete");
    expect(detector).toContain("CmdRecursiveDelete");
  });

  it("keeps auto review Codex-only, opt-in, structured, and context-minimal", () => {
    const config = read("src-tauri/src/commands/workspace.rs");
    const agent = read("src-tauri/src/agent/instance/mod.rs");
    const review = read("src-tauri/src/agent/instance/auto_review.rs");
    const settingsState = read("src/composables/useSettingsState.ts");
    const settingsView = read("src/components/SettingsView.vue");
    const providers = read("src/components/settings/ApiProviders.vue");

    expect(config).toContain("pub auto_review: bool");
    expect(config).toContain("assert!(!config.auto_review)");
    expect(settingsState).toContain("autoReview: config?.autoReview === true");
    expect(settingsView).toContain(":codex-auto-review=");
    expect(settingsView).toContain("@update:codex-auto-review=");
    expect(providers).toContain("update:codexAutoReview");

    expect(agent).toContain("LlmBackend::OpenAiCodex");
    expect(agent).toContain("CodexStreamOptions::compact()");
    expect(agent).toContain("with_output_schema(\"locus_auto_review\"");
    expect(review).toContain('REVIEW_MODEL: &str = "codex-auto-review"');
    expect(review).toContain("MAX_USER_CONTEXT_APPROX_TOKENS: usize = 256");
    expect(review).toContain('const MARKER: &str = "\\n<truncated/>\\n"');
    expect(review).toContain('"historyMessagesIncluded": 1');
    expect(review).toContain('"toolsAvailable": false');
    expect(review).toContain('"networkAvailable": false');
    expect(review).toContain("matchedCommand");
    expect(review).toContain("riskFlags");
    expect(review).toContain("<omitted: matched command supplied in localInspection>");
  });

  it("records the reviewer as an independent model call", () => {
    const agent = read("src-tauri/src/agent/instance/mod.rs");

    expect(agent).toContain("record_codex_auto_review_usage");
    expect(agent).toContain("auto_review::REVIEW_MODEL");
    expect(agent).toContain('"OpenAI Codex"');
    expect(agent).toContain('"auto_review"');
    expect(agent).toContain("record_model_usage(");
    expect(agent).toContain("record_model_usage_event(");
  });
});
