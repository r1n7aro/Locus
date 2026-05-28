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
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(chatView).toContain("function isPendingSessionRestoreAwaitingMessages()");
    expect(chatView).toContain("if (isPendingSessionRestoreAwaitingMessages()) return;");
    expect(chatView).toContain("function finishPendingSessionRestore(targetSessionId: string)");
    expect(chatView).toContain("const shouldRestoreImmediately = !!nextSessionId && previousSessionId === null && !showWelcomeState.value;");
    expect(chatView).toContain("scrollToBottomScheduler.cancel();");
    expect(chatView).toContain("pendingRestoreMessagesRef.value = nextSessionId && !shouldRestoreImmediately ? props.messages : null;");
    expect(chatView).toContain("if (shouldRestoreImmediately) {");
    expect(chatView).toContain("restorePendingSessionScroll({ defer: true });");
    expect(chatView).toContain("restorePendingSessionScroll();");
    expect(chatView).toContain("scheduleSessionRestoreFollowup(targetSessionId, remembered);");
    expect(chatView).toContain("function resolvePendingSessionRestoreState(state: SessionScrollState | null)");
    expect(chatView).toContain("shouldRestoreBottomFromTopAnchorState(");
    expect(chatView).toContain('traceSessionScroll("pending-restore:top-anchor-to-bottom"');
    expect(chatView).toContain("function traceSessionScroll(stage: string, detail: Record<string, unknown> = {})");
    expect(chatView).toContain("[Locus chat-scroll]");
    expect(chatView).toContain('traceSessionScroll("messages-length-changed"');
    expect(chatView).toContain('{ flush: "post" },');
    expect(chatView).toContain("const sessionRestoreLayoutStabilizing = ref(false);");
    expect(chatView).toContain("beginSessionRestoreLayoutStabilization(\"session-id-change\");");
    expect(chatView).toContain("finishSessionRestoreLayoutStabilization(\"session-restore-followup\",");
    expect(chatView).toContain("function isSessionRestoreViewportGuardActive()");
    expect(chatView).toContain("const sessionRestoreViewportGuarding = ref(false);");
    expect(chatView).toContain("sessionRestoreViewportGuarding.value = true;");
    expect(chatView).toContain("function finishSessionRestoreLayoutStabilization(");
    expect(chatView).toContain("restoreAfterLayoutClassSettled(\"layout-stabilize:final-restore\");");
    expect(chatView).toContain("restoreAfterLayoutClassSettled(\"layout-stabilize:final-restore-settled\");");
    expect(chatView).toContain("traceSessionScroll(\"layout-stabilize:guard-end\"");
    expect(chatView).toContain("traceSessionScroll(\"scroll-capture:skip-session-restore\"");
    expect(chatView).toContain("traceSessionScroll(\"tool-anchor:skip-session-restore\"");
    expect(chatView).toContain("traceSessionScroll(\"resize-reconcile:skip-session-restore-stabilizing\"");
    expect(chatView).toContain("traceSessionScroll(\"reconcile:skip-session-restore-stabilizing\"");
    expect(chatView).toContain("traceSessionScroll(\"preserve-anchor:skip-session-restore\"");
    expect(chatView).toContain(":class=\"{ 'is-session-restore-stabilizing': sessionRestoreLayoutStabilizing }\"");
    expect(transcript).toContain(".chat-transcript-scroll.is-session.is-session-restore-stabilizing .chat-transcript-message.is-session");
    expect(transcript).toContain("content-visibility: visible;");
    expect(chatView).not.toContain("chat-transcript-restoring");
    expect(chatView).not.toContain("visibility: hidden;");
  });
});
