// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { createPinia } from "pinia";
import ts from "typescript";
import { createApp, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import WorkbenchEditorTabs from "../components/workbench/WorkbenchEditorTabs.vue";
import { workbenchComposerEditorAttachment } from "../components/workbench/workbenchComposerDrop";
import { WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE } from "../components/workbench/workbenchDrag";
import { workbenchSplitDirectionAtPoint, workbenchTabInsertionIndexAtPoint } from "../components/workbench/workbenchDropGeometry";
import { emptyComposerIntent } from "../composables/chatInputIntents";
import { provideInternalDragController, type InternalDragController } from "../composables/useInternalDrag";
import { createWorkbenchEditorInput } from "../stores/workbench";
import type { WorkbenchResourceRef, WorkbenchWindowState } from "../types/workbench";

vi.mock("../stores/chat", () => ({
  useChatStore: () => ({ streamingSessionIds: new Set(), sessions: [] }),
}));

// Execute the actual workbench handlers without booting unrelated Unity/backend services.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const handlerNames = [
  "attachmentDraft", "editorWorkingDir", "editorKnowledgeDocument", "composerDraftForInternalDrop",
  "resolveWorkbenchInternalDrop", "handleWorkbenchInternalTargetChange", "commitWorkbenchInternalDrop",
];
const handlers = parsed.statements.filter((statement) => (
  ts.isFunctionDeclaration(statement) && !!statement.name && handlerNames.includes(statement.name.text)
)).map((statement) => statement.getText(parsed)).join("\n");
const code = ts.transpileModule(handlers, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const cleanups: Array<() => void> = [];
afterEach(() => {
  cleanups.splice(0).reverse().forEach((cleanup) => cleanup());
  document.body.replaceChildren();
  vi.restoreAllMocks();
});

function pointer(type: string, x: number, y: number) {
  const event = new MouseEvent(type, {
    bubbles: true, cancelable: true, button: 0, buttons: type === "pointerup" ? 0 : 1,
    clientX: x, clientY: y,
  });
  Object.defineProperties(event, {
    pointerId: { value: 1 }, isPrimary: { value: true }, pointerType: { value: "mouse" },
  });
  return event;
}

async function fixture(resource: WorkbenchResourceRef, split: boolean) {
  const sourceEditor = createWorkbenchEditorInput(resource, "Source", {
    checkoutBinding: { checkoutId: "checkout" }, sourcePath: "F:/Game/docs/notes.md", preview: true, pinned: false,
  });
  const composerEditor = createWorkbenchEditorInput({ kind: "newSession", projectId: "p" }, "Chat", { checkoutBinding: { checkoutId: "checkout" } });
  const workbenchWindow = ref<WorkbenchWindowState>({
    schemaVersion: 1, windowId: "main", sidebar: { width: 260, collapsed: false }, focusedPaneId: "target",
    layout: split
      ? { kind: "split", splitId: "split", orientation: "horizontal", ratio: 0.4, first: { kind: "group", paneId: "source" }, second: { kind: "group", paneId: "target" } }
      : { kind: "group", paneId: "target" },
    groups: {
      ...(split ? { source: { paneId: "source", tabs: [sourceEditor], activeEditorId: sourceEditor.editorId } } : {}),
      target: { paneId: "target", tabs: split ? [composerEditor] : [sourceEditor, composerEditor], activeEditorId: composerEditor.editorId },
    },
  });
  const before = JSON.stringify(workbenchWindow.value);
  const host = document.createElement("div");
  document.body.append(host);
  let controller!: InternalDragController;
  const activate = vi.fn();
  const externalize = vi.fn();
  const app = createApp({ setup() {
    controller = provideInternalDragController();
    return () => h("div", { class: "development-workbench" }, Object.values(workbenchWindow.value.groups).map((group) => (
      h("section", { class: "workbench-editor-group", "data-workbench-pane-id": group.paneId }, [
        h(WorkbenchEditorTabs, { windowId: "main", group, showSingleTab: true, onActivate: activate, "onDrag-externalize": externalize }),
        h("div", { class: "editor-body" }, group.paneId === "target"
          ? [h("div", { class: "chat-composer" }, [h("textarea")])]
          : []),
      ])
    )));
  } });
  app.use(createPinia());
  app.mount(host);
  cleanups.push(() => { controller.dispose(); app.unmount(); });
  await nextTick();
  const sourceTab = host.querySelector<HTMLElement>(`[data-workbench-tab-id="${sourceEditor.editorId}"]`)!;
  const composer = host.querySelector("textarea")!;
  const body = host.querySelector('[data-workbench-pane-id="target"] .editor-body')!;
  const hitTest = vi.fn<() => Element | null>(() => body);
  Object.defineProperty(document, "elementFromPoint", { configurable: true, value: hitTest });
  const append = vi.fn(async () => undefined);
  const focus = vi.fn(async () => undefined);
  const move = vi.fn();
  const editorDropIntent = ref(null);
  const composerDropTarget = ref(null);
  const deps = {
    WORKBENCH_WINDOW_ID: "main", WORKBENCH_EDITOR_TAB_INTERNAL_DRAG_TYPE,
    KNOWLEDGE_INTERNAL_DRAG_TYPE: "knowledge", WORKBENCH_REFERENCE_INTERNAL_DRAG_TYPE: "reference",
    WORKSPACE_LAYOUT_INTERNAL_DRAG_TYPE: "layout", VIEW_TREE_INTERNAL_DRAG_TYPE: "view",
    workbenchWindow, workbenchRootRef: ref(host), explorerRootRef: ref(null),
    editorDropIntent, composerDropTarget, layoutDropIntent: ref(null), dropTargetKey: ref(null),
    pinDropProjectId: ref(null), pinDropInsertion: ref(null),
    workbenchGroup: (paneId: string) => workbenchWindow.value.groups[paneId],
    editorForPane: (paneId: string) => {
      const group = workbenchWindow.value.groups[paneId];
      return group?.tabs.find((editor) => editor.editorId === group.activeEditorId) ?? null;
    },
    workspaceContextStore: { checkoutsById: { checkout: { root: "F:/Game" } } },
    explorerStore: { resources: { p: { knowledge: [{ id: "doc", type: "design", path: "combat.md", sourceRoot: "F:/Game" }] } } },
    secondaryDocuments: ref({}), workbenchComposerEditorAttachment, emptyComposerIntent,
    workbenchSplitDirectionAtPoint, workbenchTabInsertionIndexAtPoint,
    focusWorkbenchEditor: focus, nextTick,
    sessionEditorRefs: new Map([[composerEditor.editorId, { appendComposerDraft: append }]]),
    workbenchStore: { moveEditor: move },
  };
  const runtime = new Function(...Object.keys(deps), `${code}\nreturn { resolveWorkbenchInternalDrop, handleWorkbenchInternalTargetChange, commitWorkbenchInternalDrop };`)(...Object.values(deps));
  controller.registerTarget({
    id: "workbench", root: () => host, accepts: () => true,
    resolve: runtime.resolveWorkbenchInternalDrop,
    onTargetChange: runtime.handleWorkbenchInternalTargetChange,
    drop: ({ source, decision }) => runtime.commitWorkbenchInternalDrop(source.payload.type, source.payload.data, decision.intent),
  });
  return { host, sourceTab, composer, body, hitTest, controller, append, move, focus, activate, externalize, editorDropIntent, composerDropTarget, before, workbenchWindow };
}

describe("tab group drops into the composer", () => {
  it.each([false, true])("copies a document attachment and preserves tabs and layout (split=%s)", async (split) => {
    const f = await fixture({ kind: "knowledge", projectId: "p", documentId: "doc" }, split);
    f.sourceTab.dispatchEvent(pointer("pointerdown", 30, 16));
    window.dispatchEvent(pointer("pointermove", 90, 90));
    expect(f.controller.activeTarget.value?.decision.operation).toBe("move");
    expect(f.editorDropIntent.value).not.toBeNull();
    f.hitTest.mockReturnValue(f.composer);
    window.dispatchEvent(pointer("pointermove", 140, 220));
    expect(f.controller.activeTarget.value?.decision.operation).toBe("copy");
    expect(f.controller.previewMode.value).toBe("inline");
    expect(f.editorDropIntent.value).toBeNull();
    expect(f.composerDropTarget.value).not.toBeNull();
    window.dispatchEvent(pointer("pointerup", 140, 220));
    for (let i = 0; i < 4; i++) await nextTick();
    f.sourceTab.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(f.append).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      text: "", assetRefs: [{ kind: "knowledge", path: "design/combat.md", name: "Source", source: "manual" }],
    }));
    expect(JSON.stringify(f.workbenchWindow.value)).toBe(f.before);
    expect(f.move).not.toHaveBeenCalled();
    expect(f.externalize).not.toHaveBeenCalled();
    expect(f.activate).not.toHaveBeenCalled();
  });

  it("rejects session tabs over the composer without falling back to a layout change", async () => {
    const f = await fixture({ kind: "session", projectId: "p", sessionId: "session" }, false);
    f.sourceTab.dispatchEvent(pointer("pointerdown", 30, 16));
    window.dispatchEvent(pointer("pointermove", 90, 90));
    expect(f.controller.activeTarget.value?.decision.operation).toBe("move");
    f.hitTest.mockReturnValue(f.composer);
    window.dispatchEvent(pointer("pointerup", 140, 220));
    await nextTick();
    expect(f.append).not.toHaveBeenCalled();
    expect(f.move).not.toHaveBeenCalled();
    expect(f.controller.activeTarget.value).toBeNull();
    expect(JSON.stringify(f.workbenchWindow.value)).toBe(f.before);
  });

  it("attaches a file at the release point even when the last move hovered a split target", async () => {
    const f = await fixture({ kind: "workspaceFile", projectId: "p", path: "docs/notes.md" }, true);
    f.sourceTab.dispatchEvent(pointer("pointerdown", 30, 16));
    window.dispatchEvent(pointer("pointermove", 90, 90));
    expect(f.controller.activeTarget.value?.decision.operation).toBe("move");
    f.hitTest.mockReturnValue(f.composer);
    window.dispatchEvent(pointer("pointerup", 140, 220));
    for (let i = 0; i < 4; i++) await nextTick();
    expect(f.append).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({
      localFiles: [expect.objectContaining({ path: "F:/Game/docs/notes.md", isDir: false })],
    }));
    expect(f.move).not.toHaveBeenCalled();
    expect(JSON.stringify(f.workbenchWindow.value)).toBe(f.before);
  });

  it("continues moving tabs when released on an editor layout target", async () => {
    const f = await fixture({ kind: "knowledge", projectId: "p", documentId: "doc" }, true);
    f.sourceTab.dispatchEvent(pointer("pointerdown", 30, 16));
    window.dispatchEvent(pointer("pointermove", 90, 90));
    const intent = f.controller.activeTarget.value?.decision.intent as { direction: string };
    window.dispatchEvent(pointer("pointerup", 90, 90));
    await nextTick();
    expect(f.move).toHaveBeenCalledExactlyOnceWith(
      "main", "source", f.sourceTab.dataset.workbenchTabId, "target", { direction: intent.direction, index: undefined },
    );
    expect(f.append).not.toHaveBeenCalled();
  });
});
