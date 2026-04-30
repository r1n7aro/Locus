import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const webviewWindowMocks = vi.hoisted(() => ({
  createdWindows: [] as Array<unknown[]>,
  getByLabelMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: vi.fn(() => ({ label: "main" })),
  WebviewWindow: class {
    static getByLabel = webviewWindowMocks.getByLabelMock;

    constructor(...args: unknown[]) {
      webviewWindowMocks.createdWindows.push(args);
    }

    once(event: string, handler: () => void) {
      if (event === "tauri://created") handler();
    }
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
    webviewWindowMocks.getByLabelMock.mockReset();
    webviewWindowMocks.createdWindows.length = 0;
  });

  it("builds a dedicated window url", () => {
    expect(buildKnowledgeLexicalProgressWindowUrl()).toBe("/knowledge-lexical-progress?knowledgeLexicalProgress=1");
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

  it("reuses an existing progress window without forcing focus", async () => {
    const existingWindow = {
      setFocus: vi.fn(),
    };
    webviewWindowMocks.getByLabelMock.mockResolvedValue(existingWindow);

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

    expect(existingWindow.setFocus).not.toHaveBeenCalled();
    expect(webviewWindowMocks.createdWindows).toHaveLength(0);
  });

  it("creates a closable frameless progress window", async () => {
    webviewWindowMocks.getByLabelMock.mockResolvedValue(null);

    await openKnowledgeLexicalProgressWindow();

    expect(webviewWindowMocks.createdWindows).toHaveLength(1);
    const [, options] = webviewWindowMocks.createdWindows[0] as [
      string,
      Record<string, unknown>,
    ];
    expect(options.decorations).toBe(false);
    expect(options.closable).toBe(true);
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
