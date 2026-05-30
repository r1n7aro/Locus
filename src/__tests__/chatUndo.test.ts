import { describe, expect, it } from "vitest";
import { findUndoRestoreUserMessage, findUndoRestoreUserText } from "../services/chatUndo";

describe("findUndoRestoreUserText", () => {
  it("returns the nearest preceding user message for the undone assistant round", () => {
    const messages = [
      { id: "user-1", role: "user" as const, content: "第一轮", createdAt: 1 },
      { id: "assistant-1", role: "assistant" as const, content: "收到", createdAt: 2 },
      {
        id: "user-2",
        role: "user" as const,
        content: "把这个脚本改成异步",
        createdAt: 3,
        images: [{ data: "abc", mimeType: "image/png" }],
        assetRefs: [{ path: "Assets/Foo.prefab", kind: "asset" as const }],
        intentMeta: {
          kind: "user_intent_v1" as const,
          mode: "build" as const,
          skills: [{ source: "app" as const, dirName: "view", name: "View" }],
        },
      },
      { id: "assistant-2", role: "assistant" as const, content: "已修改", createdAt: 4 },
      { id: "tool-1", role: "tool" as const, content: "done", createdAt: 5 },
    ];

    const restoreText = findUndoRestoreUserText(
      messages,
      "assistant-2",
    );
    const restoreMessage = findUndoRestoreUserMessage(messages, "assistant-2");

    expect(restoreText).toBe("把这个脚本改成异步");
    expect(restoreMessage?.images).toHaveLength(1);
    expect(restoreMessage?.assetRefs?.[0]?.path).toBe("Assets/Foo.prefab");
    expect(restoreMessage?.intentMeta?.skills[0]?.dirName).toBe("view");
  });

  it("returns null when the target assistant message does not exist", () => {
    const restoreText = findUndoRestoreUserText(
      [
        { id: "user-1", role: "user", content: "第一轮", createdAt: 1 },
        { id: "assistant-1", role: "assistant", content: "收到", createdAt: 2 },
      ],
      "assistant-missing",
    );

    expect(restoreText).toBeNull();
  });
});
