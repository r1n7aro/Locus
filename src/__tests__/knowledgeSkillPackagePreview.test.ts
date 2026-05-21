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
    expect(preview).toContain("BaseDropdown");
    expect(preview).toContain("knowledge.skillPackage.version");
    expect(preview).toContain("knowledge.skillPackage.documents");
    expect(preview).toContain('import LucideIcon from "../icons/LucideIcon.vue"');
    expect(view).toContain("@update-config=\"handleUpdatePackageConfig\"");
  });
});
