import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();

function read(path: string): string {
  return readFileSync(resolve(root, path), "utf8");
}

describe("development workbench inline create", () => {
  it("creates workspace folders inside the target tree level", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("function appendInlineCreate(");
    expect(workbench).toContain("inlineCreateDepth: depth");
    expect(workbench).toContain('class="development-inline-create-row"');
    expect(workbench).toContain('@keydown.enter.prevent="submitInlineCreate"');
    expect(workbench).toContain('@keydown.esc.prevent.stop="cancelInlineCreate"');
    expect(workbench).toContain('document.addEventListener("pointerdown", handleInlineCreatePointerDown, true)');
    expect(workbench).not.toContain('mode: "create",\n    projectId: item.meta.projectId');
  });

  it("keeps knowledge folders and documents on the established inline path", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("@click=\"openCreateInline('folder')\"");
    expect(explorer).toContain("@click=\"openCreateInline('document')\"");
    expect(explorer).toContain('class="kx-create-row"');
    expect(explorer).toContain('@keydown.enter.prevent="submitInlineCreate"');
    expect(explorer).toContain('@keydown.esc.prevent="closeInlineCreate"');
  });
});
