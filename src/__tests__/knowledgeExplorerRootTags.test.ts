import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer root retrieval tags", () => {
  it("passes root directory configs from the view into the explorer", () => {
    const view = read("src/components/KnowledgeView.vue");

    expect(view).toContain(':root-directory-configs="rootDirectoryConfigs"');
  });

  it("renders type-root and first-level folder config tags", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("Record<string, KnowledgeDirectoryConfigRecord>");
    expect(explorer).toContain("externalDirectorySources: Record<string, KnowledgeExternalSource[]>;");
    expect(explorer).toContain("if (isBuiltinSkillGroupFolder(node)) return tags;");
    expect(explorer).toContain('return node.type === "skill" && node.depth === 1 && node.relativePath === "builtin";');
    expect(explorer).toContain('props.rootDirectoryConfigs[node.type][""]');
    expect(explorer).toContain("if (node.depth !== rootDirectoryDepth) return tags;");
    expect(explorer).toContain("props.rootDirectoryConfigs[node.type][node.relativePath]");
    expect(explorer).toContain("buildExternalFolderTag(");
    expect(explorer).toContain("buildFolderListTags({");
    expect(explorer).toContain("'flag-external': tag.tone === 'external'");
    expect(explorer).toContain("'flag-inject': tag.tone === 'inject'");
    expect(explorer).toContain("'flag-search-on': tag.tone === 'search-on'");
    expect(explorer).toMatch(/\.kx-flag\.flag-external\s*\{/);
    expect(explorer).toMatch(/\.kx-flag\.flag-search-on\s*\{/);
    expect(explorer).not.toContain("flag-search-off");
  });

  it("reuses ordinary folder and document tags while filtering the tree", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("function documentTags(node: DocumentNode)");
    expect(explorer).toContain("function folderTags(node: FolderNode)");
    expect(explorer).toContain("isSearchMode.value ? undefined : props.folderStats");
    expect(explorer).not.toContain("buildKnowledgeSearchMatchTags");
    expect(explorer).not.toContain("searchResultTags");
  });
});
