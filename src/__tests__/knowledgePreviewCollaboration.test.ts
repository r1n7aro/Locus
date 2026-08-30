// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import KnowledgePreview from "../components/knowledge/KnowledgePreview.vue";
import { KnowledgeEditorWorkspaceSessionStore } from "../components/knowledge/knowledgeEditorWorkspaceSession";
import { useMarkdownEditorViewMode } from "../components/ui/markdownEditorViewMode";
import type { KnowledgeDocument, KnowledgeDocumentEditOperation } from "../types";

vi.mock("../components/ui/BaseMarkdownEditor.vue", async () => {
  const { defineComponent, h } = await import("vue");
  const { Text } = await import("@codemirror/state");
  return {
    default: defineComponent({
      props: {
        modelValue: { type: String, default: "" },
        disabled: Boolean,
        transactionModel: Boolean,
      },
      emits: ["update:modelValue", "documentChange", "shortcutSave"],
      setup(props, { emit }) {
        return () => h("div", { class: "base-markdown-editor" }, [
          h("textarea", {
            class: "base-markdown-editor-textarea",
            value: props.modelValue,
            disabled: props.disabled,
            onInput: (event: Event) => {
              const value = (event.target as HTMLTextAreaElement).value;
              if (props.transactionModel) {
                emit("documentChange", {
                  doc: Text.of(value.replace(/\r\n/g, "\n").split("\n")),
                  changes: null,
                });
                return;
              }
              emit("update:modelValue", value);
            },
          }),
        ]);
      },
    }),
  };
});

function makeDocument(
  body: string,
  modifiedAt = 1,
  id = "design-1",
  path = "combat/core-loop.md",
): KnowledgeDocument {
  return {
    id,
    type: "design",
    path,
    title: "核心循环",
    injectMode: "excerpt",
    effectiveInjectMode: "excerpt",
    readOnly: false,
    aiMaintained: false,
    effectiveAiMaintained: false,
    summary: "summary",
    body,
    maintenanceRules: null,
    effectiveMaintenanceRules: null,
    modifiedAt,
  };
}

async function mountPreview(saveEdits = vi.fn()) {
  useMarkdownEditorViewMode().setMarkdownEditorViewMode("native");
  const document = ref(makeDocument("alpha\nbeta\ngamma"));
  const workspaceRef = ref({ checkoutId: "checkout-a", expectedGeneration: 1 });
  const visible = ref(true);
  const sessionStore = new KnowledgeEditorWorkspaceSessionStore();
  const host = documentOwner().createElement("div");
  documentOwner().body.appendChild(host);
  const Root = defineComponent(() => () => visible.value
    ? h(KnowledgePreview, {
        document: document.value,
        loading: false,
        saveLoading: false,
        embedded: true,
        workspaceRef: workspaceRef.value,
        sessionStore,
        saveEdits,
      })
    : h("div", { class: "alternate-preview" }));
  const app = createApp(Root);
  app.use(createPinia());
  app.mount(host);
  await nextTick();
  return { app, document, host, saveEdits, sessionStore, visible, workspaceRef };
}

function documentOwner(): Document {
  return window.document;
}

function bodyEditor(host: HTMLElement): HTMLTextAreaElement {
  const editor = host.querySelector<HTMLTextAreaElement>(
    ".document-body .base-markdown-editor-textarea",
  );
  if (!editor) throw new Error("body editor missing");
  return editor;
}

function input(editor: HTMLTextAreaElement, value: string) {
  editor.value = value;
  editor.dispatchEvent(new Event("input", { bubbles: true }));
}

afterEach(() => {
  vi.useRealTimers();
  useMarkdownEditorViewMode().setMarkdownEditorViewMode("rendered");
  documentOwner().body.innerHTML = "";
});

