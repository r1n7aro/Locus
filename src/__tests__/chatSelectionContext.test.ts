import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import {
  SELECTION_CONTEXT_HIT_TOLERANCE_PX,
  selectionTextAtPoint,
  type SelectionLike,
  type SelectionRectLike,
} from "../composables/chatSelectionContext";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

function rect(left: number, top: number, right: number, bottom: number): SelectionRectLike {
  return { left, top, right, bottom, width: right - left, height: bottom - top };
}

function makeSelection(options: {
  text: string;
  rangeRects: SelectionRectLike[][];
  isCollapsed?: boolean;
}): SelectionLike {
  return {
    isCollapsed: options.isCollapsed ?? false,
    rangeCount: options.rangeRects.length,
    toString: () => options.text,
    getRangeAt: (index: number) => ({
      getClientRects: () => options.rangeRects[index] ?? [],
    }),
  };
}

describe("selectionTextAtPoint", () => {
  it("returns empty for missing, collapsed, or empty-range selections", () => {
    expect(selectionTextAtPoint(null, { x: 10, y: 10 })).toBe("");
    expect(selectionTextAtPoint(undefined, { x: 10, y: 10 })).toBe("");
    expect(
      selectionTextAtPoint(
        makeSelection({ text: "hello", rangeRects: [[rect(0, 0, 50, 10)]], isCollapsed: true }),
        { x: 5, y: 5 },
      ),
    ).toBe("");
    expect(
      selectionTextAtPoint(makeSelection({ text: "hello", rangeRects: [] }), { x: 5, y: 5 }),
    ).toBe("");
  });

  it("returns empty for whitespace-only selections even when the point hits", () => {
    const selection = makeSelection({ text: " \n\t", rangeRects: [[rect(0, 0, 50, 10)]] });
    expect(selectionTextAtPoint(selection, { x: 5, y: 5 })).toBe("");
  });

  it("returns the full selection text when the point falls inside a rect", () => {
    const selection = makeSelection({ text: "mainten", rangeRects: [[rect(100, 40, 160, 56)]] });
    expect(selectionTextAtPoint(selection, { x: 120, y: 48 })).toBe("mainten");
  });

  it("returns empty when the point is outside every rect", () => {
    const selection = makeSelection({ text: "mainten", rangeRects: [[rect(100, 40, 160, 56)]] });
    expect(selectionTextAtPoint(selection, { x: 300, y: 200 })).toBe("");
  });

  it("applies the hit tolerance around rect edges", () => {
    const selection = makeSelection({ text: "edge", rangeRects: [[rect(100, 40, 160, 56)]] });
    const within = 160 + SELECTION_CONTEXT_HIT_TOLERANCE_PX;
    expect(selectionTextAtPoint(selection, { x: within, y: 48 })).toBe("edge");
    expect(selectionTextAtPoint(selection, { x: within + 1, y: 48 })).toBe("");
  });

  it("hits any line rect of a multi-line selection", () => {
    const selection = makeSelection({
      text: "line one\nline two",
      rangeRects: [[rect(100, 40, 300, 56), rect(100, 60, 220, 76)]],
    });
    expect(selectionTextAtPoint(selection, { x: 150, y: 68 })).toBe("line one\nline two");
  });

  it("ignores zero-size rects but still matches real ones", () => {
    const zeroOnly = makeSelection({ text: "ghost", rangeRects: [[rect(100, 40, 100, 40)]] });
    expect(selectionTextAtPoint(zeroOnly, { x: 100, y: 40 })).toBe("");

    const mixed = makeSelection({
      text: "ghost",
      rangeRects: [[rect(100, 40, 100, 40), rect(120, 40, 180, 56)]],
    });
    expect(selectionTextAtPoint(mixed, { x: 130, y: 44 })).toBe("ghost");
  });

  it("checks every range of a multi-range selection", () => {
    const selection = makeSelection({
      text: "alpha beta",
      rangeRects: [[rect(0, 0, 40, 10)], [rect(100, 100, 160, 116)]],
    });
    expect(selectionTextAtPoint(selection, { x: 150, y: 108 })).toBe("alpha beta");
  });
});

describe("chat message context menu selection wiring", () => {
  it("prefers the text selection over whole-message actions in the transcript menu", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain('import { selectionTextAtPoint } from "../composables/chatSelectionContext";');
    expect(chatView).toContain("selectionTextAtPoint(window.getSelection(), { x: e.clientX, y: e.clientY })");
    expect(chatView).toContain("messageId: string | null;");
    expect(chatView).toContain("selectionText: string;");
    expect(chatView).toContain("if (!selectionText && !messageId) return;");
    expect(chatView).toContain("async function doMessageCopySelection()");
    expect(chatView).toContain("writeChatMessageClipboard({ text: selectionText, draft: null, serializedDraft: null })");
    expect(chatView).toContain('v-if="messageCtxMenu.selectionText"');
    expect(chatView).toContain('@click="doMessageCopySelection"');
    expect(chatView).toContain('t("chat.messageMenu.copySelection")');
    expect(chatView).toContain('v-else-if="messageCtxMenu.messageId"');
    expect(chatView).toContain('<template v-if="messageCtxMenu.messageId">');
  });

  it("keeps the asset-ref menu behind the selection check", () => {
    const chatView = read("src/components/ChatView.vue");
    const handler = chatView.slice(
      chatView.indexOf("function handleContentContextMenu"),
      chatView.indexOf("function handleContentClick"),
    );
    expect(handler.indexOf("selectionTextAtPoint")).toBeGreaterThan(-1);
    expect(handler.indexOf("selectionTextAtPoint")).toBeLessThan(
      handler.indexOf("assetContextTargetFromElement"),
    );
    expect(handler).toContain("if (!selectionText) {");
  });

  it("localizes the copy-selection entry in both languages", () => {
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");
    expect(zh).toContain('"chat.messageMenu.copySelection": "复制选中内容"');
    expect(en).toContain('"chat.messageMenu.copySelection": "Copy selection"');
  });
});
