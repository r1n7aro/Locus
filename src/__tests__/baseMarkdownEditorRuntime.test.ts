// @vitest-environment jsdom
import { undo, undoDepth } from "@codemirror/commands";
import type { EditorView } from "@codemirror/view";
import { createApp, h, nextTick, ref, type App } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import BaseMarkdownEditor from "../components/ui/BaseMarkdownEditor.vue";
import type { MarkdownEditorDocumentChange } from "../components/ui/markdown-editor/markdownEditorDocumentChange";
import type { MarkdownReferenceToken } from "../components/ui/markdown-editor/markdownComplexTokens";
import { MarkdownEditorSessionCache } from "../components/ui/markdown-editor/markdownEditorSessionCache";

type EditorExpose = {
  getEditorView(): EditorView | null;
};

let app: App<Element> | null = null;

beforeEach(() => {
  if (!(Range.prototype as Range & { getClientRects?: unknown }).getClientRects) {
    Object.defineProperty(Range.prototype, "getClientRects", {
      configurable: true,
      value: () => [],
    });
  }
  if (!(Range.prototype as Range & { getBoundingClientRect?: unknown }).getBoundingClientRect) {
    Object.defineProperty(Range.prototype, "getBoundingClientRect", {
      configurable: true,
      value: () => ({
        left: 0,
        right: 0,
        top: 0,
        bottom: 0,
        width: 0,
        height: 0,
      }),
    });
  }
});

afterEach(() => {
  app?.unmount();
  app = null;
  document.body.replaceChildren();
});

