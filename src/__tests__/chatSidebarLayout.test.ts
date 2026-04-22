import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat sidebar layout", () => {
  it("uses a single right sidebar that stacks todos above file changes", () => {
    const app = read("src/App.vue");
    const sidebar = read("src/components/ChatSidebarPanel.vue");
    const todoPanel = read("src/components/TodoPanel.vue");
    const changesPanel = read("src/components/ChatChangesPanel.vue");
    const settingsState = read("src/composables/useSettingsState.ts");

    expect(app).toContain("<ChatSidebarPanel");
    expect(sidebar).toContain("<TodoPanel");
    expect(sidebar).toContain("<ChatChangesPanel");
    expect(sidebar).toContain("class=\"chat-sidebar-panel\"");
    expect(sidebar).toContain("class=\"chat-sidebar-shell\"");
    expect(sidebar).toContain("chat-sidebar-resize-handle");
    expect(sidebar).toContain("chat-sidebar-section-todo");
    expect(sidebar).toContain("chat-sidebar-section-changes");
    expect(sidebar).toContain("chat-sidebar-close");
    expect(sidebar).toContain("has-both-sections");
    expect(sidebar).toContain("STORAGE_KEY_SIDEBAR_WIDTH = \"locus:chatSidebarWidth\"");
    expect(sidebar).toContain(":show-close=\"false\"");
    expect(sidebar).toContain("onSidebarResizeMouseDown");
    expect(sidebar).toContain("localStorage.setItem(STORAGE_KEY_SIDEBAR_WIDTH");
    expect(sidebar).toContain(".todo-panel.embedded.chat-sidebar-section-todo.closing");
    expect(todoPanel).toContain("embedded?: boolean;");
    expect(todoPanel).toContain("props.embedded ? \"max-height\" : \"width\"");
    expect(changesPanel).toContain("embedded?: boolean;");
    expect(changesPanel).toContain(":class=\"{ embedded: props.embedded }\"");
    expect(settingsState).toContain("localStorage.removeItem(\"locus:chatSidebarWidth\")");
  });

  it("keeps non-user chat surfaces on the assistant background", () => {
    const app = read("src/App.vue");
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const sidebar = read("src/components/ChatSidebarPanel.vue");
    const todoPanel = read("src/components/TodoPanel.vue");
    const changesPanel = read("src/components/ChatChangesPanel.vue");
    const toolCollection = read("src/components/ToolCallCollection.vue");

    expect(app).toContain("--msg-user-bg: var(--bg-color);");
    expect(transcript).toContain(".chat-transcript-scroll.is-session {");
    expect(transcript).toContain("background: var(--msg-assistant-bg);");
    expect(transcript).toContain(".chat-transcript-message.is-session.user {");
    expect(transcript).toContain("background: var(--msg-user-bg);");
    expect(transcript).toContain("border-top: 1px solid var(--msg-divider);");
    expect(transcript).toContain("border-bottom: 1px solid var(--msg-divider);");
    expect(transcript).toContain(".chat-transcript-message.is-session.user + .chat-transcript-message.is-session.assistant {");
    expect(transcript).toContain("border-top: none;");
    expect(transcript).toContain(".chat-transcript-message.is-session.compact-handoff + .chat-transcript-message.is-session.user {");
    expect(transcript).toContain(".chat-transcript-message.is-session.assistant.transient.continuation {");
    expect(transcript).toContain(".chat-transcript-message.is-embedded.transient.continuation {");
    expect(transcript).toContain(".chat-transcript-message.is-session.continuation {");
    expect(transcript).toContain("padding-top: 6px;");
    expect(transcript).toContain(".chat-transcript-message.is-session.assistant.transient.waiting-placeholder {");
    expect(chatView).toContain("background: var(--msg-assistant-bg);");
    expect(sidebar).toContain("background: var(--msg-assistant-bg);");
    expect(todoPanel).toContain("background: var(--msg-assistant-bg);");
    expect(changesPanel).toContain("background: var(--msg-assistant-bg);");
    expect(toolCollection).toContain("var(--msg-assistant-bg)");
  });

  it("animates tool batch collapse upward instead of dropping the list abruptly", () => {
    const toolCollection = read("src/components/ToolCallCollection.vue");

    expect(toolCollection).toContain("const panelVisible = ref(false);");
    expect(toolCollection).toContain("const summaryOpen = computed(() =>");
    expect(toolCollection).toContain("height 220ms cubic-bezier(0.2, 0, 0, 1)");
    expect(toolCollection).toContain("transformOrigin = \"top center\"");
    expect(toolCollection).toContain("<Transition");
    expect(toolCollection).toContain(":css=\"false\"");
    expect(toolCollection).toContain("@leave=\"onPanelLeave\"");
    expect(toolCollection).toContain("translateY(-4px) scaleY(0.97)");
    expect(toolCollection).toContain("class=\"tool-call-collection-panel\"");
  });

  it("keeps tool batches expanded during streaming and releases them when the round ends", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const toolBlock = read("src/components/ToolCallBlock.vue");
    const toolCollection = read("src/components/ToolCallCollection.vue");

    expect(chatView).toContain("const recentlyCompletedToolMessageId = ref<string | null>(null);");
    expect(chatView).toContain("findTrailingAssistantToolMessageId(props.messages)");
    expect(chatView).toContain("props.isStreaming");
    expect(chatView).toContain("if (!isStreaming) {");
    expect(chatView).toContain("recentlyCompletedToolMessageId.value = null;");
    expect(chatView).toContain(":pinned-tool-message-id=\"recentlyCompletedToolMessageId\"");
    expect(transcript).toContain("const collapseCompletedToolCalls = computed(() => !props.isStreaming);");
    expect(transcript).toContain("const nonCollapsibleToolItemIds = computed(() =>");
    expect(transcript).toContain("ids.add(props.pinnedToolMessageId);");
    expect(transcript).toContain(":allow-collapse=\"!nonCollapsibleToolItemIds.has(item.id)\"");
    expect(transcript).toContain(":collapse-enabled=\"collapseCompletedToolCalls\"");
    expect(transcript).toContain(":collapse-enabled=\"false\"");
    expect(toolBlock).toContain("collapseEnabled?: boolean;");
    expect(toolBlock).toContain("<ToolCallCollection :tool-calls=\"toolCall.nestedToolCalls\" :collapse-enabled=\"collapseEnabled\">");
    expect(toolBlock).toContain("<ToolCallBlock :tool-call=\"nestedToolCall\" :collapse-enabled=\"collapseEnabled\" />");
    expect(toolCollection).toContain("collapseEnabled?: boolean;");
    expect(toolCollection).toContain("props.allowCollapse && props.collapseEnabled");
  });
});
