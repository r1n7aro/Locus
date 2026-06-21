import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("skill package namespace settings", () => {
  it("keeps backend compatibility while removing the standalone settings entry", () => {
    const rustConfig = read("src-tauri/src/config.rs");
    const rustConfigRegistry = read("src-tauri/src/config_registry.rs");
    const rustSkill = read("src-tauri/src/commands/skill.rs");
    const rustApp = read("src-tauri/src/lib.rs");
    const settingsView = read("src/components/SettingsView.vue");
    const settingsState = read("src/composables/useSettingsState.ts");
    const uiStore = read("src/stores/ui.ts");
    const knowledgeService = read("src/services/knowledge.ts");
    const skillCreateTool = read("tools/skill_create.json");
    const createSkillDoc = read("knowledge/skill/create-skill.md");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(rustConfig).toContain("default_skill_package_namespace");
    expect(rustConfig).toContain("set_default_skill_package_namespace");
    expect(rustConfigRegistry).toContain("default_skill_package_namespace");
    expect(rustSkill).toContain("get_default_skill_package_namespace");
    expect(rustSkill).toContain("set_default_skill_package_namespace");
    expect(rustSkill).toContain("create_skill_sync_with_default_package_namespace");
    expect(rustSkill).toContain("skill_package_slug_from_name");
    expect(rustApp).toContain("commands::get_default_skill_package_namespace");
    expect(rustApp).toContain("commands::set_default_skill_package_namespace");

    expect(settingsView).not.toContain("KnowledgeSettings");
    expect(settingsView).not.toContain("activeCategory === 'knowledge'");
    expect(settingsView).not.toContain("settings.tab.knowledge");
    expect(settingsState).not.toContain('"knowledge"');
    expect(uiStore).not.toContain('"knowledge" | "archived"');
    expect(knowledgeService).toContain("get_default_skill_package_namespace");
    expect(knowledgeService).toContain("set_default_skill_package_namespace");

    expect(skillCreateTool).toContain("short kebab-case package id");
    expect(skillCreateTool).toContain("studio.tools.asset-audit");
    expect(createSkillDoc).toContain("short kebab-case package ids");
    expect(createSkillDoc).toContain("studio.tools.asset-audit");
    expect(zh).not.toContain('"settings.tab.knowledge"');
    expect(en).not.toContain('"settings.tab.knowledge"');
    expect(zh).not.toContain('"settings.knowledge.defaultSkillPackageNamespace"');
    expect(en).not.toContain('"settings.knowledge.defaultSkillPackageNamespace"');
  });
});
