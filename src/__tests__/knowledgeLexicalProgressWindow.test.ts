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

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

import {
  LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
  buildKnowledgeLexicalProgressWindowUrl,
  getKnowledgeLexicalProgressRunKey,
  isKnowledgeLexicalProgressWindowLocation,
  openKnowledgeLexicalProgressWindow,
  shouldAutoOpenKnowledgeLexicalProgressWindow,
} from "../services/knowledgeLexicalProgressWindow";

describe("knowledgeLexicalProgressWindow", () => {
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

  it("builds a dedicated window url", () => {
    expect(buildKnowledgeLexicalProgressWindowUrl()).toBe("/window.html?knowledgeLexicalProgress=1");
  });

  it("detects lexical progress window locations", () => {
    expect(isKnowledgeLexicalProgressWindowLocation({
      pathname: "/knowledge-lexical-progress",
      search: "",
    } as Location)).toBe(true);
    expect(isKnowledgeLexicalProgressWindowLocation({
      pathname: "/",
      search: "?knowledgeLexicalProgress=1",
    } as Location)).toBe(true);
    expect(isKnowledgeLexicalProgressWindowLocation({
      pathname: "/knowledge",
      search: "",
    } as Location)).toBe(false);
  });

  it("opens only for large running rebuilds", () => {
    expect(shouldAutoOpenKnowledgeLexicalProgressWindow({
      running: true,
      stage: "indexing",
      detail: "Indexing docs",
      currentFile: "reference/unity/a.md",
      processedDocs: 64,
      totalDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: null,
    })).toBe(true);

    expect(shouldAutoOpenKnowledgeLexicalProgressWindow({
      running: true,
      stage: "indexing",
      detail: "Indexing docs",
      currentFile: "reference/unity/a.md",
      processedDocs: 12,
      totalDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD - 1,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: null,
    })).toBe(false);

    expect(shouldAutoOpenKnowledgeLexicalProgressWindow({
      running: false,
      stage: "completed",
      detail: "Done",
      currentFile: null,
      processedDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
      totalDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: "2026-04-16T00:01:00Z",
    })).toBe(false);
  });

  it("uses startedAt as a stable run key", () => {
    expect(getKnowledgeLexicalProgressRunKey({
      running: true,
      stage: "preparing",
      detail: "Preparing docs",
      currentFile: null,
      processedDocs: 12,
      totalDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: null,
    })).toBe("2026-04-16T00:00:00Z");

    expect(getKnowledgeLexicalProgressRunKey({
      running: true,
      stage: "committing",
      detail: "Commit docs",
      currentFile: null,
      processedDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
      totalDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD * 2,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: null,
    })).toBe("2026-04-16T00:00:00Z");
  });

  it("opens without stealing focus from the main window", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "knowledge-lexical-progress",
      existing: false,
      pooled: false,
    });

    await openKnowledgeLexicalProgressWindow({
      running: true,
      stage: "indexing",
      detail: "Indexing docs",
      currentFile: "reference/unity/a.md",
      processedDocs: 64,
      totalDocs: LARGE_LEXICAL_REBUILD_DOC_THRESHOLD,
      error: null,
      startedAt: "2026-04-16T00:00:00Z",
      completedAt: null,
    });

    expect(subWindowMocks.invokeMock).toHaveBeenCalledWith("sub_window_open", {
      request: expect.objectContaining({
        kind: "knowledge-lexical-progress",
        focusExisting: false,
        resizable: false,
        closable: true,
        query: expect.stringContaining("knowledgeLexicalProgress=1"),
      }),
    });
  });

  it("reuses an existing progress window without emitting a payload", async () => {
    subWindowMocks.invokeMock.mockResolvedValue({
      label: "sub-pool-2",
      existing: true,
      pooled: false,
    });
    const existingWindow = { emit: vi.fn(), setFocus: vi.fn() };
    subWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

    await openKnowledgeLexicalProgressWindow();

    expect(existingWindow.emit).not.toHaveBeenCalled();
    expect(existingWindow.setFocus).not.toHaveBeenCalled();
  });

  it("uses a titlebar close action instead of duplicate progress text", () => {
    const component = read("src/components/KnowledgeLexicalProgressWindow.vue");
    const knowledgeService = read("src/services/knowledge.ts");
    const tauriCommands = read("src-tauri/src/commands/knowledge.rs");
    const capability = read("src-tauri/capabilities/default.json");

    expect(component).toContain('class="lexical-window-close"');
    expect(component).toContain('@click.stop="void requestWindowClose()"');
    expect(component).toContain("await appWindow.destroy()");
    expect(component).toContain("knowledgeCloseLexicalProgressWindow");
    expect(component).not.toContain('class="lexical-window-titlebar-progress"');
    expect(knowledgeService).toContain(
      'ipcInvoke<void>("knowledge_close_lexical_progress_window")',
    );
    expect(tauriCommands).toContain("knowledge_close_lexical_progress_window");
    expect(tauriCommands).toMatch(
      /window\s*\.\s*destroy\(\)\s*\.\s*or_else\(\|_\|\s*window\.close\(\)\)/,
    );
    expect(capability).toContain('"core:window:allow-destroy"');
    expect(capability).toContain('"core:window:allow-set-closable"');
  });
});
