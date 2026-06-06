import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("EmbeddedChatPane contract", () => {
  it("exposes reusable action slots and shared interaction cards", () => {
    const inputShell = read("src/components/chat/ChatInputShell.vue");
    const richInput = read("src/components/chat/RichChatInput.vue");
    const pane = read("src/components/chat/EmbeddedChatPane.vue");
    const knowledgePane = read("src/components/knowledge/KnowledgeChatPane.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const chatView = read("src/components/ChatView.vue");
    const embeddedSession = read("src/composables/useEmbeddedChatSession.ts");

    expect(inputShell).toContain('<slot name="top-start" />');
    expect(inputShell).toContain('<slot name="top-end" />');
    expect(inputShell).toContain('<slot name="floating" />');
    expect(inputShell).toContain('<slot name="before-composer" />');
    expect(inputShell).toContain('<slot name="footer" />');
    expect(richInput).toContain("<ChatInputShell>");
    expect(richInput).toContain("<ChatComposer");
    expect(richInput).toContain("<MentionPopup");
    expect(richInput).toContain("useCommandRegistry");
    expect(richInput).toContain("listDirEntriesPage");
    expect(richInput).toContain("searchWorkspaceAssets");
    expect(richInput).toContain("insertInlineMention");
    expect(pane).toContain('<slot name="header-actions" />');
    expect(pane).toContain('<slot name="composer-start" />');
    expect(pane).toContain('<slot name="composer-actions" />');
    expect(pane).toContain("<RichChatInput");
    expect(pane).toContain("<ChatTranscript");
    expect(pane).toContain("const viewportStates = new Map<string, SessionScrollState>()");
    expect(pane).toContain("const toolHandoffViewportQuiet = ref(false);");
    expect(pane).toContain("watch(() => props.activeToolCalls, () => reconcileViewport(), { deep: true });");
    expect(pane).toContain("@tool-handoff-quiet-change=\"handleToolHandoffQuietChange\"");
    expect(pane).toContain("@scroll=\"handleTranscriptScroll\"");
    expect(pane).toContain("@user-scroll-intent=\"markTranscriptUserScrollIntent\"");
    expect(pane).toContain(":session-key=\"getViewportStateKey()\"");
    expect(embeddedSession).toContain("buildToolResultMessages(sourceToolCalls)");
    expect(embeddedSession).toContain("function replaceMessageById");
    expect(embeddedSession).toContain("state.messages = replaceMessageById(state.messages, mutation.message)");
    expect(embeddedSession).toContain("async function reloadSessionMessagesAfterError");
    expect(embeddedSession).toContain("sessionService.loadSession(sessionId)");
    expect(embeddedSession).toContain("hydrateChatMessagesIntent(detail.messages)");
    expect(embeddedSession).toContain("sessionService.queueChatInput");
    expect(embeddedSession).toContain("sessionService.deletePendingChatInput");
    expect(pane).toContain("<AskUserCard");
    expect(pane).toContain("<ToolConfirmCard");
    expect(pane).toContain('queuedFollowUp?: { displayText: string; canInsert?: boolean; isInserting?: boolean } | null;');
    expect(pane).toContain('@click="emit(\'insertQueuedFollowUp\')"');
    expect(pane).toContain('@click="emit(\'deleteQueuedFollowUp\')"');
    expect(pane).toContain('class="embedded-queued-follow-up"');
    expect(pane).toContain('class="embedded-chat-pane"');
    expect(knowledgePane).toContain("<AgentSelector");
    expect(knowledgePane).toContain("<ModelEffortSelector");
    expect(knowledgePane).toContain("<template #composer-start>");
    expect(knowledgePane).toContain('agent.id === "knowledge"');
    expect(knowledgePane).toContain("show-user-images");
    expect(knowledgePane).toContain('user-content-mode="asset"');
    expect(knowledgePane).toContain(':tool-confirm-layout-key="sessionKey"');
    expect(knowledgePane).toContain(':queued-follow-up="queuedFollowUp"');
    expect(knowledgePane).toContain('@delete-queued-follow-up="deleteQueuedFollowUp"');
    expect(knowledgePane).toContain(":waiting-label=\"t('chat.transcript.waiting')\"");
    expect(chatView).toContain("<RichChatInput");
    expect(chatView).toContain("<ChatTranscript");
    expect(transcript).toContain("const isWaitingForResponse = computed(");
    expect(transcript).toContain("shouldShowAssistantContinuation");
  });
});
