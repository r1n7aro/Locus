import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("knowledge skill package preview wiring", () => {
  it("renders package information when a package node is selected", () => {
    const view = read("src/components/KnowledgeView.vue");
    const preview = read(
      "src/components/knowledge/KnowledgeSkillPackagePreview.vue",
    );

    expect(view).toContain("selectedPackageDocument");
    expect(view).toContain("@select-package=\"handleSelectPackage\"");
    expect(view).toContain("<KnowledgeSkillPackagePreview");
    expect(preview).toContain("knowledge.skillPackage.packageId");
    expect(preview).toContain("knowledge.skillPackage.config");
    expect(preview).toContain("(e: \"updateConfig\"");
    expect(preview).toContain("(e: \"exportPackage\"");
    expect(preview).toContain("BaseDropdown");
    expect(preview).toContain("BaseButton");
    expect(preview).toContain("knowledge.skillPackage.export");
    expect(preview).toContain("knowledge.skillPackage.version");
    expect(preview).toContain("knowledge.skillPackage.documents");
    expect(preview).toContain('import LucideIcon from "../icons/LucideIcon.vue"');
    expect(preview).toContain("unityAssetIconClassForPath");
    expect(preview).toContain("unityAssetIconNodeForPath");
    expect(preview).toContain(':class="documentIconClass(document)"');
    expect(view).toContain("@update-config=\"handleUpdatePackageConfig\"");
    expect(view).toContain("@export-package=\"handleExportPackage\"");
  });

  it("wires package import and export through explorer and services", () => {
    const view = read("src/components/KnowledgeView.vue");
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const state = read("src/composables/useKnowledgeState.ts");
    const service = read("src/services/knowledge.ts");
    const rust = read("src-tauri/src/commands/skill.rs");
    const rustLib = read("src-tauri/src/lib.rs");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(view).toContain("@import-skill-package=\"handleImportSkillPackage\"");
    expect(view).toContain("@export-package=\"handleExportPackageNode\"");
    expect(explorer).toContain("(e: \"importSkillPackage\"");
    expect(explorer).toContain("(e: \"exportPackage\"");
    expect(explorer).toContain("knowledge.explorer.importSkillPackage");
    expect(explorer).toContain("knowledge.explorer.exportSkillPackage");
    expect(state).toContain('import { open, save } from "@tauri-apps/plugin-dialog"');
    expect(state).toContain("async function importSkillPackageArchive()");
    expect(state).toContain("async function exportSkillPackageArchive(packageId: string)");
    expect(state).toContain("filters: [");
    expect(state).toContain("extensions: [\"zip\"]");
    expect(service).toContain("export function importSkillPackage");
    expect(service).toContain("export function exportSkillPackage");
    expect(rust).toContain("pub async fn import_skill_package");
    expect(rust).toContain("pub async fn export_skill_package");
    expect(rust).toContain("zip::ZipWriter");
    expect(rust).toContain("zip::ZipArchive");
    expect(rustLib).toContain("commands::import_skill_package");
    expect(rustLib).toContain("commands::export_skill_package");
    expect(zh).toContain('"knowledge.explorer.importSkillPackage": "导入 Package"');
    expect(zh).toContain('"knowledge.skillPackage.exported": "已导出 Package: {0}"');
    expect(en).toContain('"knowledge.explorer.importSkillPackage": "Import Package"');
    expect(en).toContain('"knowledge.skillPackage.exported": "Exported package: {0}"');
  });
});
