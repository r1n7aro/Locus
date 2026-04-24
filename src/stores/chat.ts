import { ref, computed } from "vue";
import { defineStore } from "pinia";
import { useModelStore } from "./model";
import { useAgentStore } from "./agent";
import { useNotificationStore } from "./notification";
import { normalizeAppError } from "../services/errors";
import { getToolPermissionMode, saveToolPermissionMode } from "../services/permissions";
import * as sessionService from "../services/session";
import * as undoService from "../services/undo";
import { buildToolResultMessages, reduceStreamEvent, type StreamMutation } from "../composables/useStreamReducer";
import { hydrateChatMessagesIntent } from "../composables/chatInputIntents";
import type { SessionScrollState } from "../composables/chatScrollState";
import { t } from "../i18n";
import { useChatChangesStore } from "./chatChanges";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";
import type {
  SessionSummary, SessionDetail, ChatMessage, TokenUsage,
  TodoItem, StreamEvent, ImageAttachment, ToolCallDisplay,
  PendingQuestion, PendingToolConfirm,
  UserIntentMeta,
  KnowledgeProposalStatus,
  UndoConflictInfo,
  TodoSnapshot,
  TodoPanelMode,
  SessionEventRecord,
  SessionRunSummary,
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

function isActiveRuntimeStatus(status: SessionSummary["runtimeStatus"]): boolean {
  return status === "running"
    || status === "waiting_input"
    || status === "cancelling"
    || status === "starting"
    || status === "queued";
}

function isActiveRunStatus(status: SessionRunSummary["status"] | null | undefined): boolean {
  return status === "running"
    || status === "waiting_input"
    || status === "cancelling"
    || status === "starting"
    || status === "queued";
}

function streamEventFromRecord(record: SessionEventRecord): StreamEvent | null {
  if (!record.payload || typeof record.payload !== "object") return null;
  const payload = record.payload as Record<string, unknown>;
  if (typeof payload.type !== "string") return null;
  return {
    ...payload,
    runId: record.runId,
  } as StreamEvent;
}

function activeReplayEvents(
  records: SessionEventRecord[],
  afterSeq: number,
): SessionEventRecord[] {
  if (afterSeq > 0) return records;
  const lastCompletedRoundIndex = records
    .map((record) => record.eventType)
    .lastIndexOf("toolCallRoundDone");
  if (lastCompletedRoundIndex < 0) return records;
  return records.slice(lastCompletedRoundIndex + 1);
}

function normalizeToolPermissionMode(mode: string | null | undefined): ToolPermissionMode {
  return mode === "ask" ? "ask" : "auto";
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
  const isStreaming = ref(false);
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
  const replayedSessionEventSeqs = new Map<string, number>();
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
  let streamReplayDepth = 0;
  const isCancelling = computed(() => {
    if (!activeSessionId.value || !currentRunId.value) return false;
    return cancelRequestedRunIds.get(activeSessionId.value) === currentRunId.value;
  });

  function nextPendingMessageId(): string {
    pendingMessageSeq += 1;
    return `user_pending_${Date.now()}_${pendingMessageSeq}`;
  }

  function resolveSessionType(sessionId: string | null): string | null {
    if (!sessionId) return null;
    if (sessionId === activeSessionId.value && activeSessionType.value) {
      return activeSessionType.value;
    }
    return sessions.value.find((session) => session.id === sessionId)?.sessionType ?? null;
  }

  function resetStreamRuntimeState() {
    resetStreamAnim();
    streamingThinking.value = "";
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
    messages.value = hydrateMessages(detail.messages);
    tokenUsage.value = { ...usage, contextTokens: 0, contextLimit: 0 };
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
  }

  function clearLoadedSessionState() {
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
  }

  function runEventReplayKey(sessionId: string, runId: string): string {
    return `${sessionId}\u{0}${runId}`;
  }

  function forgetRunReplaySeq(sessionId: string, runId?: string | null) {
    if (runId) {
      replayedSessionEventSeqs.delete(runEventReplayKey(sessionId, runId));
      return;
    }
    for (const key of Array.from(replayedSessionEventSeqs.keys())) {
      if (key.startsWith(`${sessionId}\u{0}`)) {
        replayedSessionEventSeqs.delete(key);
      }
    }
  }

  function trackActiveRun(sessionId: string, runId: string) {
    streamingSessionIds.value.add(sessionId);
    sessionRunIds.value.set(sessionId, runId);
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
    forgetRunReplaySeq(sessionId, runId);
  }

  async function listRunEvents(sessionId: string, runId: string, afterSeq: number) {
    const all: SessionEventRecord[] = [];
    let cursor = afterSeq;
    const pageSize = 2_000;

    for (;;) {
      const page = await sessionService.listSessionEvents(sessionId, cursor, pageSize);
      const runEvents = page.filter((record) => record.runId === runId);
      all.push(...runEvents);
      if (page.length === 0) break;
      cursor = Math.max(cursor, ...page.map((record) => record.seq));
      if (page.length < pageSize) break;
    }

    return all;
  }

  async function replayActiveRunEvents(sessionId: string, runId: string) {
    const key = runEventReplayKey(sessionId, runId);
    const afterSeq = replayedSessionEventSeqs.get(key) ?? 0;
    const records = await listRunEvents(sessionId, runId, afterSeq);
    if (records.length === 0) return;

    replayedSessionEventSeqs.set(
      key,
      Math.max(afterSeq, ...records.map((record) => record.seq)),
    );

    const events = activeReplayEvents(records, afterSeq)
      .map(streamEventFromRecord)
      .filter((event): event is StreamEvent => !!event);
    if (events.length === 0) return;

    streamReplayDepth += 1;
    try {
      for (const event of events) {
        handleStreamEvent(event);
      }
    } finally {
      streamReplayDepth -= 1;
    }
  }

  async function hydrateSessionActiveRun(
    sessionId: string,
    replay: boolean,
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
      if (previousRunId && previousRunId !== run.runId) {
        forgetRunReplaySeq(sessionId, previousRunId);
      }

      if (replay && activeSessionId.value === sessionId) {
        await replayActiveRunEvents(sessionId, run.runId);
      }
    } catch (e) {
      console.warn("hydrate active session run failed:", e);
    }
  }

  async function hydrateActiveRuns(nextSessions: SessionSummary[]) {
    const activeSessions = nextSessions.filter((session) =>
      isActiveRuntimeStatus(session.runtimeStatus ?? null));
    await Promise.all(activeSessions.map(async (session) => {
      const alreadyTracked = sessionRunIds.value.has(session.id);
      await hydrateSessionActiveRun(
        session.id,
        activeSessionId.value === session.id && !alreadyTracked,
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
      await hydrateSessionActiveRun(id, true);
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
  // canvasAutoOpenCallback is set by App.vue (UI shell behavior stays outside store)
  let canvasAutoOpenCallback: ((toolCallId: string, spec: unknown) => void) | null = null;

  function setCanvasAutoOpenCallback(cb: (toolCallId: string, spec: unknown) => void) {
    canvasAutoOpenCallback = cb;
  }

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
      switch (m.type) {
      case "appendRawText":
        {
          const rawLenBefore = rawStreamText.value.length;
          const streamingLenBefore = streamingText.value.length;
          rawStreamText.value += m.text;
          if (!streamingText.value) streamingText.value = rawStreamText.value.charAt(0);
          logToolCollapseTrace("chat-store", "appendRawText", {
            deltaLen: m.text.length,
            deltaPreview: previewTraceText(m.text, 48),
            rawLenBefore,
            rawLenAfter: rawStreamText.value.length,
            streamingLenBefore,
            streamingLenAfter: streamingText.value.length,
            injectedFirstVisibleChar: streamingLenBefore === 0 && streamingText.value.length > 0,
          });
          startStreamAnim();
        }
        break;
      case "appendThinking":
        streamingThinking.value += m.text;
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
      case "pushMessage":
        messages.value.push(m.message);
        if (m.message.role === "assistant") {
          logToolCollapseTrace("chat-store", "pushMessage", {
            messageId: m.message.id,
            contentLen: m.message.content.length,
            toolCallCount: m.message.toolCalls?.length ?? 0,
            thinkingLen: m.message.thinkingContent?.length ?? 0,
          });
        }
        break;
      case "upsertMessage":
        messages.value = replaceMessageById(messages.value, m.message);
        break;
      case "replaceMessages":
        messages.value = hydrateMessages(m.messages);
        break;
      case "pushToolResults":
        logToolCollapseTrace("chat-store", "pushToolResults", {
          toolCallCount: activeToolCalls.value.length,
          toolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
        });
        messages.value.push(...buildToolResultMessages(activeToolCalls.value));
        break;
      case "resetRound":
        logToolCollapseTrace("chat-store", "resetRound", {
          rawStreamLen: rawStreamText.value.length,
          streamingLen: streamingText.value.length,
          thinkingLen: streamingThinking.value.length,
          activeToolCallCount: activeToolCalls.value.length,
          activeToolCallIds: activeToolCalls.value.map((toolCall) => toolCall.id),
        });
        resetStreamAnim();
        streamingThinking.value = "";
        thinkingStartTime.value = 0;
        thinkingDuration.value = 0;
        isThinking.value = false;
        activeToolCalls.value = [];
        break;
      case "clearPendingInputs":
        pendingQuestion.value = null;
        pendingToolConfirms.value = [];
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
          logToolCollapseTrace("chat-store", "setStreaming", {
            previous: isStreaming.value,
            next: m.value,
          });
        }
        isStreaming.value = m.value;
        break;
      case "canvasAutoOpen":
        if (streamReplayDepth === 0) {
          canvasAutoOpenCallback?.(m.toolCallId, m.spec);
        }
        break;
    }
  }

  // -- Stream event handler --
  function handleStreamEvent(event: StreamEvent): boolean {
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
        activeSessionId.value = event.sessionId;
        if (managedStreamingSessionIds.has(event.sessionId)) {
          activeSessionType.value = "chat";
        }
      }

      if (event.sessionId === activeSessionId.value) {
        currentRunId.value = event.runId;
        if (!managedStreamingSessionIds.has(event.sessionId) && resolveSessionType(event.sessionId) !== "chat") {
          resetStreamRuntimeState();
          isStreaming.value = true;
        }
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
      activeSessionId.value = event.sessionId;
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

    if (event.type === "done" || event.type === "error" || event.type === "cancelled") {
      const trackedRunId = sessionRunIds.value.get(event.sessionId) ?? null;
      const wasStreaming = streamingSessionIds.value.has(event.sessionId);
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
      void useChatChangesStore().refresh(event.sessionId);
    }

    if (event.type === "done") {
      sessionLatestCompletedRunIds.value.set(event.sessionId, event.runId);
      useChatChangesStore().setLatestCompletedRunId(event.sessionId, event.runId);
      void useChatChangesStore().refresh(event.sessionId, { allowAutoOpen: false });
    }

    if (event.sessionId !== activeSessionId.value) return true;

    const shouldUseIncrementalReducer =
      managedStreamingSessionIds.has(event.sessionId) || resolveSessionType(event.sessionId) === "chat";

    if (!shouldUseIncrementalReducer) {
      if (event.type === "done" || event.type === "error" || event.type === "cancelled") {
        resetStreamRuntimeState();
        isStreaming.value = false;
        if (event.type === "done") {
          void useChatChangesStore().refresh(activeSessionId.value, { allowAutoOpen: false });
        }
        void loadSessionState(event.sessionId);
      }

      if (event.type === "error") {
        useNotificationStore().addNotice("error", event.error.message, {
          code: event.error.code,
          operation: "chat",
        });
      }
      return true;
    }

    // Build current state snapshot for reducer
    const state = {
      messages: messages.value,
      streamingText: streamingText.value,
      rawStreamText: rawStreamText.value,
      streamingThinking: streamingThinking.value,
      isStreaming: isStreaming.value,
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
        logToolCollapseTrace("chat-store", "handleStreamEvent:textDelta", {
          sessionId: event.sessionId,
          runId: event.runId,
          textLen: event.text.length,
          textPreview: previewTraceText(event.text, 48),
          activeToolCallCount: activeToolCalls.value.length,
          isStreaming: isStreaming.value,
        });
        break;
      case "toolCallRoundDone":
        logToolCollapseTrace("chat-store", "handleStreamEvent:toolCallRoundDone", {
          sessionId: event.sessionId,
          runId: event.runId,
          messageId: event.messageId,
          fullTextLen: event.fullText.length,
          toolCallCount: event.toolCalls.length,
          activeToolCallCount: activeToolCalls.value.length,
        });
        break;
      case "done":
        logToolCollapseTrace("chat-store", "handleStreamEvent:done", {
          sessionId: event.sessionId,
          runId: event.runId,
          messageId: event.messageId,
          fullTextLen: event.fullText.length,
          rawStreamLen: rawStreamText.value.length,
          streamingLen: streamingText.value.length,
          activeToolCallCount: activeToolCalls.value.length,
        });
        break;
    }

    const mutations = reduceStreamEvent(state, event);
    for (const m of mutations) {
      applyMutation(m);
    }

    // Push stream errors to global notification
    if (event.type === "error") {
      useNotificationStore().addNotice("error", event.error.message, {
        code: event.error.code,
        operation: "chat",
      });
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

    return true;
  }

  // -- Actions --
  async function refreshSessions() {
    try {
      const rawSessions = await sessionService.listSessions();
      const nextSessions = normalizeSessionRuntimeStatuses(rawSessions);
      sessions.value = nextSessions;
      await hydrateActiveRuns(nextSessions);
      reconcileStreamingSessions(nextSessions);
    } catch (e) {
      console.error("list_sessions failed:", e);
    }
  }

  async function selectSession(id: string) {
    if (id === activeSessionId.value) return;
    persistTodoPanelState();
    activeSessionId.value = id;
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

  function newChat() {
    const oldSessionId = activeSessionId.value;
    persistTodoPanelState(oldSessionId);
    activeSessionId.value = null;
    activeSessionType.value = null;
    currentRunId.value = null;
    pendingSessionId = null;
    pendingManagedSessionId = null;
    pendingManagedUnboundSession = false;
    closedRunIds.clear();
    cancelRequestedRunIds.clear();
    replayedSessionEventSeqs.clear();
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
    activeSessionId.value = null;
    activeSessionType.value = null;
    currentRunId.value = null;
    pendingSessionId = null;
    pendingManagedSessionId = null;
    pendingManagedUnboundSession = false;
    closedRunIds.clear();
    cancelRequestedRunIds.clear();
    replayedSessionEventSeqs.clear();
    sessions.value = [];
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
      closedRunIds.delete(id);
      clearSessionScrollState(id);
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
      closedRunIds.delete(id);
      clearSessionScrollState(id);
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

  async function sendMessage(
    text: string,
    images: ImageAttachment[] = [],
    overrides?: { displayText?: string; mode?: string; userIntent?: UserIntentMeta | null },
  ) {
    const modelStore = useModelStore();
    const agentStore = useAgentStore();

    const displayText = overrides?.displayText ?? text;

    // Auto-close file changes panel when starting a new round
    const { state: displaySettings } = useDisplaySettings();
    if (displaySettings.changesAutoClose) {
      useChatChangesStore().closePanel();
    }

    const staleSessionId = activeSessionId.value;
    const markedStale = updateKnowledgeProposalStatuses("stale");
    if (markedStale && staleSessionId) {
      sessionService.staleKnowledgeProposals(staleSessionId).catch((e) => {
        console.warn("stale_knowledge_proposals failed:", e);
      });
    }

    messages.value.push({
      id: nextPendingMessageId(),
      role: "user",
      content: displayText,
      createdAt: Date.now() / 1000,
      images: images.length > 0 ? images : undefined,
      thinkingSignature: overrides?.userIntent ? JSON.stringify(overrides.userIntent) : undefined,
      intentMeta: overrides?.userIntent ?? undefined,
    });
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
        mode: overrides?.mode || null,
        userIntent: overrides?.userIntent || null,
        subagentModels: Object.keys(modelStore.modelDefaults.subagentModels).length > 0 ? modelStore.modelDefaults.subagentModels : null,
      });
      logChatStreamDebug("chat request resolved", {
        sessionId: sid,
        runId,
        activeSessionId: activeSessionId.value,
        pendingManagedSessionId,
      });

      const previousRunId = sessionRunIds.value.get(sid) ?? null;
      streamingSessionIds.value.add(sid);
      sessionRunIds.value.set(sid, runId);
      if (previousRunId && previousRunId !== runId) {
        forgetRunReplaySeq(sid, previousRunId);
      }
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
        activeSessionId.value = sid;
        activeSessionType.value = resolveSessionType(sid) ?? "chat";
        currentRunId.value = runId;
        sessionAgentId.value = agentStore.selectedAgentId || null;
        await refreshSessions();
      }
    } catch (e) {
      console.error("chat failed:", e);
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
      pendingSessionId = null;
      if (activeSessionId.value) {
        managedStreamingSessionIds.delete(activeSessionId.value);
      }
      pendingManagedSessionId = null;
      pendingManagedUnboundSession = false;
    }
  }

  async function cancelSession(sessionId: string) {
    if (!sessionId) return;
    const trackedRunId = sessionRunIds.value.get(sessionId)
      ?? (activeSessionId.value === sessionId ? currentRunId.value : null);
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
    if (!activeSessionId.value || !isStreaming.value) return;
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

  async function checkUndoConflicts(assistantMessageId: string): Promise<UndoConflictInfo[]> {
    if (!activeSessionId.value) return [];
    return undoService.undoCheckConflicts(activeSessionId.value, assistantMessageId);
  }

  async function performUndo(
    assistantMessageId: string,
    options?: { force?: boolean },
  ): Promise<boolean> {
    if (!activeSessionId.value) return false;
    try {
      await undoService.undoPerform(
        activeSessionId.value,
        assistantMessageId,
        options?.force ?? false,
      );
      // loadChanges returns undo entries, so we reuse it instead of calling undoList twice
      const [detail, undoEntries] = await Promise.all([
        sessionService.loadSession(activeSessionId.value),
        useChatChangesStore().loadChanges(activeSessionId.value),
      ]);
      useChatChangesStore().setLatestCompletedRunId(
        activeSessionId.value,
        detail.latestCompletedRunId ?? null,
      );
      sessionLatestCompletedRunIds.value.set(
        activeSessionId.value,
        detail.latestCompletedRunId ?? null,
      );
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

  return {
    sessions,
    activeSessionId,
    messages,
    streamingText,
    rawStreamText,
    streamingThinking,
    isStreaming,
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
    setCanvasAutoOpenCallback,
    refreshSessions,
    loadToolPermissionMode,
    setToolPermissionMode,
    toggleToolPermissionMode,
    selectSession,
    newChat,
    resetWorkspaceScope,
    openThinkingPanel,
    renameSession,
    archiveSession,
    deleteSession,
    sendMessage,
    cancelSession,
    cancelSessions,
    cancelChat,
    answerQuestion,
    answerToolConfirm,
    answerAllToolConfirms,
    ignoreKnowledgeProposal,
    applyKnowledgeProposal,
    checkUndoConflicts,
    performUndo,
    cleanupAnim,
  };
});
