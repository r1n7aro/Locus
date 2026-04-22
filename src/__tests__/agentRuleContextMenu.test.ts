import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("AgentView rule context menu", () => {
  it("adds contextual create and delete actions to the rule section", () => {
    const source = read("src/components/AgentView.vue");

    expect(source).toContain("const ruleContextMenu = ref<{ x: number; y: number; rule: RuleItem | null } | null>(null);");
    expect(source).toContain("function openRuleContextMenu(event: MouseEvent, rule: RuleItem | null = null) {");
    expect(source).toContain('class="rule-section" @contextmenu.prevent="onRuleListContextMenu"');
    expect(source).toContain('@contextmenu.prevent.stop="openRuleContextMenu($event, rule)"');
    expect(source).toContain('class="agent-rule-ctx-menu"');
    expect(source).toContain('requestDeleteRuleFromContext');
    expect(source).toContain('{{ t("agent.newRule") }}');
    expect(source).toContain('{{ t("common.delete") }}');
  });
});
