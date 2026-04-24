import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat session switch stability", () => {
  it("keeps the transcript visible while waiting for the target session messages", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain("function isPendingSessionRestoreAwaitingMessages()");
    expect(chatView).toContain("if (isPendingSessionRestoreAwaitingMessages()) return;");
    expect(chatView).toContain("function finishPendingSessionRestore(targetSessionId: string)");
    expect(chatView).toContain("const shouldRestoreImmediately = !!nextSessionId && previousSessionId === null && !showWelcomeState.value;");
    expect(chatView).toContain("scrollToBottomScheduler.cancel();");
    expect(chatView).toContain("pendingRestoreMessagesRef.value = nextSessionId && !shouldRestoreImmediately ? props.messages : null;");
    expect(chatView).toContain("if (shouldRestoreImmediately) {");
    expect(chatView).toContain("restorePendingSessionScroll({ defer: true });");
    expect(chatView).toContain("restorePendingSessionScroll();");
    expect(chatView).not.toContain("chat-transcript-restoring");
    expect(chatView).not.toContain("visibility: hidden;");
  });
});
