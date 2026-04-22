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

    once(..._args: unknown[]) {}
  },
}));

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
});
