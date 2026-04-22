import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat pending panel wheel passthrough", () => {
  it("forwards wheel input from question and approval cards back to the transcript scroller", () => {
    const helper = read("src/composables/chatWheelPassthrough.ts");
    const chatView = read("src/components/ChatView.vue");
    const embeddedPane = read("src/components/chat/EmbeddedChatPane.vue");

    expect(helper).toContain("export function forwardWheelToElement");
    expect(helper).toContain("findScrollableAncestorWithin");
    expect(helper).toContain("event.preventDefault()");

    expect(chatView).toContain('class="chat-pending-stack"');
    expect(chatView).toContain('@wheel="handleBottomPanelWheel"');
    expect(chatView).toContain("forwardWheelToElement(event, getMessagesElement())");

    expect(embeddedPane).toContain('class="embedded-chat-panels"');
    expect(embeddedPane).toContain('@wheel="handleBottomPanelWheel"');
    expect(embeddedPane).toContain("forwardWheelToElement(event, getTranscriptElement())");
  });
});
