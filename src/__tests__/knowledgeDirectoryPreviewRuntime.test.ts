// @vitest-environment jsdom
import { createPinia } from "pinia";
import { createApp, defineComponent, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import KnowledgeDirectoryPreview from "../components/knowledge/KnowledgeDirectoryPreview.vue";
import { KnowledgeEditorWorkspaceSessionStore } from "../components/knowledge/knowledgeEditorWorkspaceSession";
import { useMarkdownEditorViewMode } from "../components/ui/markdownEditorViewMode";
import type { KnowledgeDirectoryConfig, KnowledgeDirectoryConfigRecord } from "../types";

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

function makeDirectory(
  summary: string,
  updatedAt = 1,
  path = "combat",
): KnowledgeDirectoryConfigRecord {
  return {
    type: "design",
    path,
    configPath: `${path}/.locus.json`,
    exists: true,
    updatedAt,
    version: 4,
    summary,
    injectMode: "inherit",
    effectiveInjectMode: "excerpt",
    aiMaintained: "inherit",
    effectiveAiMaintained: false,
    lexicalSearch: "inherit",
    vectorSearch: "inherit",
    inheritToChildren: true,
    allowCreateDocuments: true,
    allowCreateDirectories: true,
    allowMoveDocuments: true,
    allowMoveDirectories: true,
    maintenanceRules: null,
    effectiveMaintenanceRules: null,
    effectiveLexicalSearch: { enabled: true, source: "default" },
    effectiveVectorSearch: { enabled: true, source: "default" },
  };
}

afterEach(() => {
  vi.useRealTimers();
  useMarkdownEditorViewMode().setMarkdownEditorViewMode("rendered");
  document.body.innerHTML = "";
});

describe("KnowledgeDirectoryPreview runtime", () => {
  it("restores a dirty Text buffer when navigating A to B to A", async () => {
    useMarkdownEditorViewMode().setMarkdownEditorViewMode("native");
    const directory = ref(makeDirectory("alpha", 1, "combat"));
    const sessionStore = new KnowledgeEditorWorkspaceSessionStore();
    const Root = defineComponent(() => () => h(KnowledgeDirectoryPreview, {
      workspaceRef: { checkoutId: "checkout", expectedGeneration: 1 },
      directory: directory.value,
      loading: false,
      saveLoading: false,
      sessionStore,
    }));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp(Root);
    app.use(createPinia());
    app.mount(host);
    await nextTick();

    const summaryEditor = () => host.querySelector<HTMLTextAreaElement>(
      ".directory-inline-summary .base-markdown-editor-textarea",
    );
    const editor = summaryEditor();
    if (!editor) throw new Error("directory summary editor missing");
    editor.value = "alpha local draft";
    editor.dispatchEvent(new Event("input", { bubbles: true }));

    directory.value = makeDirectory("bravo", 1, "enemies");
    await nextTick();
    expect(summaryEditor()?.value).toBe("bravo");

    directory.value = makeDirectory("alpha", 1, "combat");
    await nextTick();
    expect(summaryEditor()?.value).toBe("alpha local draft");
    expect(host.querySelector(".directory-footnote")?.classList.contains("is-warning")).toBe(true);
    app.unmount();
  });

  it("keeps a dirty Text buffer when the same directory receives an external update", async () => {
    useMarkdownEditorViewMode().setMarkdownEditorViewMode("native");
    const directory = ref(makeDirectory("alpha", 1, "combat"));
    const sessionStore = new KnowledgeEditorWorkspaceSessionStore();
    const Root = defineComponent(() => () => h(KnowledgeDirectoryPreview, {
      workspaceRef: { checkoutId: "checkout", expectedGeneration: 1 },
      directory: directory.value,
      loading: false,
      saveLoading: false,
      sessionStore,
    }));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp(Root);
    app.use(createPinia());
    app.mount(host);
    await nextTick();

    const editor = host.querySelector<HTMLTextAreaElement>(
      ".directory-inline-summary .base-markdown-editor-textarea",
    );
    if (!editor) throw new Error("directory summary editor missing");
    editor.value = "alpha local draft";
    editor.dispatchEvent(new Event("input", { bubbles: true }));

    directory.value = makeDirectory("alpha agent update", 2, "combat");
    await nextTick();
    expect(host.querySelector<HTMLTextAreaElement>(
      ".directory-inline-summary .base-markdown-editor-textarea",
    )?.value).toBe("alpha local draft");
    expect(host.querySelector(".directory-footnote")?.classList.contains("is-warning")).toBe(true);
    app.unmount();
  });

  it("keeps accepting body input while an autosave is in flight", async () => {
    vi.useFakeTimers();
    useMarkdownEditorViewMode().setMarkdownEditorViewMode("native");
    const directory = ref(makeDirectory("initial"));
    const saveLoading = ref(false);
    let submitted: KnowledgeDirectoryConfig = makeDirectory("initial");
    const onSave = vi.fn((_path: string, config: KnowledgeDirectoryConfig) => {
      submitted = config;
      saveLoading.value = true;
    });
    const Root = defineComponent(() => () => h(KnowledgeDirectoryPreview, {
      workspaceRef: { checkoutId: "checkout" },
      directory: directory.value,
      loading: false,
      saveLoading: saveLoading.value,
      onSave,
    }));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp(Root);
    app.use(createPinia());
    app.mount(host);
    await nextTick();

    const editor = host.querySelector<HTMLTextAreaElement>(
      ".directory-inline-summary .base-markdown-editor-textarea",
    );
    if (!editor) throw new Error("directory summary editor missing");
    editor.value = "submitted";
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    vi.advanceTimersByTime(1000);
    await nextTick();
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(editor.disabled).toBe(false);

    editor.value = "typed during save";
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    directory.value = {
      ...directory.value,
      ...submitted,
      updatedAt: 2,
    };
    saveLoading.value = false;
    await nextTick();

    const currentEditor = host.querySelector<HTMLTextAreaElement>(
      ".directory-inline-summary .base-markdown-editor-textarea",
    );
    expect(currentEditor?.value).toBe("typed during save");
    app.unmount();
  });
});
