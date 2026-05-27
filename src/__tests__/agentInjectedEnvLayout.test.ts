import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AgentView injected env entry", () => {
  it("renders env.md inside the injected section instead of the system prompt section", () => {
    const source = read("src/components/AgentView.vue");

    expect(source).toContain("const injectedContextEntryCount = computed(() =>");
    expect(source).toContain("getAgentRenderedEnvPrompt");
    expect(source).toContain("const envPreviewMode = ref<EnvPreviewMode>");
    expect(source).toContain("<BaseSegmented");
    expect(source).toContain('class="env-preview-mode"');
    expect(source).toContain('class="injected-section"');
    expect(source).toContain('class="kb-item injected-item"');
    expect(source).toContain(':class="{ selected: selected?.type === \'env\' }"');
    expect(source).toContain('{{ t("agent.envTemplate") }}');
    expect(source).toContain('{{ t("agent.injected.context") }}');
    expect(source).not.toContain(
      `<template>
            <div class="section-label">
              <span>{{ t("agent.injected") }}</span>`,
    );
    expect(source).not.toContain(
      `class="kb-item prompt-item"
            :class="{ selected: selected?.type === 'env' }"`,
    );
  });
});
