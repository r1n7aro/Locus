import { describe, expect, it } from "vitest";
import type { ChatMessage } from "../types";
import {
  buildChatMessageClipboardPayload,
  buildUserMessageDraft,
  copyableChatMessageText,
  LOCUS_CHAT_MESSAGE_DRAFT_MIME,
  readUserMessageDraftFromClipboardData,
} from "../composables/chatMessageDraft";

describe("chatMessageDraft", () => {
  const userMessage: ChatMessage = {
    id: "user-1",
    role: "user",
    createdAt: 1,
    content: [
      "使用图片向我展示 store",
      "",
      "<locus-references>",
      "Use Unity refs as exact asset anchors. Use project knowledge refs as exact knowledge_read paths.",
      "- asset: {@Assets/UI/Store.prefab}",
      "- project knowledge: `skill/ui.md` (use `knowledge_read`)",
      "</locus-references>",
      "",
      "<locus-local-files>",
      "These are local paths supplied by drag and drop. Read contents only when needed, using `read` for files and `list` for folders.",
      "- file: `E:/cache/store.png`; type: png",
      "</locus-local-files>",
      "",
      "<locus-console>",
      "Use these Unity Console entries as diagnostic context.",
      "",
      "## Entry 1: [Warning] Slow call",
      "Source: unity-console",
      "Chars: 19",
      "",
      "[Warning] Slow call",
      "</locus-console>",
    ].join("\n"),
    images: [{ data: "abc", mimeType: "image/png" }],
    assetRefs: [{
      path: "Assets/Textures/store.png",
      kind: "asset",
      name: "store.png",
      source: "unity",
    }],
    intentMeta: {
      kind: "user_intent_v1",
      mode: "build",
      skills: [{ source: "app", dirName: "view", name: "View" }],
    },
  };

  it("builds a pasteable draft from a user message", () => {
    const draft = buildUserMessageDraft(userMessage);

    expect(draft.text).toBe("使用图片向我展示 store");
    expect(draft.images).toEqual([{ data: "abc", mimeType: "image/png" }]);
    expect(draft.assetRefs.map((ref) => `${ref.kind}:${ref.path}`)).toEqual([
      "asset:Assets/Textures/store.png",
      "asset:Assets/UI/Store.prefab",
      "knowledge:skill/ui.md",
    ]);
    expect(draft.localFiles).toEqual([{
      path: "E:/cache/store.png",
      isDir: false,
      typeLabel: "png",
      source: "message",
    }]);
    expect(draft.consoleTexts).toEqual([{
      title: "[Warning] Slow call",
      source: "unity-console",
      level: "Warning",
      text: "[Warning] Slow call",
    }]);
    expect(draft.intent.skills).toEqual([{ source: "app", dirName: "view", name: "View" }]);
  });

  it("stores user message draft data in the clipboard payload", () => {
    const payload = buildChatMessageClipboardPayload(userMessage);
    const clipboardData = {
      getData(type: string) {
        return type === LOCUS_CHAT_MESSAGE_DRAFT_MIME ? payload.serializedDraft ?? "" : "";
      },
    } as DataTransfer;

    const draft = readUserMessageDraftFromClipboardData(clipboardData);

    expect(payload.text).toBe("使用图片向我展示 store");
    expect(draft?.assetRefs.map((ref) => ref.path)).toContain("skill/ui.md");
    expect(draft?.images).toHaveLength(1);
    expect(draft?.intent.skills[0]?.dirName).toBe("view");
  });

  it("copies visible user text and raw assistant text", () => {
    expect(copyableChatMessageText(userMessage)).toBe("使用图片向我展示 store");
    expect(copyableChatMessageText({
      id: "assistant-1",
      role: "assistant",
      createdAt: 2,
      content: "已处理\n\n- 结果",
    })).toBe("已处理\n\n- 结果");
  });
});
