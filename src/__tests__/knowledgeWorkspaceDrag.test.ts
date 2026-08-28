import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import {
  buildKnowledgeDocumentWorkspaceDragPayload,
  buildKnowledgeFolderWorkspaceDragPayload,
  KNOWLEDGE_INTERNAL_DRAG_TYPE,
  knowledgeInternalDragSource,
} from "../components/knowledge/knowledgeWorkspaceDrag";

describe("knowledge internal drag payload", () => {
  it("connects opened document and directory headers to the pointer drag controller", () => {
    const documentPreview = readFileSync(
      new URL("../components/knowledge/KnowledgePreview.vue", import.meta.url),
      "utf8",
    );
    const directoryPreview = readFileSync(
      new URL("../components/knowledge/KnowledgeDirectoryPreview.vue", import.meta.url),
      "utf8",
    );

    expect(documentPreview).toContain("startKnowledgeInternalDrag(internalDrag, event");
    expect(documentPreview).toContain('@pointerdown="onDocumentDragPointerDown"');
    expect(documentPreview).not.toContain("@dragstart");
    expect(directoryPreview).toContain("startKnowledgeInternalDrag(internalDrag, event");
    expect(directoryPreview).toContain('@pointerdown="onDirectoryDragPointerDown"');
    expect(directoryPreview).not.toContain("@dragstart");
  });

  it("routes knowledge placement and new-session attachment through one registered target", () => {
    const workbench = readFileSync(
      new URL("../components/workbench/DevelopmentWorkbench.vue", import.meta.url),
      "utf8",
    );
    const overlay = readFileSync(
      new URL("../components/ui/InternalDragOverlay.vue", import.meta.url),
      "utf8",
    );

    expect(workbench).toContain("workbenchInternalDropTarget");
    expect(workbench).toContain("KNOWLEDGE_INTERNAL_DRAG_TYPE");
    expect(workbench).toContain('intent: { kind: "newSession", target: rowHit.item }');
    expect(workbench).toContain("createNewSessionWithAttachments");
    expect(workbench).toContain("placeKnowledgeWorkspaceDrag");
    expect(workbench).toContain("internalDrag.registerTarget(workbenchInternalDropTarget)");
    expect(workbench).toContain("internalLayoutDragPreview");
    expect(workbench).toContain("node.nodeId !== internalSourceNodeId");
    expect(workbench).toContain('"floating-with-gap"');
    expect(workbench).not.toContain("setDragImage");
    expect(workbench).not.toContain("KNOWLEDGE_WORKSPACE_DRAG_STATE_EVENT");
    expect(overlay).toContain("data-internal-drag-preview");
    expect(overlay).toContain("drag.previewMode.value !== 'inline'");
    expect(overlay).toContain("drag.subscribeVisualPoint(applyVisualPoint)");
    expect(overlay).toContain("internalDragFloatingTransform");
    expect(overlay).not.toContain("window.innerWidth");
    expect(overlay).not.toContain("Math.round");
  });

  it("builds one typed internal source with activation and finish callbacks", () => {
    const activated = vi.fn();
    const finished = vi.fn();
    const payload = {
      version: 1 as const,
      entries: [{
        kind: "document" as const,
        type: "design" as const,
        path: "design/overview.md",
        name: "overview.md",
        documentId: "document-overview",
      }],
    };

    const source = knowledgeInternalDragSource(
      { payload },
      { onActivated: activated, onFinished: finished },
    );

    expect(source.payload.type).toBe(KNOWLEDGE_INTERNAL_DRAG_TYPE);
    expect(source.payload.data.payload).toEqual(payload);
    expect(source.allowedOperations).toEqual(["move", "copy"]);
    expect(source.preview).toMatchObject({ label: "overview.md", kind: "file", count: 1 });
    source.onActivated?.();
    source.onFinished?.({ dropped: false, reason: "cancel" });
    expect(activated).toHaveBeenCalledOnce();
    expect(finished).toHaveBeenCalledOnce();
  });

  it("builds the same document payload for an opened knowledge page", () => {
    const payload = buildKnowledgeDocumentWorkspaceDragPayload({
      id: "document-overview",
      type: "design",
      path: "systems\\overview.md",
      title: "Overview",
      injectMode: "inherit",
      effectiveInjectMode: "path",
      readOnly: false,
      aiMaintained: "inherit",
      effectiveAiMaintained: false,
      modifiedAt: 1,
    });

    expect(payload.entries).toEqual([{
      kind: "document",
      type: "design",
      path: "design/systems/overview.md",
      name: "overview.md",
      documentId: "document-overview",
    }]);
  });

  it("builds folder payloads for opened directory pages and skips type roots", () => {
    const directory = {
      version: 4,
      type: "reference" as const,
      path: "unity\\manual",
      configPath: "unity/manual/.locus.json",
      exists: true,
      updatedAt: 1,
      summary: "",
      injectMode: "inherit" as const,
      effectiveInjectMode: "path" as const,
      aiMaintained: "inherit" as const,
      effectiveAiMaintained: false,
      lexicalSearch: "inherit" as const,
      vectorSearch: "inherit" as const,
      inheritToChildren: true,
      allowCreateDocuments: true,
      allowCreateDirectories: true,
      allowMoveDocuments: true,
      allowMoveDirectories: true,
      maintenanceRules: null,
      effectiveMaintenanceRules: null,
      effectiveLexicalSearch: { enabled: true, source: "default" as const },
      effectiveVectorSearch: { enabled: true, source: "default" as const },
    };

    expect(buildKnowledgeFolderWorkspaceDragPayload(directory)?.entries).toEqual([{
      kind: "folder",
      type: "reference",
      path: "reference/unity/manual",
      relativePath: "unity/manual",
      name: "manual",
    }]);
    expect(buildKnowledgeFolderWorkspaceDragPayload({ ...directory, path: "" })).toBeNull();
  });
});
