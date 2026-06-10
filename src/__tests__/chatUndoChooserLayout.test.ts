import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat undo chooser", () => {
  it("defaults to file undo when available and supports keyboard selection", () => {
    const chatView = read("src/components/ChatView.vue");

    expect(chatView).toContain('type UndoChoice = "conversation" | "files";');
    expect(chatView).toContain('const selectedUndoChoice = ref<UndoChoice>("conversation");');
    expect(chatView).toContain('return canUndoFilesAndConversation.value ? "files" : "conversation";');
    expect(chatView).toContain("selectedUndoChoice.value = defaultUndoChoice();");
    expect(chatView).toContain('ref="undoChooserRef"');
    expect(chatView).toContain('tabindex="-1"');
    expect(chatView).toContain('@keydown="handleUndoChooserKeydown"');
    expect(chatView).toContain('if (event.key === "ArrowDown") {');
    expect(chatView).toContain("moveUndoChoice(1);");
    expect(chatView).toContain('if (event.key === "ArrowUp") {');
    expect(chatView).toContain("moveUndoChoice(-1);");
    expect(chatView).toContain('if (event.key === "Enter") {');
    expect(chatView).toContain("runSelectedUndoChoice();");
    expect(chatView).toContain(':class="{ \'is-selected\': selectedUndoChoice === \'files\' }"');
    expect(chatView).toContain(".undo-chooser-action.is-selected:not(:disabled)");
  });

  it("routes message context menu actions through exact message rollback and fork", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const richChatInput = read("src/components/chat/RichChatInput.vue");
    const sessionService = read("src/services/session.ts");
    const undoService = read("src/services/undo.ts");

    expect(transcript).toContain('data-chat-message-id');
    expect(chatView).toContain('e.target.closest("[data-chat-message-id]")');
    expect(chatView).toContain("messageCtxMenu");
    expect(chatView).toContain("contextSelectedMessageId");
    expect(chatView).toContain(":selected-message-id=\"contextSelectedMessageId\"");
    expect(chatView).toContain("rollbackTargetForMessage");
    expect(chatView).toContain("props.messages.slice(messageIndex + 1)");
    expect(chatView).toContain("userMessage: null");
    expect(chatView).toContain("chatStore.rollbackToMessage(targetMessageId");
    expect(chatView).toContain("chatStore.forkSessionFromMessage(messageId)");
    expect(chatView).toContain("messageContextShouldShowReEdit");
    expect(chatView).toContain("isLastUserMessageWithoutAssistantAfter");
    expect(chatView).toContain("lastRenderableMessage()?.id !== message.id");
    expect(chatView).toContain("&& !props.isStreaming");
    expect(chatView).toContain("chatStore.undoLatestConversationTurn()");
    expect(chatView).toContain("uiStore.stageChatDraftPrefill(draft)");
    expect(chatView).toContain("prefill.sessionId !== undefined");
    expect(chatView).toContain("prefill.requireEmptyComposer");
    expect(chatView).toContain("composerPanelRef.value?.isDraftEmpty()");
    expect(chatView).toContain("message.assetRefs && message.assetRefs.length > 0");
    expect(richChatInput).toContain("function isDraftEmpty()");
    expect(richChatInput).toContain("isDraftEmpty,");
    expect(chatView).toContain('v-if="messageContextShouldShowReEdit"');
    expect(chatView).toContain('t("chat.messageMenu.reEditUserMessage")');
    expect(transcript).toContain("selectedMessageId?: string | null");
    expect(transcript).toContain("isContextSelectedMessage");
    expect(transcript).toContain("isContextSelectedAssistantGroup");
    expect(transcript).toContain("data-chat-message-group-end-id");
    expect(chatView).toContain("chatMessageGroupEndId");
    expect(transcript).toContain("historyRenderSegmentsForGroup(group)");
    expect(transcript).toContain("v-for=\"segment in historyRenderSegmentsForGroup(group)\"");
    expect(transcript).not.toContain("historyRenderMessageBlocksForGroup");
    expect(transcript).toContain("'is-context-selected': isContextSelectedAssistantGroup(group)");
    expect(transcript).toContain("'is-context-selected'");
    expect(transcript).not.toContain("'is-context-selected': isContextSelectedMessage(block.itemId)");
    expect(transcript).not.toContain("'is-context-selected': isContextSelectedMessage(segment.itemId)");
    expect(sessionService).toContain('ipcInvoke<string>("fork_session_from_message"');
    expect(sessionService).toContain('ipcInvoke<SessionDetail>("rollback_session_to_message"');
    expect(undoService).toContain('ipcInvoke("undo_perform_to_message"');
  });
});
