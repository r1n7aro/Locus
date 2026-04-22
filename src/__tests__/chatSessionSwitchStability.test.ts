import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat session switch stability", () => {
  it("hides the transcript until session scroll restoration completes", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain("const isRestoringSessionView = ref(false)");
    expect(chatView).toContain("let sessionRestoreRevealFrame = 0;");
    expect(chatView).toContain("clearSessionRestoreRevealFrame()");
    expect(chatView).toContain("const shouldRestoreImmediately = !!nextSessionId && previousSessionId === null && !showWelcomeState.value;");
    expect(chatView).toContain("isRestoringSessionView.value = !!nextSessionId && !shouldRestoreImmediately;");
    expect(chatView).toContain("if (shouldRestoreImmediately) {");
    expect(chatView).toContain("restorePendingSessionScroll();");
    expect(chatView).toContain(":class=\"{ 'chat-transcript-restoring': isRestoringSessionView }\"");
    expect(chatView).toContain(".chat-transcript-scroll.chat-transcript-restoring");
    expect(chatView).toContain("visibility: hidden;");
  });
});
