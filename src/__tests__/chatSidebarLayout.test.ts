import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const cwd = process.cwd();

function read(relPath: string) {
  return readFileSync(resolve(cwd, relPath), "utf8");
}

describe("chat sidebar layout", () => {
  it("defaults chat file changes to tree view", () => {
    const changesPanel = read("src/components/ChatChangesPanel.vue");

    expect(changesPanel).toContain("const CHAT_CHANGES_VIEW_MODE_STORAGE_KEY = \"locus.chat.changesViewMode\";");
    expect(changesPanel).toMatch(
      /function readStoredChatChangesViewMode\(\): StagingViewMode \{[\s\S]*if \(raw === "tree"\) return "tree";[\s\S]*if \(raw === "list"\) return "list";[\s\S]*return "tree";[\s\S]*\}/,
    );
  });

  it("locks the chat changes undo action while undo is running", () => {
    const changesPanel = read("src/components/ChatChangesPanel.vue");
    const zh = read("src/language/zh.json");
    const en = read("src/language/en.json");

    expect(changesPanel).toContain("const isUndoing = ref(false);");
    expect(changesPanel).toContain("const undoButtonBusy = computed(() => checkingUndoConflicts.value || isUndoing.value);");
    expect(changesPanel).toContain("if (isUndoing.value) return t(\"chat.changes.undoing\");");
    expect(changesPanel).toContain("if (!targetId || isUndoing.value) return;");
    expect(changesPanel).toContain("isUndoing.value = true;");
    expect(changesPanel).toContain("isUndoing.value = false;");
    expect(changesPanel).toContain("if (isUndoing.value) return;");
    expect(changesPanel).toContain(":disabled=\"undoButtonBusy\"");
    expect(changesPanel).toContain("{{ undoButtonLabel }}");
    expect(changesPanel).toContain(":disabled=\"isUndoing\" @click=\"cancelUndo\"");
    expect(changesPanel).toContain("{{ isUndoing ? t('chat.changes.undoing') : t('chat.changes.confirmOk') }}");
    expect(changesPanel).toContain("{{ isUndoing ? t('chat.changes.undoing') : t('chat.changes.undoConflictForce') }}");
    expect(zh).toContain("\"chat.changes.undoing\": \"正在撤销中\"");
    expect(en).toContain("\"chat.changes.undoing\": \"Undoing\"");
  });

  it("uses a single right sidebar that stacks todos above file changes", () => {
    const workspace = read("src/components/ChatWorkspaceView.vue");
    const sidebar = read("src/components/ChatSidebarPanel.vue");
    const todoPanel = read("src/components/TodoPanel.vue");
    const changesPanel = read("src/components/ChatChangesPanel.vue");
    const settingsState = read("src/composables/useSettingsState.ts");

    expect(workspace).toContain(":css=\"false\"");
    expect(workspace).toContain("@before-enter=\"beforeEnterSidebarPanel\"");
    expect(workspace).toContain("@enter=\"enterSidebarPanel\"");
    expect(workspace).toContain("@after-enter=\"afterEnterSidebarPanel\"");
    expect(workspace).toContain("@before-leave=\"beforeLeaveSidebarPanel\"");
    expect(workspace).toContain("@leave=\"leaveSidebarPanel\"");
    expect(workspace).toContain("SIDEBAR_ENTER_TRANSITION_MS");
    expect(workspace).toContain("<ChatSidebarPanel");
    expect(workspace).toContain(":layout=\"isVerticalLayout ? 'bottom' : 'side'\"");
    expect(workspace).toContain("shell.style.width = \"0px\";");
    expect(workspace).toContain("shell.style.minWidth = \"0px\";");
    expect(workspace).toContain('shell.style.transform = "translateX(100%)";');
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
    expect(sidebar).toContain("STORAGE_KEY_SIDEBAR_HEIGHT = \"locus:chatSidebarHeight\"");
    expect(sidebar).toContain("storageScope?: string;");
    expect(sidebar).toContain("scopedSidebarStorageKey");
    expect(sidebar).toContain("effectiveMaxSideWidth");
    expect(sidebar).toContain(":show-close=\"false\"");
    expect(sidebar).toContain("onSidebarResizeMouseDown");
    expect(sidebar).toContain("localStorage.setItem(sidebarWidthStorageKey.value");
    expect(sidebar).toContain("localStorage.setItem(sidebarHeightStorageKey.value");
    expect(sidebar).toContain("layout-bottom");
    expect(sidebar).toContain(".todo-panel.embedded.chat-sidebar-section-todo.closing");
    expect(todoPanel).toContain("embedded?: boolean;");
    expect(todoPanel).toContain("props.embedded ? \"max-height\" : \"width\"");
    expect(changesPanel).toContain("embedded?: boolean;");
    expect(changesPanel).toContain(":class=\"{ embedded: props.embedded }\"");
    expect(settingsState).toContain("localStorage.removeItem(\"locus:chatSidebarWidth\")");
    expect(settingsState).toContain("localStorage.removeItem(\"locus:chatSidebarHeight\")");
    expect(settingsState).toContain("localStorage.removeItem(\"locus:unity:chatSidebarWidth\")");
    expect(settingsState).toContain("localStorage.removeItem(\"locus:unity:chatSidebarHeight\")");
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
    expect(transcript).toContain("--chat-transcript-session-bottom-gap: 40px;");
    expect(transcript).toContain(".chat-transcript-scroll.is-session > .chat-transcript-content {");
    expect(transcript).toContain("padding-bottom: var(--chat-transcript-session-bottom-gap);");
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
    expect(transcript).toContain("--chat-transcript-session-segment-gap: 10px;");
    expect(transcript).toContain("padding-top: var(--chat-transcript-session-segment-gap);");
    expect(transcript).toContain("gap: var(--chat-transcript-session-segment-gap);");
    expect(transcript).not.toContain("padding-top: 6px;");
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
    expect(toolCollection).toContain("const panelLeaving = ref(false);");
    expect(toolCollection).toContain("const autoCollapseFloatingSummary = ref(startsExpandedForCollapseAnimation);");
    expect(toolCollection).toContain("const floatingSummaryHeight = ref(30);");
    expect(toolCollection).toContain("const summaryOpen = computed(() =>");
    expect(toolCollection).toContain("const floatingSummary = computed(() =>");
    expect(toolCollection).toContain("const floatingSummaryStyle = computed(() =>");
    expect(toolCollection).toContain("height 320ms cubic-bezier(0.2, 0, 0, 1)");
    expect(toolCollection).toContain("transformOrigin = \"top center\"");
    expect(toolCollection).toContain("function collapsedSummaryHeight()");
    expect(toolCollection).toContain("function updateFloatingSummaryHeight()");
    expect(toolCollection).toContain("function emitViewportAnchorStart()");
    expect(toolCollection).toContain("<Transition");
    expect(toolCollection).toContain(":css=\"false\"");
    expect(toolCollection).toContain("@leave=\"onPanelLeave\"");
    expect(toolCollection).toContain("const targetHeight = autoCollapseFloatingSummary.value ? updateFloatingSummaryHeight() : 0;");
    expect(toolCollection).toContain("emit(\"collapseFinished\");");
    expect(toolCollection).toContain("emitViewportAnchorStart();");
    expect(toolCollection).toContain("translateY(-4px) scaleY(0.97)");
    expect(toolCollection).toContain("class=\"tool-call-collection-panel\"");
    expect(toolCollection).toContain(":style=\"floatingSummaryStyle\"");
    expect(toolCollection).toContain("'is-collapsing': batchState.canCollapse && panelLeaving");
    expect(toolCollection).toContain("'has-floating-summary': floatingSummary");
    expect(toolCollection).toContain(":class=\"{ open: expanded }\"");
    expect(toolCollection).toContain(".tool-call-batch-summary.open.closing");
    expect(toolCollection).toContain(".tool-call-collection.has-floating-summary {");
    expect(toolCollection).toContain("min-height: var(--tool-call-summary-height, 30px);");
    expect(toolCollection).toContain(".tool-call-collection.has-floating-summary .tool-call-batch-summary");
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
    expect(transcript).toContain("collapseCandidateToolCalls: ToolCallDisplay[];");
    expect(transcript).toContain("collapseFinished: boolean;");
    expect(transcript).toContain("function shouldRetainCollapsedToolCallHandoff(handoff: ToolCallHandoffState)");
    expect(transcript).toContain("return handoff.collapseFinished || (handoff.collapseArmed && handoff.willAutoCollapse);");
    expect(transcript).toContain("collectToolCallDisplayMatchState(retainedToolCalls)");
    expect(transcript).toContain("collapseCandidateToolCalls: cloneToolCallDisplays(transientCollapseCandidateToolCalls.value)");
    expect(transcript).toContain("willAutoCollapse: summarizeToolCallBatch(toolCalls, displaySettings.compactToolCalls).canCollapse");
    expect(transcript).toContain("setToolCallHandoffQuiet(true);");
    expect(transcript).toContain("if (!transientToolCallsCanCollapse.value) {");
    expect(transcript).toContain("if (shouldArmToolCallHandoffCollapse.value) {");
    expect(transcript).toContain("watch(shouldArmToolCallHandoffCollapse, (shouldArm) => {");
    expect(transcript).toContain("clearToolCallHandoff(\"stream-ended-after-collapse\")");
    expect(transcript).toContain("const shouldPromoteHistoryToolCalls = computed(");
    expect(transcript).toContain("props.activeToolCalls.length > 0");
    expect(transcript).toContain("const promotableHistoryToolCalls = computed<PromotedHistoryToolCallsState>(() => {");
    expect(transcript).toContain("const segments = historyRenderSegmentsForGroup(lastGroup);");
    expect(transcript).toContain("if (!segment || segment.type !== \"toolCalls\") break;");
    expect(transcript).toContain("const transientCollapseCandidateToolCalls = computed(() => {");
    expect(transcript).toContain("if (promotableHistoryToolCalls.value.toolCalls.length === 0) {");
    expect(transcript).toContain("const transientToolCallsCanCollapse = computed(() =>");
    expect(transcript).toContain("const shouldKeepPromotedHistoryToolCallsInTransient = computed(() =>");
    expect(transcript).toContain("|| (!!toolCallHandoff.value && transientToolCallsCanCollapse.value)");
    expect(transcript).toContain("const shouldHidePromotedHistoryToolCalls = computed(() =>");
    expect(transcript).toContain("&& shouldKeepPromotedHistoryToolCallsInTransient.value");
    expect(transcript).toContain("promotedHistoryToolCallsVisibilityChanged");
    expect(transcript).toContain("keepPromotedInTransient: shouldKeepPromotedHistoryToolCallsInTransient.value");
    expect(transcript).toContain("const shouldRenderPromotedHistoryToolCallsInTransient = computed(() =>");
    expect(transcript).toContain("transientPromotedToolCallsCoverage");
    expect(transcript).toContain("promotedHistoryToolCallsRenderGap");
    expect(transcript).toContain("missingPromotedToolCallIds");
    expect(transcript).toContain("@collapse-finished=\"onTransientToolCallsCollapseFinished\"");
    expect(transcript).toContain(":tool-calls=\"segment.toolCalls\"");
    expect(transcript).toContain("function transientToolHandoffPart(toolCalls: ToolCallDisplay[])");
    expect(transcript).toContain("let hasRenderedToolSegment = segments.some((segment) => segment.type === \"toolCalls\");");
    expect(transcript).toContain("const promotedToolCalls = promotableHistoryToolCalls.value.toolCalls;");
    expect(transcript).toContain("const firstToolSegmentIndex = segments.findIndex((segment) => segment.type === \"toolCalls\");");
    expect(transcript).toContain("const firstContentSegmentIndex = segments.findIndex((segment) => segment.type === \"content\");");
    expect(transcript).toContain("const firstToolPrecedesContent =");
    expect(transcript).toContain("const shouldCollapsePromotedPrefix =");
    expect(transcript).toContain("|| firstContentSegmentIndex >= 0;");
    expect(transcript).toContain("mergeToolCallDisplaysWithoutDuplicates(");
    expect(transcript).toContain("firstToolSegment.key = transientToolSegmentKey(mergedToolCalls);");
    expect(transcript).toContain("firstToolSegment.allowCollapse = true;");
    expect(transcript).toContain("firstToolSegment.collapseEnabled = shouldCollapsePromotedPrefix;");
    expect(transcript).toContain("segments.unshift({");
    expect(transcript).toContain("allowCollapse: true,");
    expect(transcript).toContain("collapseEnabled: shouldCollapsePromotedPrefix,");
    expect(transcript).toContain("hasRenderedToolSegment = segments.some((segment) => segment.type === \"toolCalls\");");
    expect(transcript).toContain("if (!hasRenderedToolSegment && transientToolCalls.value.length > 0) {");
    expect(transcript).toContain("function transientToolSegmentKey(toolCalls: ToolCallDisplay[])");
    expect(transcript).toContain("key: transientToolSegmentKey(pendingToolCalls),");
    expect(transcript).toContain("key: transientToolSegmentKey(transientToolCalls.value),");
    expect(transcript).toContain("animateCollapseOnMount: !!toolCallHandoff.value?.collapseArmed,");
    expect(transcript).toContain(":animate-collapse-on-mount=\"segment.animateCollapseOnMount\"");
    expect(transcript).toContain("if (!props.isStreaming && shouldReleaseToolCallHandoffToHistory(props.messages, previousMatchState))");
    expect(transcript).toContain("if (!props.isStreaming && shouldReleaseToolCallHandoffToHistory(messages, toolCallHandoff.value.toolCallMatchState))");
    expect(transcript).toContain(":allow-collapse=\"segment.allowCollapse\"");
    expect(transcript).toContain(":collapse-enabled=\"segment.collapseEnabled\"");
    expect(transcript).toContain(":collapse-enabled=\"segment.collapseEnabled\"");
    expect(chatView).toContain("const toolHandoffViewportQuiet = ref(false);");
    expect(chatView).toContain("function handleToolHandoffQuietChange(quiet: boolean) {");
    expect(chatView).toContain("@tool-handoff-quiet-change=\"handleToolHandoffQuietChange\"");
    expect(transcript).toContain(":allow-collapse=\"!shouldKeepToolSegmentExpanded(segment)\"");
    expect(transcript).toContain(":collapse-enabled=\"!shouldKeepToolSegmentExpanded(segment)\"");
    expect(toolBlock).toContain("collapseEnabled?: boolean;");
    expect(toolBlock).toContain(":tool-calls=\"toolCall.nestedToolCalls\"");
    expect(toolBlock).toContain(":collapse-enabled=\"collapseEnabled\"");
    expect(toolBlock).toContain("@viewport-anchor-start=\"emitToolViewportAnchorStart\"");
    expect(toolBlock).toContain("@tool-viewport-anchor-start=\"emitToolViewportAnchorStart\"");
    expect(toolCollection).toContain("collapseEnabled?: boolean;");
    expect(toolCollection).toContain("animateCollapseOnMount?: boolean;");
    expect(toolCollection).toContain("const startsExpandedForCollapseAnimation =");
    expect(toolCollection).toContain("onMounted(() => {");
    expect(toolCollection).toContain("props.allowCollapse && props.collapseEnabled");
  });

  it("keeps nested subagent tool rows compact", () => {
    const toolBlock = read("src/components/ToolCallBlock.vue");

    expect(toolBlock).toContain(".nested-tool-calls :deep(.tool-call-header) {");
    expect(toolBlock).toMatch(/\.nested-tool-calls :deep\(\.tool-call-header\)\s*\{[\s\S]*min-height:\s*18px/);
    expect(toolBlock).toMatch(/\.nested-tool-calls :deep\(\.tool-call-collection-list\)\s*\{[\s\S]*gap:\s*2px/);
    expect(toolBlock).toMatch(/\.nested-tool-calls :deep\(\.spinner-anim\)\s*\{[\s\S]*width:\s*8px/);
  });

  it("auto-collapses completed subagent tool blocks", () => {
    const toolBlock = read("src/components/ToolCallBlock.vue");

    expect(toolBlock).toContain("function shouldAutoExpandSubagentTool(toolCall: ToolCallDisplay) {");
    expect(toolBlock).toContain("return isSubagentToolName(toolCall.name) && toolCall.status === \"running\";");
    expect(toolBlock).toContain("const expanded = ref(shouldAutoExpandSubagentTool(props.toolCall));");
    expect(toolBlock).toContain("if (previousStatus === \"running\" && nextStatus !== \"running\") {");
    expect(toolBlock).toContain("setExpanded(false, true);");
    expect(toolBlock).toContain("} else if (previousStatus !== \"running\" && nextStatus === \"running\") {");
    expect(toolBlock).toContain("setExpanded(true, true);");
  });

  it("filters history tool calls while the transient handoff batch owns the same ids", () => {
    const chatView = read("src/components/ChatView.vue");
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("const hasLiveToolCalls = computed(() => props.activeToolCalls.length > 0);");
    expect(transcript).toContain("const hasTransientToolCalls = computed(() => transientToolCalls.value.length > 0);");
    expect(transcript).toContain("const hasToolCallHandoff = computed(() => hasTransientToolCalls.value && !hasLiveToolCalls.value);");
    expect(transcript).toContain("const canonicalLiveRenderParts = computed(() => {");
    expect(transcript).toContain("canonicalLiveRenderParts.value.some((part) => part.kind === \"text\" || part.kind === \"toolCall\")");
    expect(transcript).toContain("const activeToolCallMatchState = computed<ToolCallMatchState>(() => {");
    expect(transcript).toContain("return toolCallHandoff.value?.toolCallMatchState ?? {");
    expect(transcript).toContain("const baseGroupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(activeToolCallMatchState.value));");
    expect(transcript).toContain("const historyHiddenToolCallMatchState = computed<ToolCallMatchState>(() => {");
    expect(transcript).toContain("return mergeToolCallMatchStates(");
    expect(transcript).toContain("const groupedMessages = computed<MessageGroup[]>(() => buildGroupedMessages(historyHiddenToolCallMatchState.value));");
    expect(transcript).toContain("toolCallTreeHasAnyIds(message.toolCalls, toolCallHandoff.value!.toolCallMatchState)");
    expect(transcript).toContain("function shouldReleaseToolCallHandoffToHistory(");
    expect(transcript).toContain("function hasVisibleUserMessageAfterToolCallMatch(");
    expect(transcript).toContain("clearToolCallHandoff(\"active-cleared-history-before-inserted-user\")");
    expect(transcript).toContain("clearToolCallHandoff(\"handoff-history-before-inserted-user\")");
    expect(transcript).toContain("clearToolCallHandoff(\"handoff-followed-by-history-message\")");
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

  it("keeps handoff waiting out of the transient tool group layout", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const toolCollection = read("src/components/ToolCallCollection.vue");
    const waitingIndicator = read("src/components/chat/ChatWaitingIndicator.vue");
    const toolWaitingIndex = transcript.indexOf("<div v-if=\"segment.showWaiting && isToolWaitingRowVisible\" class=\"chat-transcript-tool-waiting-row\">");
    const standaloneWaitingIndex = transcript.indexOf("v-else-if=\"segment.type === 'waiting'\"");
    const toolGroupIndex = transcript.indexOf("v-else-if=\"segment.type === 'toolCalls'\"");

    expect(toolGroupIndex).toBeGreaterThan(-1);
    expect(toolWaitingIndex).toBeGreaterThan(toolGroupIndex);
    expect(toolWaitingIndex).toBeLessThan(standaloneWaitingIndex);
    expect(transcript).toContain("'waiting-placeholder': isStandaloneWaitingPlaceholder");
    expect(transcript).toContain("const isToolWaitingForResponse = computed(() => isWaitingForResponse.value && hasTransientToolCalls.value);");
    expect(transcript).toContain("const isToolWaitingRowVisible = computed(() => isToolWaitingForResponse.value && !hasToolCallHandoff.value);");
    expect(transcript).toContain("const isToolWaitingStatusVisible = computed(() => isToolWaitingForResponse.value && hasToolCallHandoff.value);");
    expect(transcript).toContain("const isStandaloneWaitingPlaceholder = computed(() => isWaitingForResponse.value && !hasTransientToolCalls.value);");
    expect(transcript).toContain("showWaiting: false,");
    expect(transcript).toContain("lastToolSegment.showWaiting = true;");
    expect(transcript).toContain("segment.showWaiting && isToolWaitingRowVisible");
    expect(transcript).toContain(":show-waiting-status=\"segment.showWaiting && isToolWaitingStatusVisible\"");
    expect(transcript).toContain(":waiting-label=\"waitingLabel\"");
    expect(transcript).toContain(":data-tool-layout-waiting-status=\"String(segment.showWaiting && isToolWaitingStatusVisible)\"");
    expect(transcript).not.toContain("segment.showWaiting && isToolWaitingForResponse");
    expect(toolCollection).toContain("showWaitingStatus?: boolean;");
    expect(toolCollection).toContain("waitingLabel?: string;");
    expect(toolCollection).toContain("class=\"tool-call-collection-waiting-status\"");
    expect(toolCollection).toContain("position: absolute;");
    expect(toolCollection).toContain("left: 4px;");
    expect(toolCollection).toContain("top: calc(100% + 6px);");
    expect(toolCollection).toContain("pointer-events: none;");
    expect(transcript).toContain("import ChatWaitingIndicator from \"./ChatWaitingIndicator.vue\";");
    expect(transcript).toContain("<ChatWaitingIndicator :label=\"waitingLabel\" compact />");
    expect(transcript).toContain("<ChatWaitingIndicator :label=\"segment.label\" />");
    expect(toolCollection).toContain("import ChatWaitingIndicator from \"./chat/ChatWaitingIndicator.vue\";");
    expect(toolCollection).toContain("<ChatWaitingIndicator :label=\"waitingLabel\" compact />");
    expect(toolCollection).not.toContain("tool-call-collection-waiting-dot");
    expect(toolCollection).not.toContain("tool-call-collection-waiting-label");
    expect(waitingIndicator).toContain("class=\"chat-waiting-indicator\"");
    expect(waitingIndicator).toContain("chat-waiting-indicator-spinner");
    expect(waitingIndicator).toContain("chat-waiting-indicator-label");
    expect(transcript).toContain(".chat-transcript-tool-waiting-row {");
    expect(transcript).not.toContain("contain-intrinsic-size: auto 0;");
    expect(transcript).not.toContain("padding-top: 8px;");
    expect(transcript).not.toContain("'waiting-placeholder': isWaitingForResponse");
  });

  it("sorts assistant segments by persisted render order", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("function renderPartsForMessage(item: MessageRenderItem): AssistantRenderPart[]");
    expect(transcript).toContain("assertCanonicalRenderParts(item.message.renderParts, `message:${item.message.id}`);");
    expect(transcript).toContain("synthesizeLegacyRenderParts(item.message, {");
    expect(transcript).toContain("const canonicalLiveRenderParts = computed(() => {");
    expect(transcript).toContain("props.liveRenderParts.length > 0");
    expect(transcript).toContain("const hasVisibleActiveThinkingBlock = computed(() =>");
    expect(transcript).toContain(":class=\"{ active: segment.active, 'is-clickable': true }\"");
    expect(transcript).toContain("data-render-part-kind=\"toolCall\"");
    expect(transcript).toContain("data-render-part-kind=\"text\"");
    expect(transcript).toContain("data-render-part-kind=\"waiting\"");
    expect(transcript).toContain("data-render-part-scope=\"history\"");
    expect(transcript).toContain("data-render-part-scope=\"transient\"");
    expect(transcript).toContain(":data-render-part-key=\"segment.key\"");
    expect(transcript).toMatch(
      /<div class="chat-transcript-message-content" :class="`is-\$\{variant\}`">\s*<div class="chat-transcript-item-stack" :class="`is-\$\{variant\}`">\s*<template\s+v-for="segment in transientRenderSegments"/,
    );
    expect(transcript).not.toContain("splitToolCallsByRenderOrder");
  });

  it("keeps live thinking above transient status overlays", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const diagnostics = read("src/services/layoutDiagnostics.ts");

    expect(transcript).toContain("'has-live-thinking': hasVisibleActiveThinkingBlock");
    expect(transcript).toContain("'has-tool-waiting-status': isToolWaitingStatusVisible");
    expect(transcript).toContain(".chat-transcript-message.is-session.assistant.transient.has-live-thinking");
    expect(transcript).toContain(".chat-transcript-message.is-session.assistant.transient.has-tool-waiting-status");
    expect(transcript).toContain("content-visibility: visible;");
    expect(transcript).toContain("contain: layout;");
    expect(transcript).toContain(".chat-transcript-item-stack > [data-render-part-kind]");
    expect(transcript).toContain(".chat-transcript-tool-calls-group {");
    expect(transcript).toContain("z-index: 0;");
    expect(transcript).toContain(".chat-transcript-thinking-block[data-render-part-scope=\"transient\"]");
    expect(transcript).toContain("z-index: 3;");
    expect(transcript).toContain("<ChatWaitingIndicator :label=\"thinkingActiveLabel\" />");
    expect(transcript).toContain("<ChatWaitingIndicator v-if=\"segment.active\" :label=\"thinkingActiveLabel\" compact />");
    expect(transcript).not.toContain("chat-transcript-thinking-spinner");
    expect(transcript).not.toContain("chat-transcript-thinking-shimmer");
    expect(transcript).toContain("traceTranscriptPaintOcclusion");
    expect(transcript).toContain("hasTransientStatusPaintTarget");
    expect(transcript).toContain("traceTransientStatusPaint");
    expect(transcript).toContain("transientStatusPaintStateChanged");
    expect(transcript).toContain("standaloneWaiting: isStandaloneWaitingPlaceholder.value");
    expect(transcript).toContain("toolWaitingStatusVisible: isToolWaitingStatusVisible.value");
    expect(transcript).toContain("transientSegmentPaintState()");
    expect(diagnostics).toContain("[Locus layout][paint-occlusion]");
    expect(diagnostics).toContain("document.elementsFromPoint");
    expect(diagnostics).toContain("TRANSCRIPT_STANDALONE_WAITING_SELECTOR");
    expect(diagnostics).toContain("TRANSCRIPT_TOOL_WAITING_STATUS_SELECTOR");
    expect(diagnostics).toContain("tool-waiting-status");
    expect(diagnostics).toContain("insideTargetWhenTargetHitTestable");
    expect(diagnostics).toContain("occludedTargets");
    expect(diagnostics).toContain("intersectingCandidates");
    expect(diagnostics).toContain("TRANSCRIPT_ACTIVE_THINKING_SELECTOR");
  });

  it("coalesces consecutive tool-only assistant rounds before rendering", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");
    const batches = read("src/composables/toolCallBatches.ts");

    expect(batches).toContain("let pendingToolOnlyItem: T | null = null;");
    expect(batches).toContain("pendingToolOnlyItem ??= item;");
    expect(batches).toContain("const displayToolCalls = pendingToolCalls.length > 0 ? [...pendingToolCalls] : undefined;");
    expect(transcript).toContain("function historyRenderSegmentsForGroup(group: MessageGroup): HistoryRenderSegment[]");
    expect(transcript).toContain("function historyToolSegmentKey(toolCalls: ToolCallDisplay[], fallbackId: string)");
    expect(transcript).toContain("key: historyToolSegmentKey(pendingToolCalls, pendingToolPart.id),");
    expect(transcript).toContain("pendingToolCalls.push(...segment.toolCalls);");
    expect(transcript).toContain("const hasToolFilter = hasExplicitDisplayToolCalls(item) || !!toolCallInfosForMessage(item.message);");
    expect(transcript).toContain(".filter((part) => part.kind !== \"toolCall\" || !hasToolFilter || visibleToolIds.has(part.toolCall.id))");
    expect(transcript).toContain("hiddenToolCallsByItemId.set(item.id, toolCalls ?? []);");
    expect(batches).toContain("const hasToolCallsProperty = Object.prototype.hasOwnProperty.call(item, \"toolCalls\");");
    expect(transcript).toContain("'tool-only': isToolOnlyMessageGroup(group),");
    expect(transcript).not.toContain("tool-only-followup");
    expect(transcript).not.toContain("shouldTightenToolOnlyGap");
  });

  it("attaches knowledge proposals only inside their assistant message group", () => {
    const transcript = read("src/components/chat/ChatTranscript.vue");

    expect(transcript).toContain("for (const group of groups) {");
    expect(transcript).toContain("if (group.role !== \"assistant\") continue;");
    expect(transcript).toContain("const nextRequestTool = group.items.find(");
    expect(transcript).toContain("const prevRequestTool = [...group.items].reverse().find(");
  });
});
