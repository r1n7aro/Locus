// @vitest-environment jsdom

import { createApp, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectExplorerFilePreview } from "../types/workbench";
import WorkspaceFilePreview from "../components/workbench/WorkspaceFilePreview.vue";

const workspaceExplorerMocks = vi.hoisted(() => ({
  preview: vi.fn(),
  write: vi.fn(),
  workspacePreview: vi.fn(),
  revision: vi.fn(),
  workspaceRevision: vi.fn(),
  workspaceChangeHandlers: [] as Array<(event: any) => void>,
  workspaceWrite: vi.fn(),
  editorDispatch: vi.fn(),
  editorFocus: vi.fn(),
}));

vi.mock("../services/workspaceExplorer", () => ({
  projectExplorerPreviewFile: workspaceExplorerMocks.preview,
  projectExplorerFileRevision: workspaceExplorerMocks.revision,
  projectExplorerWriteFile: workspaceExplorerMocks.write,
  workspaceFilePreview: workspaceExplorerMocks.workspacePreview,
  workspaceFileRevision: workspaceExplorerMocks.workspaceRevision,
  workspaceFileWrite: workspaceExplorerMocks.workspaceWrite,
  subscribeWorkspaceFileChanges: vi.fn(async (handler: (event: any) => void) => {
    workspaceExplorerMocks.workspaceChangeHandlers.push(handler);
    return () => {
      const index = workspaceExplorerMocks.workspaceChangeHandlers.indexOf(handler);
      if (index >= 0) workspaceExplorerMocks.workspaceChangeHandlers.splice(index, 1);
    };
  }),
}));

vi.mock("../components/ui/BaseMarkdownEditor.vue", async () => {
  const { Text } = await import("@codemirror/state");
  const { defineComponent, h: render } = await import("vue");
  return {
    default: defineComponent({
      props: {
        modelValue: { type: String, default: "" },
        contentPath: { type: String, default: "" },
        viewMode: { type: String, default: "rendered" },
      },
      emits: ["documentChange", "shortcutSave"],
      setup(props, { emit, expose }) {
        const editorView = {
          state: { doc: Text.of(["line one", "line two"]) },
          dispatch: workspaceExplorerMocks.editorDispatch,
          focus: workspaceExplorerMocks.editorFocus,
        };
        expose({ getEditorView: () => editorView });
        return () => render("button", {
          class: "workspace-file-editor-test-change",
          "data-content-path": props.contentPath,
          "data-view-mode": props.viewMode,
          "data-model-value": props.modelValue,
          onClick: () => emit("documentChange", {
            doc: Text.of(props.contentPath.toLowerCase().endsWith(".md")
              ? ["# Edited", ""]
              : ["export const value = 2;", ""]),
          }),
        }, "change");
      },
    }),
  };
});

function textPreview(text: string, contentHash: string): ProjectExplorerFilePreview {
  return {
    path: "F:\\Game\\src\\value.ts",
    name: "value.ts",
    extension: "ts",
    size: text.length,
    kind: "text",
    mimeType: "text/plain",
    text,
    contentHash,
    totalLines: 2,
    truncated: false,
    editable: true,
    revision: {
      exists: true,
      size: text.length,
      modifiedAtNanos: contentHash,
      key: `${text.length}:${contentHash}`,
    },
  };
}

function markdownPreview(text: string, contentHash: string): ProjectExplorerFilePreview {
  return {
    ...textPreview(text, contentHash),
    path: "F:\\Game\\Docs\\combat.md",
    name: "combat.md",
    extension: "md",
    mimeType: "text/markdown",
    totalLines: text.split(/\r\n|\r|\n/).length,
  };
}

async function flush(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await nextTick();
}

function emitWorkspaceFileChange(path: string, workspaceGeneration = 7): void {
  for (const handler of workspaceExplorerMocks.workspaceChangeHandlers) {
    handler({
      eventName: "workspace-file-changed",
      streamRevision: 1,
      projectId: "project-a",
      checkoutId: "checkout-a",
      workspaceGeneration,
      payload: {
        seq: 1,
        generation: 1,
        path,
        kind: "upsert",
        source: "os_watcher",
      },
    });
  }
}

afterEach(() => {
  document.body.innerHTML = "";
  workspaceExplorerMocks.preview.mockReset();
  workspaceExplorerMocks.write.mockReset();
  workspaceExplorerMocks.workspacePreview.mockReset();
  workspaceExplorerMocks.revision.mockReset();
  workspaceExplorerMocks.workspaceRevision.mockReset();
  workspaceExplorerMocks.workspaceChangeHandlers.length = 0;
  workspaceExplorerMocks.workspaceWrite.mockReset();
  workspaceExplorerMocks.editorDispatch.mockReset();
  workspaceExplorerMocks.editorFocus.mockReset();
});

