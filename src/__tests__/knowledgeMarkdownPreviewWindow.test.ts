import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const subWindowMocks = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  getByLabelMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: subWindowMocks.invokeMock,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: vi.fn(() => ({ label: "main" })),
  WebviewWindow: class {
    static getByLabel = subWindowMocks.getByLabelMock;
  },
}));

import {
  KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT,
  buildKnowledgeMarkdownPreviewWindowUrl,
  getKnowledgeMarkdownPreviewWindowPayload,
  openKnowledgeMarkdownPreviewWindow,
} from "../services/knowledgeMarkdownPreviewWindow";

const cwd = process.cwd();

function read(relativePath: string) {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

describe("knowledgeMarkdownPreviewWindow", () => {
  const payload = { docType: "memory" as const, path: "memory/project-notes.md" };

  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue(null);
    Object.defineProperty(globalThis, "window", {
      configurable: true,
      value: {
        location: { pathname: "/", search: "" },
        __TAURI_INTERNALS__: {
          invoke: vi.fn(),
          metadata: { currentWindow: { label: "main" } },
        },
      },
    });
  });

  it("builds and parses lightweight-window URLs", () => {
    const url = buildKnowledgeMarkdownPreviewWindowUrl(payload);

    expect(url).toContain("/window.html?knowledgeMarkdownPreview=1");
    expect(getKnowledgeMarkdownPreviewWindowPayload(url.slice(url.indexOf("?")))).toEqual(payload);
  });

  it("updates an existing Markdown preview window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-4",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openKnowledgeMarkdownPreviewWindow(payload);

    expect(existingWindow.emit).toHaveBeenCalledWith(
      KNOWLEDGE_MARKDOWN_PREVIEW_WINDOW_EVENT,
      payload,
    );
  });

  it("opens through the shared sub-window route", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "knowledge-markdown-preview",
      existing: false,
      pooled: false,
    });

    await expect(openKnowledgeMarkdownPreviewWindow(payload)).resolves.toBe(true);
    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "knowledge-markdown-preview",
        query: expect.stringContaining("path=memory%2Fproject-notes.md"),
        width: 920,
        height: 720,
      }),
    });
  });

  it("wires the display preference to Memory references and the preview window", () => {
    const displaySettings = read("src/composables/useDisplaySettings.ts");
    const displayPanel = read("src/components/settings/DisplaySettings.vue");
    const documentOpen = read("src/composables/useKnowledgeDocumentOpen.ts");
    const chatView = read("src/components/ChatView.vue");
    const assetChip = read("src/components/AssetChip.vue");
    const windowApp = read("src/WindowApp.vue");
    const viewer = read("src/components/KnowledgeMarkdownPreviewWindow.vue");
    const capabilities = read("src-tauri/capabilities/default.json");

    expect(displaySettings).toContain('export type MemoryFileOpenTarget = "window" | "knowledge";');
    expect(displaySettings).toContain("memoryFileOpenTarget: MemoryFileOpenTarget;");
    expect(displaySettings).toContain('memoryFileOpenTarget: "knowledge",');
    expect(displayPanel).toContain(':model-value="display.memoryFileOpenTarget"');
    expect(displayPanel).toContain("settings.display.memoryFileOpenWindow");
    expect(displayPanel).toContain("settings.display.memoryFileOpenKnowledge");
    expect(documentOpen).toContain('docType === "memory"');
    expect(documentOpen).toContain('displaySettings.memoryFileOpenTarget === "window"');
    expect(chatView).toContain("openKnowledgeDocument(docType, path)");
    expect(assetChip).toContain("await openKnowledgeDocument(knowledgeRef.value.docType");
    expect(windowApp).toContain('kind: "knowledge-markdown-preview"');
    expect(viewer).toContain("<MarkdownRenderer v-else :content=\"content\" />");
    expect(viewer).toContain('part: "full"');
    expect(capabilities).toContain('"knowledge-markdown-preview"');
  });
});