describe("KnowledgePreview collaborative editing", () => {
  it("restores a dirty document session after the preview is unmounted", async () => {
    const mounted = await mountPreview();
    input(bodyEditor(mounted.host), "alpha\nbeta-local\ngamma");

    mounted.visible.value = false;
    await nextTick();
    expect(mounted.host.querySelector(".alternate-preview")).not.toBeNull();
    mounted.visible.value = true;
    await nextTick();

    expect(bodyEditor(mounted.host).value).toBe("alpha\nbeta-local\ngamma");
    mounted.app.unmount();
  });

  it("isolates the same document id across checkout generations", async () => {
    const mounted = await mountPreview();
    input(bodyEditor(mounted.host), "generation one local draft");

    mounted.workspaceRef.value = { checkoutId: "checkout-a", expectedGeneration: 2 };
    mounted.document.value = makeDocument("generation two remote");
    await nextTick();
    expect(bodyEditor(mounted.host).value).toBe("generation two remote");

    mounted.workspaceRef.value = { checkoutId: "checkout-a", expectedGeneration: 1 };
    mounted.document.value = makeDocument("alpha\nbeta\ngamma");
    await nextTick();
    expect(bodyEditor(mounted.host).value).toBe("generation one local draft");
    mounted.app.unmount();
  });

  it("restores a bounded document session when navigating A to B to A", async () => {
    const mounted = await mountPreview();
    input(bodyEditor(mounted.host), "alpha\nbeta-local\ngamma");
    mounted.document.value = makeDocument(
      "bravo",
      1,
      "design-2",
      "combat/enemies.md",
    );
    await nextTick();
    expect(bodyEditor(mounted.host).value).toBe("bravo");

    mounted.document.value = makeDocument("alpha\nbeta\ngamma", 1);
    await nextTick();
    expect(bodyEditor(mounted.host).value).toBe("alpha\nbeta-local\ngamma");
    mounted.app.unmount();
  });

  it("routes a delayed save response back to its initiating document", async () => {
    vi.useFakeTimers();
    let resolveSave!: (document: KnowledgeDocument) => void;
    const saveEdits = vi.fn(() => new Promise<KnowledgeDocument>((resolve) => {
      resolveSave = resolve;
    }));
    const mounted = await mountPreview(saveEdits);
    input(bodyEditor(mounted.host), "alpha\nbeta-saved\ngamma");
    vi.advanceTimersByTime(800);
    await nextTick();
    expect(saveEdits).toHaveBeenCalledTimes(1);

    mounted.document.value = makeDocument(
      "bravo-current",
      1,
      "design-2",
      "combat/enemies.md",
    );
    await nextTick();
    resolveSave(makeDocument("alpha\nbeta-saved\ngamma", 2));
    await Promise.resolve();
    await nextTick();
    expect(bodyEditor(mounted.host).value).toBe("bravo-current");

    mounted.document.value = makeDocument("alpha\nbeta-saved\ngamma", 2);
    await nextTick();
    expect(bodyEditor(mounted.host).value).toBe("alpha\nbeta-saved\ngamma");
    mounted.app.unmount();
  });

  it("retains an unresolved conflict when navigating A to B to A", async () => {
    const mounted = await mountPreview();
    input(bodyEditor(mounted.host), "alpha\nbeta-local\ngamma");
    mounted.document.value = makeDocument("alpha\nbeta-agent\ngamma", 2);
    await nextTick();
    expect(mounted.host.querySelector(".document-conflict")).not.toBeNull();

    mounted.document.value = makeDocument(
      "bravo",
      1,
      "design-2",
      "combat/enemies.md",
    );
    await nextTick();
    mounted.document.value = makeDocument("alpha\nbeta-agent\ngamma", 2);
    await nextTick();

    expect(bodyEditor(mounted.host).value).toBe("alpha\nbeta-local\ngamma");
    expect(mounted.host.querySelector(".document-conflict")).not.toBeNull();
    mounted.app.unmount();
  });

  it("applies a non-overlapping Agent edit into the active local draft", async () => {
    const mounted = await mountPreview();
    const editor = bodyEditor(mounted.host);
    input(editor, "alpha\nbeta-local\ngamma");
    mounted.document.value = makeDocument("alpha\nbeta\ngamma-agent", 2);
    await nextTick();
    await nextTick();

    expect(bodyEditor(mounted.host).value).toBe("alpha\nbeta-local\ngamma-agent");
    expect(mounted.host.querySelector(".document-conflict")).toBeNull();
    mounted.app.unmount();
  });

  it("pauses autosave for an overlapping edit and resumes after keeping local", async () => {
    vi.useFakeTimers();
    const saveEdits = vi.fn(async (edits: KnowledgeDocumentEditOperation[]) => {
      expect(edits.length).toBeGreaterThan(0);
      return makeDocument("alpha\nbeta-local\ngamma", 3);
    });
    const mounted = await mountPreview(saveEdits);
    input(bodyEditor(mounted.host), "alpha\nbeta-local\ngamma");
    mounted.document.value = makeDocument("alpha\nbeta-agent\ngamma", 2);
    await nextTick();
    await nextTick();

    vi.advanceTimersByTime(800);
    await nextTick();
    expect(saveEdits).not.toHaveBeenCalled();
    expect(mounted.host.querySelector(".document-conflict")).not.toBeNull();

    const buttons = mounted.host.querySelectorAll<HTMLButtonElement>(
      ".document-conflict-actions button",
    );
    buttons[1]!.click();
    await nextTick();
    vi.advanceTimersByTime(800);
    await Promise.resolve();
    await nextTick();
    expect(saveEdits).toHaveBeenCalledTimes(1);
    mounted.app.unmount();
  });
});
