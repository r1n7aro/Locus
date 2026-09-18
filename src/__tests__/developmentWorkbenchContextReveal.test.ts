import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("development workbench file context actions", () => {
  it("selects knowledge documents in the secondary Knowledge list", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const knowledgeView = read("src/components/KnowledgeView.vue");

    expect(workbench).toContain("selectContextKnowledgeInList");
    expect(workbench).toContain('await showSecondaryNavigation(item, "knowledge"');
    expect(workbench).toContain(':selected-document-target="secondaryKnowledgeSelection?.document ?? null"');
    expect(workbench).toContain(':selection-request-id="secondaryKnowledgeSelection?.requestId ?? 0"');
    expect(knowledgeView).toContain("async function revealListDocument(summary: KnowledgeDocumentSummary)");
    expect(knowledgeView).toContain("if (props.listOnly) {");
    expect(knowledgeView).toContain("void revealListDocument(summary);");
    expect(knowledgeView).toContain("expandAncestors(path);");
  });

  it("reveals knowledge and mounted files through their scoped workspace", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("async function revealContextFileInFileSystem()");
    expect(workbench).toContain("await knowledgeRevealTarget({");
    expect(workbench).toContain("await showInFolder(workspaceRef, filePath);");
    expect(workbench).toContain("document?.sourceCheckoutId ?? item.meta.checkoutId");
    expect(workbench).toContain('<ResourceFileMenuItems :target="contextFileTarget"');
    expect(read("src/components/explorer/ResourceFileMenuItems.vue")).toContain("t('knowledge.explorer.openInFileSystem')");
  });

  it("provides localized labels for the Knowledge-list action", () => {
    expect(read("src/language/zh.json")).toContain(
      '\"development.selectInKnowledgeList\": \"在“知识”列表中选中\"',
    );
    expect(read("src/language/en.json")).toContain(
      '\"development.selectInKnowledgeList\": \"Select in Knowledge list\"',
    );
  });
});
