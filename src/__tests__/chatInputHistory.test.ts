import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  createChatInputHistoryState,
  navigateChatInputHistory,
  shouldNavigateChatInputHistory,
} from "../composables/chatInputHistory";

interface Draft {
  text: string;
}

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat input history", () => {
  it("walks backward through history and restores the preserved draft", () => {
    const history: Draft[] = [
      { text: "first" },
      { text: "second" },
      { text: "latest" },
    ];
    const current = { text: "unfinished" };

    const latest = navigateChatInputHistory(
      history,
      createChatInputHistoryState<Draft>(),
      "previous",
      current,
    );
    expect(latest?.value).toEqual({ text: "latest" });

    const second = navigateChatInputHistory(history, latest!.state, "previous", latest!.value);
    expect(second?.value).toEqual({ text: "second" });

    const first = navigateChatInputHistory(history, second!.state, "previous", second!.value);
    expect(first?.value).toEqual({ text: "first" });

    const firstBoundary = navigateChatInputHistory(history, first!.state, "previous", first!.value);
    expect(firstBoundary?.value).toEqual({ text: "first" });

    const newerSecond = navigateChatInputHistory(history, firstBoundary!.state, "next", firstBoundary!.value);
    const newerLatest = navigateChatInputHistory(history, newerSecond!.state, "next", newerSecond!.value);
    const restored = navigateChatInputHistory(history, newerLatest!.state, "next", newerLatest!.value);

    expect(restored?.value).toEqual(current);
    expect(restored?.state).toEqual(createChatInputHistoryState<Draft>());
  });

  it("leaves ArrowDown available while history navigation is idle", () => {
    const result = navigateChatInputHistory(
      [{ text: "latest" }],
      createChatInputHistoryState<Draft>(),
      "next",
      { text: "draft" },
    );

    expect(result).toBeNull();
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowDown" },
      { value: "draft", selectionStart: 5, selectionEnd: 5, isNavigating: false },
    )).toBe(false);
  });

  it("uses ArrowUp on a single line or the first line and preserves multiline cursor movement", () => {
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowUp" },
      { value: "single line", selectionStart: 6, selectionEnd: 6, isNavigating: false },
    )).toBe(true);
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowUp" },
      { value: "first\nsecond", selectionStart: 3, selectionEnd: 3, isNavigating: false },
    )).toBe(true);
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowUp" },
      { value: "first\nsecond", selectionStart: 9, selectionEnd: 9, isNavigating: false },
    )).toBe(false);
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowDown" },
      { value: "first\nsecond", selectionStart: 3, selectionEnd: 3, isNavigating: true },
    )).toBe(false);
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowDown" },
      { value: "first\nsecond", selectionStart: 9, selectionEnd: 9, isNavigating: true },
    )).toBe(true);
  });

  it("ignores modified shortcuts, selections, and IME composition", () => {
    const selection = {
      value: "draft",
      selectionStart: 2,
      selectionEnd: 2,
      isNavigating: false,
    };

    expect(shouldNavigateChatInputHistory({ key: "ArrowUp", ctrlKey: true }, selection)).toBe(false);
    expect(shouldNavigateChatInputHistory({ key: "ArrowUp", isComposing: true }, selection)).toBe(false);
    expect(shouldNavigateChatInputHistory(
      { key: "ArrowUp" },
      { ...selection, selectionStart: 0, selectionEnd: 3 },
    )).toBe(false);
  });

  it("connects both full and embedded chat sessions to the shared history input", () => {
    const richInput = read("src/components/chat/RichChatInput.vue");
    const chatView = read("src/components/ChatView.vue");
    const embeddedPane = read("src/components/chat/EmbeddedChatPane.vue");

    expect(richInput).toContain("messageHistory?: ChatMessage[];");
    expect(richInput).toContain("buildUserMessageDraft(message)");
    expect(chatView).toContain(':message-history="messages"');
    expect(embeddedPane).toContain(':message-history="messages"');
  });
});
