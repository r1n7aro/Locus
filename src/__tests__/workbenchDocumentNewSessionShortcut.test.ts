// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { createPinia, setActivePinia } from "pinia";
import ts from "typescript";
import { ref } from "vue";
import { beforeEach, describe, expect, it } from "vitest";
import { matchesShortcut } from "../composables/useKeyboardShortcuts";
import {
  createWorkbenchEditorInput,
  isWorkbenchDocumentPath,
  useWorkbenchStore,
  workbenchPaneRightOf,
} from "../stores/workbench";
import type { WorkbenchEditorInput, WorkbenchResourceRef } from "../types/workbench";

// Run the production shortcut handlers against the real workbench tab store.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const names = [
  "handleDocumentNewSessionShortcut", "openNewSessionBesideDocumentEditor", "isTextDocumentEditor",
  "openWorkbenchResourceBeside", "groupHostsSessions",
];
const handlers = parsed.statements.filter((statement) => ts.isFunctionDeclaration(statement)
  && !!statement.name && names.includes(statement.name.text)).map((statement) => statement.getText(parsed)).join("\n");
const code = ts.transpileModule(handlers, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

beforeEach(() => {
  localStorage.clear();
  setActivePinia(createPinia());
});

const NEW_CHAT = { ctrl: true, meta: false, alt: false, shift: false, key: "n" };

function input(resource: WorkbenchResourceRef, title: string): WorkbenchEditorInput {
  return createWorkbenchEditorInput(resource, title, {
    checkoutBinding: { checkoutId: "checkout" },
    preview: false,
    pinned: true,
  });
}

const knowledge = () => input({ kind: "knowledge", projectId: "project", documentId: "doc" }, "doc.md");
const session = (sessionId: string) => input({ kind: "session", projectId: "project", sessionId }, sessionId);
const newSession = () => input({ kind: "newSession", projectId: "project" }, "chat.session.newSession");
const file = (path: string) => input({ kind: "workspaceFile", projectId: "project", path }, path);

function fixture() {
  const store = useWorkbenchStore();
  const state = store.ensureWindow("main");
  const workbenchWindow = ref(state);
  const focusedComposers: string[] = [];
  const visible = ref(true);
  // Mounted file previews by editor id, reporting whether their content is text.
  const workspaceFileEditorRefs = new Map<string, { isTextContent: () => boolean }>();
  const showFile = (path: string, text: boolean) => {
    const editor = store.openEditor("main", file(path));
    workspaceFileEditorRefs.set(editor.editorId, { isTextContent: () => text });
    return editor;
  };
  const focusWorkbenchEditor = async (paneId: string, editorId: string) => {
    store.activateEditor("main", paneId, editorId);
  };
  const dependencies = {
    WORKBENCH_WINDOW_ID: "main",
    workbenchStore: store,
    workbenchWindow,
    workbenchPaneRightOf,
    workbenchGroup: (paneId: string) => workbenchWindow.value.groups[paneId] ?? null,
    workspaceFileEditorRefs,
    isWorkbenchDocumentPath,
    editorForPane: (paneId: string) => store.activeEditor("main", paneId),
    usesCheckoutScopedWorkbench: () => false,
    activateCheckoutScopedWorkbench: async () => true,
    createEditorForResource: (resource: WorkbenchResourceRef, options: { title: string }) => (
      input(resource, options.title)
    ),
    openWorkbenchResource: async (
      descriptor: { resource: WorkbenchResourceRef; title: string },
      options: { paneId: string },
    ) => {
      const editor = store.openEditor("main", input(descriptor.resource, descriptor.title), {
        paneId: options.paneId,
        preview: false,
        pinned: true,
        allowDuplicate: true,
      });
      await focusWorkbenchEditor(options.paneId, editor.editorId);
      return editor;
    },
    focusWorkbenchEditor,
    mountedSessionEditor: async (editorId: string) => ({
      focusComposerInput: async () => { focusedComposers.push(editorId); },
    }),
    matchesShortcut,
    shortcutState: { newChat: NEW_CHAT },
    workbenchRootRef: ref({ getClientRects: () => (visible.value ? [{}] : []) }),
    workspaceOverlayShown: ref(false),
    t: (key: string) => key,
  };
  const handle = new Function(
    ...Object.keys(dependencies),
    `${code}\nreturn handleDocumentNewSessionShortcut;`,
  )(...Object.values(dependencies)) as (event: KeyboardEvent) => void;
  async function press(): Promise<KeyboardEvent> {
    const event = new KeyboardEvent("keydown", { key: "n", ctrlKey: true, cancelable: true });
    handle(event);
    for (let tick = 0; tick < 10; tick += 1) await Promise.resolve();
    return event;
  }
  return { store, state, focusedComposers, visible, press, showFile };
}

describe("new session shortcut on a text document", () => {
  it("splits a new session pane off to the right of the document", async () => {
    const view = fixture();
    const doc = view.store.openEditor("main", knowledge());

    const event = await view.press();

    expect(event.defaultPrevented).toBe(true);
    expect(view.state.layout.kind).toBe("split");
    const rightPaneId = workbenchPaneRightOf(view.state.layout, "main")!;
    const created = view.state.groups[rightPaneId]!.tabs;
    expect(created.map((editor) => editor.resource.kind)).toEqual(["newSession"]);
    expect(view.state.groups.main!.activeEditorId).toBe(doc.editorId);
    expect(view.state.focusedPaneId).toBe(rightPaneId);
    expect(view.focusedComposers).toEqual([created[0]!.editorId]);
  });

  it("creates the session as a new tab in an existing session pane on the right", async () => {
    const view = fixture();
    view.store.openEditor("main", knowledge());
    const rightPaneId = view.store.splitPane("main", "main", "right", session("existing"))!;
    view.store.focusPane("main", "main");

    await view.press();

    expect(Object.keys(view.state.groups)).toHaveLength(2);
    const tabs = view.state.groups[rightPaneId]!.tabs;
    expect(tabs.map((editor) => editor.resource.kind)).toEqual(["session", "newSession"]);
    expect(view.state.groups[rightPaneId]!.activeEditorId).toBe(tabs[1]!.editorId);
    expect(view.state.focusedPaneId).toBe(rightPaneId);
    expect(view.focusedComposers).toEqual([tabs[1]!.editorId]);
  });

  it("reuses an untouched new session already active on the right", async () => {
    const view = fixture();
    view.store.openEditor("main", knowledge());
    const blank = newSession();
    const rightPaneId = view.store.splitPane("main", "main", "right", blank)!;
    view.store.focusPane("main", "main");

    await view.press();

    expect(view.state.groups[rightPaneId]!.tabs.map((editor) => editor.editorId)).toEqual([blank.editorId]);
    expect(view.state.focusedPaneId).toBe(rightPaneId);
    expect(view.focusedComposers).toEqual([blank.editorId]);
  });

  it("splits a session pane between the document and a right pane without sessions", async () => {
    const view = fixture();
    view.store.openEditor("main", knowledge());
    const documentsPaneId = view.store.splitPane("main", "main", "right", knowledge())!;
    view.store.focusPane("main", "main");

    await view.press();

    expect(Object.keys(view.state.groups)).toHaveLength(3);
    expect(view.state.groups[documentsPaneId]!.tabs.map((editor) => editor.resource.kind)).toEqual(["knowledge"]);
    const sessionPaneId = workbenchPaneRightOf(view.state.layout, "main")!;
    expect(sessionPaneId).not.toBe(documentsPaneId);
    expect(view.state.groups[sessionPaneId]!.tabs.map((editor) => editor.resource.kind)).toEqual(["newSession"]);
    expect(workbenchPaneRightOf(view.state.layout, sessionPaneId)).toBe(documentsPaneId);
  });

  it("splits a session off code, Markdown and other text files", async () => {
    for (const path of ["src/main.ts", "README.md", "data/table.csv"]) {
      localStorage.clear();
      setActivePinia(createPinia());
      const view = fixture();
      const opened = view.showFile(path, true);

      expect((await view.press()).defaultPrevented).toBe(true);

      const rightPaneId = workbenchPaneRightOf(view.state.layout, "main")!;
      const created = view.state.groups[rightPaneId]!.tabs;
      expect(created.map((editor) => editor.resource.kind)).toEqual(["newSession"]);
      expect(view.state.groups.main!.activeEditorId).toBe(opened.editorId);
      expect(view.focusedComposers).toEqual([created[0]!.editorId]);
    }
  });

  it("leaves images, media and still-loading file previews alone", async () => {
    const view = fixture();
    view.showFile("art/cover.png", false);
    let before = JSON.stringify(view.state);
    expect((await view.press()).defaultPrevented).toBe(false);
    expect(JSON.stringify(view.state)).toBe(before);

    // A preview that has not mounted yet reports nothing.
    view.store.openEditor("main", file("src/pending.ts"));
    before = JSON.stringify(view.state);
    expect((await view.press()).defaultPrevented).toBe(false);
    expect(JSON.stringify(view.state)).toBe(before);
    expect(view.focusedComposers).toEqual([]);
  });

  it("leaves the shortcut to session editors and ignores a hidden workbench", async () => {
    const view = fixture();
    view.store.openEditor("main", session("current"));
    let before = JSON.stringify(view.state);
    expect((await view.press()).defaultPrevented).toBe(false);
    expect(JSON.stringify(view.state)).toBe(before);

    view.store.openEditor("main", knowledge());
    view.visible.value = false;
    before = JSON.stringify(view.state);
    expect((await view.press()).defaultPrevented).toBe(false);
    expect(JSON.stringify(view.state)).toBe(before);
    expect(view.focusedComposers).toEqual([]);
  });
});
