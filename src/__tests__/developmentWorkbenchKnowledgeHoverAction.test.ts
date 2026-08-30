import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = process.cwd();

describe("development workbench knowledge row actions", () => {
  it("removes knowledge documents from the workspace instead of toggling hidden state", () => {
    const workbench = readFileSync(
      resolve(root, "src/components/workbench/DevelopmentWorkbench.vue"),
      "utf8",
    );

    expect(workbench).toContain('"is-knowledge-row": true');
    expect(workbench).toContain("development-knowledge-remove-button");
    expect(workbench).toContain("@click.stop=\"removeKnowledgeItemFromWorkspace(item as DevelopmentTreeItem)\"");
    expect(workbench).toContain('kind: "removeResourcePlacement"');
    expect(workbench).toContain('resourceKind: "knowledge"');
    expect(workbench).toContain("resourceId: item.meta.knowledge.id");
    expect(workbench).toContain('@click="removeContextKnowledgeItemFromWorkspace"');
    expect(workbench).toContain(".workspace-tree-row-shell.is-knowledge-row:hover .development-knowledge-remove-button");
    expect(workbench).toContain("t('development.removeFromWorkspace')");
    expect(workbench).not.toContain("toggleKnowledgeItemHidden");
  });

  it("shows the same hover removal action for mounted knowledge folders", () => {
    const workbench = readFileSync(
      resolve(root, "src/components/workbench/DevelopmentWorkbench.vue"),
      "utf8",
    );

    expect(workbench).toContain("function isKnowledgeFolderPlacement");
    expect(workbench).toContain('node.sourceKind === "knowledge"');
    expect(workbench).toContain('"is-knowledge-row": node.sourceKind === "knowledge" && mountedDirectory');
    expect(workbench).toContain('v-else-if="isKnowledgeFolderPlacement(item as DevelopmentTreeItem)"');
    expect(workbench).toContain('@click.stop="removeKnowledgeFolderFromWorkspace(item as DevelopmentTreeItem)"');
    expect(workbench).toContain('kind: "removeNode"');
  });

  it("keeps hidden state on Locus system entries and archives sessions", () => {
    const workbench = readFileSync(
      resolve(root, "src/components/workbench/DevelopmentWorkbench.vue"),
      "utf8",
    );
    const sessionMenuStart = workbench.indexOf(
      '<template v-else-if="contextMenu.item.meta.kind === \'session\'">',
    );
    const sessionMenuEnd = workbench.indexOf("<template v-else>", sessionMenuStart);
    const sessionMenu = workbench.slice(sessionMenuStart, sessionMenuEnd);

    expect(sessionMenuStart).toBeGreaterThan(-1);
    expect(sessionMenuEnd).toBeGreaterThan(sessionMenuStart);
    expect(sessionMenu).toContain("archiveContextSession");
    expect(sessionMenu).not.toContain("setContextNodeHidden");
    expect(workbench).toContain(
      "node.resourceKind !== SYSTEM_RESOURCE_KIND",
    );
    expect(workbench).toContain(
      "contextMenu.item.meta.explorerNode?.resourceKind === SYSTEM_RESOURCE_KIND",
    );
  });
});
