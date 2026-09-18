// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { createPinia, setActivePinia } from "pinia";
import ts from "typescript";
import { createApp, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import WorkspaceTree, { type WorkspaceTreeItem } from "../components/explorer/WorkspaceTree.vue";
import { workspaceSecondarySection } from "../components/workbench/workspaceSecondaryNavigation";
import { workbenchSplitDirectionAtPoint, workbenchTabInsertionIndexAtPoint } from "../components/workbench/workbenchDropGeometry";
import { provideInternalDragController, type InternalDragController, type InternalDropCommitContext, type InternalDropTargetRegistration } from "../composables/useInternalDrag";
import { createWorkbenchEditorInput, useWorkbenchStore } from "../stores/workbench";
import type { ProjectExplorerNode, WorkbenchResourceRef } from "../types/workbench";

// Run the production drag handlers and tree row with the real drag controller/store.
// Only unrelated backend services and reference-attachment drops are stubbed.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const names = [
  "makeRow", "isNewSessionNode", "appendNewSessionRow", "onDragPointerDown", "treeEditorDescriptor",
  "canSetTreeItemState", "developmentTreeItemFromHit", "pinnedDividerTarget", "resolveLayoutDropIntentAt",
  "resolveExplorerRootDropIntent", "canMoveExplorerNodeToIntent", "resolveWorkbenchInternalDrop",
  "commitWorkbenchInternalDrop", "openWorkbenchResource",
];
const handlers = parsed.statements.filter((statement) => (
  ts.isFunctionDeclaration(statement) && !!statement.name && names.includes(statement.name.text)
)).map((statement) => statement.getText(parsed)).join("\n");
const code = ts.transpileModule(handlers, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const cleanups: Array<() => void> = [];
afterEach(() => {
  cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
  document.body.replaceChildren();
  localStorage.clear();
  vi.restoreAllMocks();
});

function pointer(type: string, x = 320, y = 20) {
  const event = new MouseEvent(type, {
    bubbles: true, cancelable: true, button: 0, buttons: type === "pointerup" ? 0 : 1,
    clientX: x, clientY: y,
  });
  Object.defineProperties(event, {
    pointerId: { value: 1 }, isPrimary: { value: true }, pointerType: { value: "mouse" },
  });
  return event;
}

async function fixture(checkoutScoped = false, initialCheckout = "checkout") {
  setActivePinia(createPinia());
  const store = useWorkbenchStore();
  const state = store.ensureWindow("main");
  const editor = (sessionId: string) => createWorkbenchEditorInput(
    { kind: "session", projectId: "project", sessionId }, sessionId,
    { checkoutBinding: { checkoutId: "checkout" }, preview: false, pinned: true },
  );
  store.openEditor("main", editor("original"));
  const targetPaneId = store.splitPane("main", "main", "right", editor("target"))!;
  store.focusPane("main", "main");
  const before = JSON.stringify(state);
  const node: ProjectExplorerNode = {
    nodeId: "new-session", projectId: "project", nodeKind: "resource", resourceKind: "system",
    resourceId: "newSession", position: 8, parentNodeId: "folder", hidden: false,
  };
  const folder: ProjectExplorerNode = {
    nodeId: "folder", projectId: "project", nodeKind: "folder", position: 0, hidden: false,
  };
  const snapshot = { nodes: [folder, node], itemStates: [] };
  const treeItems = ref<Array<WorkspaceTreeItem & { meta: { kind: string; projectId: string; explorerNode: ProjectExplorerNode } }>>([]);
  const host = document.createElement("div");
  document.body.append(host);
  let controller!: InternalDragController;
  let runtime: {
    onDragPointerDown: (item: WorkspaceTreeItem, event: PointerEvent) => void;
    appendNewSessionRow: (items: WorkspaceTreeItem[], project: { projectId: string; checkouts: unknown[] }, depth: number) => void;
    resolveWorkbenchInternalDrop: InternalDropTargetRegistration["resolve"];
    commitWorkbenchInternalDrop: (sourceType: string, sourceData: unknown, intent: unknown) => Promise<void>;
  };
  const activate = vi.fn();
  const app = createApp({ setup() {
    controller = provideInternalDragController();
    return () => h("div", [
      h("aside", { class: "explorer" }, [h(WorkspaceTree, {
        items: treeItems.value, onDragPointerDown: (item, event) => runtime.onDragPointerDown(item, event), onActivate: activate,
      })]),
      h("section", { class: "workbench-editor-group", "data-workbench-pane-id": targetPaneId }, [
        h("div", { class: "workbench-editor-tabs", "data-workbench-pane-id": targetPaneId }, "Target tabs"),
      ]),
    ]);
  } });
  app.mount(host);
  cleanups.push(() => { controller.dispose(); app.unmount(); });
  const explorer = host.querySelector<HTMLElement>(".explorer")!;
  const tabs = host.querySelector<HTMLElement>(".workbench-editor-tabs")!;
  const group = host.querySelector<HTMLElement>(".workbench-editor-group")!;
  vi.spyOn(group, "getBoundingClientRect").mockReturnValue(new DOMRect(300, 0, 400, 400));
  vi.spyOn(tabs, "getBoundingClientRect").mockReturnValue(new DOMRect(300, 0, 400, 30));
  const hitTest = vi.fn<() => Element | null>(() => tabs);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hitTest });
  const applyOperations = vi.fn();
  let currentCheckout = initialCheckout;
  const deps = {
    SYSTEM_RESOURCE_KIND: "system", NEW_SESSION_SYSTEM_RESOURCE_ID: "newSession",
    WORKBENCH_WINDOW_ID: "main", WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE: "layout",
    WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE: "tab", KNOWLEDGE_INTERNAL_DRAG_TYPE: "knowledge",
    WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE: "reference", VIEW_TREE_INTERNAL_DRAG_TYPE: "view",
    internalDrag: controller, treeItems, workbenchWindow: ref(state), workbenchRootRef: ref(host), explorerRootRef: ref(explorer),
    settlingLayoutDrop: ref(null), activeResource: ref(null), dragging: ref(null),
    dropTargetKey: ref(null), layoutDropIntent: ref(null), editorDropIntent: ref(null), composerDropTarget: ref(null),
    pinDropProjectId: ref(null), contextMenu: ref(null), displayMenu: ref(null), workspaceMenu: ref(null),
    presetProjectId: ref("project"), chatStore: { activeSessionId: null },
    workspaceContextStore: { focusedCheckout: { projectId: "project", checkoutId: "checkout" } },
    explorerStore: { snapshots: { project: snapshot }, resources: { project: { knowledge: [] } }, applyOperations },
    renderedExplorerSnapshot: () => snapshot, isExplorerNodeVisible: () => true,
    isNewSessionDropAvailable: () => false, newSessionDropDraft: () => null,
    isKnowledgeTypeFolder: () => false, itemIcon: () => undefined, itemIconClass: () => undefined,
    workspaceSecondarySection, workbenchSplitDirectionAtPoint, workbenchTabInsertionIndexAtPoint,
    t: (key: string) => key,
    usesCheckoutScopedWorkbench: () => checkoutScoped,
    activateCheckoutScopedWorkbench: vi.fn(async () => { currentCheckout = "checkout"; return true; }),
    workbenchStore: { ...store, workspaceScope: () => currentCheckout },
    createEditorForResource: (resource: WorkbenchResourceRef, options: { title: string; checkoutId: string; preview?: boolean; pinned?: boolean }) => (
      createWorkbenchEditorInput(resource, options.title, {
        preview: options.preview, pinned: options.pinned, checkoutBinding: { checkoutId: options.checkoutId },
      })
    ),
    focusWorkbenchEditor: async (paneId: string, editorId: string) => store.activateEditor("main", paneId, editorId),
    refreshWorkbenchFileEditor: async () => undefined,
  };
  runtime = new Function(...Object.keys(deps), `${code}\nreturn { appendNewSessionRow, onDragPointerDown, resolveWorkbenchInternalDrop, commitWorkbenchInternalDrop };`)(...Object.values(deps));
  runtime.appendNewSessionRow(treeItems.value, { projectId: "project", checkouts: [] }, 0);
  treeItems.value.push({
    key: "folder", meta: { kind: "folder", projectId: "project", explorerNode: folder },
    treeRow: { key: "folder", name: "Folder", depth: 0, kind: "folder" },
  });
  await nextTick();
  const drop = vi.fn(({ source, decision }: InternalDropCommitContext) => runtime.commitWorkbenchInternalDrop(source.payload.type, source.payload.data, decision.intent));
  controller.registerTarget({ id: "workbench", root: () => host, accepts: () => true, resolve: runtime.resolveWorkbenchInternalDrop, drop });
  const row = host.querySelector<HTMLButtonElement>('[data-tree-key="new-session:project"] button')!;
  const drag = () => {
    row.dispatchEvent(pointer("pointerdown", 20, 20));
    window.dispatchEvent(pointer("pointermove"));
  };
  const release = async () => {
    window.dispatchEvent(pointer("pointerup"));
    await vi.waitFor(() => expect(controller.phase.value).toBe("idle"));
    await nextTick();
  };
  return { state, before, node, folder, snapshot, targetPaneId, row, explorer, group, hitTest, controller, drag, release, drop, applyOperations, activate, treeItems };
}

