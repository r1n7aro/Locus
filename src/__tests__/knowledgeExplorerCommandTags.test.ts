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
});
