import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("KnowledgeExplorer skill command tags", () => {
  it("renders the command trigger tag on skill document rows", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");

    // documentTags() must surface the document's commandTrigger so a builtin
    // skill like create-skill shows its /create-skill tag in the tree.
    expect(explorer).toContain("const trigger = node.document.commandTrigger?.trim();");
    expect(explorer).toContain('tone: "command",');
    expect(explorer).toContain('title: t("knowledge.skill.commandTrigger"),');

    // The document tag span must map the command tone to the flag-command class.
    expect(explorer).toContain("'flag-command': tag.tone === 'command'");
    expect(explorer).toMatch(/\.kx-flag\.flag-command\s*\{/);
  });

  it("does not duplicate the command tag on a package's SKILL.md child", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const state = read("src/composables/useKnowledgeState.ts");

    // The package folder row already shows the trigger via packageTags(); its
    // SKILL.md child reuses the same document, so documentTags() must skip it.
    expect(explorer).toContain(
      "if (trigger && !isSkillPackageRootDocument(node.document)) {",
    );
    // The guard helper is imported from the shared state module (single source
    // of truth for package-root detection).
    expect(explorer).toContain("isSkillPackageRootDocument,");
    expect(explorer).toContain(
      'from "../../composables/useKnowledgeState";',
    );
    expect(state).toContain("export function isSkillPackageRootDocument(");
  });

  it("keeps the command tag even when it matches the row name", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const labels = read("src/components/knowledge/knowledgeMetaLabels.ts");

    // A skill's trigger almost always mirrors its package/file name (`/view`
    // on the view package), so a "redundant name" filter would hide nearly
    // every command chip. The chip is the row's primary command affordance —
    // packageTags() must render the trigger unconditionally.
    expect(labels).not.toContain("isRedundantCommandTrigger");
    expect(explorer).not.toContain("isRedundantCommandTrigger");

    const packageTagsStart = explorer.indexOf("function packageTags(");
    const packageTagsEnd = explorer.indexOf(
      "function deleteMenuLabel",
      packageTagsStart,
    );
    const packageTagsBlock = explorer.slice(packageTagsStart, packageTagsEnd);
    expect(packageTagsBlock).toContain("if (trigger) {");
  });

  it("renders L1 injection tags on package rows without surfacing L0", () => {
    const explorer = read("src/components/knowledge/KnowledgeExplorer.vue");
    const packageTagsStart = explorer.indexOf("function packageTags(");
    const packageTagsEnd = explorer.indexOf(
      "function deleteMenuLabel",
      packageTagsStart,
    );
    const packageTagsBlock = explorer.slice(packageTagsStart, packageTagsEnd);

    expect(packageTagsBlock).toContain(
      'if (node.document.injectMode === "excerpt") {',
    );
    expect(packageTagsBlock).toContain("buildKnowledgeListTags({");
    expect(packageTagsBlock).toContain("injectMode: node.document.injectMode,");
    expect(packageTagsBlock).toContain("aiMaintained: false,");
    expect(packageTagsBlock).not.toContain('"path"');
    expect(explorer).toMatch(
      /v-else-if="entry\.row\.node\.kind === 'package'"[\s\S]*'flag-inject': tag\.tone === 'inject'[\s\S]*'flag-command': tag\.tone === 'command'/,
    );
  });
});