describe("workspace New Session drag", () => {
  it.each([false, true])("creates independent tabs in the hovered group (checkout scoped: %s)", async (checkoutScoped) => {
    const f = await fixture(checkoutScoped);
    for (let i = 0; i < 2; i++) {
      f.drag();
      expect(f.controller.source.value?.allowedOperations).toEqual(["copy"]);
      expect(f.controller.activeTarget.value?.decision.operation).toBe("copy");
      await f.release();
    }
    const tabs = f.state.groups[f.targetPaneId]!.tabs;
    expect(tabs.map((tab) => tab.resource.kind)).toEqual(["session", "newSession", "newSession"]);
    expect(new Set(tabs.map((tab) => tab.editorId)).size).toBe(3);
    expect(tabs.slice(1).every((tab) => tab.pinned && !tab.preview && tab.checkoutBinding?.checkoutId === "checkout")).toBe(true);
    expect(f.state.groups.main!.tabs).toHaveLength(1);
    expect(f.treeItems.value[0]!.key).toBe("new-session:project");
    expect(f.node.parentNodeId).toBe("folder");
    expect(f.node.position).toBe(8);
    expect(f.applyOperations).not.toHaveBeenCalled();
    f.row.click();
    expect(f.activate).not.toHaveBeenCalled();
  });

  it("creates a new session in a split beside the hovered group", async () => {
    const f = await fixture(true);
    f.hitTest.mockReturnValue(f.group);
    f.drag();
    await f.release();
    const newGroup = Object.values(f.state.groups).find((group) => group.tabs[0]?.resource.kind === "newSession");
    expect(newGroup?.tabs).toHaveLength(1);
    expect(f.state.layout.kind).toBe("split");
    if (f.state.layout.kind === "split") {
      expect(f.state.layout.first).toEqual({ kind: "group", paneId: "main" });
      expect(f.state.layout.second.kind).toBe("split");
    }
    expect(f.applyOperations).not.toHaveBeenCalled();
  });

  it("uses the restored focused group when a drop switches checkout layouts", async () => {
    const f = await fixture(true, "previous-checkout");
    f.drag();
    await f.release();
    expect(f.state.groups.main!.tabs.map((tab) => tab.resource.kind)).toEqual(["session", "newSession"]);
    expect(f.state.groups[f.targetPaneId]!.tabs).toHaveLength(1);
    expect(f.applyOperations).not.toHaveBeenCalled();
  });

  it.each(["root", "folder", "pin", "self"])("does not move the fixed entry onto %s", async (target) => {
    const f = await fixture();
    if (target === "pin") f.treeItems.value[1]!.treeRow!.pinned = true;
    await nextTick();
    f.hitTest.mockReturnValue(target === "root" ? f.explorer : target === "self" ? f.row
      : f.explorer.querySelector('[data-tree-key="folder"]')!);
    f.drag();
    expect(f.controller.phase.value).toBe("dragging");
    expect(f.controller.activeTarget.value).toBeNull();
    await f.release();
    expect(f.drop).not.toHaveBeenCalled();
    expect(f.applyOperations).not.toHaveBeenCalled();
    expect(JSON.stringify(f.state)).toBe(f.before);
  });

  it("cancels without creating a session and retains ordinary click activation", async () => {
    const f = await fixture();
    f.row.click();
    expect(f.activate).toHaveBeenCalledOnce();
    f.drag();
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(f.controller.phase.value).toBe("idle");
    expect(f.drop).not.toHaveBeenCalled();
    expect(JSON.stringify(f.state)).toBe(f.before);
  });
});
