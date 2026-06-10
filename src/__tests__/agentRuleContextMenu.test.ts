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

  it("keeps plugin rule enablement controlled by plugin state", () => {
    const source = read("src/components/AgentView.vue");
    const backend = read("src-tauri/src/commands/knowledge.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(source).toContain("function canToggleRule(rule: RuleItem | null | undefined): boolean");
    expect(source).toContain("return !!rule && !rule.pluginId;");
    expect(source).toContain("if (!canToggleRule(rule)) return;");
    expect(source).toContain(':disabled="!canToggleRule(rule)"');
    expect(source).toContain(':disabled="!canToggleRule(selectedRule())"');
    expect(source).toContain("agent.rulePluginEnableManagedByPlugin");
    expect(backend).toContain("Plugin Rule enablement is controlled by plugin state");
    expect(backend).toContain("enabled: true,");
    expect(zh).toContain('"agent.rulePluginEnableManagedByPlugin": "由插件状态控制"');
    expect(en).toContain('"agent.rulePluginEnableManagedByPlugin": "Controlled by plugin state"');
  });

  it("refreshes rule and injected context when installed plugins change", () => {
    const source = read("src/components/AgentView.vue");

    expect(source).toContain('listen<void>("plugins-changed"');
    expect(source).toContain("await loadAllAgents();");
    expect(source).toContain("refreshAll();");
    expect(source).toContain("pluginsChangedUnlisten?.();");
    expect(source).toContain("function preferredAgentId(agents: AgentInfo[]): string");
  });
});
