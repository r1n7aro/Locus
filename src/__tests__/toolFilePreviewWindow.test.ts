import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const subWindowMocks = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  getByLabelMock: vi.fn(),
  emitMock: vi.fn(),
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
  buildToolFilePreviewWindowUrl,
  createToolFilePreviewEditHighlight,
  getToolFilePreviewWindowPayload,
  isToolFilePreviewWindowLocation,
  openToolFilePreviewWindow,
  resolveToolFilePreviewHighlightRanges,
} from "../services/toolFilePreviewWindow";

const cwd = process.cwd();

function read(relativePath: string) {
  return readFileSync(resolve(cwd, relativePath), "utf8");
}

function stubTauriWindow() {
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
}

describe("toolFilePreviewWindow", () => {
  beforeEach(() => {
    subWindowMocks.invokeMock.mockReset();
    subWindowMocks.getByLabelMock.mockReset();
    subWindowMocks.emitMock.mockReset();
    subWindowMocks.getByLabelMock.mockResolvedValue({ emit: subWindowMocks.emitMock });
    stubTauriWindow();
  });

  it("builds and parses a preview URL for workspace-relative paths", () => {
    const target = createToolFilePreviewEditHighlight({
      oldText: "before\nold\nafter",
      newText: "before\nnew\nafter",
      startLine: 8,
    });
    const url = buildToolFilePreviewWindowUrl({
      filePath: "Locus/knowledge/memory/design.md",
      highlight: { mode: "edit", targets: target ? [target] : [] },
    });
    const search = url.slice(url.indexOf("?"));
    expect(isToolFilePreviewWindowLocation({ search } as Location)).toBe(true);
    expect(getToolFilePreviewWindowPayload(search)).toEqual({
      filePath: "Locus/knowledge/memory/design.md",
      highlight: { mode: "edit", targets: target ? [target] : [] },
    });
  });

  it("highlights exact edit matches and drops stale matches", () => {
    const target = createToolFilePreviewEditHighlight({
      oldText: "before\nold\nafter",
      newText: "before\nnew\nafter",
      startLine: 3,
    });
    expect(target).not.toBeNull();
    const highlight = { mode: "edit" as const, targets: target ? [target] : [] };
    expect(resolveToolFilePreviewHighlightRanges(
      "zero\none\nbefore\nnew\nafter\ntail",
      1,
      highlight,
    )).toEqual([{ startLine: 4, endLine: 4 }]);
    expect(resolveToolFilePreviewHighlightRanges(
      "zero\none\nbefore\nchanged again\nafter\ntail",
      1,
      highlight,
    )).toEqual([]);
  });

  it("highlights every displayed line for write previews", () => {
    expect(resolveToolFilePreviewHighlightRanges(
      "first\nsecond\nthird",
      5,
      { mode: "all" },
    )).toEqual([{ startLine: 5, endLine: 7 }]);
  });

  it("opens the reusable Locus file preview window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "tool-file-preview",
      existing: true,
      pooled: false,
    });

    await expect(openToolFilePreviewWindow({ filePath: "src/App.vue" })).resolves.toBe(true);
    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "tool-file-preview",
        title: "Locus - App.vue",
        query: expect.stringContaining("filePath=src%2FApp.vue"),
      }),
    });
    expect(subWindowMocks.emitMock).toHaveBeenCalledWith(
      "tool-file-preview:payload",
      { filePath: "src/App.vue" },
    );
  });

  it("wires read/edit/write hover actions into the tool header", () => {
    const toolBlock = read("src/components/ToolCallBlock.vue");
    const diffViewer = read("src/components/diff/FileDiffViewer.vue");
    const windowApp = read("src/WindowApp.vue");
    const capabilities = read("src-tauri/capabilities/default.json");
    const backend = read("src-tauri/src/commands/knowledge.rs");

    expect(toolBlock).toContain("resolveToolFilePreviewPayload");
    expect(toolBlock).toContain("tool-file-preview-action");
    expect(toolBlock).toContain("openToolFilePreviewWindow");
    expect(diffViewer).toContain("textToolbarActionLabel");
    expect(diffViewer).toContain("emit('textToolbarAction')");
    expect(windowApp).toContain("ToolFilePreviewWindow.vue");
    expect(capabilities).toContain('"tool-file-preview"');
    expect(backend).toContain("FULL_PREVIEW_MAX_FILE_BYTES");
    expect(backend).toContain("let max_lines: usize = if full { 20_000 } else { 50 };");
  });
});
