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

    expect(view).toContain(':root-directory-configs="rootDirectoryConfigs[activeType]"');
  });

  it("renders folder config tags only for first-level folders", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain("rootDirectoryConfigs: Record<string, KnowledgeDirectoryConfigRecord>;");
    expect(explorer).toContain("externalDirectorySources: Record<string, KnowledgeExternalSource[]>;");
    expect(explorer).toContain("if (isBuiltinSkillGroupFolder(node)) return tags;");
    expect(explorer).toContain('props.activeType === "skill" && node.depth === 1 && node.relativePath === "builtin"');
    expect(explorer).toContain("if (node.depth !== 1) return tags;");
    expect(explorer).toContain("buildExternalFolderTag(");
    expect(explorer).toContain("buildFolderListTags({");
    expect(explorer).toContain("'flag-external': tag.tone === 'external'");
    expect(explorer).toContain("'flag-inject': tag.tone === 'inject'");
    expect(explorer).toContain("'flag-search-on': tag.tone === 'search-on'");
    expect(explorer).toMatch(/\.kx-flag\.flag-external\s*\{/);
    expect(explorer).toMatch(/\.kx-flag\.flag-search-on\s*\{/);
    expect(explorer).not.toContain("flag-search-off");
  });

  it("renders recall-path tags inside search results", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    expect(explorer).toContain('function searchResultTags(result: KnowledgeSearchResult) {');
    expect(explorer).toContain("return buildKnowledgeSearchMatchTags(result.matchKind);");
    expect(explorer).toContain("buildKnowledgeSearchMatchTags(result.matchKind)");
    expect(explorer).toContain("flex-wrap: wrap;");
  });
});
