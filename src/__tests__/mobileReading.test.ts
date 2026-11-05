// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  capturePosition,
  documentFontScale,
  loadReadingPosition,
  normalizeFontScale,
  pinchFontScale,
  POSITION_LIMIT,
  positionDelta,
  readingKey,
  resetReadingCache,
  saveReadingPosition,
  setDocumentFontScale,
  stepFontScale,
} from "../mobile/reading";
import type { DocumentSource } from "../mobile/routes";

const workspaceRef = { checkoutId: "c1", expectedGeneration: 1, expectedMaterializationEpoch: null };

/** Blocks stacked from `top`, with the given heights. */
function blocks(top: number, heights: number[]): Element[] {
  let y = top;
  return heights.map((height) => {
    const element = document.createElement("p");
    const rect = { top: y, bottom: y + height, height };
    element.getBoundingClientRect = () => rect as DOMRect;
    y += height;
    return element;
  });
}

beforeEach(() => {
  localStorage.clear();
  resetReadingCache();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("mobile document text size", () => {
  it("clamps, steps by tenths and snaps pinches to a coarser grid", () => {
    expect(normalizeFontScale(null)).toBe(1);
    expect(normalizeFontScale("x")).toBe(1);
    expect(normalizeFontScale("1.3")).toBe(1.3);
    expect(normalizeFontScale(0.2)).toBe(0.8);
    expect(normalizeFontScale(5)).toBe(2);
    expect(stepFontScale(1, 1)).toBe(1.1);
    expect(stepFontScale(1, -1)).toBe(0.9);
    // Between tenths after a pinch: to the neighbouring tenth.
    expect(stepFontScale(1.15, 1)).toBe(1.2);
    expect(stepFontScale(1.15, -1)).toBe(1.1);
    expect(stepFontScale(2, 1)).toBe(2);
    expect(stepFontScale(0.8, -1)).toBe(0.8);
    expect(pinchFontScale(1, 1.03)).toBe(1.05);
    expect(pinchFontScale(1, 1.012)).toBe(1);
    expect(pinchFontScale(1.5, 3)).toBe(2);
  });

  it("persists the size unless a pinch is still running", () => {
    setDocumentFontScale(1.2, { persist: false });
    expect(documentFontScale.value).toBe(1.2);
    expect(localStorage.getItem("locus-remote.documentFontScale")).toBeNull();
    setDocumentFontScale(1.2);
    expect(localStorage.getItem("locus-remote.documentFontScale")).toBe("1.2");
    resetReadingCache();
    expect(documentFontScale.value).toBe(1.2);
    setDocumentFontScale(1);
    expect(localStorage.getItem("locus-remote.documentFontScale")).toBeNull();
  });
});

describe("mobile reading positions", () => {
  it("keys documents independent of path spelling", () => {
    const knowledge = (path: string): DocumentSource => ({ kind: "knowledge", workspaceRef, docType: "design", path });
    expect(readingKey(knowledge("design/a/b.md"), null)).toBe(readingKey(knowledge("/a/b.md"), null));
    const file: DocumentSource = { kind: "file", workspaceRef, path: "F:\\Proj\\Docs\\A.md" };
    expect(readingKey(file, "Docs/A.md")).toBe(readingKey({ kind: "file", workspaceRef, path: "docs/a.md" }, "docs/a.md"));
    expect(readingKey(file, null)).not.toBe(readingKey(file, "Docs/A.md"));
    const other = { ...workspaceRef, checkoutId: "c2" };
    expect(readingKey({ kind: "file", workspaceRef: other, path: "docs/a.md" }, "docs/a.md"))
      .not.toBe(readingKey(file, "Docs/A.md"));
  });

  it("stores positions across reloads, forgets on null and drops the oldest", () => {
    const position = { block: 3, offset: 0.25, blocks: 10, ratio: 0.4 };
    saveReadingPosition("k", position);
    resetReadingCache();
    expect(loadReadingPosition("k")).toEqual(position);
    saveReadingPosition("k", null);
    resetReadingCache();
    expect(loadReadingPosition("k")).toBeNull();
    expect(localStorage.getItem("locus-remote.readingPositions")).toBeNull();

    let now = 1_000;
    vi.spyOn(Date, "now").mockImplementation(() => now++);
    for (let index = 0; index <= POSITION_LIMIT; index += 1) saveReadingPosition(`doc${index}`, position);
    resetReadingCache();
    expect(loadReadingPosition("doc0")).toBeNull();
    expect(loadReadingPosition("doc1")).toEqual(position);
    expect(loadReadingPosition(`doc${POSITION_LIMIT}`)).toEqual(position);
  });

  it("ignores damaged storage", () => {
    localStorage.setItem("locus-remote.readingPositions", "{not json");
    resetReadingCache();
    expect(loadReadingPosition("k")).toBeNull();
    localStorage.setItem("locus-remote.readingPositions", JSON.stringify({ k: { block: "1" } }));
    resetReadingCache();
    expect(loadReadingPosition("k")).toBeNull();
  });

  it("anchors to the block under a line and scrolls back to it", () => {
    // Viewport top at y=100; blocks 0..2 span 50..150, 150..350, 350..400.
    const before = blocks(50, [100, 200, 50]);
    const position = capturePosition(before, 200, 0.5)!;
    expect(position).toEqual({ block: 1, offset: 0.25, blocks: 3, ratio: 0.5 });
    // Larger text: the same blocks are taller and the page moved.
    const after = blocks(-400, [200, 400, 100]);
    // Block 1 spans -200..200; a quarter in is y=-100, to be brought to y=200.
    expect(positionDelta(after, position, 200)).toBe(-300);
    // A changed document falls back to the ratio.
    expect(positionDelta(blocks(0, [10, 10]), position, 200)).toBeNull();
  });

  it("clamps lines above the first or below the last block", () => {
    const list = blocks(100, [50, 50]);
    expect(capturePosition(list, 0, 0)).toMatchObject({ block: 0, offset: 0 });
    expect(capturePosition(list, 500, 1)).toMatchObject({ block: 1, offset: 1 });
    expect(capturePosition([], 0, 0)).toBeNull();
  });
});
