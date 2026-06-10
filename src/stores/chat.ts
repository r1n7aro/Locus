import { ref, computed } from "vue";
import { defineStore } from "pinia";
import { useModelStore } from "./model";
import { useAgentStore } from "./agent";
import { useUiStore } from "./ui";
import { useNotificationStore } from "./notification";
import { normalizeAppError } from "../services/errors";
import { getToolPermissionMode, saveToolPermissionMode } from "../services/permissions";
import * as sessionService from "../services/session";
import * as undoService from "../services/undo";
import {
  buildToolResultMessages,
  isMatchingPendingUserMessage,
  isPendingUserMessageId,
  mergeUserMessage,
  reduceStreamEvent,
  type StreamMutation,
} from "../composables/useStreamReducer";
import { resolveToolCallDisplayShape } from "../composables/toolCallBatches";
import { hydrateChatMessagesIntent, withClientMessageId } from "../composables/chatInputIntents";
import { buildUserMessageDraft } from "../composables/chatMessageDraft";
import type { SessionScrollState } from "../composables/chatScrollState";
import { t } from "../i18n";
import { useChatChangesStore } from "./chatChanges";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { useKnowledgeAccessMode } from "../composables/useKnowledgeAccessMode";
import { useChatInputSettings } from "../composables/useChatInputSettings";
import { isToolCollapseTraceEnabled, logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";
import type {
  SessionSummary, SessionDetail, ChatMessage, TokenUsage,
  TodoItem, StreamEvent, ImageAttachment, AssetRefAttachment, ToolCallDisplay,
  PendingQuestion, PendingToolConfirm,
  UserIntentMeta,
  KnowledgeProposalStatus,
  UndoConflictInfo,
  ChangedFile,
  TodoSnapshot,
  TodoPanelMode,
  SessionRunSummary,
  AssistantRenderPart,
  PendingSessionInput,
} from "../types";

type ToolPermissionMode = "auto" | "ask";

function emptyTokenUsage(): TokenUsage {
  return {
    totalInputTokens: 0, totalOutputTokens: 0,
    totalCacheReadTokens: 0, totalCacheWriteTokens: 0,
    totalCostUsd: 0, pricedRounds: 0, contextTokens: 0, contextLimit: 0,
  };
}

function hydrateMessages(messages: ChatMessage[]): ChatMessage[] {
  return hydrateChatMessagesIntent(messages);
}

function replaceMessageById(list: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = list.findIndex((item) => item.id === message.id);
  if (index < 0) return [...list, message];
  const next = [...list];
  next.splice(index, 1, message);
  return next;
}

function pendingInputMergeKey(sessionId: string, runId: string, mergeGroupId: string) {
  return `${sessionId}\u{0}${runId}\u{0}${mergeGroupId}`;
}

function runScopedKey(sessionId: string, runId: string) {
  return `${sessionId}\u{0}${runId}`;
}

function traceMessageOrder(messages: ChatMessage[]) {
  return messages.map((message, index) => ({
    index,
    id: message.id,
    role: message.role,
    contentLen: message.content.length,
    contentPreview: previewTraceText(message.content, 48),
    toolCallId: message.toolCallId ?? null,
    toolCallIds: message.toolCalls?.map((toolCall) => toolCall.id) ?? [],
    renderPartKinds: message.renderParts?.map((part) => part.kind) ?? [],
  }));
}

function traceToolCallOrder(toolCalls: ToolCallDisplay[]) {
  return toolCalls.map((toolCall, index) => ({
    index,
    id: toolCall.id,
    name: toolCall.name,
    status: toolCall.status,
    order: toolCall.order ?? null,
    nestedIds: toolCall.nestedToolCalls?.map((nested) => nested.id) ?? [],
  }));
}

function traceStreamEvent(event: StreamEvent) {
  const base = {
    type: event.type,
    sessionId: event.sessionId,
    runId: event.runId,
  };

  switch (event.type) {
    case "userMessage":
      return {
        ...base,
        messageId: event.message.id,
        contentLen: event.message.content.length,
        contentPreview: previewTraceText(event.message.content, 48),
      };
    case "pendingInputAccepted":
      return {
        ...base,
        pendingInputId: event.pendingInputId,
        messageId: event.messageId,
      };
    case "pendingInputQueued":
      return {
        ...base,
        pendingInputId: event.input.id,
        delivery: event.input.delivery ?? "after_run",
        mergeGroupId: event.input.mergeGroupId,
      };
    case "pendingInputDeleted":
      return {
        ...base,
        pendingInputId: event.pendingInputId,
      };
    case "toolCallStart":
    case "toolCallDone":
    case "toolCallDelta":
    case "toolCallProgress":
      return {
        ...base,
        toolCallId: event.toolCallId,
        toolName: "toolName" in event ? event.toolName : undefined,
      };
    case "toolCallRoundDone":
      return {
        ...base,
        messageId: event.messageId,
        fullTextLen: event.fullText.length,
        toolCallIds: event.toolCalls.map((toolCall) => toolCall.id),
        renderPartKinds: event.renderParts?.map((part) => part.kind) ?? [],
      };
    case "done":
      return {
        ...base,
        messageId: event.messageId,
        fullTextLen: event.fullText.length,
      };
    default:
      return base;
  }
}

function traceStreamMutation(mutation: StreamMutation) {
  switch (mutation.type) {
    case "pushMessage":
    case "upsertMessage":
    case "upsertUserMessage":
      return {
        type: mutation.type,
        messageId: mutation.message.id,
        role: mutation.message.role,
        contentLen: mutation.message.content.length,
        toolCallIds: mutation.message.toolCalls?.map((toolCall) => toolCall.id) ?? [],
        renderPartKinds: mutation.message.renderParts?.map((part) => part.kind) ?? [],
      };
    case "pushToolResults":
      return {
        type: mutation.type,
        toolCallIds: mutation.toolCallIds ?? null,
      };
    case "addToolCall":
      return {
        type: mutation.type,
        toolCall: traceToolCallOrder([mutation.toolCall])[0],
      };
    case "updateToolCall":
      return {
        type: mutation.type,
        id: mutation.id,
        updates: mutation.updates,
      };
    case "resetRound":
    case "resetRoundKeepToolCalls":
    case "clearLiveRenderParts":
    case "setStreaming":
      return mutation;
    default:
      return { type: mutation.type };
  }
}

function mergePendingInputList(
  list: PendingSessionInput[],
  input: PendingSessionInput,
): PendingSessionInput[] {
  const index = list.findIndex((item) =>
    item.id === input.id
    || (
      item.runId === input.runId
      && item.mergeGroupId === input.mergeGroupId
      && item.status !== "accepted"
      && item.status !== "restored"
    ));
  if (index < 0) return [...list, input];
  const next = [...list];
  next.splice(index, 1, input);
  return next;
}

function visiblePendingInputs(inputs: PendingSessionInput[] | undefined): PendingSessionInput[] {
  return (inputs ?? []).filter((input) =>
    input.status === "queued" || input.status === "delivering");
}

function pendingInputDelivery(input: PendingSessionInput): "after_run" | "immediate" {
  return input.delivery === "immediate" ? "immediate" : "after_run";
}

function joinPendingText(existing: string, next: string): string {
  const existingTrimmed = existing.trim();
  const nextTrimmed = next.trim();
  if (!existingTrimmed && !nextTrimmed) return "";
  if (!existingTrimmed) return next;
  if (!nextTrimmed) return existing;
  return `${existing}\n${next}`;
}

function isPendingInputFallbackError(code: string): boolean {
  return code === "session.pending_input.run_closed"
    || code === "session.pending_input.no_active_run"
    || code === "session.pending_input.run_mismatch"
    || code === "session.run_locked";
}

function isActiveRuntimeStatus(status: SessionSummary["runtimeStatus"]): boolean {
  return status === "running"
    || status === "waiting_input"
    || status === "finishing"
    || status === "cancelling"
    || status === "starting"
    || status === "queued";
}

function isActiveRunStatus(status: SessionRunSummary["status"] | null | undefined): boolean {
  return status === "running"
    || status === "waiting_input"
    || status === "finishing"
    || status === "cancelling"
    || status === "starting"
    || status === "queued";
}

function cloneRuntimeToolCalls(toolCalls: ToolCallDisplay[] | undefined): ToolCallDisplay[] {
  return (toolCalls ?? []).map((toolCall) => {
    const displayShape = resolveToolCallDisplayShape({
      name: toolCall.name,
      arguments: toolCall.arguments,
    });
    const nestedToolCalls = toolCall.nestedToolCalls
      ? cloneRuntimeToolCalls(toolCall.nestedToolCalls)
      : undefined;
    return {
      ...toolCall,
      name: displayShape.name,
      arguments: displayShape.arguments,
      images: toolCall.images?.map((image) => ({ ...image })),
      progress: toolCall.progress ? { ...toolCall.progress } : toolCall.progress,
      nestedToolCalls,
    };
  });
}

function cloneRuntimeJson<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function normalizeToolPermissionMode(mode: string | null | undefined): ToolPermissionMode {
  return mode === "ask" ? "ask" : "auto";
}

function isLocalPendingUserMessage(message: ChatMessage): boolean {
  return message.role === "user" && isPendingUserMessageId(message.id);
}

function logChatStreamDebug(message: string, detail?: Record<string, unknown>) {
  console.info(`[chat-stream] ${message}`, detail ?? {});
}

export const useChatStore = defineStore("chat", () => {
  // -- State --
  const sessions = ref<SessionSummary[]>([]);
  const activeSessionId = ref<string | null>(null);
  const activeSessionType = ref<string | null>(null);
  const messages = ref<ChatMessage[]>([]);
  const streamingText = ref("");
  const rawStreamText = ref("");
  const streamingThinking = ref("");
  const streamSequence = ref(0);
  const streamingTextOrder = ref(0);
  const thinkingOrder = ref(0);
  const liveRenderParts = ref<AssistantRenderPart[]>([]);
  const isStreaming = ref(false);
  const isCompacting = ref(false);
  const currentRunId = ref<string | null>(null);
  const isThinking = ref(false);
  const thinkingStartTime = ref(0);
  const thinkingDuration = ref(0);
  const showThinkingPanel = ref(false);
  const thinkingPanelContent = ref("");
  const activeToolCalls = ref<ToolCallDisplay[]>([]);
  const tokenUsage = ref<TokenUsage>(emptyTokenUsage());
  const todos = ref<TodoItem[]>([]);
  const todoWriteVersion = ref(0);
  const showTodoPanel = ref(false);
  const todoPanelVisibility = ref(new Map<string, boolean>());
  const todoMode = ref<TodoPanelMode>("current");
  const sessionLatestTodoRunIds = ref(new Map<string, string | null>());
  const sessionLatestCompletedRunIds = ref(new Map<string, string | null>());
  const pendingQuestion = ref<PendingQuestion | null>(null);
  const pendingToolConfirms = ref<PendingToolConfirm[]>([]);
  const streamingSessionIds = ref(new Set<string>());
  const undoableMessageIds = ref(new Set<string>());
  const sessionRunIds = ref(new Map<string, string>());
  const pendingInputsBySession = ref(new Map<string, PendingSessionInput[]>());
  const acceptedPendingInputIds = new Set<string>();
  const deferredUserMessagesByRun = new Map<string, ChatMessage[]>();
  const localPendingInputGroups = new Set<string>();
  const localFallbackPendingInputGroups = new Set<string>();
  const sessionScrollStates = ref(new Map<string, SessionScrollState>());
  const sessionAgentId = ref<string | null>(null);
  const toolPermissionMode = ref<ToolPermissionMode>("auto");
  const sessionAgentLocked = computed(() => !!activeSessionId.value && !!sessionAgentId.value);
  const todoRunBoundaryId = computed(() => {
    const sessionId = activeSessionId.value;
    if (!sessionId) return null;
    if (isStreaming.value) return currentRunId.value;
    return sessionLatestCompletedRunIds.value.get(sessionId) ?? null;
  });
  const latestTodoRunId = computed(() => {
    const sessionId = activeSessionId.value;
    if (!sessionId) return null;
    return sessionLatestTodoRunIds.value.get(sessionId) ?? null;
  });
  const currentTodos = computed(() => {
    if (!todoRunBoundaryId.value) return [];
    if (latestTodoRunId.value !== todoRunBoundaryId.value) return [];
    return todos.value;
  });
  const visibleTodos = computed(() => currentTodos.value);
  const hasAnyTodos = computed(() => todos.value.length > 0);
  const visibleTodoCount = computed(() => visibleTodos.value.length);
  const todoCelebrationEnabled = computed(() => (
    !!todoRunBoundaryId.value && latestTodoRunId.value === todoRunBoundaryId.value
  ));
  const todoCelebrationVersion = computed(() => (
    todoCelebrationEnabled.value ? todoWriteVersion.value : 0
  ));
  const activeQueuedFollowUps = computed(() => {
    const sessionId = activeSessionId.value;
    if (!sessionId) return [];
    const inputs = visiblePendingInputs(pendingInputsBySession.value.get(sessionId));
    const runId = currentRunId.value;
    return runId ? inputs.filter((input) => input.runId === runId) : inputs;
  });
  const activeQueuedFollowUp = computed(() => {
    const inputs = activeQueuedFollowUps.value;
    if (inputs.length === 0) return null;
    const displayText = inputs
      .map((input) => input.displayText || input.text)
      .filter((text) => text.trim().length > 0)
      .join("\n");
    return {
      inputs,
      displayText,
      canInsert: inputs.some((input) => pendingInputDelivery(input) !== "immediate"),
      isInserting: inputs.every((input) => pendingInputDelivery(input) === "immediate"),
      imageCount: inputs.reduce((total, input) => total + (input.images?.length ?? 0), 0),
      assetRefCount: inputs.reduce((total, input) => total + (input.assetRefs?.length ?? 0), 0),
    };
  });

  // Plan run tracking (bound to runId + sessionId to avoid stale state)
  const pendingPlanRun = ref<{
    runId: string | null;  // null until first stream event arrives
    sessionId: string;
    agentId: string;
    requestText: string;
  } | null>(null);

  let pendingSessionId: string | null = null;
  let pendingMessageSeq = 0;
  let sessionLoadSeq = 0;
  let pendingManagedSessionId: string | null = null;
  let pendingManagedUnboundSession = false;
  // Sessions started from ChatView can be updated incrementally in-memory.
  // Externally driven sessions (Git/docgen) must be reloaded from the store.
  const managedStreamingSessionIds = new Set<string>();
  const closedRunIds = new Map<string, string>();
  const cancelRequestedRunIds = new Map<string, string>();
  let activeSessionSelectionRestoreAttempted = false;
  let activeSessionSelectionPersistSeq = 0;
  // A cancel clicked while the chat launch is still in flight (no run id yet)
  // is remembered here and re-fired once the run is registered.
  const pendingLaunchCancelRequested = ref(false);
  const isCancelling = computed(() => {
    if (pendingLaunchCancelRequested.value) return true;
    if (!activeSessionId.value || !currentRunId.value) return false;
    return cancelRequestedRunIds.get(activeSessionId.value) === currentRunId.value;
  });

  function nextPendingMessageId(): string {
    pendingMessageSeq += 1;
    return `user_pending_${Date.now()}_${pendingMessageSeq}`;
  }

  function deferredUserMessagesTraceState() {
    return Array.from(deferredUserMessagesByRun.entries()).map(([key, deferredMessages]) => ({
      key,
      count: deferredMessages.length,
      messages: traceMessageOrder(deferredMessages),
    }));
  }

  function traceChatStore(event: string, detail: () => Record<string, unknown>) {
    if (!isToolCollapseTraceEnabled(event)) return;
    logToolCollapseTrace("chat-store", event, detail());
  }

  function traceStoreOrder(event: string, detail: () => Record<string, unknown> = () => ({})) {
    if (!isToolCollapseTraceEnabled(event)) return;
    logToolCollapseTrace("chat-store:order", event, {
      activeSessionId: activeSessionId.value,
      currentRunId: currentRunId.value,
      isStreaming: isStreaming.value,
      messageCount: messages.value.length,
      messages: traceMessageOrder(messages.value),
      activeToolCalls: traceToolCallOrder(activeToolCalls.value),
      deferredUserMessages: deferredUserMessagesTraceState(),
      ...detail(),
    });
  }

  function restoreDraftFromFailedUserMessage(
    message: ChatMessage,
    options: { sessionId?: string | null; requireEmptyComposer?: boolean } = {},
  ) {
    useUiStore().stageChatDraftPrefill(buildUserMessageDraft(message), options);
  }

  function failedUserMessageFromPayload(
    id: string,
    displayText: string,
    images: ImageAttachment[],
    assetRefs: AssetRefAttachment[],
    userIntent: UserIntentMeta | null | undefined,
  ): ChatMessage {
    return {
      id,
      role: "user",
      content: displayText,
      createdAt: Date.now() / 1000,
      images: images.length > 0 ? images : undefined,
      assetRefs: assetRefs.length > 0 ? assetRefs : undefined,
      intentMeta: userIntent ?? undefined,
      thinkingSignature: userIntent ? JSON.stringify(userIntent) : undefined,
    };
  }

  async function loadSessionStatePreservingFailedUserDraft(sessionId: string) {
    const pendingUserMessages = messages.value.filter(isLocalPendingUserMessage);
    const pendingUserMessage = pendingUserMessages[pendingUserMessages.length - 1];
    await loadSessionState(sessionId);
    if (!pendingUserMessage) return;
    if (activeSessionId.value !== sessionId) return;
    if (messages.value.some((message) => isMatchingPendingUserMessage(pendingUserMessage, message))) {
      return;
    }
    restoreDraftFromFailedUserMessage(pendingUserMessage, {
      sessionId,
      requireEmptyComposer: true,
    });
  }

  function resolveSessionType(sessionId: string | null): string | null {
    if (!sessionId) return null;
    if (sessionId === activeSessionId.value && activeSessionType.value) {
      return activeSessionType.value;
    }
    return sessions.value.find((session) => session.id === sessionId)?.sessionType ?? null;
  }

  function persistActiveSessionSelection(sessionId: string | null) {
    const seq = ++activeSessionSelectionPersistSeq;
    sessionService.saveActiveSessionSelection(sessionId).catch((e) => {
      if (seq !== activeSessionSelectionPersistSeq) return;
      console.warn("save_active_session_selection failed:", e);
    });
  }

  function setActiveSessionSelection(
    sessionId: string | null,
    options: { persist?: boolean } = {},
  ) {
    activeSessionId.value = sessionId;
    if (options.persist !== false) {
      persistActiveSessionSelection(sessionId);
    }
  }

  async function restoreActiveSessionSelection(nextSessions: SessionSummary[]) {
    if (activeSessionSelectionRestoreAttempted || activeSessionId.value) return;
    activeSessionSelectionRestoreAttempted = true;

    let savedSessionId: string | null = null;
    try {
      savedSessionId = await sessionService.getActiveSessionSelection();
    } catch (e) {
      console.warn("get_active_session_selection failed:", e);
      return;
    }

    const normalizedSessionId = savedSessionId?.trim();
    if (!normalizedSessionId) return;

    const restoredSession = nextSessions.find((session) => session.id === normalizedSessionId);
    if (!restoredSession) {
      persistActiveSessionSelection(null);
      return;
    }

    if (activeSessionId.value) return;
    setActiveSessionSelection(normalizedSessionId, { persist: false });
    activeSessionType.value = restoredSession.sessionType ?? null;
    currentRunId.value = sessionRunIds.value.get(normalizedSessionId) ?? null;
    await loadSessionState(normalizedSessionId);
  }

  function resetStreamRuntimeState() {
    resetStreamAnim();
    streamingThinking.value = "";
    streamSequence.value = 0;
    streamingTextOrder.value = 0;
    thinkingOrder.value = 0;
    liveRenderParts.value = [];
    isCompacting.value = false;
    isThinking.value = false;
    thinkingStartTime.value = 0;
    thinkingDuration.value = 0;
    activeToolCalls.value = [];
    pendingQuestion.value = null;
    pendingToolConfirms.value = [];
  }

  function applySessionData(
    detail: SessionDetail,
    usage: TokenUsage,
    sessionTodos: TodoSnapshot,
    undoEntries: Array<{ assistantMessageId: string }>,
  ) {
    clearDeferredUserMessagesForSession(detail.id);
    messages.value = hydrateMessages(detail.messages);
    setSessionPendingInputs(detail.id, detail.pendingInputs ?? []);
    tokenUsage.value = usage;
    todos.value = sessionTodos.items;
    sessionLatestTodoRunIds.value.set(detail.id, sessionTodos.latestRunId);
    sessionLatestCompletedRunIds.value.set(detail.id, detail.latestCompletedRunId ?? null);
    restoreTodoPanelState(detail.id, sessionTodos.items.length > 0);
    undoableMessageIds.value = new Set(undoEntries.map((e) => e.assistantMessageId));
    sessionAgentId.value = detail.agentId ?? null;
    activeSessionType.value = detail.sessionType;
    if (detail.agentId) {
      useAgentStore().selectAgent(detail.agentId);
    }
    applySessionRuntimeSnapshot(detail);
  }

  function clearLoadedSessionState() {
    if (activeSessionId.value) {
      clearDeferredUserMessagesForSession(activeSessionId.value);
      setSessionPendingInputs(activeSessionId.value, []);
    }
    messages.value = [];
    tokenUsage.value = emptyTokenUsage();
    todos.value = [];
    todoWriteVersion.value = 0;
    showTodoPanel.value = false;
    todoMode.value = "current";
    undoableMessageIds.value = new Set();
    sessionAgentId.value = null;
    activeSessionType.value = null;
    pendingQuestion.value = null;
    pendingToolConfirms.value = [];
    isCompacting.value = false;
  }

  function trackActiveRun(sessionId: string, runId: string) {
    streamingSessionIds.value.add(sessionId);
    sessionRunIds.value.set(sessionId, runId);
    useChatChangesStore().setActiveRunId(sessionId, runId);
    closedRunIds.delete(sessionId);
    if (activeSessionId.value === sessionId) {
      currentRunId.value = runId;
      isStreaming.value = true;
    }
  }

  function clearTrackedRun(sessionId: string, runId?: string | null) {
    streamingSessionIds.value.delete(sessionId);
    sessionRunIds.value.delete(sessionId);
    cancelRequestedRunIds.delete(sessionId);
    managedStreamingSessionIds.delete(sessionId);
    if (runId) {
      deferredUserMessagesByRun.delete(runScopedKey(sessionId, runId));
    }
  }

  function applySessionRuntimeSnapshot(detail: SessionDetail) {
    const runtime = detail.runtime;
    if (!runtime || !isActiveRunStatus(runtime.activeRun.status)) {
      const previousRunId = sessionRunIds.value.get(detail.id);
      clearTrackedRun(detail.id, previousRunId);
      if (activeSessionId.value === detail.id) {
        currentRunId.value = null;
        isStreaming.value = false;
        resetStreamRuntimeState();
      }
      return;
    }

    trackActiveRun(detail.id, runtime.activeRun.runId);
    if (activeSessionId.value !== detail.id) return;

    resetStreamAnim();
    rawStreamText.value = runtime.streamingText ?? "";
    streamingText.value = rawStreamText.value;
    streamingThinking.value = runtime.streamingThinking ?? "";
    streamSequence.value = runtime.streamSequence ?? 0;
    streamingTextOrder.value = runtime.streamingTextOrder ?? 0;
    thinkingOrder.value = runtime.thinkingOrder ?? 0;
    liveRenderParts.value = cloneRuntimeJson(runtime.liveRenderParts ?? []);
    isThinking.value = runtime.isThinking === true;
    thinkingStartTime.value = isThinking.value ? Date.now() : 0;
    thinkingDuration.value = runtime.thinkingDuration ?? 0;
    activeToolCalls.value = cloneRuntimeToolCalls(runtime.activeToolCalls);
    pendingQuestion.value = runtime.pendingQuestion
      ? cloneRuntimeJson(runtime.pendingQuestion)
      : null;
    pendingToolConfirms.value = cloneRuntimeJson(runtime.pendingToolConfirms ?? []);
    isCompacting.value = runtime.isCompacting === true;
  }

  function clearDeferredUserMessagesForSession(sessionId: string) {
    for (const key of Array.from(deferredUserMessagesByRun.keys())) {
      if (key.startsWith(`${sessionId}\u{0}`)) {
        deferredUserMessagesByRun.delete(key);
      }
    }
  }

  function setSessionPendingInputs(sessionId: string, inputs: PendingSessionInput[]) {
    const next = new Map(pendingInputsBySession.value);
    const visible = visiblePendingInputs(inputs);
    if (visible.length === 0) {
      next.delete(sessionId);
    } else {
      next.set(sessionId, visible);
    }
    pendingInputsBySession.value = next;
  }

  function upsertPendingInput(input: PendingSessionInput) {
    if (acceptedPendingInputIds.has(input.id)) return;
    const next = new Map(pendingInputsBySession.value);
    const list = visiblePendingInputs(next.get(input.sessionId));
    const merged = visiblePendingInputs(mergePendingInputList(list, input));
    if (merged.length === 0) {
      next.delete(input.sessionId);
    } else {
      next.set(input.sessionId, merged);
    }
    pendingInputsBySession.value = next;
  }

  function removePendingInput(sessionId: string, predicate: (input: PendingSessionInput) => boolean) {
    const next = new Map(pendingInputsBySession.value);
    const list = visiblePendingInputs(next.get(sessionId)).filter((input) => !predicate(input));
    if (list.length === 0) {
      next.delete(sessionId);
    } else {
      next.set(sessionId, list);
    }
    pendingInputsBySession.value = next;
  }

  function clearPendingInputTracking(sessionId: string, runId: string, mergeGroupId: string) {
    const key = pendingInputMergeKey(sessionId, runId, mergeGroupId);
    localPendingInputGroups.delete(key);
    localFallbackPendingInputGroups.delete(key);
  }

  function markPendingInputDeleted(sessionId: string, pendingInputId: string) {
    const deleted = visiblePendingInputs(pendingInputsBySession.value.get(sessionId))
      .find((input) => input.id === pendingInputId);
    if (deleted) {
      clearPendingInputTracking(sessionId, deleted.runId, deleted.mergeGroupId);
    }
    removePendingInput(sessionId, (input) => input.id === pendingInputId);
  }

  function clearRunPendingInputs(sessionId: string, runId: string) {
    removePendingInput(sessionId, (input) => input.runId === runId);
    for (const key of Array.from(localPendingInputGroups)) {
      if (key.startsWith(`${sessionId}\u{0}${runId}\u{0}`)) {
        localPendingInputGroups.delete(key);
      }
    }
    for (const key of Array.from(localFallbackPendingInputGroups)) {
      if (key.startsWith(`${sessionId}\u{0}${runId}\u{0}`)) {
        localFallbackPendingInputGroups.delete(key);
      }
    }
  }

  function markPendingInputAccepted(sessionId: string, pendingInputId: string) {
    acceptedPendingInputIds.add(pendingInputId);
    const accepted = visiblePendingInputs(pendingInputsBySession.value.get(sessionId))
      .find((input) => input.id === pendingInputId);
    if (accepted) {
      clearPendingInputTracking(sessionId, accepted.runId, accepted.mergeGroupId);
    }
    removePendingInput(sessionId, (input) => input.id === pendingInputId);
  }

  function shouldDeferUserMessage(event: Extract<StreamEvent, { type: "userMessage" }>) {
    if (event.sessionId !== activeSessionId.value) return false;
    if (event.runId !== currentRunId.value) return false;
    return activeToolCalls.value.length > 0;
  }

  function deferUserMessage(event: Extract<StreamEvent, { type: "userMessage" }>) {
    const key = runScopedKey(event.sessionId, event.runId);
    const messagesForRun = deferredUserMessagesByRun.get(key) ?? [];
    deferredUserMessagesByRun.set(key, mergeUserMessage(messagesForRun, event.message));
    traceStoreOrder("deferUserMessageDuringToolRound", () => ({
      key,
      event: traceStreamEvent(event),
      deferredForRun: traceMessageOrder(deferredUserMessagesByRun.get(key) ?? []),
    }));
  }

  function flushDeferredUserMessages(sessionId: string, runId: string) {
    const key = runScopedKey(sessionId, runId);
    const deferredMessages = deferredUserMessagesByRun.get(key);
    if (!deferredMessages || deferredMessages.length === 0) return;

    const traceFlushDeferredUserMessages = isToolCollapseTraceEnabled("flushDeferredUserMessages");
    const beforeMessages = traceFlushDeferredUserMessages ? traceMessageOrder(messages.value) : null;
    if (activeSessionId.value === sessionId) {
      for (const message of deferredMessages) {
        messages.value = mergeUserMessage(messages.value, message);
      }
    }
    deferredUserMessagesByRun.delete(key);
    traceStoreOrder("flushDeferredUserMessages", () => ({
      key,
      sessionId,
      runId,
      flushedMessages: traceMessageOrder(deferredMessages),
      messagesBeforeFlush: beforeMessages,
      messagesAfterFlush: traceMessageOrder(messages.value),
    }));
  }

  function localQueuedInputsForRun(
    sessionId: string,
    runId: string,
    includeServerQueued = false,
  ): PendingSessionInput[] {
    const groups = includeServerQueued ? localPendingInputGroups : localFallbackPendingInputGroups;
    const inputs = visiblePendingInputs(pendingInputsBySession.value.get(sessionId))
      .filter((input) => input.runId === runId);
    return inputs.filter((input) =>
      groups.has(
        pendingInputMergeKey(sessionId, runId, input.mergeGroupId),
      ));
  }

  function upsertLocalPendingInput(params: {
    sessionId: string;
    runId: string;
    mergeGroupId: string;
    text: string;
    displayText: string;
    images: ImageAttachment[];
    assetRefs: AssetRefAttachment[];
    mode?: string | null;
    userIntent?: UserIntentMeta | null;
    clientMessageId?: string | null;
  }): PendingSessionInput {
    const existing = visiblePendingInputs(pendingInputsBySession.value.get(params.sessionId))
      .find((input) =>
        input.runId === params.runId && input.mergeGroupId === params.mergeGroupId);
    const now = Date.now() / 1000;
    const pending: PendingSessionInput = existing
      ? {
        ...existing,
        text: joinPendingText(existing.text, params.text),
        displayText: joinPendingText(existing.displayText, params.displayText),
        images: [...(existing.images ?? []), ...params.images],
        assetRefs: [...(existing.assetRefs ?? []), ...params.assetRefs],
        mode: existing.mode === "plan" || params.mode === "plan"
          ? "plan"
          : params.mode ?? existing.mode ?? null,
        userIntent: params.userIntent ?? existing.userIntent ?? null,
        clientMessageId: existing.clientMessageId ?? params.clientMessageId ?? null,
        updatedAt: now,
      }
      : {
        id: params.mergeGroupId,
        sessionId: params.sessionId,
        runId: params.runId,
        mergeGroupId: params.mergeGroupId,
        status: "queued",
        delivery: "after_run",
        text: params.text,
        displayText: params.displayText,
        images: params.images.length > 0 ? [...params.images] : undefined,
        assetRefs: params.assetRefs.length > 0 ? [...params.assetRefs] : undefined,
        mode: params.mode ?? null,
        userIntent: params.userIntent ?? null,
        clientMessageId: params.clientMessageId ?? null,
        messageId: null,
        createdAt: now,
        updatedAt: now,
      };
    upsertPendingInput(pending);
    localPendingInputGroups.add(
      pendingInputMergeKey(params.sessionId, params.runId, params.mergeGroupId),
    );
    return pending;
  }

  async function hydrateSessionActiveRun(
    sessionId: string,
    options: { clearMissing?: boolean } = {},
  ) {
    try {
      const run = await sessionService.getSessionActiveRun(sessionId);
      if (!run || !isActiveRunStatus(run.status)) {
        if (options.clearMissing === false && sessionRunIds.value.has(sessionId)) {
          return;
        }
        const previousRunId = sessionRunIds.value.get(sessionId);
        clearTrackedRun(sessionId, previousRunId);
        if (activeSessionId.value === sessionId) {
          currentRunId.value = null;
          isStreaming.value = false;
        }
        return;
      }

      const previousRunId = sessionRunIds.value.get(sessionId);
      trackActiveRun(sessionId, run.runId);
      if (previousRunId && previousRunId !== run.runId && activeSessionId.value === sessionId) {
        activeToolCalls.value = [];
        pendingQuestion.value = null;
        pendingToolConfirms.value = [];
      }
    } catch (e) {
      console.warn("hydrate active session run failed:", e);
    }
  }

  async function hydrateActiveRuns(nextSessions: SessionSummary[]) {
    const activeSessions = nextSessions.filter((session) =>
      isActiveRuntimeStatus(session.runtimeStatus ?? null));
    await Promise.all(activeSessions.map(async (session) => {
      await hydrateSessionActiveRun(
        session.id,
        { clearMissing: false },
      );
    }));
  }

  async function loadSessionState(id: string) {
    const loadSeq = ++sessionLoadSeq;
    isStreaming.value = streamingSessionIds.value.has(id);

    const undoEntriesPromise = useChatChangesStore().loadChanges(id, { allowAutoOpen: false });

    try {
      const [detail, usage, sessionTodos, undoEntries] = await Promise.all([
        sessionService.loadSession(id),
        sessionService.getSessionUsage(id),
        sessionService.getTodos(id),
        undoEntriesPromise,
      ]);

      if (loadSeq !== sessionLoadSeq || activeSessionId.value !== id) return;
      useChatChangesStore().setLatestCompletedRunId(
        detail.id,
        detail.latestCompletedRunId ?? null,
      );
      applySessionData(detail, usage, sessionTodos, undoEntries);
    } catch (e) {
      if (loadSeq !== sessionLoadSeq || activeSessionId.value !== id) return;
      console.error("load_session failed:", e);
      clearLoadedSessionState();
      isStreaming.value = streamingSessionIds.value.has(id);
    }
  }

  // -- Stream animation --
  let streamAnimFrame: number | null = null;
  let streamAnimLastTime = 0;
  const STREAM_ANIM_INTERVAL = 25;

  function streamAnimTick(ts: number) {
    if (ts - streamAnimLastTime < STREAM_ANIM_INTERVAL) {
      streamAnimFrame = requestAnimationFrame(streamAnimTick);
      return;
    }
    streamAnimLastTime = ts;
    const target = rawStreamText.value;
    const current = streamingText.value;
    if (current.length >= target.length) {
      streamAnimFrame = null;
      return;
    }
    const remaining = target.length - current.length;
    const speed = Math.max(2, Math.ceil(remaining * 0.35));
    streamingText.value = target.substring(0, Math.min(current.length + speed, target.length));
    streamAnimFrame = requestAnimationFrame(streamAnimTick);
  }

  function startStreamAnim() {
    if (streamAnimFrame === null) {
      streamAnimFrame = requestAnimationFrame(streamAnimTick);
    }
  }

  function resetStreamAnim() {
    rawStreamText.value = "";
    streamingText.value = "";
    if (streamAnimFrame !== null) {
      cancelAnimationFrame(streamAnimFrame);
      streamAnimFrame = null;
    }
  }

  function cleanupAnim() {
    if (streamAnimFrame !== null) {
      cancelAnimationFrame(streamAnimFrame);
      streamAnimFrame = null;
    }
  }

  // -- Mutation applier --
  function persistTodoPanelState(sessionId: string | null = activeSessionId.value) {
    if (!sessionId) return;
    todoPanelVisibility.value.set(sessionId, showTodoPanel.value);
  }

  function setTodoPanelVisible(visible: boolean, sessionId: string | null = activeSessionId.value) {
    showTodoPanel.value = visible;
    if (!sessionId) return;
    todoPanelVisibility.value.set(sessionId, visible);
  }

  function setTodoMode(_mode: TodoPanelMode, _sessionId: string | null = activeSessionId.value) {
    todoMode.value = "current";
  }

  function restoreTodoPanelState(sessionId: string, hasTodos: boolean) {
    todoMode.value = "current";
    if (!hasTodos) {
      setTodoPanelVisible(false, sessionId);
      return;
    }
    showTodoPanel.value = todoPanelVisibility.value.get(sessionId) ?? false;
  }

  function clearTodoPanelState(sessionId: string | null) {
    if (!sessionId) return;
    todoPanelVisibility.value.delete(sessionId);
    sessionLatestTodoRunIds.value.delete(sessionId);
    sessionLatestCompletedRunIds.value.delete(sessionId);
  }

  async function loadToolPermissionMode() {
    try {
      toolPermissionMode.value = normalizeToolPermissionMode(await getToolPermissionMode());
    } catch {
      toolPermissionMode.value = "auto";
    }
  }

  async function setToolPermissionMode(mode: ToolPermissionMode) {
    const previous = toolPermissionMode.value;
    toolPermissionMode.value = mode;
    try {
      await saveToolPermissionMode(mode);
    } catch (e) {
      toolPermissionMode.value = previous;
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("settings.perms.saveFailed", err.message), {
        code: err.code,
        operation: "saveToolPermissionMode",
      });
    }
  }

  async function toggleToolPermissionMode() {
    await setToolPermissionMode(toolPermissionMode.value === "auto" ? "ask" : "auto");
  }

  function rememberSessionScrollState(sessionId: string | null, state: SessionScrollState) {
    if (!sessionId) return;
    sessionScrollStates.value.set(sessionId, state);
  }

  function getSessionScrollState(sessionId: string | null = activeSessionId.value): SessionScrollState | null {
    if (!sessionId) return null;
    return sessionScrollStates.value.get(sessionId) ?? null;
  }

  function clearSessionScrollState(sessionId: string | null) {
    if (!sessionId) return;
    sessionScrollStates.value.delete(sessionId);
  }

  function updateKnowledgeProposalStatuses(status: KnowledgeProposalStatus, proposalId?: string) {
    let changed = false;
    messages.value = messages.value.map((message) => {
      const proposal = message.knowledgeProposal;
      if (!proposal || proposal.status !== "pending") return message;
      if (proposalId && proposal.proposalId !== proposalId) return message;
      changed = true;
      return {
        ...message,
        knowledgeProposal: {
          ...proposal,
          status,
          updatedAt: Math.floor(Date.now() / 1000),
        },
      };
    });
    return changed;
  }

  function reconcileStreamingSessions(nextSessions: SessionSummary[]) {
    if (streamingSessionIds.value.size === 0) return;

    const nextSessionMap = new Map(nextSessions.map((session) => [session.id, session]));
    const staleIds = Array.from(streamingSessionIds.value).filter((sessionId) => {
      const session = nextSessionMap.get(sessionId);
      if (!session) return true;
      if (isActiveRuntimeStatus(session.runtimeStatus ?? null)) return false;

      // Subagent runs are tracked locally via stream events and are not present in
      // backend runtime_status. Keep them visible until their own terminal event arrives.
      const hasLocalChildRun = !!session.parentSessionId && sessionRunIds.value.has(sessionId);
      return !hasLocalChildRun;
    });

    if (staleIds.length === 0) return;

    logChatStreamDebug("reconcile stale streaming sessions", {
      staleSessionIds: staleIds,
      activeSessionId: activeSessionId.value,
      currentRunId: currentRunId.value,
      streamingSessionIds: Array.from(streamingSessionIds.value),
      sessions: staleIds.map((sessionId) => {
        const session = nextSessionMap.get(sessionId);
        return {
          sessionId,
          runtimeStatus: session?.runtimeStatus ?? null,
          parentSessionId: session?.parentSessionId ?? null,
          expectedRunId: sessionRunIds.value.get(sessionId) ?? null,
        };
      }),
    });

    const nextStreamingSessionIds = new Set(streamingSessionIds.value);
    let shouldReloadActiveSession = false;

    for (const sessionId of staleIds) {
      nextStreamingSessionIds.delete(sessionId);
      const staleRunId = sessionRunIds.value.get(sessionId) ?? null;
      clearTrackedRun(sessionId, staleRunId);
      closedRunIds.delete(sessionId);

      if (pendingManagedSessionId === sessionId) {
        pendingManagedSessionId = null;
      }
      if (pendingSessionId === sessionId) {
        pendingSessionId = null;
      }
      if (activeSessionId.value === sessionId) {
        currentRunId.value = null;
        pendingPlanRun.value = null;
        isStreaming.value = false;
        resetStreamRuntimeState();
        shouldReloadActiveSession = true;
      }
    }

    streamingSessionIds.value = nextStreamingSessionIds;

    if (shouldReloadActiveSession && activeSessionId.value) {
      void loadSessionState(activeSessionId.value);
    }
  }

  function normalizeSessionRuntimeStatuses(nextSessions: SessionSummary[]): SessionSummary[] {
    let changed = false;
    const normalized = nextSessions.map((session) => {
      const runtimeStatus = session.runtimeStatus ?? null;
      if (!isActiveRuntimeStatus(runtimeStatus)) {
        closedRunIds.delete(session.id);
        return session;
      }

      if (streamingSessionIds.value.has(session.id)) {
        return session;
      }

      const closedRunId = closedRunIds.get(session.id);
      if (!closedRunId) {
        return session;
      }

      changed = true;
      logChatStreamDebug("suppress stale runtime status after terminal event", {
        sessionId: session.id,
        runtimeStatus,
        closedRunId,
      });
      return {
        ...session,
        runtimeStatus: null,
      };
    });

    return changed ? normalized : nextSessions;
  }

  function applyMutation(m: StreamMutation) {
    const traceApplyStreamMutation = isToolCollapseTraceEnabled("applyStreamMutation");
    const messagesBeforeMutation = traceApplyStreamMutation ? traceMessageOrder(messages.value) : null;
    const activeToolCallsBeforeMutation = traceApplyStreamMutation ? traceToolCallOrder(activeToolCalls.value) : null;
    switch (m.type) {
      case "appendRawText":
        {
          const rawLenBefore = rawStreamText.value.length;
          const streamingLenBefore = streamingText.value.length;
          rawStreamText.value += m.text;
          if (!streamingText.value) streamingText.value = rawStreamText.value.charAt(0);
          traceChatStore("appendRawText", () => ({
            deltaLen: m.text.length,
            deltaPreview: previewTraceText(m.text, 48),
            rawLenBefore,
            rawLenAfter: rawStreamText.value.length,
            streamingLenBefore,
            streamingLenAfter: streamingText.value.length,
            injectedFirstVisibleChar: streamingLenBefore === 0 && streamingText.value.length > 0,
          }));
          startStreamAnim();
        }
        break;
      case "appendThinking":
        streamingThinking.value += m.text;
        break;
      case "setStreamSequence":
        streamSequence.value = Math.max(streamSequence.value, m.value);
        break;
      case "setStreamingTextOrder":
        streamingTextOrder.value = m.order;
        break;
      case "setThinkingOrder":
        thinkingOrder.value = m.order;
        break;
      case "upsertLiveRenderPart": {
        const index = liveRenderParts.value.findIndex((part) => part.id === m.part.id);
        if (index < 0) {
          liveRenderParts.value = [...liveRenderParts.value, m.part];
        } else {
          const next = [...liveRenderParts.value];
          next.splice(index, 1, { ...next[index]!, ...m.part } as AssistantRenderPart);
          liveRenderParts.value = next;
        }
        break;
      }
      case "appendLiveRenderPartContent":
        liveRenderParts.value = liveRenderParts.value.map((part) => {
          if (part.id !== m.partId) return part;
          if (part.kind !== "thinking" && part.kind !== "text") return part;
          return { ...part, content: part.content + m.text };
        });
        break;
      case "deactivateLiveThinkingParts":
        liveRenderParts.value = liveRenderParts.value.map((part) =>
          part.kind === "thinking"
            ? { ...part, active: false, duration: m.duration ?? part.duration }
            : part,
        );
        break;
      case "updateLiveToolPart":
        liveRenderParts.value = liveRenderParts.value.map((part) =>
          part.kind === "toolCall" && part.toolCall.id === m.toolCallId
            ? { ...part, toolCall: { ...part.toolCall, ...m.updates } }
            : part,
        );
        break;
      case "clearLiveRenderParts":
        liveRenderParts.value = [];
        break;
      case "setThinking":
        isThinking.value = m.value;
        if (m.startTime !== undefined) thinkingStartTime.value = m.startTime;
        break;
      case "updateThinkingDuration":
        thinkingDuration.value = m.duration;
        break;
      case "addToolCall":
        activeToolCalls.value.push(m.toolCall);
        break;
      case "updateToolCall": {
        const tc = activeToolCalls.value.find((t) => t.id === m.id);
        if (tc) Object.assign(tc, m.updates);
        break;
      }
      case "addNestedToolCall": {
        const parent = activeToolCalls.value.find((t) => t.id === m.parentId);
        if (parent) {
          if (!parent.nestedToolCalls) parent.nestedToolCalls = [];
          parent.nestedToolCalls.push(m.toolCall);
        }
        break;
      }
      case "updateNestedToolCall": {
        const parentTc = activeToolCalls.value.find((t) => t.id === m.parentId);
        const nested = parentTc?.nestedToolCalls?.find((t) => t.id === m.childId);
        if (nested) Object.assign(nested, m.updates);
        break;
      }
      case "appendToolDelta": {
        const tcDelta = activeToolCalls.value.find((t) => t.id === m.id);
        if (tcDelta) tcDelta.output = (tcDelta.output || "") + m.delta;
        break;
      }
      case "updateToolProgress": {
        const tcProgress = activeToolCalls.value.find((t) => t.id === m.id);
        if (tcProgress) tcProgress.progress = m.progress;
        break;
      }
      case "pushMessage":
        messages.value = replaceMessageById(messages.value, m.message);
        if (m.message.role === "assistant") {
          traceChatStore("pushMessage", () => ({
            messageId: m.message.id,
            contentLen: m.message.content.length,
            toolCallCount: m.message.toolCalls?.length ?? 0,
            thinkingLen: m.message.thinkingContent?.length ?? 0,
          }));
        }
        break;
      case "upsertMessage":
        messages.value = replaceMessageById(messages.value, m.message);
        break;
      case "upsertUserMessage":
        messages.value = mergeUserMessage(messages.value, m.message);
        break;
      case "removeMessage":
        messages.value = messages.value.filter((message) => message.id !== m.messageId);
        break;
      case "replaceMessages":
        messages.value = hydrateMessages(m.messages);
        break;
      case "pushToolResults":
        traceChatStore("pushToolResults", () => ({
          toolCallCount: activeToolCalls.value.length,
          toolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
          targetToolCallIds: m.toolCallIds ?? null,
        }));
        {
          const targetIds = m.toolCallIds ? new Set(m.toolCallIds) : null;
          const sourceToolCalls = targetIds
            ? activeToolCalls.value.filter((toolCall) => targetIds.has(toolCall.id))
            : activeToolCalls.value;
          for (const message of buildToolResultMessages(sourceToolCalls)) {
            messages.value = replaceMessageById(messages.value, message);
          }
        }
        break;
      case "resetRound":
        traceChatStore("resetRound", () => ({
          rawStreamLen: rawStreamText.value.length,
          streamingLen: streamingText.value.length,
          thinkingLen: streamingThinking.value.length,
          activeToolCallCount: activeToolCalls.value.length,
          activeToolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
        }));
        resetStreamAnim();
        streamingThinking.value = "";
        streamingTextOrder.value = 0;
        thinkingOrder.value = 0;
        liveRenderParts.value = [];
        thinkingStartTime.value = 0;
        thinkingDuration.value = 0;
        isThinking.value = false;
        activeToolCalls.value = [];
        break;
      case "resetRoundKeepToolCalls":
        traceChatStore("resetRoundKeepToolCalls", () => ({
          rawStreamLen: rawStreamText.value.length,
          streamingLen: streamingText.value.length,
          thinkingLen: streamingThinking.value.length,
          activeToolCallCount: activeToolCalls.value.length,
          activeToolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
        }));
        resetStreamAnim();
        streamingThinking.value = "";
        streamingTextOrder.value = 0;
        thinkingOrder.value = 0;
        liveRenderParts.value = [];
        thinkingStartTime.value = 0;
        thinkingDuration.value = 0;
        isThinking.value = false;
        break;
      case "clearPendingInputs":
        pendingQuestion.value = null;
        pendingToolConfirms.value = [];
        break;
      case "clearPendingInput":
        if (pendingQuestion.value?.questionId === m.questionId) {
          pendingQuestion.value = null;
        }
        pendingToolConfirms.value = pendingToolConfirms.value.filter(
          (item) => item.questionId !== m.questionId,
        );
        break;
      case "updateUsage":
        tokenUsage.value = m.usage;
        break;
      case "setQuestion":
        pendingQuestion.value = m.question;
        break;
      case "enqueueToolConfirm": {
        const next = pendingToolConfirms.value.filter((item) => item.questionId !== m.confirm.questionId);
        next.push(m.confirm);
        pendingToolConfirms.value = next;
        break;
      }
      case "addUndoable":
        undoableMessageIds.value.add(m.messageId);
        break;
      case "setTodos":
        todos.value = m.todos;
        if (activeSessionId.value) {
          sessionLatestTodoRunIds.value.set(activeSessionId.value, m.runId);
        }
        todoWriteVersion.value += 1;
        if (m.todos.length > 0 && useDisplaySettings().state.todoAutoOpen) {
          setTodoPanelVisible(true);
        } else {
          persistTodoPanelState();
        }
        break;
      case "setStreaming":
        if (isStreaming.value !== m.value) {
          traceChatStore("setStreaming", () => ({
            previous: isStreaming.value,
            next: m.value,
          }));
        }
        isStreaming.value = m.value;
        break;
      case "setCompacting":
        isCompacting.value = m.value;
        break;
    }
    traceStoreOrder("applyStreamMutation", () => ({
      mutation: traceStreamMutation(m),
      messagesBeforeMutation,
      messagesAfterMutation: traceMessageOrder(messages.value),
      activeToolCallsBeforeMutation,
      activeToolCallsAfterMutation: traceToolCallOrder(activeToolCalls.value),
    }));
  }

  // -- Stream event handler --
  function handleStreamEvent(event: StreamEvent): boolean {
    traceStoreOrder("streamEventReceived", () => ({
      event: traceStreamEvent(event),
      expectedRunId: sessionRunIds.value.get(event.sessionId)
        ?? (event.sessionId === activeSessionId.value ? currentRunId.value : null),
      pendingInputs: visiblePendingInputs(pendingInputsBySession.value.get(event.sessionId)).map((input, index) => ({
        index,
        id: input.id,
        runId: input.runId,
        mergeGroupId: input.mergeGroupId,
        delivery: input.delivery ?? "after_run",
        status: input.status,
        displayTextLen: (input.displayText || input.text).length,
        displayTextPreview: previewTraceText(input.displayText || input.text, 48),
      })),
    }));

    if (event.type === "runStart") {
      const closedRunId = closedRunIds.get(event.sessionId);
      if (closedRunId && closedRunId === event.runId) {
        logChatStreamDebug("ignoring runStart for already-closed run", {
          sessionId: event.sessionId,
          runId: event.runId,
          closedRunId,
        });
        return false;
      }

      const expectedRunId = sessionRunIds.value.get(event.sessionId);
      if (expectedRunId && expectedRunId !== event.runId) {
        logChatStreamDebug("ignoring runStart with unexpected run id", {
          sessionId: event.sessionId,
          runId: event.runId,
          expectedRunId,
        });
        return false;
      }

      closedRunIds.delete(event.sessionId);
      cancelRequestedRunIds.delete(event.sessionId);
      streamingSessionIds.value.add(event.sessionId);
      sessionRunIds.value.set(event.sessionId, event.runId);
      useChatChangesStore().setActiveRunId(event.sessionId, event.runId);

      if (managedStreamingSessionIds.has(event.sessionId)) {
        pendingManagedSessionId = event.sessionId;
        pendingManagedUnboundSession = false;
      } else if (pendingManagedSessionId === event.sessionId) {
        managedStreamingSessionIds.add(event.sessionId);
        pendingManagedUnboundSession = false;
      } else if (pendingManagedUnboundSession) {
        managedStreamingSessionIds.add(event.sessionId);
        pendingManagedSessionId = event.sessionId;
        pendingManagedUnboundSession = false;
      }

      if (!sessions.value.some((session) => session.id === event.sessionId)) {
        void refreshSessions();
      }

      if (!activeSessionId.value && isStreaming.value && pendingSessionId === null) {
        pendingSessionId = event.sessionId;
        setActiveSessionSelection(event.sessionId);
        if (managedStreamingSessionIds.has(event.sessionId)) {
          activeSessionType.value = "chat";
        }
      }

      if (event.sessionId === activeSessionId.value) {
        currentRunId.value = event.runId;
        if (!managedStreamingSessionIds.has(event.sessionId)) {
          resetStreamRuntimeState();
        }
        isStreaming.value = true;
      }

      if (
        pendingPlanRun.value &&
        !pendingPlanRun.value.runId &&
        pendingPlanRun.value.sessionId === event.sessionId
      ) {
        pendingPlanRun.value.runId = event.runId;
      }

      logChatStreamDebug("accepted runStart", {
        sessionId: event.sessionId,
        runId: event.runId,
        activeSessionId: activeSessionId.value,
        currentRunId: currentRunId.value,
        pendingSessionId,
        pendingManagedSessionId,
        managedStreaming: managedStreamingSessionIds.has(event.sessionId),
        streamingSessionIds: Array.from(streamingSessionIds.value),
      });
      return true;
    }

    if (event.type === "knowledgeProposal") {
      if (event.sessionId === activeSessionId.value) {
        applyMutation({ type: "upsertMessage", message: event.message });
      }
      return true;
    }

    // Session auto-assignment
    if (!activeSessionId.value && isStreaming.value && pendingSessionId === null) {
      pendingSessionId = event.sessionId;
      setActiveSessionSelection(event.sessionId);
      streamingSessionIds.value.add(event.sessionId);
      refreshSessions();
    }

    const closedRunId = closedRunIds.get(event.sessionId);
    if (closedRunId && closedRunId === event.runId) {
      logChatStreamDebug("ignoring terminal event for already-closed run", {
        sessionId: event.sessionId,
        runId: event.runId,
        eventType: event.type,
        closedRunId,
      });
      return false;
    }

    const expectedRunId = sessionRunIds.value.get(event.sessionId)
      ?? (event.sessionId === activeSessionId.value ? currentRunId.value : null);
    if (expectedRunId && event.runId !== expectedRunId) {
      logChatStreamDebug("ignoring event with unexpected run id", {
        sessionId: event.sessionId,
        runId: event.runId,
        expectedRunId,
        eventType: event.type,
      });
      return false;
    }

    if (event.type === "pendingInputQueued") {
      upsertPendingInput(event.input);
      return true;
    }

    if (event.type === "pendingInputDeleted") {
      markPendingInputDeleted(event.sessionId, event.pendingInputId);
      return true;
    }

    if (event.type === "pendingInputAccepted") {
      markPendingInputAccepted(event.sessionId, event.pendingInputId);
      for (const key of Array.from(localPendingInputGroups)) {
        if (key.includes(`\u{0}${event.runId}\u{0}`)) {
          localPendingInputGroups.delete(key);
        }
      }
      for (const key of Array.from(localFallbackPendingInputGroups)) {
        if (key.includes(`\u{0}${event.runId}\u{0}`)) {
          localFallbackPendingInputGroups.delete(key);
        }
      }
      return true;
    }

    if (event.type === "userMessage" && shouldDeferUserMessage(event)) {
      deferUserMessage(event);
      return true;
    }

    if (event.type === "done" || event.type === "error" || event.type === "cancelled") {
      const trackedRunId = sessionRunIds.value.get(event.sessionId) ?? null;
      const wasStreaming = streamingSessionIds.value.has(event.sessionId);
      flushDeferredUserMessages(event.sessionId, event.runId);
      clearTrackedRun(event.sessionId, trackedRunId ?? event.runId);
      closedRunIds.set(event.sessionId, event.runId);
      if (pendingManagedSessionId === event.sessionId) {
        pendingManagedSessionId = null;
      }
      if (pendingSessionId === event.sessionId) {
        pendingSessionId = null;
      }
      if (event.sessionId === activeSessionId.value) {
        currentRunId.value = null;
      }
      logChatStreamDebug("processed terminal stream event", {
        sessionId: event.sessionId,
        runId: event.runId,
        eventType: event.type,
        trackedRunId,
        wasStreaming,
        activeSessionId: activeSessionId.value,
        currentRunId: currentRunId.value,
        remainingStreamingSessionIds: Array.from(streamingSessionIds.value),
      });
      void refreshSessions();
    }

    if (event.type === "undoAvailable") {
      logChatStreamDebug("received undoAvailable", {
        sessionId: event.sessionId,
        runId: event.runId,
        assistantMessageId: event.assistantMessageId,
        activeSessionId: activeSessionId.value,
        isStreaming: isStreaming.value,
      });
      useChatChangesStore().setActiveRunId(event.sessionId, event.runId);
      void useChatChangesStore().refresh(event.sessionId);
    }

    if (event.type === "done") {
      sessionLatestCompletedRunIds.value.set(event.sessionId, event.runId);
      useChatChangesStore().setLatestCompletedRunId(event.sessionId, event.runId);
      void useChatChangesStore().refresh(event.sessionId, { allowAutoOpen: false });
    }

    if (event.type === "cancelled") {
      useChatChangesStore().setLatestCompletedRunId(event.sessionId, event.runId);
      void useChatChangesStore().refresh(event.sessionId);
    }

    if (event.sessionId !== activeSessionId.value) return true;

    const shouldUseIncrementalReducer =
      event.type === "compactStart"
      || event.type === "compactDone"
      || managedStreamingSessionIds.has(event.sessionId)
      || resolveSessionType(event.sessionId) === "chat";

    if (!shouldUseIncrementalReducer) {
      if (event.type === "done" || event.type === "error" || event.type === "cancelled") {
        resetStreamRuntimeState();
        isStreaming.value = false;
        if (event.type === "done") {
          void useChatChangesStore().refresh(activeSessionId.value, { allowAutoOpen: false });
        }
        if (event.type === "error" || event.type === "cancelled") {
          void loadSessionStatePreservingFailedUserDraft(event.sessionId);
        } else {
          void loadSessionState(event.sessionId);
        }
      }

      if (event.type === "error") {
        useNotificationStore().addNotice("error", event.error.message, {
          code: event.error.code,
          operation: "chat",
        });
      }
      return true;
    }

    if (event.type === "compactStart" && event.trigger === "reactive") {
      useNotificationStore().addNotice("warning", t("chat.transcript.reactiveCompactNotice"), {
        code: "reactive_compact",
        operation: "chat",
      });
    }

    // Build current state snapshot for reducer
    const state = {
      messages: messages.value,
      streamingText: streamingText.value,
      rawStreamText: rawStreamText.value,
      streamingThinking: streamingThinking.value,
      streamSequence: streamSequence.value,
      streamingTextOrder: streamingTextOrder.value,
      thinkingOrder: thinkingOrder.value,
      liveRenderParts: liveRenderParts.value,
      isStreaming: isStreaming.value,
      isCompacting: isCompacting.value,
      isThinking: isThinking.value,
      thinkingStartTime: thinkingStartTime.value,
      thinkingDuration: thinkingDuration.value,
      activeToolCalls: activeToolCalls.value,
      tokenUsage: tokenUsage.value,
      todos: todos.value,
      showTodoPanel: showTodoPanel.value,
      pendingQuestion: pendingQuestion.value,
      pendingToolConfirms: pendingToolConfirms.value,
      undoableMessageIds: undoableMessageIds.value,
    };

    switch (event.type) {
      case "textDelta":
        traceChatStore("handleStreamEvent:textDelta", () => ({
          sessionId: event.sessionId,
          runId: event.runId,
          textLen: event.text.length,
          textPreview: previewTraceText(event.text, 48),
          activeToolCallCount: activeToolCalls.value.length,
          isStreaming: isStreaming.value,
        }));
        break;
      case "toolCallRoundDone":
        traceChatStore("handleStreamEvent:toolCallRoundDone", () => ({
          sessionId: event.sessionId,
          runId: event.runId,
          messageId: event.messageId,
          fullTextLen: event.fullText.length,
          toolCallCount: event.toolCalls.length,
          activeToolCallCount: activeToolCalls.value.length,
        }));
        break;
      case "done":
        traceChatStore("handleStreamEvent:done", () => ({
          sessionId: event.sessionId,
          runId: event.runId,
          messageId: event.messageId,
          fullTextLen: event.fullText.length,
          rawStreamLen: rawStreamText.value.length,
          streamingLen: streamingText.value.length,
          activeToolCallCount: activeToolCalls.value.length,
        }));
        break;
    }

    const mutations = reduceStreamEvent(state, event);
    traceStoreOrder("streamEventMutationBatch", () => ({
      event: traceStreamEvent(event),
      mutationCount: mutations.length,
      mutations: mutations.map(traceStreamMutation),
    }));
    for (const m of mutations) {
      applyMutation(m);
    }
    if (event.type === "toolCallRoundDone") {
      flushDeferredUserMessages(event.sessionId, event.runId);
    }

    // Push stream errors to global notification
    if (event.type === "error") {
      useNotificationStore().addNotice("error", event.error.message, {
        code: event.error.code,
        operation: "chat",
      });
      void loadSessionStatePreservingFailedUserDraft(event.sessionId);
    }

    if (event.type === "done") {
      // Save plan artifact on successful plan completion
      if (
        pendingPlanRun.value &&
        pendingPlanRun.value.runId === event.runId &&
        pendingPlanRun.value.sessionId === event.sessionId
      ) {
        sessionService.savePlanArtifact(
          pendingPlanRun.value.sessionId,
          pendingPlanRun.value.agentId,
          pendingPlanRun.value.requestText,
          event.fullText,
        ).catch((e) => console.warn("[plan] save artifact failed:", e));
      }
    }

    if (event.type === "done" || event.type === "cancelled") {
      const queued = localQueuedInputsForRun(
        event.sessionId,
        event.runId,
        event.type === "cancelled",
      );
      if (queued.length > 0) {
        clearRunPendingInputs(event.sessionId, event.runId);
        const next = queued[0]!;
        globalThis.setTimeout(() => {
          void sendMessage(
            next.text,
            next.images ?? [],
            next.assetRefs ?? [],
            {
              displayText: next.displayText,
              mode: next.mode ?? undefined,
              userIntent: next.userIntent ?? null,
            },
          );
        }, 0);
      }
    }

    if (event.type === "error" || event.type === "cancelled") {
      clearRunPendingInputs(event.sessionId, event.runId);
    }

    if (event.type === "cancelled" && event.removedUserMessage) {
      // The backend revoked the user message because the cancel landed before
      // any assistant output; hand the text back to the composer.
      const removed = event.removedUserMessage;
      messages.value = messages.value.filter((message) =>
        !(isLocalPendingUserMessage(message) && isMatchingPendingUserMessage(message, removed)));
      restoreDraftFromFailedUserMessage(removed, {
        sessionId: event.sessionId,
        requireEmptyComposer: true,
      });
    } else if (event.type === "cancelled" && messages.value.some(isLocalPendingUserMessage)) {
      // A still-local pending id after a cancel means the run ended before the
      // backend confirmed the user message; reload and hand the text back to the
      // composer instead of leaving an orphaned message in the transcript.
      void loadSessionStatePreservingFailedUserDraft(event.sessionId);
    }

    return true;
  }

  // -- Actions --
  async function refreshSessions() {
    try {
      const rawSessions = await sessionService.listSessions();
      const nextSessions = normalizeSessionRuntimeStatuses(rawSessions);
      sessions.value = nextSessions;
      await restoreActiveSessionSelection(nextSessions);
      await hydrateActiveRuns(nextSessions);
      reconcileStreamingSessions(nextSessions);
    } catch (e) {
      console.error("list_sessions failed:", e);
    }
  }

  async function selectSession(id: string, options: { persist?: boolean } = {}) {
    if (id === activeSessionId.value) return;
    persistTodoPanelState();
    setActiveSessionSelection(id, { persist: options.persist });
    activeSessionType.value = sessions.value.find((session) => session.id === id)?.sessionType ?? null;
    currentRunId.value = sessionRunIds.value.get(id) ?? null;
    pendingPlanRun.value = null;
    resetStreamRuntimeState();
    showThinkingPanel.value = false;
    thinkingPanelContent.value = "";
    todoWriteVersion.value = 0;
    showTodoPanel.value = false;
    todoMode.value = "current";
    await loadSessionState(id);
  }

  async function syncActiveSessionSelection(sessionId: string | null | undefined) {
    const normalizedSessionId = sessionId?.trim() || null;
    if (normalizedSessionId === activeSessionId.value) {
      if (normalizedSessionId && !sessions.value.some((session) => session.id === normalizedSessionId)) {
        await refreshSessions();
      }
      return;
    }

    if (!normalizedSessionId) {
      newChat({ persistSelection: false });
      return;
    }

    await selectSession(normalizedSessionId, { persist: false });
    if (!sessions.value.some((session) => session.id === normalizedSessionId)) {
      await refreshSessions();
    }
  }

  function newChat(options: { persistSelection?: boolean } = {}) {
    const oldSessionId = activeSessionId.value;
    persistTodoPanelState(oldSessionId);
    setActiveSessionSelection(null, { persist: options.persistSelection !== false });
    if (options.persistSelection === false) {
      activeSessionSelectionRestoreAttempted = false;
    }
    activeSessionType.value = null;
    currentRunId.value = null;
    pendingSessionId = null;
    pendingManagedSessionId = null;
    pendingManagedUnboundSession = false;
    closedRunIds.clear();
    cancelRequestedRunIds.clear();
    pendingInputsBySession.value = new Map();
    acceptedPendingInputIds.clear();
    deferredUserMessagesByRun.clear();
    localPendingInputGroups.clear();
    localFallbackPendingInputGroups.clear();
    messages.value = [];
    resetStreamRuntimeState();
    isStreaming.value = false;
    tokenUsage.value = emptyTokenUsage();
    todos.value = [];
    todoWriteVersion.value = 0;
    showTodoPanel.value = false;
    todoMode.value = "current";
    showThinkingPanel.value = false;
    thinkingPanelContent.value = "";
    undoableMessageIds.value = new Set();
    sessionAgentId.value = null;
    pendingPlanRun.value = null;
    useAgentStore().resetToDefault();

    // Clear chat changes for the old session
    useChatChangesStore().clear(oldSessionId);
  }

  function resetWorkspaceScope() {
    const oldSessionId = activeSessionId.value;
    persistTodoPanelState(oldSessionId);
    setActiveSessionSelection(null);
    activeSessionSelectionRestoreAttempted = false;
    activeSessionType.value = null;
    currentRunId.value = null;
    pendingSessionId = null;
    pendingManagedSessionId = null;
    pendingManagedUnboundSession = false;
    closedRunIds.clear();
    cancelRequestedRunIds.clear();
    sessions.value = [];
    pendingInputsBySession.value = new Map();
    acceptedPendingInputIds.clear();
    deferredUserMessagesByRun.clear();
    localPendingInputGroups.clear();
    localFallbackPendingInputGroups.clear();
    messages.value = [];
    resetStreamRuntimeState();
    isStreaming.value = false;
    streamingSessionIds.value = new Set();
    undoableMessageIds.value = new Set();
    sessionRunIds.value = new Map();
    sessionScrollStates.value = new Map();
    tokenUsage.value = emptyTokenUsage();
    todos.value = [];
    todoWriteVersion.value = 0;
    showTodoPanel.value = false;
    todoPanelVisibility.value = new Map();
    todoMode.value = "current";
    sessionLatestTodoRunIds.value = new Map();
    sessionLatestCompletedRunIds.value = new Map();
    showThinkingPanel.value = false;
    thinkingPanelContent.value = "";
    sessionAgentId.value = null;
    pendingPlanRun.value = null;
    useAgentStore().resetToDefault();
    const chatChangesStore = useChatChangesStore();
    chatChangesStore.clear(oldSessionId);
    chatChangesStore.closeInlineDiff();
  }

  function closeTodoPanel() {
    setTodoPanelVisible(false);
  }

  function toggleTodoPanel() {
    setTodoPanelVisible(!showTodoPanel.value);
  }

  function openThinkingPanel(content: string) {
    thinkingPanelContent.value = content || "";
    showThinkingPanel.value = true;
  }

  async function renameSession(id: string, title: string) {
    try {
      await sessionService.renameSession(id, title);
      await refreshSessions();
    } catch (e) {
      console.error("rename_session failed:", e);
    }
  }

  async function archiveSession(id: string) {
    try {
      await sessionService.archiveSession(id);
      useChatChangesStore().clear(id);
      clearTrackedRun(id, sessionRunIds.value.get(id) ?? null);
      clearDeferredUserMessagesForSession(id);
      closedRunIds.delete(id);
      clearSessionScrollState(id);
      setSessionPendingInputs(id, []);
      if (activeSessionId.value === id) {
        newChat();
      }
      clearTodoPanelState(id);
      await refreshSessions();
      useNotificationStore().addNotice("success", t("chat.session.archived"), {
        operation: "archiveSession",
      });
    } catch (e) {
      console.error("archive_session failed:", e);
    }
  }

  async function deleteSession(id: string) {
    try {
      await sessionService.deleteSession(id);
      useChatChangesStore().clear(id);
      clearTrackedRun(id, sessionRunIds.value.get(id) ?? null);
      clearDeferredUserMessagesForSession(id);
      closedRunIds.delete(id);
      clearSessionScrollState(id);
      setSessionPendingInputs(id, []);
      if (activeSessionId.value === id) {
        newChat();
      }
      clearTodoPanelState(id);
      await refreshSessions();
      useNotificationStore().addNotice("success", t("chat.session.deleted"), {
        operation: "deleteSession",
      });
    } catch (e) {
      console.error("delete_session failed:", e);
    }
  }

  function localPendingGroupForRun(sessionId: string, runId: string): string | null {
    const prefix = `${sessionId}\u{0}${runId}\u{0}`;
    const existing = Array.from(localPendingInputGroups).find((key) => key.startsWith(prefix));
    return existing ? existing.slice(prefix.length) : null;
  }

  async function queueRunningMessage(
    text: string,
    displayText: string,
    images: ImageAttachment[],
    assetRefs: AssetRefAttachment[],
    overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null },
  ): Promise<boolean> {
    const sessionId = activeSessionId.value;
    const runId = currentRunId.value;
    if (!sessionId || !runId) return false;
    const { state: chatInputSettings } = useChatInputSettings();
    const delivery = chatInputSettings.runningSendMode === "insert" ? "immediate" : "after_run";

    let mergeGroupId = localPendingGroupForRun(sessionId, runId);
    if (!mergeGroupId) {
      mergeGroupId = nextPendingMessageId();
      localPendingInputGroups.add(pendingInputMergeKey(sessionId, runId, mergeGroupId));
    }
    const clientMessageId = mergeGroupId;
    const userIntent = withClientMessageId(overrides?.userIntent, clientMessageId);

    logChatStreamDebug("queue running chat input", {
      sessionId,
      runId,
      mergeGroupId,
      textLength: text.length,
      imageCount: images.length,
      assetRefCount: assetRefs.length,
    });

    try {
      const pending = await sessionService.queueChatInput({
        sessionId,
        runId,
        mergeGroupId,
        text,
        displayText,
        images: images.length > 0 ? images : null,
        assetRefs: assetRefs.length > 0 ? assetRefs : null,
        mode: overrides?.mode || null,
        userIntent,
        clientMessageId,
        delivery,
      });
      if (
        activeSessionId.value !== sessionId
        || currentRunId.value !== runId
        || !isStreaming.value
      ) {
        if (!acceptedPendingInputIds.has(pending.id)) {
          upsertPendingInput(pending);
        }
        return true;
      }
      if (!acceptedPendingInputIds.has(pending.id)) {
        upsertPendingInput(pending);
        localPendingInputGroups.add(
          pendingInputMergeKey(pending.sessionId, pending.runId, pending.mergeGroupId),
        );
      }
      return true;
    } catch (e) {
      console.warn("queue_chat_input failed:", e);
      const err = normalizeAppError(e);
      if (isPendingInputFallbackError(err.code)) {
        const pending = upsertLocalPendingInput({
          sessionId,
          runId,
          mergeGroupId,
          text,
          displayText,
          images,
          assetRefs,
          mode: overrides?.mode ?? null,
          userIntent,
          clientMessageId,
        });
        localFallbackPendingInputGroups.add(
          pendingInputMergeKey(sessionId, runId, pending.mergeGroupId),
        );
        if (!isStreaming.value || currentRunId.value !== runId) {
          clearRunPendingInputs(sessionId, runId);
          globalThis.setTimeout(() => {
            void sendMessage(
              pending.text,
              pending.images ?? [],
              pending.assetRefs ?? [],
              {
                displayText: pending.displayText,
                mode: pending.mode ?? undefined,
                userIntent: pending.userIntent ?? null,
              },
            );
          }, 0);
        }
        return true;
      }
      useNotificationStore().addNotice("error", t("app.sendFailed", err.message), {
        code: err.code,
        operation: "chat",
        skipConsoleLog: true,
      });
      restoreDraftFromFailedUserMessage(failedUserMessageFromPayload(
        mergeGroupId,
        displayText,
        images,
        assetRefs,
        userIntent,
      ), {
        sessionId,
        requireEmptyComposer: true,
      });
      return false;
    }
  }

  async function insertActiveQueuedFollowUp(): Promise<boolean> {
    const sessionId = activeSessionId.value;
    const runId = currentRunId.value;
    const pending = activeQueuedFollowUps.value.find((input) =>
      pendingInputDelivery(input) !== "immediate");
    if (!sessionId || !runId || !pending) return false;

    try {
      const inserted = await sessionService.insertPendingChatInput(
        sessionId,
        runId,
        pending.id,
      );
      if (!acceptedPendingInputIds.has(inserted.id)) {
        upsertPendingInput(inserted);
      }
      localPendingInputGroups.add(
        pendingInputMergeKey(inserted.sessionId, inserted.runId, inserted.mergeGroupId),
      );
      return true;
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.sendFailed", err.message), {
        code: err.code,
        operation: "chat",
        skipConsoleLog: true,
      });
      return false;
    }
  }

  async function deleteActiveQueuedFollowUp(): Promise<boolean> {
    const sessionId = activeSessionId.value;
    const targets = activeQueuedFollowUps.value;
    if (!sessionId || targets.length === 0) return false;

    try {
      await Promise.all(
        targets.map((input) =>
          sessionService.deletePendingChatInput(
            input.sessionId,
            input.runId,
            input.id,
          )),
      );
      for (const input of targets) {
        markPendingInputDeleted(input.sessionId, input.id);
      }
      return true;
    } catch (e) {
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.sendFailed", err.message), {
        code: err.code,
        operation: "chat",
        skipConsoleLog: true,
      });
      return false;
    }
  }

  async function sendMessage(
    text: string,
    images: ImageAttachment[] = [],
    assetRefs: AssetRefAttachment[] = [],
    overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null },
  ) {
    const modelStore = useModelStore();
    const agentStore = useAgentStore();
    const { state: knowledgeAccessState } = useKnowledgeAccessMode();

    const displayText = overrides?.displayText ?? text;

    // Auto-close file changes panel when starting a new round
    const { state: displaySettings } = useDisplaySettings();
    if (displaySettings.changesAutoClose) {
      useChatChangesStore().closePanel();
    }

    const requestSessionId = activeSessionId.value;
    const staleSessionId = activeSessionId.value;
    const markedStale = updateKnowledgeProposalStatuses("stale");
    if (markedStale && staleSessionId) {
      sessionService.staleKnowledgeProposals(staleSessionId).catch((e) => {
        console.warn("stale_knowledge_proposals failed:", e);
      });
    }

    if (isStreaming.value && activeSessionId.value && currentRunId.value) {
      await queueRunningMessage(text, displayText, images, assetRefs, overrides);
      return;
    }

    pendingLaunchCancelRequested.value = false;
    const pendingMessageId = nextPendingMessageId();
    const userIntent = withClientMessageId(overrides?.userIntent, pendingMessageId);
    const userIntentSignature = JSON.stringify(userIntent);

    const pendingUserMessage: ChatMessage = {
      id: pendingMessageId,
      role: "user",
      content: displayText,
      createdAt: Date.now() / 1000,
      images: images.length > 0 ? images : undefined,
      assetRefs: assetRefs.length > 0 ? assetRefs : undefined,
      thinkingSignature: userIntentSignature,
      intentMeta: userIntent,
    };
    messages.value.push(pendingUserMessage);
    resetStreamRuntimeState();
    isStreaming.value = true;

    pendingManagedSessionId = activeSessionId.value;
    pendingManagedUnboundSession = !activeSessionId.value;
    if (activeSessionId.value) {
      managedStreamingSessionIds.add(activeSessionId.value);
    }

    // For plan mode, temporarily use planModel if configured
    let model = modelStore.selectedModelId || null;
    if (overrides?.mode === "plan") {
      const planModel = modelStore.modelDefaults.planModel;
      if (planModel && modelStore.availableModels.some((m) => m.id === planModel)) {
        model = planModel;
      }
    }

    logChatStreamDebug("chat request start", {
      sessionId: activeSessionId.value,
      mode: overrides?.mode || "build",
      textLength: text.length,
      imageCount: images.length,
      assetRefCount: assetRefs.length,
      model,
      agentId: agentStore.selectedAgentId || null,
    });

    try {
      const { sessionId: sid, runId } = await sessionService.chat({
        sessionId: activeSessionId.value,
        text,
        agentId: agentStore.selectedAgentId || null,
        model,
        effort: modelStore.effortSupported ? modelStore.effort : null,
        images: images.length > 0 ? images : null,
        assetRefs: assetRefs.length > 0 ? assetRefs : null,
        mode: overrides?.mode || null,
        userIntent,
        subagentModels: Object.keys(modelStore.modelDefaults.subagentModels).length > 0 ? modelStore.modelDefaults.subagentModels : null,
        knowledgeMode: knowledgeAccessState.mode,
      });
      logChatStreamDebug("chat request resolved", {
        sessionId: sid,
        runId,
        activeSessionId: activeSessionId.value,
        pendingManagedSessionId,
      });

      streamingSessionIds.value.add(sid);
      sessionRunIds.value.set(sid, runId);
      useChatChangesStore().setActiveRunId(sid, runId);
      closedRunIds.delete(sid);
      cancelRequestedRunIds.delete(sid);
      pendingSessionId = null;
      pendingManagedSessionId = sid;
      pendingManagedUnboundSession = false;
      managedStreamingSessionIds.add(sid);
      if (overrides?.mode === "plan") {
        pendingPlanRun.value = {
          runId,
          sessionId: sid,
          agentId: agentStore.selectedAgentId || "",
          requestText: overrides.displayText ?? text,
        };
      }
      if (!activeSessionId.value || activeSessionId.value === sid) {
        setActiveSessionSelection(sid);
        activeSessionType.value = resolveSessionType(sid) ?? "chat";
        currentRunId.value = runId;
        sessionAgentId.value = agentStore.selectedAgentId || null;
        await refreshSessions();
      }
      if (pendingLaunchCancelRequested.value) {
        pendingLaunchCancelRequested.value = false;
        void cancelSession(sid);
      }
    } catch (e) {
      console.error("chat failed:", e);
      pendingLaunchCancelRequested.value = false;
      logChatStreamDebug("chat request failed", {
        sessionId: activeSessionId.value,
        error: e instanceof Error ? e.message : String(e),
      });
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.sendFailed", err.message), {
        code: err.code,
        operation: "chat",
        skipConsoleLog: true,
      });
      isStreaming.value = false;
      resetStreamAnim();
      messages.value = messages.value.filter((message) => message.id !== pendingMessageId);
      restoreDraftFromFailedUserMessage(pendingUserMessage, {
        sessionId: requestSessionId,
        requireEmptyComposer: true,
      });
      pendingSessionId = null;
      if (activeSessionId.value) {
        managedStreamingSessionIds.delete(activeSessionId.value);
        await loadSessionState(activeSessionId.value);
      }
      pendingManagedSessionId = null;
      pendingManagedUnboundSession = false;
    }
  }

  async function compactSession() {
    if (!activeSessionId.value || isStreaming.value) return;

    const modelStore = useModelStore();
    const agentStore = useAgentStore();
    const { state: knowledgeAccessState } = useKnowledgeAccessMode();
    const sessionId = activeSessionId.value;

    const { state: displaySettings } = useDisplaySettings();
    if (displaySettings.changesAutoClose) {
      useChatChangesStore().closePanel();
    }

    pendingLaunchCancelRequested.value = false;
    resetStreamRuntimeState();
    isStreaming.value = true;
    pendingManagedSessionId = sessionId;
    pendingManagedUnboundSession = false;
    managedStreamingSessionIds.add(sessionId);

    const model = modelStore.selectedModelId || null;

    logChatStreamDebug("compact request start", {
      sessionId,
      model,
      agentId: agentStore.selectedAgentId || null,
    });

    try {
      const { sessionId: sid, runId } = await sessionService.chat({
        sessionId,
        text: "",
        agentId: agentStore.selectedAgentId || null,
        model,
        effort: modelStore.effortSupported ? modelStore.effort : null,
        images: null,
        assetRefs: null,
        mode: "compact",
        userIntent: null,
        subagentModels: Object.keys(modelStore.modelDefaults.subagentModels).length > 0 ? modelStore.modelDefaults.subagentModels : null,
        knowledgeMode: knowledgeAccessState.mode,
      });

      logChatStreamDebug("compact request resolved", {
        sessionId: sid,
        runId,
        activeSessionId: activeSessionId.value,
        pendingManagedSessionId,
      });

      streamingSessionIds.value.add(sid);
      sessionRunIds.value.set(sid, runId);
      useChatChangesStore().setActiveRunId(sid, runId);
      closedRunIds.delete(sid);
      cancelRequestedRunIds.delete(sid);
      pendingSessionId = null;
      pendingManagedSessionId = sid;
      pendingManagedUnboundSession = false;
      managedStreamingSessionIds.add(sid);
      currentRunId.value = runId;
      sessionAgentId.value = agentStore.selectedAgentId || null;
      await refreshSessions();
      if (pendingLaunchCancelRequested.value) {
        pendingLaunchCancelRequested.value = false;
        void cancelSession(sid);
      }
    } catch (e) {
      console.error("compact failed:", e);
      pendingLaunchCancelRequested.value = false;
      logChatStreamDebug("compact request failed", {
        sessionId,
        error: e instanceof Error ? e.message : String(e),
      });
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.sendFailed", err.message), {
        code: err.code,
        operation: "compact",
        skipConsoleLog: true,
      });
      isStreaming.value = false;
      isCompacting.value = false;
      resetStreamAnim();
      pendingSessionId = null;
      managedStreamingSessionIds.delete(sessionId);
      pendingManagedSessionId = null;
      pendingManagedUnboundSession = false;
    }
  }

  function forkedSessionTitle(session: SessionSummary | undefined): string {
    const title = session?.title?.trim() || t("chat.session.newSession");
    return t("chat.session.forkTitle", title);
  }

  async function forkSession() {
    const sourceSessionId = activeSessionId.value;
    if (!sourceSessionId || isStreaming.value) return;

    const sourceSession = sessions.value.find((session) => session.id === sourceSessionId);
    if (sourceSession?.parentSessionId) {
      useNotificationStore().addNotice("warning", t("chat.session.forkChildBlocked"), {
        code: "session.fork_child",
        operation: "forkSession",
      });
      return;
    }

    try {
      const forkedId = await sessionService.forkSession(
        sourceSessionId,
        forkedSessionTitle(sourceSession),
      );
      await refreshSessions();
      await selectSession(forkedId);
      useNotificationStore().addNotice("success", t("chat.session.forked"), {
        operation: "forkSession",
      });
    } catch (e) {
      const err = normalizeAppError(e);
      const isChildFork = err.code === "session.fork_child";
      useNotificationStore().addNotice(
        isChildFork ? "warning" : "error",
        isChildFork ? t("chat.session.forkChildBlocked") : t("chat.session.forkFailed", err.message),
        {
          code: err.code,
          operation: "forkSession",
          skipConsoleLog: true,
        },
      );
    }
  }

  async function forkSessionFromMessage(messageId: string) {
    const sourceSessionId = activeSessionId.value;
    if (!sourceSessionId || isStreaming.value || !messageId) return;

    const sourceSession = sessions.value.find((session) => session.id === sourceSessionId);
    if (sourceSession?.parentSessionId) {
      useNotificationStore().addNotice("warning", t("chat.session.forkChildBlocked"), {
        code: "session.fork_child",
        operation: "forkSession",
      });
      return;
    }

    try {
      const forkedId = await sessionService.forkSessionFromMessage(
        sourceSessionId,
        messageId,
        forkedSessionTitle(sourceSession),
      );
      await refreshSessions();
      await selectSession(forkedId);
      useNotificationStore().addNotice("success", t("chat.session.forked"), {
        operation: "forkSession",
      });
    } catch (e) {
      const err = normalizeAppError(e);
      const isChildFork = err.code === "session.fork_child";
      useNotificationStore().addNotice(
        isChildFork ? "warning" : "error",
        isChildFork ? t("chat.session.forkChildBlocked") : t("chat.session.forkFailed", err.message),
        {
          code: err.code,
          operation: "forkSession",
          skipConsoleLog: true,
        },
      );
    }
  }

  async function cancelSession(sessionId: string) {
    if (!sessionId) return;
    const trackedRunId = sessionRunIds.value.get(sessionId)
      ?? (activeSessionId.value === sessionId ? currentRunId.value : null);
    if (!trackedRunId && pendingManagedSessionId === sessionId) {
      // The run is still launching; the backend has nothing to cancel yet, so
      // remember the request and re-fire once the launch resolves.
      pendingLaunchCancelRequested.value = true;
    }
    if (trackedRunId && cancelRequestedRunIds.get(sessionId) === trackedRunId) return;
    if (trackedRunId) {
      cancelRequestedRunIds.set(sessionId, trackedRunId);
    }
    if (pendingPlanRun.value?.sessionId === sessionId) {
      pendingPlanRun.value = null;
    }
    try {
      await sessionService.cancelChat(sessionId);
      if (activeSessionId.value === sessionId) {
        pendingQuestion.value = null;
        pendingToolConfirms.value = [];
      }
    } catch (e) {
      console.error("cancel_chat failed:", e);
      cancelRequestedRunIds.delete(sessionId);
      throw e;
    }
  }

  async function cancelSessions(sessionIds: string[]) {
    const targets = Array.from(new Set(sessionIds.filter((sessionId) => !!sessionId)));
    if (targets.length === 0) return;
    await Promise.all(targets.map((sessionId) => cancelSession(sessionId)));
  }

  async function cancelChat() {
    if (!isStreaming.value) return;
    if (!activeSessionId.value) {
      // First message of a brand-new session: no session id exists until the
      // chat command resolves, so just flag the cancel for the launch hook.
      pendingLaunchCancelRequested.value = true;
      return;
    }
    await cancelSession(activeSessionId.value);
  }

  async function answerQuestion(answer: string) {
    const q = pendingQuestion.value;
    if (!q) return;
    pendingQuestion.value = null;
    try {
      await sessionService.answerQuestion(q.questionId, answer);
    } catch (e) {
      console.error("answer_question failed:", e);
    }
  }

  async function answerToolConfirm(questionId: string, answer: string) {
    const tc = pendingToolConfirms.value.find((item) => item.questionId === questionId);
    if (!tc) return;
    pendingToolConfirms.value = pendingToolConfirms.value.filter((item) => item.questionId !== questionId);
    try {
      await sessionService.answerQuestion(tc.questionId, answer);
    } catch (e) {
      console.error("answer_tool_confirm failed:", e);
    }
  }

  async function answerAllToolConfirms(questionIds: string[], answer: string) {
    if (questionIds.length === 0) return;
    const targets = pendingToolConfirms.value.filter((item) => questionIds.includes(item.questionId));
    if (targets.length === 0) return;
    pendingToolConfirms.value = pendingToolConfirms.value.filter((item) => !questionIds.includes(item.questionId));
    await Promise.all(
      targets.map((item) =>
        sessionService.answerQuestion(item.questionId, answer).catch((e) => {
          console.error("answer_tool_confirm failed:", e);
        })),
    );
  }

  async function ignoreKnowledgeProposal(proposalId: string) {
    if (!activeSessionId.value) return;
    updateKnowledgeProposalStatuses("invalidated", proposalId);
    try {
      await sessionService.ignoreKnowledgeProposal(activeSessionId.value, proposalId);
    } catch (e) {
      console.error("ignore_knowledge_proposal failed:", e);
      await loadSessionState(activeSessionId.value);
    }
  }

  async function applyKnowledgeProposal(proposalId: string) {
    if (!activeSessionId.value) return;
    messages.value = messages.value.map((message) => {
      const proposal = message.knowledgeProposal;
      if (!proposal || proposal.proposalId !== proposalId) return message;
      return {
        ...message,
        knowledgeProposal: {
          ...proposal,
          status: "applying",
          updatedAt: Math.floor(Date.now() / 1000),
        },
      };
    });
    try {
      await sessionService.applyKnowledgeProposal(activeSessionId.value, proposalId);
    } catch (e) {
      console.error("apply_knowledge_proposal failed:", e);
      await loadSessionState(activeSessionId.value);
    }
  }

  async function refreshSessionAfterExternalChange(sessionId: string): Promise<void> {
    if (!sessionId) return;
    await refreshSessions();
    if (activeSessionId.value !== sessionId) return;
    useChatChangesStore().closeInlineDiff();
    await loadSessionState(sessionId);
  }

  async function checkUndoConflicts(assistantMessageId: string): Promise<UndoConflictInfo[]> {
    if (!activeSessionId.value) return [];
    return undoService.undoCheckConflicts(activeSessionId.value, assistantMessageId);
  }

  async function checkUndoDirty(assistantMessageId: string): Promise<ChangedFile[]> {
    if (!activeSessionId.value) return [];
    return undoService.undoCheckDirty(activeSessionId.value, assistantMessageId);
  }

  async function performUndo(
    assistantMessageId: string,
    options?: { force?: boolean; acceptDirty?: boolean },
  ): Promise<boolean> {
    if (!activeSessionId.value) return false;
    try {
      await undoService.undoPerform(
        activeSessionId.value,
        assistantMessageId,
        options?.force ?? false,
        options?.acceptDirty ?? false,
      );
      // loadChanges returns undo entries, so we reuse it instead of calling undoList twice
      const [detail, undoEntries] = await Promise.all([
        sessionService.loadSession(activeSessionId.value),
        useChatChangesStore().loadChanges(activeSessionId.value, { allowAutoOpen: false }),
      ]);
      useChatChangesStore().setLatestCompletedRunId(
        activeSessionId.value,
        detail.latestCompletedRunId ?? null,
      );
      sessionLatestCompletedRunIds.value.set(
        activeSessionId.value,
        detail.latestCompletedRunId ?? null,
      );
      setSessionPendingInputs(activeSessionId.value, detail.pendingInputs ?? []);
      messages.value = hydrateMessages(detail.messages);
      undoableMessageIds.value = new Set(undoEntries.map((e) => e.assistantMessageId));
      activeSessionType.value = detail.sessionType;
      return true;
    } catch (e) {
      console.error("undo_perform failed:", e);
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.undoFailed", err.message), {
        code: err.code,
        operation: "undo",
        skipConsoleLog: true,
      });
      return false;
    }
  }

  async function rollbackToMessage(
    messageId: string,
    options?: { includeFiles?: boolean; fileUndoTarget?: string | null; acceptDirty?: boolean },
  ): Promise<boolean> {
    if (!activeSessionId.value || !messageId) return false;
    const sessionId = activeSessionId.value;
    const includeFiles = options?.includeFiles === true;
    const fileUndoTarget = options?.fileUndoTarget ?? null;
    if (includeFiles && !fileUndoTarget) {
      useNotificationStore().addNotice("warning", t("chat.undo.noFileUndo"), {
        code: "undo.no_file_target",
        operation: "rollbackToMessage",
      });
      return false;
    }

    try {
      if (includeFiles) {
        await undoService.undoPerformToMessage(
          sessionId,
          fileUndoTarget!,
          messageId,
          false,
          options?.acceptDirty ?? false,
        );
      } else {
        await sessionService.rollbackSessionToMessage(sessionId, messageId);
      }

      const [detail, undoEntries] = await Promise.all([
        sessionService.loadSession(sessionId),
        useChatChangesStore().loadChanges(sessionId, { allowAutoOpen: false }),
      ]);
      useChatChangesStore().setLatestCompletedRunId(
        sessionId,
        detail.latestCompletedRunId ?? null,
      );
      sessionLatestCompletedRunIds.value.set(
        sessionId,
        detail.latestCompletedRunId ?? null,
      );
      setSessionPendingInputs(sessionId, detail.pendingInputs ?? []);
      messages.value = hydrateMessages(detail.messages);
      undoableMessageIds.value = new Set(undoEntries.map((e) => e.assistantMessageId));
      activeSessionType.value = detail.sessionType;
      await refreshSessions();
      return true;
    } catch (e) {
      console.error("rollback_to_message failed:", e);
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.undoFailed", err.message), {
        code: err.code,
        operation: "rollbackToMessage",
        skipConsoleLog: true,
      });
      return false;
    }
  }

  async function undoLatestConversationTurn(): Promise<boolean> {
    if (!activeSessionId.value) return false;
    try {
      const sessionId = activeSessionId.value;
      const detail = await sessionService.undoLatestConversationTurn(sessionId);
      const undoEntries = await useChatChangesStore().loadChanges(sessionId, { allowAutoOpen: false });
      useChatChangesStore().setLatestCompletedRunId(
        sessionId,
        detail.latestCompletedRunId ?? null,
      );
      sessionLatestCompletedRunIds.value.set(
        sessionId,
        detail.latestCompletedRunId ?? null,
      );
      setSessionPendingInputs(sessionId, detail.pendingInputs ?? []);
      messages.value = hydrateMessages(detail.messages);
      undoableMessageIds.value = new Set(undoEntries.map((e) => e.assistantMessageId));
      activeSessionType.value = detail.sessionType;
      return true;
    } catch (e) {
      console.error("undo_latest_conversation_turn failed:", e);
      const err = normalizeAppError(e);
      useNotificationStore().addNotice("error", t("app.undoFailed", err.message), {
        code: err.code,
        operation: "undo",
        skipConsoleLog: true,
      });
      return false;
    }
  }

  return {
    sessions,
    activeSessionId,
    messages,
    streamingText,
    rawStreamText,
    streamingThinking,
    streamSequence,
    streamingTextOrder,
    thinkingOrder,
    liveRenderParts,
    isStreaming,
    isCompacting,
    currentRunId,
    isCancelling,
    isThinking,
    thinkingStartTime,
    thinkingDuration,
    showThinkingPanel,
    thinkingPanelContent,
    activeToolCalls,
    tokenUsage,
    todos,
    currentTodos,
    visibleTodos,
    hasAnyTodos,
    visibleTodoCount,
    todoWriteVersion,
    todoCelebrationVersion,
    todoCelebrationEnabled,
    todoMode,
    showTodoPanel,
    closeTodoPanel,
    toggleTodoPanel,
    setTodoMode,
    pendingQuestion,
    pendingToolConfirms,
    activeQueuedFollowUps,
    activeQueuedFollowUp,
    insertActiveQueuedFollowUp,
    deleteActiveQueuedFollowUp,
    streamingSessionIds,
    undoableMessageIds,
    rememberSessionScrollState,
    getSessionScrollState,
    clearSessionScrollState,
    sessionAgentId,
    toolPermissionMode,
    sessionAgentLocked,
    pendingPlanRun,
    clearPendingPlan: () => { pendingPlanRun.value = null; },
    handleStreamEvent,
    refreshSessions,
    loadToolPermissionMode,
    setToolPermissionMode,
    toggleToolPermissionMode,
    selectSession,
    syncActiveSessionSelection,
    newChat,
    resetWorkspaceScope,
    openThinkingPanel,
    renameSession,
    archiveSession,
    deleteSession,
    sendMessage,
    compactSession,
    forkSession,
    forkSessionFromMessage,
    cancelSession,
    cancelSessions,
    cancelChat,
    answerQuestion,
    answerToolConfirm,
    answerAllToolConfirms,
    ignoreKnowledgeProposal,
    applyKnowledgeProposal,
    refreshSessionAfterExternalChange,
    checkUndoConflicts,
    checkUndoDirty,
    performUndo,
    rollbackToMessage,
    undoLatestConversationTurn,
    cleanupAnim,
  };
});
