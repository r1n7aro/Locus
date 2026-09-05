import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import {
  WORKBENCH_MAX_SPLIT_RATIO,
  createWorkbenchEditorInput,
  shouldShowWorkbenchTabStrip,
  useWorkbenchStore,
  workbenchResourceKey,
} from "../stores/workbench";

function sessionEditor(
  sessionId: string,
  options: { preview?: boolean; checkoutId?: string; projectId?: string } = {},
) {
  return createWorkbenchEditorInput(
    { kind: "session", projectId: options.projectId ?? "project-a", sessionId },
    sessionId,
    {
      preview: options.preview ?? true,
      pinned: options.preview === false,
      checkoutBinding: {
        checkoutId: options.checkoutId ?? "checkout-a",
        expectedGeneration: 7,
      },
    },
  );
}

describe("workbench store", () => {
  beforeEach(() => {
    const values = new Map<string, string>();
    const localStorage = {
      get length() { return values.size; },
      clear: () => values.clear(),
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      removeItem: (key: string) => { values.delete(key); },
      setItem: (key: string, value: string) => { values.set(key, String(value)); },
    } satisfies Storage;
    vi.stubGlobal("window", { localStorage });
    vi.stubGlobal("localStorage", localStorage);
    setActivePinia(createPinia());
    localStorage.clear();
  });

  it("uses project resource identity independently from checkout runtime generations", () => {
    expect(workbenchResourceKey({
      kind: "knowledge",
      projectId: "project-a",
      documentId: "doc-a",
    })).toBe("knowledge:project-a:doc-a");
    expect(workbenchResourceKey({
      kind: "session",
      projectId: "project-a",
      sessionId: "session-a",
    })).toBe("session:project-a:session-a");
    expect(workbenchResourceKey({
      kind: "asset",
      projectId: "project-a",
      path: "Assets/Player.prefab",
    })).toBe("asset:project-a:Assets/Player.prefab");
    expect(workbenchResourceKey({
      kind: "asset",
      projectId: "project-a",
      path: "Assets/Docs/Rules.md",
    })).toBe("workspace-file:project-a:Assets/Docs/Rules.md");
    expect(workbenchResourceKey({
      kind: "sceneObject",
      projectId: "project-a",
      scenePath: "Assets/Main.unity",
      objectPath: "Player/Camera",
    })).toBe("scene-object:project-a:Assets/Main.unity:Player/Camera");
    expect(workbenchResourceKey({
      kind: "localDirectory",
      projectId: "project-a",
      nodeId: "mount-a",
      relativePath: "Assets/Characters",
    })).toBe("local-directory:project-a:mount-a:Assets/Characters");
  });

  it("routes new and restored Markdown assets through the workspace file editor", () => {
    const markdownEditor = createWorkbenchEditorInput({
      kind: "asset",
      projectId: "project-a",
      path: "Assets/Docs/Rules.md",
    }, "Rules.md");
    expect(markdownEditor.resource).toEqual({
      kind: "workspaceFile",
      projectId: "project-a",
      path: "Assets/Docs/Rules.md",
    });

    const firstStore = useWorkbenchStore();
    firstStore.ensureWindow("main");
    firstStore.openEditor("main", createWorkbenchEditorInput({
      kind: "asset",
      projectId: "project-a",
      path: "Assets/Player.prefab",
    }, "Rules.md"));
    firstStore.persist("main");
    const legacy = JSON.parse(localStorage.getItem("locus:workbench-window:main")!);
    legacy.groups.main.tabs[0].resource = {
      kind: "asset",
      projectId: "project-a",
      path: "Assets/Docs/Legacy.markdown",
    };
    localStorage.setItem("locus:workbench-window:main", JSON.stringify(legacy));

    setActivePinia(createPinia());
    const restored = useWorkbenchStore().ensureWindow("main");
    expect(restored.groups.main!.tabs[0]!.resource).toEqual({
      kind: "workspaceFile",
      projectId: "project-a",
      path: "Assets/Docs/Legacy.markdown",
    });
  });

  it("replaces one unpinned preview and derives tab-strip visibility from tab count", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const group = state.groups.main!;

    store.openEditor("main", sessionEditor("session-a"));
    expect(group.tabs.map((tab) => tab.title)).toEqual(["session-a"]);
    expect(shouldShowWorkbenchTabStrip(group)).toBe(false);
    expect(shouldShowWorkbenchTabStrip(group, true)).toBe(true);

    store.openEditor("main", sessionEditor("session-b"));
    expect(group.tabs.map((tab) => tab.title)).toEqual(["session-b"]);
    expect(shouldShowWorkbenchTabStrip(group)).toBe(false);

    store.pinEditor("main", "main", group.tabs[0]!.editorId);
    store.openEditor("main", sessionEditor("session-c"));
    expect(group.tabs.map((tab) => tab.title)).toEqual(["session-b", "session-c"]);
    expect(shouldShowWorkbenchTabStrip(group)).toBe(true);
  });

  it("can preserve previews from different workspaces during continuity fallback", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const group = state.groups.main!;

    store.openEditor("main", sessionEditor("session-a"));
    store.openEditor("main", sessionEditor("session-b", {
      checkoutId: "checkout-b",
      projectId: "project-b",
    }), { replacePreview: false });

    expect(group.tabs.map((tab) => tab.resource.projectId)).toEqual([
      "project-a",
      "project-b",
    ]);
  });

  it("closes active tabs within their checkout without activating a foreign project", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const group = state.groups.main!;
    const firstB = store.openEditor("main", sessionEditor("session-b-1", {
      preview: false,
      checkoutId: "checkout-b",
      projectId: "project-b",
    }));
    store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const secondB = store.openEditor("main", sessionEditor("session-b-2", {
      preview: false,
      checkoutId: "checkout-b",
      projectId: "project-b",
    }));

    store.closeEditor("main", "main", secondB.editorId);
    expect(group.activeEditorId).toBe(firstB.editorId);
    expect(group.focusedCheckoutId).toBe("checkout-b");

    store.closeEditor("main", "main", firstB.editorId);
    expect(group.tabs.map((tab) => tab.resource.projectId)).toEqual(["project-a"]);
    expect(group.activeEditorId).toBeNull();
    expect(group.focusedCheckoutId).toBeNull();
  });

  it("creates horizontal and vertical binary splits with stable pane ids", () => {
    const store = useWorkbenchStore();
    store.ensureWindow("main");
    store.openEditor("main", sessionEditor("session-a", { preview: false }));

    const rightPane = store.splitPane("main", "main", "right", sessionEditor("session-b"));
    expect(rightPane).toMatch(/^pane-/);
    const state = store.windows.main!;
    expect(state.layout.kind).toBe("split");
    if (state.layout.kind !== "split") throw new Error("expected split");
    expect(state.layout.orientation).toBe("horizontal");
    expect(state.layout.first).toEqual({ kind: "group", paneId: "main" });
    expect(state.layout.second).toEqual({ kind: "group", paneId: rightPane });

    const bottomPane = store.splitPane("main", rightPane!, "bottom", sessionEditor("session-c"));
    expect(bottomPane).toMatch(/^pane-/);
    expect(Object.keys(state.groups)).toHaveLength(3);
  });

  it("supports four and more independent editor groups without a fixed pane count", () => {
    const store = useWorkbenchStore();
    store.ensureWindow("main");
    store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const right = store.splitPane("main", "main", "right", sessionEditor("session-b"))!;
    const bottomLeft = store.splitPane("main", "main", "bottom", sessionEditor("session-c"))!;
    const bottomRight = store.splitPane("main", right, "bottom", sessionEditor("session-d"))!;

    const state = store.windows.main!;
    expect(Object.keys(state.groups)).toHaveLength(4);
    expect(new Set(Object.keys(state.groups)).size).toBe(4);
    expect(state.groups.main!.tabs[0]!.title).toBe("session-a");
    expect(state.groups[right]!.tabs[0]!.title).toBe("session-b");
    expect(state.groups[bottomLeft]!.tabs[0]!.title).toBe("session-c");
    expect(state.groups[bottomRight]!.tabs[0]!.title).toBe("session-d");

    store.closePane("main", bottomLeft);
    expect(Object.keys(state.groups)).toHaveLength(3);
    expect(state.groups[bottomRight]).toBeDefined();
  });

  it("rebalances repeated same-axis splits so four panes share the available width", () => {
    const store = useWorkbenchStore();
    store.ensureWindow("main");
    store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const second = store.splitPane("main", "main", "right", sessionEditor("session-b"))!;
    const third = store.splitPane("main", second, "right", sessionEditor("session-c"))!;
    store.splitPane("main", third, "right", sessionEditor("session-d"));

    const layout = store.windows.main!.layout;
    expect(layout.kind).toBe("split");
    if (layout.kind !== "split" || layout.second.kind !== "split") {
      throw new Error("expected a horizontal split chain");
    }
    expect(layout.ratio).toBeCloseTo(0.25);
    expect(layout.second.ratio).toBeCloseTo(1 / 3);
    expect(layout.second.second.kind).toBe("split");
    if (layout.second.second.kind !== "split") throw new Error("expected final split");
    expect(layout.second.second.ratio).toBeCloseTo(0.5);
  });

  it("moves tabs between groups and collapses empty split branches", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const first = store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const second = store.openEditor("main", sessionEditor("session-b", { preview: false }));
    const rightPane = store.splitPane("main", "main", "right", sessionEditor("session-c"))!;

    store.moveEditor("main", "main", second.editorId, rightPane, { direction: "center" });
    expect(state.groups.main!.tabs.map((tab) => tab.editorId)).toEqual([first.editorId]);
    expect(state.groups[rightPane]!.tabs.map((tab) => tab.title)).toEqual(["session-c", "session-b"]);

    store.closeEditor("main", "main", first.editorId);
    expect(state.groups.main).toBeUndefined();
    expect(state.layout).toEqual({ kind: "group", paneId: rightPane });
    expect(state.focusedPaneId).toBe(rightPane);
  });

  it("activates a background tab without moving editor-group focus", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const first = store.openEditor("main", sessionEditor("session-a", { preview: false }));
    store.openEditor("main", sessionEditor("session-b", { preview: false }));
    const rightPane = store.splitPane("main", "main", "right", sessionEditor("session-c"))!;

    expect(state.focusedPaneId).toBe(rightPane);
    expect(state.groups.main!.activeEditorId).not.toBe(first.editorId);

    store.activateEditor("main", "main", first.editorId, { focusPane: false });

    expect(state.groups.main!.activeEditorId).toBe(first.editorId);
    expect(state.focusedPaneId).toBe(rightPane);
  });

  it("replaces the focused editor at its exact tab position", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const first = store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const second = store.openEditor("main", sessionEditor("session-b", { preview: false }));
    store.activateEditor("main", "main", first.editorId);
    const replacement = sessionEditor("session-c", { preview: false });

    expect(store.replaceEditor("main", "main", first.editorId, replacement)).toEqual(replacement);
    expect(state.groups.main!.tabs.map((editor) => editor.editorId)).toEqual([
      replacement.editorId,
      second.editorId,
    ]);
    expect(state.groups.main!.activeEditorId).toBe(replacement.editorId);
  });

  it("accepts transferred editors into auxiliary windows and reports deduplication", () => {
    const store = useWorkbenchStore();
    const source = sessionEditor("session-a", { preview: false });
    const target = store.ensureWindow("workbench-a");
    const paneId = target.focusedPaneId;

    const first = store.acceptTransferredEditor(
      "workbench-a",
      source,
      paneId,
      { direction: "center", index: 0 },
    );
    expect(first).toEqual({ paneId, editorId: source.editorId, inserted: true });
    expect(target.groups[paneId]!.tabs[0]!.pinned).toBe(true);

    const duplicate = store.acceptTransferredEditor(
      "workbench-a",
      sessionEditor("session-a", { preview: false }),
      paneId,
      { direction: "center" },
    );
    expect(duplicate).toEqual({ paneId, editorId: source.editorId, inserted: false });
    expect(target.groups[paneId]!.tabs).toHaveLength(1);

    const explicitDuplicate = store.acceptTransferredEditor(
      "workbench-a",
      sessionEditor("session-a", { preview: false }),
      paneId,
      { direction: "center", allowDuplicate: true },
    );
    expect(explicitDuplicate?.inserted).toBe(true);
    expect(target.groups[paneId]!.tabs).toHaveLength(2);

    const split = store.acceptTransferredEditor(
      "workbench-a",
      sessionEditor("session-b", { preview: false }),
      paneId,
      { direction: "right" },
    );
    expect(split?.inserted).toBe(true);
    expect(split?.paneId).not.toBe(paneId);
    expect(target.layout.kind).toBe("split");
  });

  it("reorders within a group using tab-half insertion positions", () => {
    const store = useWorkbenchStore();
    const state = store.ensureWindow("main");
    const first = store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const second = store.openEditor("main", sessionEditor("session-b", { preview: false }));
    const third = store.openEditor("main", sessionEditor("session-c", { preview: false }));

    store.moveEditor("main", "main", first.editorId, "main", { direction: "center", index: 2 });
    expect(state.groups.main!.tabs.map((tab) => tab.editorId)).toEqual([
      second.editorId,
      first.editorId,
      third.editorId,
    ]);

    store.moveEditor("main", "main", third.editorId, "main", { direction: "center", index: 0 });
    expect(state.groups.main!.tabs.map((tab) => tab.editorId)).toEqual([
      third.editorId,
      second.editorId,
      first.editorId,
    ]);
  });

  it("clamps separator ratios and persists them", () => {
    const store = useWorkbenchStore();
    store.ensureWindow("main");
    store.splitPane("main", "main", "right", sessionEditor("session-b"));
    const layout = store.windows.main!.layout;
    if (layout.kind !== "split") throw new Error("expected split");

    store.updateSplitRatio("main", layout.splitId, 0.99);
    expect(layout.ratio).toBe(WORKBENCH_MAX_SPLIT_RATIO);
    const persisted = JSON.parse(localStorage.getItem("locus:workbench-window:main")!);
    expect(persisted.layout.ratio).toBe(WORKBENCH_MAX_SPLIT_RATIO);
  });

  it("restores stable resources while dropping runtime generations", () => {
    const firstStore = useWorkbenchStore();
    firstStore.ensureWindow("main");
    firstStore.openEditor("main", sessionEditor("session-a", { preview: false }));
    firstStore.persist("main");

    const persisted = JSON.parse(localStorage.getItem("locus:workbench-window:main")!);
    expect(persisted.groups.main.tabs[0].checkoutBinding).toEqual({ checkoutId: "checkout-a" });

    setActivePinia(createPinia());
    const restoredStore = useWorkbenchStore();
    const restored = restoredStore.ensureWindow("main");
    expect(restored.groups.main!.tabs[0]!.resource).toEqual({
      kind: "session",
      projectId: "project-a",
      sessionId: "session-a",
    });
    expect(restored.groups.main!.tabs[0]!.checkoutBinding).toEqual({ checkoutId: "checkout-a" });
  });

  it("keeps independent group layouts, tabs, and active editors for each workspace", () => {
    const store = useWorkbenchStore();
    store.switchWorkspaceScope("main", "checkout-a");
    const first = store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const second = store.openEditor("main", sessionEditor("session-b", { preview: false }));
    const rightPane = store.splitPane(
      "main",
      "main",
      "right",
      sessionEditor("session-c", { preview: false }),
    )!;
    store.activateEditor("main", "main", first.editorId);

    store.switchWorkspaceScope("main", "checkout-b");
    const checkoutBState = store.windows.main!;
    expect(checkoutBState.layout).toEqual({ kind: "group", paneId: "main" });
    const checkoutBEditor = store.openEditor(
      "main",
      sessionEditor("session-d", { preview: false, checkoutId: "checkout-b" }),
    );
    expect(checkoutBState.groups.main!.tabs.map((tab) => tab.editorId)).toEqual([
      checkoutBEditor.editorId,
    ]);

    const checkoutAState = store.switchWorkspaceScope("main", "checkout-a");
    expect(checkoutAState.layout.kind).toBe("split");
    expect(checkoutAState.groups.main!.tabs.map((tab) => tab.editorId)).toEqual([
      first.editorId,
      second.editorId,
    ]);
    expect(checkoutAState.groups.main!.activeEditorId).toBe(first.editorId);
    expect(checkoutAState.groups[rightPane]!.tabs.map((tab) => tab.title)).toEqual(["session-c"]);

    const restoredCheckoutB = store.switchWorkspaceScope("main", "checkout-b");
    expect(restoredCheckoutB.layout).toEqual({ kind: "group", paneId: "main" });
    expect(restoredCheckoutB.groups.main!.tabs.map((tab) => tab.editorId)).toEqual([
      checkoutBEditor.editorId,
    ]);
    expect(localStorage.getItem("locus:workbench-window:main:workspace:checkout-a")).not.toBeNull();
    expect(localStorage.getItem("locus:workbench-window:main:workspace:checkout-b")).not.toBeNull();
  });

  it("rejects a foreign checkout editor before it can mutate a scoped layout", () => {
    const store = useWorkbenchStore();
    store.switchWorkspaceScope("main", "checkout-a");
    store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const key = "locus:workbench-window:main:workspace:checkout-a";
    const persistedBefore = localStorage.getItem(key);

    expect(() => store.openEditor(
      "main",
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    )).toThrow(/openEditor requires checkout checkout-a; received checkout-b/);
    const activeEditorId = store.windows.main!.groups.main!.activeEditorId!;
    expect(() => store.updateEditor("main", "main", activeEditorId, {
      checkoutBinding: { checkoutId: "checkout-b" },
    })).toThrow(/updateEditor requires checkout checkout-a; received checkout-b/);
    expect(() => store.replaceEditor(
      "main",
      "main",
      activeEditorId,
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    )).toThrow(/replaceEditor requires checkout checkout-a; received checkout-b/);
    expect(() => store.splitPane(
      "main",
      "main",
      "right",
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    )).toThrow(/splitPane requires checkout checkout-a; received checkout-b/);
    expect(() => store.acceptTransferredEditor(
      "main",
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
      "main",
    )).toThrow(/acceptTransferredEditor requires checkout checkout-a; received checkout-b/);
    expect(localStorage.getItem(key)).toBe(persistedBefore);
    expect(store.windows.main!.groups.main!.tabs.map((editor) => (
      editor.checkoutBinding?.checkoutId
    ))).toEqual(["checkout-a"]);
  });

  it("repairs shifted workspace slots by each editor's checkout binding", () => {
    const firstStore = useWorkbenchStore();
    firstStore.switchWorkspaceScope("main", "checkout-a");
    firstStore.openEditor("main", sessionEditor("session-a", { preview: false }));
    firstStore.switchWorkspaceScope("main", "checkout-b");
    firstStore.openEditor(
      "main",
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    );
    firstStore.switchWorkspaceScope("main", "checkout-c");
    firstStore.openEditor(
      "main",
      sessionEditor("session-c", { preview: false, checkoutId: "checkout-c" }),
    );

    const keyA = "locus:workbench-window:main:workspace:checkout-a";
    const keyB = "locus:workbench-window:main:workspace:checkout-b";
    const keyC = "locus:workbench-window:main:workspace:checkout-c";
    const stateA = localStorage.getItem(keyA)!;
    const stateB = localStorage.getItem(keyB)!;
    const stateC = localStorage.getItem(keyC)!;
    localStorage.setItem(keyA, stateB);
    localStorage.setItem(keyB, stateC);
    localStorage.setItem(keyC, stateA);

    setActivePinia(createPinia());
    const restoredStore = useWorkbenchStore();
    const restoredA = restoredStore.switchWorkspaceScope("main", "checkout-a");
    expect(restoredA.groups.main!.tabs.map((editor) => editor.title)).toEqual(["session-a"]);
    const restoredB = restoredStore.switchWorkspaceScope("main", "checkout-b");
    expect(restoredB.groups.main!.tabs.map((editor) => editor.title)).toEqual(["session-b"]);
    const restoredC = restoredStore.switchWorkspaceScope("main", "checkout-c");
    expect(restoredC.groups.main!.tabs.map((editor) => editor.title)).toEqual(["session-c"]);
  });

  it("partitions a mixed historical layout without duplicating an existing destination editor", () => {
    const firstStore = useWorkbenchStore();
    firstStore.switchWorkspaceScope("main", "checkout-a");
    firstStore.openEditor("main", sessionEditor("session-a", { preview: false }));
    firstStore.switchWorkspaceScope("main", "checkout-b");
    firstStore.openEditor(
      "main",
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    );

    const keyA = "locus:workbench-window:main:workspace:checkout-a";
    const keyB = "locus:workbench-window:main:workspace:checkout-b";
    const stateA = JSON.parse(localStorage.getItem(keyA)!);
    const stateB = JSON.parse(localStorage.getItem(keyB)!);
    const foreignEditor = stateB.groups.main.tabs[0];
    stateA.groups.main.tabs.push(foreignEditor);
    stateA.groups.main.activeEditorId = foreignEditor.editorId;
    stateA.groups.main.focusedCheckoutId = "checkout-b";
    localStorage.setItem(keyA, JSON.stringify(stateA));

    setActivePinia(createPinia());
    const restoredStore = useWorkbenchStore();
    const restoredA = restoredStore.switchWorkspaceScope("main", "checkout-a");
    expect(restoredA.groups.main!.tabs.map((editor) => editor.title)).toEqual(["session-a"]);
    const restoredB = restoredStore.switchWorkspaceScope("main", "checkout-b");
    expect(restoredB.groups.main!.tabs.map((editor) => editor.title)).toEqual(["session-b"]);
  });

  it("re-homes a lone displaced layout and restores the source slot as empty", () => {
    const firstStore = useWorkbenchStore();
    firstStore.switchWorkspaceScope("main", "checkout-b");
    firstStore.openEditor(
      "main",
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    );

    const keyA = "locus:workbench-window:main:workspace:checkout-a";
    const keyB = "locus:workbench-window:main:workspace:checkout-b";
    localStorage.setItem(keyA, localStorage.getItem(keyB)!);
    localStorage.removeItem(keyB);

    setActivePinia(createPinia());
    const restoredStore = useWorkbenchStore();
    const restoredA = restoredStore.switchWorkspaceScope("main", "checkout-a");
    expect(restoredA.groups.main!.tabs).toEqual([]);
    const restoredB = restoredStore.switchWorkspaceScope("main", "checkout-b");
    expect(restoredB.groups.main!.tabs.map((editor) => editor.title)).toEqual(["session-b"]);
  });

  it("refuses to overwrite a scoped key when in-memory state is contaminated", () => {
    const store = useWorkbenchStore();
    store.switchWorkspaceScope("main", "checkout-a");
    const state = store.windows.main!;
    store.openEditor("main", sessionEditor("session-a", { preview: false }));
    const key = "locus:workbench-window:main:workspace:checkout-a";
    const persistedBefore = localStorage.getItem(key);
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    state.groups.main!.tabs.push(
      sessionEditor("session-b", { preview: false, checkoutId: "checkout-b" }),
    );

    store.persist("main");

    expect(localStorage.getItem(key)).toBe(persistedBefore);
    expect(error).toHaveBeenCalledWith(
      "[workbench] refused to persist a layout outside checkout scope checkout-a",
    );
    state.groups.main!.tabs = state.groups.main!.tabs.filter(
      (editor) => editor.checkoutBinding?.checkoutId === "checkout-a",
    );
    state.groups.main!.focusedCheckoutId = "checkout-b";
    store.persist("main");
    expect(localStorage.getItem(key)).toBe(persistedBefore);
    expect(error).toHaveBeenCalledTimes(2);
    error.mockRestore();
  });

  it("restores a workspace-scoped layout after recreating the store", () => {
    const firstStore = useWorkbenchStore();
    firstStore.switchWorkspaceScope("main", "checkout-a");
    firstStore.openEditor("main", sessionEditor("session-a", { preview: false }));

    setActivePinia(createPinia());
    const restoredStore = useWorkbenchStore();
    const restored = restoredStore.switchWorkspaceScope("main", "checkout-a");
    expect(restored.groups.main!.tabs.map((tab) => tab.title)).toEqual(["session-a"]);
    expect(restoredStore.workspaceScope("main")).toBe("checkout-a");
  });

  it("migrates legacy knowledge and collaboration editor identities to project sections", () => {
    localStorage.setItem("locus:workbench-window:main", JSON.stringify({
      schemaVersion: 1,
      windowId: "main",
      sidebar: { width: 300, collapsed: false },
      layout: { kind: "group", paneId: "main" },
      groups: {
        main: {
          paneId: "main",
          activeEditorId: "legacy-knowledge",
          tabs: [{
            editorId: "legacy-knowledge",
            resource: { kind: "knowledgeRoot", projectId: "project-a", checkoutId: "checkout-a" },
            title: "Knowledge",
            preview: false,
            pinned: true,
            dirty: false,
            capabilities: { split: true, detach: true, duplicate: true },
            availability: "available",
          }],
        },
      },
      focusedPaneId: "main",
    }));

    const restored = useWorkbenchStore().ensureWindow("main");
    expect(restored.groups.main!.tabs[0]!.resource).toEqual({
      kind: "section",
      projectId: "project-a",
      section: "knowledge",
    });
    expect(restored.groups.main!.tabs[0]!.checkoutBinding).toEqual({ checkoutId: "checkout-a" });
  });
});
