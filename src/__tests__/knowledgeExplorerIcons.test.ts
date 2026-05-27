import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer row icons", () => {
  it("uses the local LucideIcon wrapper for folders, packages, and documents", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain(
      'import LucideIcon from "../icons/LucideIcon.vue"',
    );
    expect(explorer).toContain("unityAssetIconClassForPath");
    expect(explorer).toContain("unityAssetIconNodeForPath");
    expect(explorer).toContain('class="kx-kind-icon folder"');
    expect(explorer).toContain('class="kx-kind-icon package"');
    expect(explorer).toContain('class="kx-kind-icon document"');
    expect(explorer).toContain(':class="{ open: entry.row.expanded }"');
    expect(explorer).toContain(':icon="entry.row.expanded ? FolderOpen : Folder"');
    expect(explorer).toContain(':icon="Package"');
    expect(explorer).toContain(':class="documentIconClass(entry.row.node)"');
    expect(explorer).toContain(':icon="documentIconNode(entry.row.node)"');
    expect(explorer).toContain(".kx-kind-icon.folder {");
    expect(explorer).toContain(".kx-kind-icon.package {");
    expect(explorer).toContain(".kx-kind-icon.document.skill-document {");
  });

  it("uses a spacer instead of a chevron for collapsed branches without child rows", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("entry.row.directChildCount > 0");
    expect(explorer).toContain('class="kx-branch-spacer"');
    expect(explorer).not.toContain("empty: entry.row.directChildCount === 0");
    expect(explorer).not.toContain(".kx-chevron.empty");
  });
});