describe("workspace file editor", () => {
  it("uses the knowledge editor's rendered CodeMirror mode for ordinary Markdown", async () => {
    workspaceExplorerMocks.preview.mockResolvedValue(markdownPreview(
      "# Combat rules\n\nKeep the loop readable.\n",
      "markdown-hash-before",
    ));
    workspaceExplorerMocks.write.mockResolvedValue(markdownPreview(
      "# Edited\n",
      "markdown-hash-after",
    ));
    const dirtyChanges: boolean[] = [];
    const editorRef = ref<{ saveFile(): Promise<boolean> } | null>(null);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          ref: editorRef,
          projectId: "project-a",
          path: "F:\\Game\\Docs\\combat.md",
          onDirtyChange: (dirty: boolean) => dirtyChanges.push(dirty),
        });
      },
    });
    app.mount(host);
    await flush();

    const editor = host.querySelector<HTMLElement>(".workspace-file-editor-test-change");
    expect(editor?.dataset.contentPath).toBe("F:\\Game\\Docs\\combat.md");
    expect(editor?.dataset.viewMode).toBe("rendered");
    expect(host.querySelector(".document-properties")).toBeNull();
    editor?.click();
    await nextTick();
    expect(dirtyChanges[dirtyChanges.length - 1]).toBe(true);
    await expect(editorRef.value?.saveFile()).resolves.toBe(true);
    expect(workspaceExplorerMocks.write).toHaveBeenCalledWith(
      "project-a",
      "F:\\Game\\Docs\\combat.md",
      "# Edited\n",
      "markdown-hash-before",
    );
    app.unmount();
  });

  it("keeps source files in native CodeMirror mode", async () => {
    workspaceExplorerMocks.preview.mockResolvedValue(textPreview(
      "export const value = 1;\n",
      "source-hash",
    ));
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          projectId: "project-a",
          path: "F:\\Game\\src\\value.ts",
        });
      },
    });
    app.mount(host);
    await flush();

    expect(
      host.querySelector<HTMLElement>(".workspace-file-editor-test-change")?.dataset.viewMode,
    ).toBe("native");
    app.unmount();
  });

  it("tracks dirty state and saves with the loaded hash while preserving CRLF", async () => {
    workspaceExplorerMocks.preview.mockResolvedValue(textPreview(
      "export const value = 1;\r\n",
      "hash-before",
    ));
    workspaceExplorerMocks.write.mockResolvedValue(textPreview(
      "export const value = 2;\r\n",
      "hash-after",
    ));
    const dirtyChanges: boolean[] = [];
    const editorRef = ref<{ saveFile(): Promise<boolean> } | null>(null);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          ref: editorRef,
          projectId: "project-a",
          path: "F:\\Game\\src\\value.ts",
          onDirtyChange: (dirty: boolean) => dirtyChanges.push(dirty),
        });
      },
    });
    app.mount(host);
    await flush();

    host.querySelector<HTMLButtonElement>(".workspace-file-editor-test-change")?.click();
    await nextTick();
    expect(dirtyChanges[dirtyChanges.length - 1]).toBe(true);
    await expect(editorRef.value?.saveFile()).resolves.toBe(true);
    expect(workspaceExplorerMocks.write).toHaveBeenCalledWith(
      "project-a",
      "F:\\Game\\src\\value.ts",
      "export const value = 2;\r\n",
      "hash-before",
    );
    expect(dirtyChanges[dirtyChanges.length - 1]).toBe(false);
    app.unmount();
  });

  it("reveals Unity line and column positions in the source editor", async () => {
    workspaceExplorerMocks.workspacePreview.mockResolvedValue(textPreview(
      "line one\nline two",
      "workspace-position-hash",
    ));
    const editorRef = ref<{
      revealPosition(line: number, column?: number): Promise<boolean>;
    } | null>(null);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          ref: editorRef,
          path: "Assets/Scripts/Player.cs",
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
        });
      },
    });
    app.mount(host);
    await flush();

    await expect(editorRef.value?.revealPosition(2, 3)).resolves.toBe(true);
    expect(workspaceExplorerMocks.editorDispatch).toHaveBeenCalledWith(expect.objectContaining({
      selection: { anchor: 11 },
    }));
    expect(workspaceExplorerMocks.editorFocus).toHaveBeenCalledOnce();
    app.unmount();
  });

  it("loads and saves checkout-scoped workspace files without mounting them in the tree", async () => {
    workspaceExplorerMocks.workspacePreview.mockResolvedValue(textPreview(
      "public class Player {}\n",
      "workspace-hash-before",
    ));
    workspaceExplorerMocks.workspaceWrite.mockResolvedValue(textPreview(
      "export const value = 2;\n",
      "workspace-hash-after",
    ));
    const editorRef = ref<{ saveFile(): Promise<boolean> } | null>(null);
    const workspaceRef = { checkoutId: "checkout-a", expectedGeneration: 7 };
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          ref: editorRef,
          projectId: "project-a",
          path: "Assets/Scripts/Player.cs",
          workspaceRef,
        });
      },
    });
    app.mount(host);
    await flush();

    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledWith(
      "Assets/Scripts/Player.cs",
      workspaceRef,
    );
    host.querySelector<HTMLButtonElement>(".workspace-file-editor-test-change")?.click();
    await nextTick();
    await expect(editorRef.value?.saveFile()).resolves.toBe(true);
    expect(workspaceExplorerMocks.workspaceWrite).toHaveBeenCalledWith(
      "Assets/Scripts/Player.cs",
      "export const value = 2;\n",
      "workspace-hash-before",
      workspaceRef,
    );
    app.unmount();
  });

  it("reloads a clean active editor only after its exact workspace file changes", async () => {
    const before = markdownPreview("# Version 4\n", "hash-v4");
    const after = markdownPreview("# Version 5\n", "hash-v5");
    workspaceExplorerMocks.workspacePreview
      .mockResolvedValueOnce(before)
      .mockResolvedValueOnce(after);
    workspaceExplorerMocks.workspaceRevision.mockResolvedValue(after.revision);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          path: "Assets/Docs/combat.md",
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
          active: true,
        });
      },
    });
    app.mount(host);
    await flush();

    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(1);
    emitWorkspaceFileChange("Assets/Other.md");
    await new Promise((resolve) => setTimeout(resolve, 160));
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(1);

    emitWorkspaceFileChange("Assets/Docs/combat.md");
    await new Promise((resolve) => setTimeout(resolve, 160));
    await flush();
    expect(workspaceExplorerMocks.workspaceRevision).toHaveBeenCalledTimes(1);
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(2);
    expect(
      host.querySelector<HTMLElement>(".workspace-file-editor-test-change")?.dataset.modelValue,
    ).toBe("# Version 5\n");
    app.unmount();
  });

  it("uses a metadata probe on focus and skips unchanged file content reads", async () => {
    const current = markdownPreview("# Stable\n", "hash-stable");
    workspaceExplorerMocks.workspacePreview.mockResolvedValue(current);
    workspaceExplorerMocks.workspaceRevision.mockResolvedValue(current.revision);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          path: "Assets/Docs/combat.md",
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
          active: true,
        });
      },
    });
    app.mount(host);
    await flush();

    window.dispatchEvent(new Event("focus"));
    await new Promise((resolve) => setTimeout(resolve, 160));
    expect(workspaceExplorerMocks.workspaceRevision).toHaveBeenCalledTimes(1);
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(1);
    app.unmount();
  });

  it("defers file probes for inactive tabs until activation", async () => {
    const before = markdownPreview("# Version 4\n", "hash-v4");
    const after = markdownPreview("# Version 5\n", "hash-v5");
    workspaceExplorerMocks.workspacePreview
      .mockResolvedValueOnce(before)
      .mockResolvedValueOnce(after);
    workspaceExplorerMocks.workspaceRevision.mockResolvedValue(after.revision);
    const active = ref(false);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          path: "Assets/Docs/combat.md",
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
          active: active.value,
        });
      },
    });
    app.mount(host);
    await flush();

    emitWorkspaceFileChange("Assets/Docs/combat.md");
    await new Promise((resolve) => setTimeout(resolve, 160));
    expect(workspaceExplorerMocks.workspaceRevision).not.toHaveBeenCalled();
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(1);

    active.value = true;
    await nextTick();
    await new Promise((resolve) => setTimeout(resolve, 160));
    await flush();
    expect(workspaceExplorerMocks.workspaceRevision).toHaveBeenCalledTimes(1);
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(2);
    app.unmount();
  });

  it("keeps dirty edits visible and exposes an explicit disk conflict choice", async () => {
    const before = markdownPreview("# Version 4\n", "hash-v4");
    const after = markdownPreview("# Version 5\n", "hash-v5");
    workspaceExplorerMocks.workspacePreview
      .mockResolvedValueOnce(before)
      .mockResolvedValueOnce(after);
    workspaceExplorerMocks.workspaceRevision.mockResolvedValue(after.revision);
    const host = document.createElement("div");
    document.body.appendChild(host);
    const app = createApp({
      setup() {
        return () => h(WorkspaceFilePreview, {
          path: "Assets/Docs/combat.md",
          workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 7 },
          active: true,
        });
      },
    });
    app.mount(host);
    await flush();
    host.querySelector<HTMLButtonElement>(".workspace-file-editor-test-change")?.click();
    await nextTick();

    emitWorkspaceFileChange("Assets/Docs/combat.md");
    await new Promise((resolve) => setTimeout(resolve, 160));
    await flush();
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(1);
    const conflict = host.querySelector<HTMLElement>(".workspace-file-preview-conflict");
    expect(conflict).not.toBeNull();

    conflict?.querySelector<HTMLButtonElement>("button")?.click();
    await flush();
    expect(workspaceExplorerMocks.workspacePreview).toHaveBeenCalledTimes(2);
    expect(host.querySelector(".workspace-file-preview-conflict")).toBeNull();
    expect(
      host.querySelector<HTMLElement>(".workspace-file-editor-test-change")?.dataset.modelValue,
    ).toBe("# Version 5\n");
    app.unmount();
  });
});