describe("BaseMarkdownEditor runtime", () => {
  it("emits an immutable document transaction without materializing v-model per keystroke", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const model = ref("alpha");
    const modelUpdates: string[] = [];
    const documentChanges: MarkdownEditorDocumentChange[] = [];

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: model.value,
          viewMode: "native",
          contentKey: "transaction-doc:body",
          transactionModel: true,
          "onUpdate:modelValue": (value: string) => {
            modelUpdates.push(value);
            model.value = value;
          },
          onDocumentChange: (change: MarkdownEditorDocumentChange) => {
            documentChanges.push(change);
          },
        });
      },
    });
    app.mount(root);
    await nextTick();

    const view = editorRef.value!.getEditorView()!;
    view.dispatch({ changes: { from: view.state.doc.length, insert: "!" } });
    await nextTick();

    expect(model.value).toBe("alpha");
    expect(modelUpdates).toEqual([]);
    expect(documentChanges).toHaveLength(1);
    expect(documentChanges[0]?.doc).toBe(view.state.doc);
    expect(documentChanges[0]?.changes.empty).toBe(false);
  });

  it("keeps a transaction-model draft when the editor is suspended and resumed", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const active = ref(true);

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          active: active.value,
          modelValue: "draft",
          viewMode: "native",
          contentKey: "transaction-doc:body",
          transactionModel: true,
        });
      },
    });
    app.mount(root);
    await nextTick();
    const initialView = editorRef.value!.getEditorView()!;
    initialView.dispatch({ changes: { from: initialView.state.doc.length, insert: "!" } });

    active.value = false;
    await nextTick();
    active.value = true;
    await nextTick();

    expect(editorRef.value!.getEditorView()!.state.doc.toString()).toBe("draft!");
  });

  it("keeps transaction-model drafts in their content-key sessions", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const contentKey = ref("doc-a:body");
    const model = ref("alpha");

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: model.value,
          viewMode: "native",
          contentKey: contentKey.value,
          transactionModel: true,
        });
      },
    });
    app.mount(root);
    await nextTick();
    const view = editorRef.value!.getEditorView()!;
    view.dispatch({ changes: { from: view.state.doc.length, insert: "!" } });

    contentKey.value = "doc-b:body";
    model.value = "bravo";
    await nextTick();
    expect(editorRef.value!.getEditorView()!.state.doc.toString()).toBe("bravo");

    contentKey.value = "doc-a:body";
    model.value = "alpha";
    await nextTick();
    expect(editorRef.value!.getEditorView()!.state.doc.toString()).toBe("alpha!");
  });

  it("queues external model updates until IME composition ends", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const model = ref("本地输入");

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: model.value,
          viewMode: "native",
          contentKey: "ime-doc:body",
          transactionModel: true,
        });
      },
    });
    app.mount(root);
    await nextTick();
    const view = editorRef.value!.getEditorView()!;

    view.contentDOM.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true }));
    model.value = "Agent 返回内容";
    await nextTick();
    expect(view.state.doc.toString()).toBe("本地输入");

    view.contentDOM.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true }));
    await Promise.resolve();
    await nextTick();
    expect(view.state.doc.toString()).toBe("Agent 返回内容");
  });

  it("materializes transaction-model text before emitting shortcutSave", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const model = ref("draft");
    const events: string[] = [];

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: model.value,
          viewMode: "native",
          contentKey: "save-doc:body",
          transactionModel: true,
          "onUpdate:modelValue": (value: string) => {
            model.value = value;
            events.push(`model:${value}`);
          },
          onShortcutSave: () => events.push(`save:${model.value}`),
        });
      },
    });
    app.mount(root);
    await nextTick();
    const view = editorRef.value!.getEditorView()!;
    view.dispatch({ changes: { from: view.state.doc.length, insert: "!" } });
    view.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "s",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    }));
    await nextTick();

    expect(events).toEqual(["model:draft!", "save:draft!"]);
  });

  it("keeps one editor DOM through edits, external updates, and mode changes", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const model = ref("# Title\n\nBody");
    const mode = ref<"rendered" | "native">("rendered");

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: model.value,
          viewMode: mode.value,
          contentKey: "doc-a:body",
          "onUpdate:modelValue": (value: string) => { model.value = value; },
        });
      },
    });
    app.mount(root);
    await nextTick();

    const view = editorRef.value?.getEditorView();
    expect(view).not.toBeNull();
    const editorDom = view!.dom;
    view!.dispatch({ changes: { from: view!.state.doc.length, insert: "!" } });
    await nextTick();
    expect(model.value).toBe("# Title\n\nBody!");
    expect(editorRef.value?.getEditorView()?.dom).toBe(editorDom);

    model.value = "# Title\n\nExternal Body!";
    await nextTick();
    expect(editorRef.value?.getEditorView()?.state.doc.toString()).toBe(model.value);
    expect(editorRef.value?.getEditorView()?.dom).toBe(editorDom);
    expect(undo(editorRef.value!.getEditorView()!)).toBe(true);
    expect(editorRef.value?.getEditorView()?.state.doc.toString()).toBe("# Title\n\nExternal Body");
    await nextTick();

    mode.value = "native";
    await nextTick();
    expect(editorRef.value?.getEditorView()?.dom).toBe(editorDom);
    expect(editorDom.classList.contains("cm-source-mode")).toBe(true);
    expect(root.querySelectorAll(".cm-editor")).toHaveLength(1);
  });

  it("forwards Live Preview reference actions and canonical drag semantics", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const opened: MarkdownReferenceToken[] = [];
    const pointerPayloads: Array<{
      reference: MarkdownReferenceToken;
      event: PointerEvent;
      element: HTMLElement;
    }> = [];

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          modelValue: [
            "`design/editor.md`",
            "",
            "@src/main.ts",
            "",
            "Assets/Prefabs/Hero.prefab",
          ].join("\n"),
          viewMode: "rendered",
          contentKey: "reference-doc:body",
          onReferenceOpen: (reference: MarkdownReferenceToken) => opened.push(reference),
          onReferencePointerDown: (payload: typeof pointerPayloads[number]) => {
            pointerPayloads.push(payload);
          },
        });
      },
    });
    app.mount(root);
    await nextTick();

    const knowledge = root.querySelector<HTMLElement>("[data-reference-kind='knowledge']")!;
    const workspace = root.querySelector<HTMLElement>("[data-reference-kind='workspace']")!;
    const unityAsset = root.querySelector<HTMLElement>("[data-reference-kind='unity-asset']")!;
    expect(knowledge.draggable).toBe(true);
    expect(knowledge.classList.contains("md-knowledge-ref")).toBe(true);
    expect(knowledge.dataset.knowledgeType).toBe("design");
    expect(knowledge.dataset.knowledgePath).toBe("design/editor.md");
    expect(workspace.dataset.workspacePath).toBe("src/main.ts");
    expect(unityAsset.dataset.assetPath).toBe("Assets/Prefabs/Hero.prefab");

    knowledge.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      pointerId: 7,
      isPrimary: true,
    }));
    expect(pointerPayloads).toHaveLength(1);
    expect(pointerPayloads[0]?.reference.kind).toBe("knowledge");
    expect(pointerPayloads[0]?.element).toBe(knowledge);

    knowledge.dispatchEvent(new MouseEvent("click", {
      bubbles: true,
      cancelable: true,
      ctrlKey: true,
    }));
    expect(opened).toEqual([expect.objectContaining({
      kind: "knowledge",
      path: "design/editor.md",
    })]);
    expect(root.querySelector("[data-reference-kind='knowledge']")).not.toBeNull();

    knowledge.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(root.querySelector("[data-reference-kind='knowledge']")).toBeNull();
    expect(root.textContent).toContain("design/editor.md");
  });

  it("restores document-local selection and undo history across content keys", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const contentKey = ref("doc-a:body");
    const model = ref("alpha");

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          modelValue: model.value,
          viewMode: "native",
          contentKey: contentKey.value,
          "onUpdate:modelValue": (value: string) => { model.value = value; },
        });
      },
    });
    app.mount(root);
    await nextTick();

    const originalDom = editorRef.value!.getEditorView()!.dom;
    editorRef.value!.getEditorView()!.dispatch({
      changes: { from: 5, insert: "!" },
      selection: { anchor: 6 },
    });
    editorRef.value!.getEditorView()!.scrollDOM.scrollTop = 37;
    editorRef.value!.getEditorView()!.scrollDOM.dispatchEvent(new Event("scroll"));
    await nextTick();
    expect(undoDepth(editorRef.value!.getEditorView()!.state)).toBe(1);

    contentKey.value = "doc-b:body";
    model.value = "bravo";
    await nextTick();
    expect(editorRef.value!.getEditorView()!.dom).toBe(originalDom);
    expect(editorRef.value!.getEditorView()!.state.doc.toString()).toBe("bravo");

    contentKey.value = "doc-a:body";
    model.value = "alpha!";
    await nextTick();
    const restored = editorRef.value!.getEditorView()!;
    expect(restored.dom).toBe(originalDom);
    expect(restored.state.selection.main.head).toBe(6);
    expect(undoDepth(restored.state)).toBe(1);
    await vi.waitFor(() => expect(restored.scrollDOM.scrollTop).toBe(37));
    expect(undo(restored)).toBe(true);
    expect(restored.state.doc.toString()).toBe("alpha");
  });

  it("releases an inactive view and restores its state when activated", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const active = ref(true);
    const model = ref("draft");

    app = createApp({
      setup() {
        return () => h(BaseMarkdownEditor, {
          ref: editorRef,
          active: active.value,
          modelValue: model.value,
          viewMode: "native",
          contentKey: "doc-a:body",
          "onUpdate:modelValue": (value: string) => { model.value = value; },
        });
      },
    });
    app.mount(root);
    await nextTick();
    editorRef.value!.getEditorView()!.dispatch({ selection: { anchor: 3 } });

    active.value = false;
    await nextTick();
    expect(editorRef.value?.getEditorView()).toBeNull();
    expect(root.querySelector(".cm-editor")).toBeNull();

    active.value = true;
    await nextTick();
    expect(editorRef.value?.getEditorView()?.state.doc.toString()).toBe("draft");
    expect(editorRef.value?.getEditorView()?.state.selection.main.head).toBe(3);
    expect(root.querySelectorAll(".cm-editor")).toHaveLength(1);
  });

  it("restores undo history after the editor component is unmounted and remounted", async () => {
    const root = document.createElement("div");
    root.style.overflowY = "auto";
    document.body.appendChild(root);
    const editorRef = ref<EditorExpose | null>(null);
    const mounted = ref(true);
    const generation = ref(1);
    const viewMode = ref<"rendered" | "native">("native");
    const disabled = ref(false);
    const sharedSessions = new MarkdownEditorSessionCache(96);
    const firstChanges: MarkdownEditorDocumentChange[] = [];
    const secondChanges: MarkdownEditorDocumentChange[] = [];
    const firstSaves: string[] = [];
    const secondSaves: string[] = [];

    app = createApp({
      setup() {
        return () => {
          const currentGeneration = generation.value;
          return mounted.value ? h(BaseMarkdownEditor, {
            ref: editorRef,
            modelValue: "draft",
            viewMode: viewMode.value,
            disabled: disabled.value,
            contentKey: "workspace:doc-a:body",
            transactionModel: true,
            autoGrow: true,
            sessionCache: sharedSessions,
            sessionPinned: true,
            onDocumentChange: (change: MarkdownEditorDocumentChange) => {
              (currentGeneration === 1 ? firstChanges : secondChanges).push(change);
            },
            onShortcutSave: () => {
              (currentGeneration === 1 ? firstSaves : secondSaves).push("save");
            },
          }) : h("div", { class: "alternate-knowledge-preview" });
        };
      },
    });
    app.mount(root);
    await nextTick();

    const initialView = editorRef.value!.getEditorView()!;
    initialView.dispatch({
      changes: { from: initialView.state.doc.length, insert: "!" },
      selection: { anchor: 6 },
    });
    root.scrollTop = 43;
    root.dispatchEvent(new Event("scroll"));
    expect(undoDepth(initialView.state)).toBe(1);
    expect(firstChanges).toHaveLength(1);

    mounted.value = false;
    await nextTick();
    expect(root.querySelector(".cm-editor")).toBeNull();
    expect(sharedSessions.get("workspace:doc-a:body")?.pinned).toBe(true);
    root.scrollTop = 0;

    generation.value = 2;
    viewMode.value = "rendered";
    mounted.value = true;
    await nextTick();
    const restoredView = editorRef.value!.getEditorView()!;
    expect(restoredView).not.toBe(initialView);
    expect(restoredView.state.doc.toString()).toBe("draft!");
    expect(restoredView.state.selection.main.head).toBe(6);
    expect(undoDepth(restoredView.state)).toBe(1);
    expect(restoredView.dom.classList.contains("cm-live-preview")).toBe(true);
    await vi.waitFor(() => expect(root.scrollTop).toBe(43));

    restoredView.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "z",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    }));
    expect(restoredView.state.doc.toString()).toBe("draft");
    expect(firstChanges).toHaveLength(1);
    expect(secondChanges).toHaveLength(1);

    restoredView.dispatch({ changes: { from: restoredView.state.doc.length, insert: "?" } });
    restoredView.contentDOM.dispatchEvent(new KeyboardEvent("keydown", {
      key: "s",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    }));
    expect(firstSaves).toEqual([]);
    expect(secondSaves).toEqual(["save"]);

    disabled.value = true;
    await nextTick();
    expect(editorRef.value!.getEditorView()!.state.readOnly).toBe(true);
    disabled.value = false;
    await nextTick();
    expect(editorRef.value!.getEditorView()!.state.readOnly).toBe(false);
  });
});
