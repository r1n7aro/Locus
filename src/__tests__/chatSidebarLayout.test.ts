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

    expect(app).toContain("--msg-user-bg: #eff0f1;");
    expect(app).toContain("--msg-user-bg: #212125;");
    expect(transcript).toContain(".chat-transcript-scroll.is-session {");
    expect(transcript).toContain("background: var(--msg-assistant-bg);");
    expect(transcript).toContain(".chat-transcript-message.is-session.user {");
    expect(transcript).toContain("background: var(--msg-assistant-bg);");
    expect(transcript).toContain("function shouldShowSessionRoundDivider(group: Pick<MessageGroup, \"role\">, index: number) {");
    expect(transcript).toContain("'has-round-divider': shouldShowSessionRoundDivider(group, idx),");
    expect(transcript).toContain(".chat-transcript-message.is-session.has-round-divider {");
    expect(transcript).toContain("border-top: 2px solid var(--msg-divider);");
    expect(transcript).toContain(".chat-transcript-footer.is-session {");
    expect(transcript).toContain(".chat-transcript-message.is-session.user .chat-transcript-plain-text {");
    expect(transcript).toContain("background: var(--msg-user-bg);");
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
    expect(toolCollection).toContain("height 320ms cubic-bezier(0.2, 0, 0, 1)");
    expect(toolCollection).toContain("transformOrigin = \"top center\"");
    expect(toolCollection).toContain("<Transition");
    expect(toolCollection).toContain(":css=\"false\"");
    expect(toolCollection).toContain("@leave=\"onPanelLeave\"");
    expect(toolCollection).toContain("emit(\"collapseFinished\");");
    expect(toolCollection).toContain("translateY(-4px) scaleY(0.97)");
    expect(toolCollection).toContain("class=\"tool-call-collection-panel\"");
  });

  it("keeps tool batches on a shared transient handoff path until the collapse leave finishes", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const toolBlock = read("src/components/ToolCallBlock.vue");
    const toolCollection = read("src/components/ToolCallCollection.vue");

    expect(transcript).toContain("interface ToolCallHandoffState {");
    expect(transcript).toContain("const TOOL_HANDOFF_MIN_VISIBLE_MS = 160;");
    expect(transcript).toContain("const hasVisibleStreamingText = computed(() => props.streamingText.trim().length > 0);");
    expect(transcript).toContain("const shouldArmToolCallHandoffCollapse = computed(");
    expect(transcript).toContain("const toolCallHandoff = ref<ToolCallHandoffState | null>(null);");
    expect(transcript).toContain("renderKey: `tool-handoff-");
    expect(transcript).toContain("collapseFinished: boolean;");
    expect(transcript).toContain("willAutoCollapse: summarizeToolCallBatch(toolCalls, displaySettings.compactToolCalls).canCollapse");
    expect(transcript).toContain("setToolCallHandoffQuiet(true);");
    expect(transcript).toContain("if (!transientToolCallsCanCollapse.value) {");
    expect(transcript).toContain("if (shouldArmToolCallHandoffCollapse.value) {");
    expect(transcript).toContain("watch(shouldArmToolCallHandoffCollapse, (shouldArm) => {");
    expect(transcript).toContain("clearToolCallHandoff(\"stream-ended-after-collapse\")");
    expect(transcript).toContain("const promotableHistoryToolCalls = computed<PromotedHistoryToolCallsState>(() => {");
    expect(transcript).toContain("const transientCollapseCandidateToolCalls = computed(() => {");
    expect(transcript).toContain("const transientToolCallsCanCollapse = computed(() =>");
    expect(transcript).toContain("@collapse-finished=\"onTransientToolCallsCollapseFinished\"");
    expect(transcript).toContain(":tool-calls=\"transientToolCalls\"");
    expect(transcript).toContain(":allow-collapse=\"transientToolCallsAllowCollapse\"");
    expect(transcript).toContain(":collapse-enabled=\"transientToolCallsCollapseEnabled\"");
    expect(chatView).toContain("const toolHandoffViewportQuiet = ref(false);");
    expect(chatView).toContain("function handleToolHandoffQuietChange(quiet: boolean) {");
    expect(chatView).toContain("@tool-handoff-quiet-change=\"handleToolHandoffQuietChange\"");
    expect(transcript).toContain(":allow-collapse=\"!shouldKeepToolItemExpanded(item.id)\"");
    expect(transcript).toContain(":collapse-enabled=\"!shouldKeepToolItemExpanded(item.id)\"");
    expect(toolBlock).toContain("collapseEnabled?: boolean;");
    expect(toolBlock).toContain(":tool-calls=\"toolCall.nestedToolCalls\"");
    expect(toolBlock).toContain(":collapse-enabled=\"collapseEnabled\"");
    expect(toolBlock).toContain("@viewport-anchor-start=\"emitToolViewportAnchorStart\"");
    expect(toolBlock).toContain("@tool-viewport-anchor-start=\"emitToolViewportAnchorStart\"");
    expect(toolCollection).toContain("collapseEnabled?: boolean;");
    expect(toolCollection).toContain("props.allowCollapse && props.collapseEnabled");
  });

  it("filters history tool calls while the transient handoff batch owns the same ids", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("const hasLiveToolCalls = computed(() => props.activeToolCalls.length > 0);");
    expect(transcript).toContain("const hasToolCallHandoff = computed(() => transientToolCalls.value.length > 0 && !hasLiveToolCalls.value);");
    expect(transcript).toContain("const hasStreamingContent = computed(() => hasVisibleStreamingText.value || hasLiveToolCalls.value);");
    expect(transcript).toContain("const activeToolCallMatchState = computed<ToolCallMatchState>(() => {");
    expect(transcript).toContain("return toolCallHandoff.value?.toolCallMatchState ?? {");
    expect(transcript).toContain("const baseGroupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(activeToolCallMatchState.value));");
    expect(transcript).toContain("const historyHiddenToolCallMatchState = computed<ToolCallMatchState>(() => {");
    expect(transcript).toContain("return mergeToolCallMatchStates(");
    expect(transcript).toContain("const groupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(historyHiddenToolCallMatchState.value));");
    expect(transcript).toContain("toolCallTreeHasAnyIds(message.toolCalls, toolCallHandoff.value!.toolCallMatchState)");
    expect(transcript).toContain("function buildTailHiddenToolCallMap(");
    expect(transcript).toContain("filterToolCallsByConsumableMatchState(");
    expect(transcript).toContain("cloneToolCallMatchState(hiddenToolCallMatchState)");
    expect(chatView).toContain(":session-key=\"activeSessionId || NEW_CHAT_DRAFT_KEY\"");
    expect(transcript).toContain("function shouldKeepToolItemExpanded(itemId: string) {");
    expect(transcript).toContain("return nonCollapsibleToolItemIds.value.has(itemId);");
    expect(transcript).toContain("if (toolCallHandoff.value?.collapseArmed) {");
    expect(transcript).toContain("|| hasToolCallHandoff.value");
    expect(transcript).toContain("collapseFinished: handoff?.collapseFinished ?? false");
    expect(chatView).toContain("toolHandoffViewportQuiet.value = false;");
    expect(chatView).toContain("if (suppressScrollCapture || toolHandoffViewportQuiet.value) return;");
  });

  it("renders the waiting placeholder after the transient tool list", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const waitingIndex = transcript.indexOf("<div v-if=\"isWaitingForResponse\" class=\"chat-transcript-thinking-block\">");
    const toolGroupIndex = transcript.indexOf("<div v-if=\"transientToolCalls.length > 0\" class=\"chat-transcript-tool-calls-group\">");

    expect(toolGroupIndex).toBeGreaterThan(-1);
    expect(waitingIndex).toBeGreaterThan(toolGroupIndex);
  });

  it("tightens spacing between consecutive tool-only assistant rounds", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("function isToolOnlyRenderItem(item: MessageRenderItem) {");
    expect(transcript).toContain("function shouldTightenToolOnlyGap(items: MessageRenderItem[], index: number) {");
    expect(transcript).toContain("'tool-only': isToolOnlyRenderItem(item),");
    expect(transcript).toContain("'tool-only-followup': shouldTightenToolOnlyGap(group.items, itemIdx),");
    expect(transcript).toContain(".chat-transcript-item-stack.is-session.tool-only-followup {");
    expect(transcript).toContain("margin-top: -8px;");
    expect(transcript).toContain(".chat-transcript-item-stack.is-embedded.tool-only-followup {");
    expect(transcript).toContain("margin-top: -6px;");
  });

  it("attaches knowledge proposals only inside their assistant message group", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("for (const group of groups) {");
    expect(transcript).toContain("if (group.role !== \"assistant\") continue;");
    expect(transcript).toContain("const nextRequestTool = group.items.find(");
    expect(transcript).toContain("const prevRequestTool = [...group.items].reverse().find(");
  });
});
