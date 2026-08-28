import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(resolve(process.cwd(), path), "utf8");

describe("internal drag architecture", () => {
  it("uses one app-level controller and one shared preview layer", () => {
    const app = read("src/App.vue");
    const controller = read("src/composables/useInternalDrag.ts");
    const overlay = read("src/components/ui/InternalDragOverlay.vue");

    expect(app).toContain("provideInternalDragController()");
    expect(app).toContain("<InternalDragOverlay />");
    expect(controller).toContain('type InternalDragPhase = "idle" | "pending" | "dragging"');
    expect(controller).toContain("setPointerCapture");
    expect(controller).toContain("elementFromPoint");
    expect(controller).toContain("requestAnimationFrame(runAutoScroll)");
    expect(overlay).toContain("data-internal-drag-preview");
  });

  it("keeps every in-document reorder and placement source off native HTML drag", () => {
    const internalSurfaces = [
      "src/components/AgentView.vue",
      "src/components/explorer/WorkspaceTree.vue",
      "src/components/knowledge/KnowledgeExplorer.vue",
      "src/components/knowledge/KnowledgePreview.vue",
      "src/components/knowledge/KnowledgeDirectoryPreview.vue",
      "src/components/chat/SessionPanel.vue",
      "src/components/ViewPackageView.vue",
    ];

    for (const path of internalSurfaces) {
      const source = read(path);
      expect(source, path).not.toContain("@dragstart");
      expect(source, path).not.toMatch(/:draggable=|draggable="true"/);
    }
  });

  it("retains native drag only at the cross-runtime Unity and file bridge", () => {
    const assetChip = read("src/components/AssetChip.vue");
    const bridge = read("src/composables/useUnityReferenceDragSource.ts");

    expect(assetChip).toContain('@dragstart="handleDragStart"');
    expect(bridge).toContain("startUnityReferenceHtmlDrag");
    expect(bridge).toContain("startLocusFileHtmlDrag");
  });

  it("keeps sortable lists floating while reserving only a layout gap", () => {
    const controller = read("src/composables/useInternalDrag.ts");
    const overlay = read("src/components/ui/InternalDragOverlay.vue");
    const knowledge = read("src/components/knowledge/KnowledgeExplorer.vue");
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");
    const agent = read("src/components/AgentView.vue");
    const session = read("src/components/chat/SessionPanel.vue");
    const viewPackage = read("src/components/ViewPackageView.vue");
    const workspaceTree = read("src/components/explorer/WorkspaceTree.vue");

    expect(controller).toContain('"floating-with-gap"');
    expect(overlay).toContain("drag.previewMode.value !== 'inline'");
    for (const source of [knowledge, workbench, agent, session, viewPackage]) {
      expect(source).toContain('"floating-with-gap"');
    }
    expect(knowledge).toContain("opacity: 0;");
    expect(workbench).toContain("opacity: 0;");
    expect(workbench).toContain(".workspace-tree-row-shell.is-drop-preview::before");
    expect(workbench).toContain("background: var(--accent-color);");
    expect(workbench).toContain("left: var(--workspace-tree-row-indent, 4px);");
    expect(workspaceTree).toContain("'--workspace-tree-row-indent': rowIndent");
    expect(agent).toContain("rule-drop-gap");
    expect(session).toContain("is-drop-preview-row");
    expect(viewPackage).toContain("is-drop-preview-row");
  });

  it("keeps the workspace layout gap mounted until an async move snapshot settles", () => {
    const workbench = read("src/components/workbench/DevelopmentWorkbench.vue");

    expect(workbench).toContain("interface SettlingLayoutDrop");
    expect(workbench).toContain("layoutDropIntent.value ?? settlingLayoutDrop.value?.intent");
    expect(workbench).toContain("settlingLayoutDrop.value?.preview");
    expect(workbench).toMatch(
      /settlingLayoutDrop\.value = \{[\s\S]*?await commitWorkbenchInternalDrop[\s\S]*?finally \{[\s\S]*?settlingLayoutDrop\.value = null;/,
    );
  });
});
