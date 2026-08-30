// @vitest-environment jsdom

import { createApp, h, nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ProjectExplorerFilePreview } from "../types/workbench";
import WorkspaceFilePreview from "../components/workbench/WorkspaceFilePreview.vue";

const workspaceExplorerMocks = vi.hoisted(() => ({
  preview: vi.fn(),
  write: vi.fn(),
  workspacePreview: vi.fn(),
  workspaceWrite: vi.fn(),
  editorDispatch: vi.fn(),
  editorFocus: vi.fn(),
}));

vi.mock("../services/workspaceExplorer", () => ({
  projectExplorerPreviewFile: workspaceExplorerMocks.preview,
  projectExplorerWriteFile: workspaceExplorerMocks.write,
  workspaceFilePreview: workspaceExplorerMocks.workspacePreview,
  workspaceFileWrite: workspaceExplorerMocks.workspaceWrite,
}));

vi.mock("../components/ui/BaseMarkdownEditor.vue", async () => {
  const { Text } = await import("@codemirror/state");
  const { defineComponent, h: render } = await import("vue");
  return {
    default: defineComponent({
      emits: ["documentChange", "shortcutSave"],
      setup(_, { emit, expose }) {
        const editorView = {
          state: { doc: Text.of(["line one", "line two"]) },
          dispatch: workspaceExplorerMocks.editorDispatch,
          focus: workspaceExplorerMocks.editorFocus,
        };
        expose({ getEditorView: () => editorView });
        return () => render("button", {
          class: "workspace-file-editor-test-change",
          onClick: () => emit("documentChange", {
            doc: Text.of(["export const value = 2;", ""]),
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
  };
}

async function flush(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
  await nextTick();
}

afterEach(() => {
  document.body.innerHTML = "";
  workspaceExplorerMocks.preview.mockReset();
  workspaceExplorerMocks.write.mockReset();
  workspaceExplorerMocks.workspacePreview.mockReset();
  workspaceExplorerMocks.workspaceWrite.mockReset();
  workspaceExplorerMocks.editorDispatch.mockReset();
  workspaceExplorerMocks.editorFocus.mockReset();
});

describe("workspace file editor", () => {
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
});
