import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const richInput = readFileSync(
  resolve(process.cwd(), "src/components/chat/RichChatInput.vue"),
  "utf8",
);

describe("chat input arrow keys", () => {
  it("leaves ArrowUp and ArrowDown to the textarea outside open suggestion popups", () => {
    expect(richInput).not.toContain("tryNavigateMessageHistory");
    expect(richInput).not.toContain("navigateChatInputHistory");
    expect(richInput).not.toContain("messageHistory?: ChatMessage[];");
  });
});
