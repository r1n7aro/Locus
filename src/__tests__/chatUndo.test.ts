import { describe, expect, it } from "vitest";
import { findUndoRestoreUserText } from "../services/chatUndo";

describe("findUndoRestoreUserText", () => {
  it("returns the nearest preceding user message for the undone assistant round", () => {
    const restoreText = findUndoRestoreUserText(
      [
        { id: "user-1", role: "user", content: "第一轮", createdAt: 1 },
        { id: "assistant-1", role: "assistant", content: "收到", createdAt: 2 },
        { id: "user-2", role: "user", content: "把这个脚本改成异步", createdAt: 3 },
        { id: "assistant-2", role: "assistant", content: "已修改", createdAt: 4 },
        { id: "tool-1", role: "tool", content: "done", createdAt: 5 },
      ],
      "assistant-2",
    );

    expect(restoreText).toBe("把这个脚本改成异步");
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
