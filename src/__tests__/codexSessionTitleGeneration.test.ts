import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("Codex session title generation", () => {
  it("keeps title generation opt-in and available only with a valid Codex login", () => {
    const rustConfig = read("src-tauri/src/commands/workspace.rs");
    const titleGeneration = read("src-tauri/src/session/title.rs");
    const settingsState = read("src/composables/useSettingsState.ts");
    const sessionCommand = read("src-tauri/src/commands/session.rs");
    const settingsView = read("src/components/SettingsView.vue");
    const apiProviders = read("src/components/settings/ApiProviders.vue");

    expect(rustConfig).toContain("pub generate_session_titles: bool");
    expect(rustConfig).toContain("#[serde(default)]");
    expect(settingsState).toContain("generateSessionTitles: config?.generateSessionTitles === true");
    expect(settingsState).toContain("setCodexSessionTitleGeneration");
    expect(settingsView).toContain(":codex-session-title-generation=");
    expect(settingsView).toContain("@update:codex-session-title-generation=");
    expect(apiProviders).toContain("codexStatus.authenticated && !codexStatus.validationFailed");
    expect(apiProviders).toContain("update:codexSessionTitleGeneration', $event");
    expect(sessionCommand).toContain("status.authenticated && !status.validation_failed");
    expect(titleGeneration).toContain("!status.authenticated || status.validation_failed");
  });

  it("uses Luna low structured output and applies the result with compare-and-set", () => {
    const titleGeneration = read("src-tauri/src/session/title.rs");
    const codex = read("src-tauri/src/llm/codex.rs");
    const sessionCommand = read("src-tauri/src/commands/session.rs");
    const sessionStore = read("src-tauri/src/session/store.rs");
    const bootstrap = read("src/composables/useAppBootstrap.ts");

    expect(titleGeneration).toContain('DEFAULT_CODEX_TITLE_MODEL: &str = "gpt-5.6-luna"');
    expect(titleGeneration).toContain('DEFAULT_CODEX_TITLE_REASONING_EFFORT: &str = "low"');
    expect(titleGeneration).toContain('with_output_schema("session_title", schema)');
    expect(codex).toContain('"type": "json_schema"');
    expect(sessionCommand).toContain("prepare_session_title_prompt(&text)");
    expect(sessionCommand).toContain("spawn_codex_session_title_generation(");
    expect(sessionStore).toContain("rename_session_if_title_matches");
    expect(bootstrap).toContain('"session-title-updated"');
    expect(bootstrap).toContain("applySessionTitleUpdate(sessionId, title)");
  });

  it("sanitizes prompt wrappers, Markdown, HTML, and invisible markup before generation", () => {
    const titleGeneration = read("src-tauri/src/session/title.rs");

    expect(titleGeneration).toContain("unwrap_codex_delegations(raw_prompt)");
    expect(titleGeneration).toContain("take_chars(&unwrapped, TITLE_PROMPT_CHAR_LIMIT)");
    expect(titleGeneration).toContain("strip_writing_block_fences(&limited)");
    expect(titleGeneration).toContain("markdown_to_plain_text(&without_writing_fences)");
    expect(titleGeneration).toContain('for tag in ["script", "style", "head", "template", "noscript"]');
    expect(titleGeneration).toContain("TITLE_PROMPT_CHAR_LIMIT");
    expect(titleGeneration).toContain("GENERATED_TITLE_CHAR_LIMIT");
  });
});
