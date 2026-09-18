// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, defineComponent, h, nextTick, type App } from "vue";
import KnowledgeExplorer from "../components/knowledge/KnowledgeExplorer.vue";
import type { ExplorerNode } from "../composables/useKnowledgeState";
import { provideInternalDragController, type InternalDragController } from "../composables/useInternalDrag";
import type { KnowledgeDocumentSummary, KnowledgeDocumentType } from "../types";

vi.mock("../i18n", () => ({ t: (key: string) => key }));

let app: App | undefined;
let controller: InternalDragController;

function folder(type: KnowledgeDocumentType, path: string, children: ExplorerNode[] = []): ExplorerNode {
  return { kind: "folder", type, path: path ? `${type}/${path}` : type, relativePath: path,
    name: path || type, depth: path ? 1 : 0, specialRoot: !path, children };
}

function documentNode(type: KnowledgeDocumentType, path: string): ExplorerNode {
  const document: KnowledgeDocumentSummary = {
    id: `${type}/${path}`, type, path, title: path, injectMode: "excerpt", effectiveInjectMode: "excerpt",
    readOnly: false, aiMaintained: false, effectiveAiMaintained: false, summary: null, modifiedAt: 1,
  };
  return { kind: "document", type, path: `${type}/${path}`, name: path.split("/").pop()!, depth: 1, document };
}

function pointer(type: string, y: number): PointerEvent {
  const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0,
    buttons: type === "pointerup" ? 0 : 1, clientX: 100, clientY: y });
  Object.defineProperties(event, {
    pointerId: { value: 1 }, isPrimary: { value: true }, pointerType: { value: "mouse" },
  });
  return event as PointerEvent;
}

async function mountTree(tree: ExplorerNode[]) {
  const moveNodes = vi.fn();
  app = createApp(defineComponent({
    setup() {
      controller = provideInternalDragController();
      return () => h(KnowledgeExplorer, {
        tree, activeType: "design", selectedPath: null,
        rootDirectoryConfigs: { design: {}, plan: {}, memory: {}, skill: {}, reference: {} },
        externalDirectorySources: {}, folderStats: {}, isPathExpanded: () => true,
        rootContentsLoaded: () => true, hasMoreRootDocuments: () => false, rootDocumentsLoading: () => false,
        hasMoreFolderDocuments: () => false, folderDocumentsLoaded: () => true, folderDocumentsLoading: () => false,
        loading: false, searchQuery: "", searchResults: [], searching: false, onMoveNodes: moveNodes,
      });
    },
  }));
  const host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
  await nextTick();
  return moveNodes;
}

async function drag(sourcePath: string, targetPath: string) {
  const source = document.querySelector<HTMLButtonElement>(`[data-tree-key="${sourcePath}"] button`)!;
  const target = document.querySelector<HTMLElement>(`[data-tree-key="${targetPath}"]`)!;
  expect(source).not.toBeNull();
  expect(target).not.toBeNull();
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: vi.fn(() => target) });
  source.dispatchEvent(pointer("pointerdown", 50));
  window.dispatchEvent(pointer("pointermove", 150));
  await nextTick();
  return () => window.dispatchEvent(pointer("pointerup", 150));
}

afterEach(() => {
  controller?.dispose();
  app?.unmount();
  app = undefined;
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

describe("knowledge explorer document drag", () => {
  it.each(["plan", "memory", "memory/combat"])("drops a document on %s and previews its destination", async (targetPath) => {
    const node = documentNode("design", "combat/note.md");
    const moveNodes = await mountTree([
      folder("design", "", [folder("design", "combat", [node])]),
      folder("plan", ""), folder("memory", "", [folder("memory", "combat")]),
    ]);
    const release = await drag(node.path, targetPath);
    expect(controller.activeTarget.value?.decision.operation).toBe("move");
    expect(document.querySelector(`[data-tree-key="${targetPath}"].drop-target`)).not.toBeNull();
    expect(document.querySelector(".is-drop-preview")).not.toBeNull();
    release();
    const [type, ...parts] = targetPath.split("/");
    expect(moveNodes).toHaveBeenCalledExactlyOnceWith([node], parts.join("/"), type);
    await nextTick();
    expect(document.querySelector(".is-drop-preview")).toBeNull();
  });

  it("rejects a document's current directory", async () => {
    const node = documentNode("design", "note.md");
    const moveNodes = await mountTree([folder("design", "", [node])]);
    const release = await drag(node.path, "design");
    expect(controller.activeTarget.value).toBeNull();
    release();
    expect(moveNodes).not.toHaveBeenCalled();
  });

  it("keeps managed Skill package contents unavailable as move targets", async () => {
    const node = documentNode("design", "note.md");
    const managed = documentNode("skill", "external/note.md");
    if (managed.kind === "document") managed.document.externalSource = { provider: "package", locator: "external://test" };
    const moveNodes = await mountTree([
      folder("design", "", [node]), folder("skill", "", [folder("skill", "external", [managed])]),
    ]);
    const release = await drag(node.path, "skill/external");
    expect(controller.activeTarget.value).toBeNull();
    release();
    expect(moveNodes).not.toHaveBeenCalled();
  });
});
