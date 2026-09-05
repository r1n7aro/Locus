import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("selector dropdown alignment", () => {
  it("anchors the model selector dropdown to the trigger's leading edge", () => {
    const source = read("src/components/ModelSelector.vue");

    expect(source).toContain(".model-trigger {");
    expect(source).toContain("min-height: 28px;");
    expect(source).toContain(".model-dropdown {");
    expect(source).toContain("left: 0;");
    expect(source).toContain("right: auto;");
    expect(source).toContain("transform-origin: bottom left;");
  });

  it("anchors the thinking selector dropdown to the trigger's leading edge", () => {
    const source = read("src/components/ThinkingSelector.vue");

    expect(source).toContain(".thinking-trigger {");
    expect(source).toContain("min-height: 28px;");
    expect(source).toContain(".thinking-dropdown {");
    expect(source).toContain("left: 0;");
    expect(source).toContain("right: auto;");
    expect(source).toContain("transform-origin: bottom left;");
  });

  it("anchors the combined Agent, model, and effort selector to the trailing edge", () => {
    const source = read("src/components/ModelEffortSelector.vue");

    expect(source).toContain(".model-effort-trigger {");
    expect(source).toContain("min-height: 28px;");
    expect(source).toContain("border: 1px solid transparent;");
    expect(source).toContain(".model-effort-dropdown {");
    expect(source).toContain("right: 0;");
    expect(source).toContain("transform-origin: bottom right;");
    expect(source).toContain(".model-effort-dropdown.has-agent.has-effort {");
    expect(source).toContain("grid-template-columns: 150px minmax(0, 1fr) 120px;");
    expect(source).toContain(".model-effort-agent-panel {");
    expect(source).toContain(".model-effort-effort-panel {");
    expect(source).toContain(".model-effort-agent {");
    expect(source).toContain("color: inherit;");
    expect(source).toContain("font-size: inherit;");
    expect(source).toContain("font-weight: inherit;");
    expect(source).toContain('content: "|";');
    expect(source).toContain("margin-left: 4px;");
    expect(source).toContain("border-left: 1px solid var(--border-color);");
    expect(source).not.toContain("model-effort-option-desc");
    expect(source).toContain("group.provider === 'openai_codex'");
    expect(source).toContain("<BaseSwitch");
    expect(source.match(/<BaseSwitch/g)).toHaveLength(1);
    expect(source).toContain("selectModel: [id: string]");
    expect(source).toContain("selectAgent: [id: string]");
    expect(source).toContain("selectEffort: [level: EffortLevel]");
  });
});
