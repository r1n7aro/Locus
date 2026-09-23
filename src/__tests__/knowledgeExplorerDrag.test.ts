// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { createApp, defineComponent, h, nextTick, reactive, type App } from "vue";
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

async function mountTree(tree: ExplorerNode[], collapsedPaths: string[] = []) {
  const moveNodes = vi.fn();
  const collapsed = reactive(new Set(collapsedPaths));
  app = createApp(defineComponent({
    setup() {
      controller = provideInternalDragController();
      return () => h(KnowledgeExplorer, {
        tree, activeType: "design", selectedPath: null,
        rootDirectoryConfigs: { design: {}, plan: {}, memory: {}, skill: {}, reference: {} },
        externalDirectorySources: {}, folderStats: {}, isPathExpanded: (path: string) => !collapsed.has(path),
        onToggle: (path: string) => collapsed.has(path) ? collapsed.delete(path) : collapsed.add(path),
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

function useRenderedRowHitTesting() {
  const rows = () => [...document.querySelectorAll<HTMLElement>(".workspace-tree-row-shell")];
  // Recompute hits from the rendered order, as elementFromPoint does after Vue
  // updates the tree. A fixed target mock hides rows moving away mid-drag.
  Object.defineProperty(document, "elementFromPoint", {
    configurable: true,
    value: vi.fn((_x: number, y: number) => rows()[Math.floor(y / 30)] ?? null),
  });
  return {
    keys: () => rows().map((row) => row.dataset.treeKey),
    center: (path: string) => rows().findIndex((row) => row.dataset.treeKey === path) * 30 + 15,
  };
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
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("knowledge explorer document drag", () => {
  it.each([
    ["design", "memory"],
    ["design", "memory/combat"],
    ["memory", "memory/combat"],
  ] as const)("keeps the target stable when moving from %s into %s", async (sourceType, targetPath) => {
    const node = documentNode(sourceType, "note.md");
    const moveNodes = await mountTree([
      folder("design", "", sourceType === "design" ? [node] : []),
      folder("memory", "", [
        folder("memory", "combat", [documentNode("memory", "combat/existing.md")]),
        ...(sourceType === "memory" ? [node] : []),
      ]),
    ]);
    const layout = useRenderedRowHitTesting();
    const initialKeys = layout.keys();
    const source = document.querySelector<HTMLButtonElement>(`[data-tree-key="${node.path}"] button`)!;
    const targetY = layout.center(targetPath);
    source.dispatchEvent(pointer("pointerdown", layout.center(node.path)));
    window.dispatchEvent(pointer("pointermove", targetY));
    await nextTick();
    expect(layout.keys()).toEqual(initialKeys);
    expect(source.isConnected).toBe(true);
    window.dispatchEvent(pointer("pointermove", targetY));
    await nextTick();
    expect(controller.activeTarget.value?.decision.key).toBe(targetPath);
    window.dispatchEvent(pointer("pointerup", targetY));

    const [type, ...parts] = targetPath.split("/");
    expect(moveNodes).toHaveBeenCalledExactlyOnceWith([node], parts.join("/"), type);
  });

  it("keeps multi-document drags stable while changing target folders", async () => {
    const nodes = [documentNode("memory", "a.md"), documentNode("memory", "b.md")];
    const moveNodes = await mountTree([
      folder("memory", "", [folder("memory", "first"), folder("memory", "second"), ...nodes]),
    ]);
    for (const node of nodes) {
      document.querySelector(`[data-tree-key="${node.path}"] button`)!.dispatchEvent(
        new MouseEvent("click", { bubbles: true, ctrlKey: true }),
      );
    }
    await nextTick();
    const layout = useRenderedRowHitTesting();
    const initialKeys = layout.keys();
    const source = document.querySelector(`[data-tree-key="${nodes[0]!.path}"] button`)!;
    const firstY = layout.center("memory/first");
    const secondY = layout.center("memory/second");
    source.dispatchEvent(pointer("pointerdown", layout.center(nodes[0]!.path)));
    for (const y of [firstY, secondY, firstY, secondY]) {
      window.dispatchEvent(pointer("pointermove", y));
      await nextTick();
      expect(layout.keys()).toEqual(initialKeys);
      expect(document.querySelectorAll(".dragging")).toHaveLength(2);
    }
    window.dispatchEvent(pointer("pointerup", secondY));
    expect(moveNodes).toHaveBeenCalledExactlyOnceWith(nodes, "second", "memory");
  });

  it("expands a hovered folder and moves into a nested folder", async () => {
    vi.useFakeTimers();
    const node = documentNode("memory", "note.md");
    const moveNodes = await mountTree([
      folder("memory", "", [folder("memory", "combat", [folder("memory", "combat/nested")]), node]),
    ], ["memory/combat"]);
    const layout = useRenderedRowHitTesting();
    const source = document.querySelector(`[data-tree-key="${node.path}"] button`)!;
    source.dispatchEvent(pointer("pointerdown", layout.center(node.path)));
    window.dispatchEvent(pointer("pointermove", layout.center("memory/combat")));
    await nextTick();
    await vi.advanceTimersByTimeAsync(600);
    await nextTick();
    expect(layout.keys()).toContain("memory/combat/nested");
    const nestedY = layout.center("memory/combat/nested");
    window.dispatchEvent(pointer("pointermove", nestedY));
    await nextTick();
    window.dispatchEvent(pointer("pointerup", nestedY));
    expect(moveNodes).toHaveBeenCalledExactlyOnceWith([node], "combat/nested", "memory");
  });

  it("cancels without moving documents or leaving target highlights", async () => {
    const node = documentNode("memory", "note.md");
    const moveNodes = await mountTree([folder("memory", "", [folder("memory", "combat"), node])]);
    const release = await drag(node.path, "memory/combat");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    release();
    await nextTick();
    expect(moveNodes).not.toHaveBeenCalled();
    expect(controller.phase.value).toBe("idle");
    expect(document.querySelector(".drop-target, .dragging")).toBeNull();
    expect(document.querySelector(`[data-tree-key="${node.path}"]`)).not.toBeNull();
  });

  it.each(["plan", "memory", "memory/combat"])("drops a document on %s and previews its destination", async (targetPath) => {
    const node = documentNode("design", "combat/note.md");
    const moveNodes = await mountTree([
      folder("design", "", [folder("design", "combat", [node])]),
      folder("plan", ""), folder("memory", "", [folder("memory", "combat")]),
    ]);
    const release = await drag(node.path, targetPath);
    expect(controller.activeTarget.value?.decision.operation).toBe("move");
    expect(document.querySelector(`[data-tree-key="${targetPath}"].drop-target`)).not.toBeNull();
    expect(document.querySelector(`[data-tree-key="${node.path}"].dragging`)).not.toBeNull();
    expect(controller.previewMode.value).toBe("floating");
    release();
    const [type, ...parts] = targetPath.split("/");
    expect(moveNodes).toHaveBeenCalledExactlyOnceWith([node], parts.join("/"), type);
    await nextTick();
    expect(document.querySelector(".drop-target, .dragging")).toBeNull();
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
