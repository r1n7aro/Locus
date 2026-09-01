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
    expect(controller).toContain("captureElement.ownerDocument");
    expect(controller).toContain("requestAnimationFrame(runAutoScroll)");
    expect(overlay).toContain("data-internal-drag-preview");
    expect(overlay).toContain(':to="overlayTarget"');
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
      "src/components/MarkdownRenderer.vue",
      "src/components/AssetChip.vue",
      "src/components/unity-preview/UnityObjectIdentity.vue",
      "src/components/workbench/WorkbenchEditorTabs.vue",
    ];

    for (const path of internalSurfaces) {
      const source = read(path);
      expect(source, path).not.toContain("@dragstart");
      expect(source, path).not.toMatch(/:draggable=|draggable="true"/);
    }
  });

  it("externalizes references through the shared semantic drag source", () => {
    const assetChip = read("src/components/AssetChip.vue");
    const markdownRenderer = read("src/components/MarkdownRenderer.vue");
    const unityIdentity = read("src/components/unity-preview/UnityObjectIdentity.vue");
    const referenceDrag = read("src/components/workbench/workbenchReferenceDrag.ts");
    const bridge = read("src/composables/useUnityReferenceDragSource.ts");

    for (const source of [assetChip, markdownRenderer, unityIdentity]) {
      expect(source).toContain("startWorkbenchReferenceInternalDrag");
      expect(source).not.toContain("startUnityReferenceHtmlDrag");
      expect(source).not.toContain("armUnityReferencePointerDrag");
    }
    expect(referenceDrag).toContain("externalizeUnityReferenceDrag");
    expect(referenceDrag).toContain("externalizeLocusFileDrag");
    expect(bridge).toContain("externalizeUnityReferenceDrag");
    expect(bridge).toContain("externalizeLocusFileDrag");
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
