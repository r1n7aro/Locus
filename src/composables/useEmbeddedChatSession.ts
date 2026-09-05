import {
  computed,
  markRaw,
  onUnmounted,
  reactive,
  shallowRef,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from "vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import * as sessionService from "../services/session";
import * as undoService from "../services/undo";
import { isToolCollapseTraceEnabled, logToolCollapseTrace, previewTraceText } from "../services/toolCollapseTrace";
import { StreamingTextChunks } from "./streamingTextChunks";
import { useThrottledStreamingText } from "./streamingRenderThrottle";
import { hydrateChatMessagesIntent, withClientMessageId } from "./chatInputIntents";
import { useChatInputSettings } from "./useChatInputSettings";
import { useDisplaySettings } from "./useDisplaySettings";
import {
  buildPendingSessionInputDraft,
  buildUserMessageDraft,
  type UserMessageDraft,
} from "./chatMessageDraft";
import {
  applyAsyncTaskUpdateToMessages,
  asyncTaskDisplayStatus,
} from "./asyncTaskUpdates";
import { useModelStore } from "../stores/model";
import type { WorkspaceRef } from "../services/project";
import {
  bindSessionAsyncTaskUpdateConsumer,
  bindSessionStreamEventConsumer,
  sessionStreamSourceMatchesWorkspace,
  subscribeSessionAsyncTaskUpdateConsumer,
  subscribeSessionExecutionStateConsumer,
  subscribeSessionStreamEventConsumer,
} from "../services/sessionStreamEventHub";
import {
  buildInterruptedTrailingToolResultMessages,
  buildToolResultMessages,
  mergeUserMessage,
  reduceStreamEvent,
  type StreamMutation,
  type StreamState,
} from "./useStreamReducer";
import { resolveToolCallDisplayShape } from "./toolCallBatches";
import { modelSupportsFastMode } from "../utils/modelDisplay";
import type {
  AsyncTaskUpdatedEvent,
  ChatComposerSendPayload,
  ChatMessage,
  EffortLevel,
  ImageAttachment,
  AssetRefAttachment,
  KnowledgeAccessMode,
  KnowledgeDocumentType,
  PendingQuestion,
  PendingToolConfirm,
  StreamEvent,
  TokenUsage,
  ToolCallDisplay,
  UserIntentMeta,
  AssistantRenderPart,
  PendingSessionInput,
  SessionDetail,
  SessionTurnPreview,
} from "../types";

export interface EmbeddedChatRequest {
  text: string;
  displayText?: string;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  images?: ImageAttachment[] | null;
  assetRefs?: AssetRefAttachment[] | null;
}

interface EmbeddedSubmittedUserMessage {
  draft: UserMessageDraft;
  ownerKey: string;
}

interface EmbeddedChatState extends StreamState {
  key: string;
  sessionId: string | null;
  hydrated: boolean;
  currentRunId: string | null;
  error: string | null;
  loading: boolean;
  sessionAgentId: string | null;
  sessionModelId: string | null;
  sessionEffort: EffortLevel | null;
  sessionFastMode: boolean | null;
  sessionMultiAgentEnabled: boolean;
  parentSessionId: string | null;
  latestCompletedRunId: string | null;
  latestTodoRunId: string | null;
  todoWriteVersion: number;
  resumeAvailable: boolean;
  planModeActive: boolean;
  planFilePath: string | null;
  historyHasMore: boolean;
  historyOldestRowId: number | null;
  historyLoading: boolean;
  userMessageIds: string[];
  pendingRun: boolean;
  isCancelling: boolean;
  pendingInputs: PendingSessionInput[];
  acceptedPendingInputIds: Set<string>;
  deferredUserMessagesByRun: Map<string, ChatMessage[]>;
  localMergeGroupId: string | null;
  localFallbackMergeGroupId: string | null;
  submittedUserMessagesByRun: Map<string, EmbeddedSubmittedUserMessage>;
  cancelPendingLaunch: boolean;
  compactPendingLaunch: boolean;
  compactQueued: boolean;
  /** Chunked mirrors of the streaming text; see the chat store's counterparts.
   * markRaw keeps the reactive proxy from deep-wrapping them — growth is
   * observed through each stream's own version ref. */
  thinkingStream: StreamingTextChunks;
  livePartStreams: Map<string, StreamingTextChunks>;
}

export interface EmbeddedChatKnowledgeFocus {
  docType: KnowledgeDocumentType;
  path: string;
}

export interface UseEmbeddedChatSessionOptions {
  sessionKey: MaybeRefOrGetter<string>;
  /** Attach the pane to an existing durable session. */
  initialSessionId?: MaybeRefOrGetter<string | null | undefined>;
  /** Required when a new embedded session is launched from an empty editor. */
  workspaceRef?: MaybeRefOrGetter<WorkspaceRef | null | undefined>;
  sessionType?: string;
  sessionTitle?: MaybeRefOrGetter<string | null | undefined>;
  selectedModelId: MaybeRefOrGetter<string>;
  selectedAgentId?: MaybeRefOrGetter<string | null | undefined>;
  effort?: MaybeRefOrGetter<EffortLevel | null | undefined>;
  effortSupported?: MaybeRefOrGetter<boolean | undefined>;
  fastMode?: MaybeRefOrGetter<boolean | undefined>;
  multiAgentEnabled?: MaybeRefOrGetter<boolean | undefined>;
  /** Knowledge access for this pane. Every launch snapshots and forwards it explicitly. */
  knowledgeMode?: MaybeRefOrGetter<KnowledgeAccessMode | null | undefined>;
  /** Knowledge document this session is scoped to; injected into the agent env by the backend. */
  knowledgeFocus?: MaybeRefOrGetter<EmbeddedChatKnowledgeFocus | null | undefined>;
  buildRequest: (input: string) => EmbeddedChatRequest | null;
}

function emptyTokenUsage(): TokenUsage {
  return {
    totalInputTokens: 0,
    totalOutputTokens: 0,
    totalCacheReadTokens: 0,
    totalCacheWriteTokens: 0,
    timedOutputTokens: 0,
    modelActiveDurationMs: 0,
    totalCostUsd: 0,
    pricedRounds: 0,
    contextTokens: 0,
    contextLimit: 0,
  };
}

function createState(key: string): EmbeddedChatState {
  return reactive({
    key,
    sessionId: null,
    hydrated: false,
    currentRunId: null,
    error: null,
    loading: false,
    sessionAgentId: null,
    sessionModelId: null,
    sessionEffort: null,
    sessionFastMode: null,
    sessionMultiAgentEnabled: false,
    parentSessionId: null,
    latestCompletedRunId: null,
    latestTodoRunId: null,
    todoWriteVersion: 0,
    resumeAvailable: false,
    planModeActive: false,
    planFilePath: null,
    historyHasMore: false,
    historyOldestRowId: null,
    historyLoading: false,
    userMessageIds: [],
    pendingRun: false,
    isCancelling: false,
    pendingInputs: [],
    acceptedPendingInputIds: new Set<string>(),
    deferredUserMessagesByRun: new Map<string, ChatMessage[]>(),
    localMergeGroupId: null,
    localFallbackMergeGroupId: null,
    submittedUserMessagesByRun: markRaw(new Map<string, EmbeddedSubmittedUserMessage>()),
    cancelPendingLaunch: false,
    compactPendingLaunch: false,
    compactQueued: false,
    messages: [] as ChatMessage[],
    streamingText: "",
    rawStreamText: "",
    streamingThinking: "",
    streamSequence: 0,
    streamingTextOrder: 0,
    thinkingOrder: 0,
    liveRenderParts: [] as AssistantRenderPart[],
    isStreaming: false,
    isCompacting: false,
    isThinking: false,
    thinkingStartTime: 0,
    thinkingDuration: 0,
    activeToolCalls: [] as ToolCallDisplay[],
    tokenUsage: emptyTokenUsage(),
    todos: [],
    showTodoPanel: false,
    pendingQuestion: null as PendingQuestion | null,
    pendingToolConfirms: [] as PendingToolConfirm[],
    undoableMessageIds: new Set<string>(),
    thinkingStream: markRaw(new StreamingTextChunks()),
    livePartStreams: markRaw(new Map<string, StreamingTextChunks>()),
  });
}

// Editor tabs can move between groups, which briefly remounts their Vue
// subtree. Keep the session reducer state keyed by the stable editor/session
// key so drafts and live output survive that reparenting.
const sharedEmbeddedChatStates = new Map<string, EmbeddedChatState>();
const sharedEmbeddedChatSessionStates = new Map<string, EmbeddedChatState>();
const sharedEmbeddedChatDrafts = new Map<string, { value: string }>();
const sharedEmbeddedRestoredDrafts = new Map<string, { value: UserMessageDraft | null }>();
const sharedEmbeddedChatRetainCounts = new WeakMap<EmbeddedChatState, number>();
const MAX_SHARED_EMBEDDED_CHAT_STATES = 64;

function sharedEmbeddedChatDraft(key: string): { value: string } {
  const existing = sharedEmbeddedChatDrafts.get(key);
  if (existing) {
    sharedEmbeddedChatDrafts.delete(key);
    sharedEmbeddedChatDrafts.set(key, existing);
    return existing;
  }
  const created = reactive({ value: "" });
  sharedEmbeddedChatDrafts.set(key, created);
  if (sharedEmbeddedChatDrafts.size > MAX_SHARED_EMBEDDED_CHAT_STATES) {
    const oldest = sharedEmbeddedChatDrafts.keys().next().value as string | undefined;
    if (oldest && oldest !== key) sharedEmbeddedChatDrafts.delete(oldest);
  }
  return created;
}

function sharedEmbeddedRestoredDraft(key: string): { value: UserMessageDraft | null } {
  const existing = sharedEmbeddedRestoredDrafts.get(key);
  if (existing) return existing;
  const created = reactive<{ value: UserMessageDraft | null }>({ value: null });
  sharedEmbeddedRestoredDrafts.set(key, created);
  if (sharedEmbeddedRestoredDrafts.size > MAX_SHARED_EMBEDDED_CHAT_STATES) {
    const oldest = sharedEmbeddedRestoredDrafts.keys().next().value as string | undefined;
    if (oldest && oldest !== key) sharedEmbeddedRestoredDrafts.delete(oldest);
  }
  return created;
}

function restoreEmbeddedSubmittedDraft(submitted: EmbeddedSubmittedUserMessage): void {
  const composer = sharedEmbeddedChatDraft(submitted.ownerKey);
  if (composer.value) return;
  sharedEmbeddedRestoredDraft(submitted.ownerKey).value = submitted.draft;
}

function removeEvictedSharedState(state: EmbeddedChatState): void {
  if ([...sharedEmbeddedChatStates.values()].some((candidate) => candidate === state)) return;
  if ((sharedEmbeddedChatRetainCounts.get(state) ?? 0) > 0) return;
  if (state.sessionId && sharedEmbeddedChatSessionStates.get(state.sessionId) === state) {
    sharedEmbeddedChatSessionStates.delete(state.sessionId);
  }
}

function retainSharedEmbeddedChatState(state: EmbeddedChatState): void {
  sharedEmbeddedChatRetainCounts.set(state, (sharedEmbeddedChatRetainCounts.get(state) ?? 0) + 1);
}

function releaseSharedEmbeddedChatState(state: EmbeddedChatState): void {
  const nextCount = Math.max(0, (sharedEmbeddedChatRetainCounts.get(state) ?? 0) - 1);
  if (nextCount > 0) {
    sharedEmbeddedChatRetainCounts.set(state, nextCount);
    return;
  }
  sharedEmbeddedChatRetainCounts.delete(state);
  removeEvictedSharedState(state);
}

function rememberSharedEmbeddedChatState(key: string, state: EmbeddedChatState): EmbeddedChatState {
  sharedEmbeddedChatStates.delete(key);
  sharedEmbeddedChatStates.set(key, state);
  if (sharedEmbeddedChatStates.size > MAX_SHARED_EMBEDDED_CHAT_STATES) {
    const oldest = sharedEmbeddedChatStates.keys().next().value as string | undefined;
    if (oldest && oldest !== key) {
      const evicted = sharedEmbeddedChatStates.get(oldest);
      sharedEmbeddedChatStates.delete(oldest);
      if (evicted) removeEvictedSharedState(evicted);
    }
  }
  return state;
}

function bindSharedEmbeddedChatSession(
  state: EmbeddedChatState,
  sessionId: string,
): EmbeddedChatState {
  const normalizedSessionId = sessionId.trim();
  if (!normalizedSessionId) return state;
  const existing = sharedEmbeddedChatSessionStates.get(normalizedSessionId);
  if (existing) return existing;
  state.sessionId = normalizedSessionId;
  sharedEmbeddedChatSessionStates.set(normalizedSessionId, state);
  return state;
}

function sharedEmbeddedChatState(key: string, sessionId?: string | null): EmbeddedChatState {
  const normalizedSessionId = sessionId?.trim() ?? "";
  if (normalizedSessionId) {
    const sessionState = sharedEmbeddedChatSessionStates.get(normalizedSessionId);
    if (sessionState) return rememberSharedEmbeddedChatState(key, sessionState);
  }
  const existing = sharedEmbeddedChatStates.get(key);
  if (existing) {
    const bound = normalizedSessionId
      ? bindSharedEmbeddedChatSession(existing, normalizedSessionId)
      : existing;
    return rememberSharedEmbeddedChatState(key, bound);
  }
  const created = createState(key);
  const bound = normalizedSessionId
    ? bindSharedEmbeddedChatSession(created, normalizedSessionId)
    : created;
  return rememberSharedEmbeddedChatState(key, bound);
}

function ensureEmbeddedPartStream(state: EmbeddedChatState, partId: string): StreamingTextChunks {
  let stream = state.livePartStreams.get(partId);
  if (!stream) {
    stream = new StreamingTextChunks();
    state.livePartStreams.set(partId, stream);
  }
  return stream;
}

function resetEmbeddedStreams(state: EmbeddedChatState) {
  state.thinkingStream.reset();
  state.livePartStreams.clear();
}

function replaceMessageById(list: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = list.findIndex((item) => item.id === message.id);
  if (index < 0) return [...list, message];
  const next = [...list];
  next.splice(index, 1, message);
  return next;
}

function traceEmbeddedMessageOrder(messages: ChatMessage[]) {
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

function traceEmbeddedToolCallOrder(toolCalls: ToolCallDisplay[]) {
  return toolCalls.map((toolCall, index) => ({
    index,
    id: toolCall.id,
    name: toolCall.name,
    status: toolCall.status,
    order: toolCall.order ?? null,
    nestedIds: toolCall.nestedToolCalls?.map((nested) => nested.id) ?? [],
  }));
}

function traceEmbeddedStreamEvent(event: StreamEvent) {
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
    case "pendingInputDeleted":
      return {
        ...base,
        pendingInputId: event.pendingInputId,
      };
    case "toolCallRoundDone":
      return {
        ...base,
        messageId: event.messageId,
        fullTextLen: event.fullText.length,
        toolCallIds: event.toolCalls.map((toolCall) => toolCall.id),
        renderPartKinds: event.renderParts?.map((part) => part.kind) ?? [],
      };
    default:
      return base;
  }
}

function traceEmbeddedStreamMutation(mutation: StreamMutation) {
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
        toolCall: traceEmbeddedToolCallOrder([mutation.toolCall])[0],
      };
    case "updateToolCall":
      return {
        type: mutation.type,
        id: mutation.id,
        updates: mutation.updates,
      };
    default:
      return { type: mutation.type };
  }
}

function traceEmbeddedOrder(
  state: EmbeddedChatState,
  event: string,
  detail: Record<string, unknown> = {},
) {
  logToolCollapseTrace("embedded-chat:order", event, {
    key: state.key,
    sessionId: state.sessionId,
    currentRunId: state.currentRunId,
    isStreaming: state.isStreaming,
    messageCount: state.messages.length,
    messages: traceEmbeddedMessageOrder(state.messages),
    activeToolCalls: traceEmbeddedToolCallOrder(state.activeToolCalls),
    deferredUserMessages: Array.from(state.deferredUserMessagesByRun.entries()).map(([runId, deferredMessages]) => ({
      runId,
      count: deferredMessages.length,
      messages: traceEmbeddedMessageOrder(deferredMessages),
    })),
    ...detail,
  });
}

function deferUserMessage(
  state: EmbeddedChatState,
  event: Extract<StreamEvent, { type: "userMessage" }>,
) {
  const messagesForRun = state.deferredUserMessagesByRun.get(event.runId) ?? [];
  state.deferredUserMessagesByRun.set(event.runId, mergeUserMessage(messagesForRun, event.message));
  traceEmbeddedOrder(state, "embeddedDeferUserMessageDuringToolRound", {
    event: traceEmbeddedStreamEvent(event),
    deferredForRun: traceEmbeddedMessageOrder(state.deferredUserMessagesByRun.get(event.runId) ?? []),
  });
}

function flushDeferredUserMessages(state: EmbeddedChatState, runId: string) {
  const deferredMessages = state.deferredUserMessagesByRun.get(runId);
  if (!deferredMessages || deferredMessages.length === 0) return;

  const messagesBeforeFlush = traceEmbeddedMessageOrder(state.messages);
  for (const message of deferredMessages) {
    state.messages = mergeUserMessage(state.messages, message);
  }
  state.deferredUserMessagesByRun.delete(runId);
  traceEmbeddedOrder(state, "embeddedFlushDeferredUserMessages", {
    runId,
    flushedMessages: traceEmbeddedMessageOrder(deferredMessages),
    messagesBeforeFlush,
    messagesAfterFlush: traceEmbeddedMessageOrder(state.messages),
  });
}

function shouldDeferUserMessage(
  state: EmbeddedChatState,
  event: Extract<StreamEvent, { type: "userMessage" }>,
) {
  if (event.runId !== state.currentRunId) return false;
  return state.activeToolCalls.length > 0;
}

function mergePendingInputList(
  list: PendingSessionInput[],
  input: PendingSessionInput,
): PendingSessionInput[] {
  const index = list.findIndex((item) =>
    item.id === input.id
    || (item.runId === input.runId && item.mergeGroupId === input.mergeGroupId));
  if (index < 0) return [...list, input];
  const next = [...list];
  next.splice(index, 1, input);
  return next;
}

function visiblePendingInputs(inputs: PendingSessionInput[]) {
  return inputs.filter((input) => input.status === "queued" || input.status === "delivering");
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

function cloneRuntimeToolCalls(toolCalls: ToolCallDisplay[] | undefined): ToolCallDisplay[] {
  return (toolCalls ?? []).map((toolCall) => {
    const displayShape = resolveToolCallDisplayShape({
      name: toolCall.name,
      arguments: toolCall.arguments,
    });
    return {
      ...toolCall,
      name: displayShape.name,
      arguments: displayShape.arguments,
      images: toolCall.images?.map((image) => ({ ...image })),
      progress: toolCall.progress ? { ...toolCall.progress } : toolCall.progress,
      nestedToolCalls: toolCall.nestedToolCalls
        ? cloneRuntimeToolCalls(toolCall.nestedToolCalls)
        : undefined,
    };
  });
}

function cloneRuntimeJson<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function applySessionRuntimeSnapshot(state: EmbeddedChatState, detail: SessionDetail) {
  const runtime = detail.runtime;
  if (!runtime) {
    state.currentRunId = null;
    state.isStreaming = false;
    state.pendingRun = false;
    state.isCancelling = false;
    state.pendingQuestion = null;
    state.pendingToolConfirms = [];
    state.isCompacting = false;
    state.compactPendingLaunch = false;
    state.compactQueued = false;
    resetRoundState(state);
    return;
  }

  state.sessionId = detail.id;
  state.currentRunId = runtime.activeRun.runId;
  state.isStreaming = true;
  state.pendingRun = false;
  state.isCancelling = runtime.activeRun.status === "cancelling";
  state.rawStreamText = runtime.streamingText ?? "";
  state.streamingText = runtime.streamingText ?? "";
  state.streamingThinking = runtime.streamingThinking ?? "";
  state.thinkingStream.reset();
  if (state.streamingThinking) {
    state.thinkingStream.append(state.streamingThinking);
  }
  state.streamSequence = runtime.streamSequence ?? 0;
  state.streamingTextOrder = runtime.streamingTextOrder ?? 0;
  state.thinkingOrder = runtime.thinkingOrder ?? 0;
  state.liveRenderParts = cloneRuntimeJson(runtime.liveRenderParts ?? []);
  // Restored parts carry accumulated content as their baseline; fresh streams
  // collect only post-restore growth.
  state.livePartStreams.clear();
  for (const part of state.liveRenderParts) {
    if (part.kind === "text" || part.kind === "thinking") {
      ensureEmbeddedPartStream(state, part.id);
    }
  }
  state.isThinking = runtime.isThinking === true;
  state.thinkingStartTime = state.isThinking ? Date.now() : 0;
  state.thinkingDuration = runtime.thinkingDuration ?? 0;
  state.activeToolCalls = cloneRuntimeToolCalls(runtime.activeToolCalls);
  state.pendingQuestion = runtime.pendingQuestion
    ? cloneRuntimeJson(runtime.pendingQuestion)
    : null;
  state.pendingToolConfirms = cloneRuntimeJson(runtime.pendingToolConfirms ?? []);
  state.isCompacting = runtime.isCompacting === true;
  state.compactPendingLaunch = false;
  state.compactQueued = runtime.compactQueued === true;
}

function updateProposalStatus(
  state: EmbeddedChatState,
  status: "stale" | "applying" | "applied" | "invalidated",
  proposalId?: string,
) {
  let changed = false;
  state.messages = state.messages.map((message) => {
    const proposal = message.knowledgeProposal;
    if (!proposal) return message;
    if (proposalId && proposal.proposalId !== proposalId) return message;
    if (!proposalId && proposal.status !== "pending") return message;
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

function resetRoundState(state: EmbeddedChatState) {
  state.streamingText = "";
  state.rawStreamText = "";
  state.streamingThinking = "";
  state.streamingTextOrder = 0;
  state.thinkingOrder = 0;
  state.liveRenderParts = [];
  state.isThinking = false;
  state.thinkingStartTime = 0;
  state.thinkingDuration = 0;
  state.activeToolCalls = [];
  resetEmbeddedStreams(state);
}

function applyMutation(state: EmbeddedChatState, mutation: StreamMutation) {
  // Sampling the pre-mutation order is O(messages) per delta; only pay for it
  // when the trace is actually enabled.
  const traceApplyMutation = isToolCollapseTraceEnabled("embeddedApplyStreamMutation");
  const messagesBeforeMutation = traceApplyMutation ? traceEmbeddedMessageOrder(state.messages) : null;
  const activeToolCallsBeforeMutation = traceApplyMutation ? traceEmbeddedToolCallOrder(state.activeToolCalls) : null;
  switch (mutation.type) {
    case "appendRawText":
      state.rawStreamText += mutation.text;
      state.streamingText = state.rawStreamText;
      break;
    case "appendThinking":
      state.streamingThinking += mutation.text;
      state.thinkingStream.append(mutation.text);
      break;
    case "setStreamSequence":
      state.streamSequence = Math.max(state.streamSequence, mutation.value);
      break;
    case "setStreamingTextOrder":
      state.streamingTextOrder = mutation.order;
      break;
    case "setThinkingOrder":
      state.thinkingOrder = mutation.order;
      break;
    case "upsertLiveRenderPart": {
      if (mutation.part.kind === "text" || mutation.part.kind === "thinking") {
        ensureEmbeddedPartStream(state, mutation.part.id);
      }
      const index = state.liveRenderParts.findIndex((part) => part.id === mutation.part.id);
      if (index < 0) {
        state.liveRenderParts = [...state.liveRenderParts, mutation.part];
      } else {
        const next = [...state.liveRenderParts];
        next.splice(index, 1, { ...next[index]!, ...mutation.part } as AssistantRenderPart);
        state.liveRenderParts = next;
      }
      break;
    }
    case "appendLiveRenderPartContent":
      // Growth goes into the part's chunk stream; finalize events carry the
      // authoritative full text (see the chat store's counterpart).
      ensureEmbeddedPartStream(state, mutation.partId).append(mutation.text);
      break;
    case "deactivateLiveThinkingParts":
      // Keep the array's identity when there is nothing to deactivate.
      if (state.liveRenderParts.some((part) => part.kind === "thinking" && part.active)) {
        state.liveRenderParts = state.liveRenderParts.map((part) =>
          part.kind === "thinking"
            ? { ...part, active: false, duration: mutation.duration ?? part.duration }
            : part,
        );
      }
      break;
    case "updateLiveToolPart":
      state.liveRenderParts = state.liveRenderParts.map((part) =>
        part.kind === "toolCall" && part.toolCall.id === mutation.toolCallId
          ? { ...part, toolCall: { ...part.toolCall, ...mutation.updates } }
          : part,
      );
      break;
    case "clearLiveRenderParts":
      state.liveRenderParts = [];
      state.livePartStreams.clear();
      break;
    case "setThinking":
      state.isThinking = mutation.value;
      if (mutation.startTime !== undefined) {
        state.thinkingStartTime = mutation.startTime;
      }
      break;
    case "updateThinkingDuration":
      state.thinkingDuration = mutation.duration;
      break;
    case "addToolCall":
      state.activeToolCalls.push(mutation.toolCall);
      break;
    case "updateToolCall": {
      const toolCall = state.activeToolCalls.find((item) => item.id === mutation.id);
      if (toolCall) Object.assign(toolCall, mutation.updates);
      break;
    }
    case "addNestedToolCall": {
      const parent = state.activeToolCalls.find((item) => item.id === mutation.parentId);
      if (!parent) break;
      if (!parent.nestedToolCalls) parent.nestedToolCalls = [];
      parent.nestedToolCalls.push(mutation.toolCall);
      break;
    }
    case "updateNestedToolCall": {
      const parent = state.activeToolCalls.find((item) => item.id === mutation.parentId);
      const child = parent?.nestedToolCalls?.find((item) => item.id === mutation.childId);
      if (child) Object.assign(child, mutation.updates);
      break;
    }
    case "appendToolDelta": {
      const toolCall = state.activeToolCalls.find((item) => item.id === mutation.id);
      if (toolCall) {
        toolCall.output = (toolCall.output || "") + mutation.delta;
      }
      break;
    }
    case "updateToolProgress": {
      const toolCall = state.activeToolCalls.find((item) => item.id === mutation.id);
      if (toolCall) {
        toolCall.progress = mutation.progress;
      }
      break;
    }
    case "pushMessage":
      state.messages = replaceMessageById(state.messages, mutation.message);
      if (mutation.message.role === "user" && !state.userMessageIds.includes(mutation.message.id)) {
        state.userMessageIds = [...state.userMessageIds, mutation.message.id];
      }
      break;
    case "upsertMessage": {
      state.messages = replaceMessageById(state.messages, mutation.message);
      if (mutation.message.role === "user" && !state.userMessageIds.includes(mutation.message.id)) {
        state.userMessageIds = [...state.userMessageIds, mutation.message.id];
      }
      break;
    }
    case "upsertUserMessage":
      state.messages = mergeUserMessage(state.messages, mutation.message);
      if (!state.userMessageIds.includes(mutation.message.id)) {
        state.userMessageIds = [...state.userMessageIds, mutation.message.id];
      }
      break;
    case "removeMessage":
      state.messages = state.messages.filter((message) => message.id !== mutation.messageId);
      state.userMessageIds = state.userMessageIds.filter((messageId) => messageId !== mutation.messageId);
      break;
    case "replaceMessages":
      state.messages = [...mutation.messages];
      break;
    case "resetRound":
      resetRoundState(state);
      break;
    case "clearPendingInputs":
      state.pendingQuestion = null;
      state.pendingToolConfirms = [];
      break;
    case "clearPendingInput":
      if (state.pendingQuestion?.questionId === mutation.questionId) {
        state.pendingQuestion = null;
      }
      state.pendingToolConfirms = state.pendingToolConfirms.filter(
        (item) => item.questionId !== mutation.questionId,
      );
      break;
    case "updateUsage":
      state.tokenUsage = mutation.usage;
      break;
    case "setQuestion":
      state.pendingQuestion = mutation.question;
      break;
    case "enqueueToolConfirm": {
      state.pendingToolConfirms = [
        ...state.pendingToolConfirms.filter((item) => item.questionId !== mutation.confirm.questionId),
        mutation.confirm,
      ];
      break;
    }
    case "setStreaming":
      state.isStreaming = mutation.value;
      break;
    case "setCompacting":
      state.isCompacting = mutation.value;
      break;
    case "pushToolResults":
      {
        const targetIds = mutation.toolCallIds ? new Set(mutation.toolCallIds) : null;
        const sourceToolCalls = targetIds
          ? state.activeToolCalls.filter((toolCall) => targetIds.has(toolCall.id))
          : state.activeToolCalls;
        // Apply the whole batch as a single messages-array replacement:
        // every replacement re-runs the transcript's O(messages) grouping
        // pipeline, so per-result replacements multiply that cost by the
        // batch size.
        const toolResultMessages = buildToolResultMessages(sourceToolCalls);
        if (toolResultMessages.length > 0) {
          let next = state.messages;
          for (const message of toolResultMessages) {
            next = replaceMessageById(next, message);
          }
          state.messages = next;
        }
      }
      break;
    case "resetRoundKeepToolCalls":
      state.streamingText = "";
      state.rawStreamText = "";
      state.streamingThinking = "";
      state.streamingTextOrder = 0;
      state.thinkingOrder = 0;
      state.liveRenderParts = [];
      state.isThinking = false;
      state.thinkingStartTime = 0;
      state.thinkingDuration = 0;
      resetEmbeddedStreams(state);
      break;
    case "setTodos":
      state.todos = mutation.todos;
      state.latestTodoRunId = mutation.runId;
      state.todoWriteVersion += 1;
      if (mutation.todos.length > 0 && useDisplaySettings().state.todoAutoOpen) {
        state.showTodoPanel = true;
      }
      break;
    case "addUndoable":
      state.undoableMessageIds.add(mutation.messageId);
      break;
  }
  if (traceApplyMutation) {
    traceEmbeddedOrder(state, "embeddedApplyStreamMutation", {
      mutation: traceEmbeddedStreamMutation(mutation),
      messagesBeforeMutation,
      messagesAfterMutation: traceEmbeddedMessageOrder(state.messages),
      activeToolCallsBeforeMutation,
      activeToolCallsAfterMutation: traceEmbeddedToolCallOrder(state.activeToolCalls),
    });
  }
}

function applyAsyncTaskUpdateToState(
  state: EmbeddedChatState,
  update: AsyncTaskUpdatedEvent,
): void {
  const displayStatus = asyncTaskDisplayStatus(update.status);
  const activeToolCall = state.activeToolCalls.find((item) => item.id === update.toolCallId);
  if (activeToolCall) {
    activeToolCall.status = displayStatus;
    activeToolCall.output = update.output;
    if (displayStatus !== "running") activeToolCall.progress = null;
  }
  state.messages = applyAsyncTaskUpdateToMessages(state.messages, update);
}

export function useEmbeddedChatSession(options: UseEmbeddedChatSessionOptions) {
  const modelStore = useModelStore();
  const statesByKey = new Map<string, EmbeddedChatState>();
  const sessionStates = new Map<string, EmbeddedChatState>();
  const activeState = shallowRef<EmbeddedChatState>(sharedEmbeddedChatState(
    toValue(options.sessionKey),
    toValue(options.initialSessionId) ?? null,
  ));
  const activeDraft = shallowRef(sharedEmbeddedChatDraft(toValue(options.sessionKey)));
  const activeRestoredDraft = shallowRef(sharedEmbeddedRestoredDraft(toValue(options.sessionKey)));
  retainSharedEmbeddedChatState(activeState.value);

  function replaceActiveState(state: EmbeddedChatState): void {
    if (activeState.value === state) return;
    const previous = activeState.value;
    retainSharedEmbeddedChatState(state);
    activeState.value = state;
    releaseSharedEmbeddedChatState(previous);
  }

  function ensureState(key: string, sessionId?: string | null) {
    const shared = sharedEmbeddedChatState(key, sessionId);
    statesByKey.set(key, shared);
    if (shared.sessionId) sessionStates.set(shared.sessionId, shared);
    return shared;
  }

  function syncActiveState(key: string, sessionId?: string | null) {
    replaceActiveState(ensureState(key, sessionId));
    activeDraft.value = sharedEmbeddedChatDraft(key);
    activeRestoredDraft.value = sharedEmbeddedRestoredDraft(key);
  }

  function captureWorkspaceRef(): WorkspaceRef | null {
    const workspaceRef = toValue(options.workspaceRef);
    return workspaceRef
      ? {
        checkoutId: workspaceRef.checkoutId,
        expectedGeneration: workspaceRef.expectedGeneration ?? null,
      }
      : null;
  }

  function applyLoadedSessionState(
    state: EmbeddedChatState,
    snapshot: Awaited<ReturnType<typeof sessionService.loadSessionView>>,
    usage: TokenUsage | null,
    sessionTodos: Awaited<ReturnType<typeof sessionService.getTodos>>,
    undoEntries: Array<{ assistantMessageId: string }>,
    resumeAvailable: boolean,
    planState: Awaited<ReturnType<typeof sessionService.getSessionPlanState>>,
  ): void {
    const detail = snapshot.session;
    state.messages = hydrateChatMessagesIntent(detail.messages);
    state.pendingInputs = visiblePendingInputs(detail.pendingInputs ?? []);
    state.sessionAgentId = detail.agentId ?? null;
    state.sessionModelId = detail.lastModelId ?? null;
    state.sessionEffort = detail.lastEffort ?? null;
    state.sessionFastMode = detail.lastFastMode ?? null;
    state.sessionMultiAgentEnabled = detail.lastMultiAgentEnabled ?? false;
    state.parentSessionId = detail.parentSessionId;
    state.latestCompletedRunId = detail.latestCompletedRunId ?? null;
    state.todos = sessionTodos.items;
    state.latestTodoRunId = sessionTodos.latestRunId;
    if (state.todos.length === 0) state.showTodoPanel = false;
    state.undoableMessageIds = new Set(
      undoEntries.map((entry) => entry.assistantMessageId),
    );
    if (usage) state.tokenUsage = usage;
    state.resumeAvailable = resumeAvailable;
    state.planModeActive = planState.active;
    state.planFilePath = planState.planFilePath || null;
    state.historyOldestRowId = snapshot.oldestMessageRowId ?? null;
    state.historyHasMore = snapshot.hasMoreHistory;
    state.historyLoading = false;
    state.userMessageIds = snapshot.userMessageIds
      ?? detail.messages
        .filter((message) => message.role === "user")
        .map((message) => message.id);
    applySessionRuntimeSnapshot(state, detail);
    state.hydrated = true;
  }

  async function loadEmbeddedSessionState(
    sessionId: string,
    workspaceRef: WorkspaceRef | null,
  ) {
    const messageLimit = useDisplaySettings().state.sessionMessagePageSize;
    return Promise.all([
      sessionService.loadSessionView(sessionId, messageLimit),
      sessionService.getSessionUsage(sessionId).catch(() => null),
      sessionService.getTodos(sessionId).catch(() => ({ items: [], latestRunId: null })),
      undoService.undoList(sessionId).catch(() => []),
      sessionService.getSessionResumeAvailable(sessionId).catch(() => false),
      sessionService.getSessionPlanState(sessionId, workspaceRef).catch(() => ({
        active: false,
        planFilePath: "",
        planFileExists: false,
      })),
    ] as const);
  }

  const sessionHydrationEpochs = new Map<string, number>();

  function replayBufferedSessionEvents(sessionId: string): void {
    for (const dispatch of bindSessionStreamEventConsumer(sessionId)) {
      if (!sessionStreamSourceMatchesWorkspace(dispatch.source, toValue(options.workspaceRef))) continue;
      handleStreamEvent(dispatch.event);
    }
  }

  function replayBufferedAsyncTaskUpdates(sessionId: string): void {
    for (const update of bindSessionAsyncTaskUpdateConsumer(sessionId)) {
      applyAsyncTaskUpdate(update);
    }
  }

  async function hydrateExistingSession(state: EmbeddedChatState, sessionId: string) {
    const normalizedSessionId = sessionId.trim();
    if (!normalizedSessionId) return;
    if (state.sessionId === normalizedSessionId && state.hydrated) {
      // The transcript is shared across pane reparenting, while stream events
      // may have arrived during the short interval with no mounted reducer.
      replayBufferedSessionEvents(normalizedSessionId);
      replayBufferedAsyncTaskUpdates(normalizedSessionId);
      return;
    }
    const epoch = (sessionHydrationEpochs.get(state.key) ?? 0) + 1;
    sessionHydrationEpochs.set(state.key, epoch);
    if (state.sessionId && state.sessionId !== normalizedSessionId) {
      sessionStates.delete(state.sessionId);
    }
    state = bindSharedEmbeddedChatSession(state, normalizedSessionId);
    state.loading = true;
    state.error = null;
    sessionStates.set(normalizedSessionId, state);
    let hydrated = false;
    try {
      const workspaceRef = captureWorkspaceRef();
      const [snapshot, usage, sessionTodos, undoEntries, resumeAvailable, planState] =
        await loadEmbeddedSessionState(normalizedSessionId, workspaceRef);
      if (
        sessionHydrationEpochs.get(state.key) !== epoch
        || state.sessionId !== normalizedSessionId
      ) return;
      applyLoadedSessionState(
        state,
        snapshot,
        usage,
        sessionTodos,
        undoEntries,
        resumeAvailable,
        planState,
      );
      hydrated = true;
    } catch (error) {
      if (
        sessionHydrationEpochs.get(state.key) !== epoch
        || state.sessionId !== normalizedSessionId
      ) return;
      state.error = normalizeAppError(error).message;
    } finally {
      if (sessionHydrationEpochs.get(state.key) === epoch) state.loading = false;
      if (hydrated) {
        // The backend snapshot includes all events received before this load.
        bindSessionStreamEventConsumer(normalizedSessionId);
        bindSessionAsyncTaskUpdateConsumer(normalizedSessionId);
      } else {
        replayBufferedSessionEvents(normalizedSessionId);
        replayBufferedAsyncTaskUpdates(normalizedSessionId);
      }
    }
  }

  function resolveStateForEvent(event: StreamEvent) {
    return sessionStates.get(event.sessionId) ?? null;
  }

  async function reloadSessionState(state: EmbeddedChatState, sessionId: string): Promise<boolean> {
    try {
      const workspaceRef = captureWorkspaceRef();
      const [snapshot, usage, sessionTodos, undoEntries, resumeAvailable, planState] =
        await loadEmbeddedSessionState(sessionId, workspaceRef);
      if (state.sessionId !== sessionId) return false;
      applyLoadedSessionState(
        state,
        snapshot,
        usage,
        sessionTodos,
        undoEntries,
        resumeAvailable,
        planState,
      );
      return true;
    } catch (error) {
      console.warn("[embedded-chat] failed to refresh session state:", error);
      return false;
    }
  }

  function handleStreamEvent(event: StreamEvent) {
    const state = resolveStateForEvent(event);
    if (!state) return;

    traceEmbeddedOrder(state, "embeddedStreamEventReceived", {
      event: traceEmbeddedStreamEvent(event),
      pendingInputs: visiblePendingInputs(state.pendingInputs).map((input, index) => ({
        index,
        id: input.id,
        runId: input.runId,
        mergeGroupId: input.mergeGroupId,
        delivery: input.delivery ?? "after_run",
        status: input.status,
        displayTextLen: (input.displayText || input.text).length,
        displayTextPreview: previewTraceText(input.displayText || input.text, 48),
      })),
    });

    // Plan transitions may be emitted by a synthetic command run. They belong
    // to the durable session and must bypass active-run filtering.
    if (event.type === "planModeChanged") {
      state.planModeActive = event.active;
      state.planFilePath = event.planFilePath?.trim() || null;
      return;
    }

    if (event.type === "runStart") {
      if (state.currentRunId && state.currentRunId !== event.runId) {
        state.deferredUserMessagesByRun.delete(state.currentRunId);
        state.submittedUserMessagesByRun.delete(state.currentRunId);
        state.streamSequence = 0;
        state.pendingQuestion = null;
        state.pendingToolConfirms = [];
        state.isCompacting = false;
        resetRoundState(state);
      }
      state.currentRunId = event.runId;
      state.isStreaming = true;
      state.pendingRun = false;
      state.isCancelling = false;
      state.resumeAvailable = false;
      state.error = null;
      return;
    }

    if (state.currentRunId && event.runId !== state.currentRunId) return;
    if (!state.currentRunId) state.currentRunId = event.runId;

    if (event.type === "pendingInputQueued") {
      if (state.acceptedPendingInputIds.has(event.input.id)) return;
      state.pendingInputs = visiblePendingInputs(
        mergePendingInputList(state.pendingInputs, event.input),
      );
      return;
    }

    if (event.type === "pendingInputDeleted") {
      const deleted = state.pendingInputs.find((input) => input.id === event.pendingInputId);
      state.pendingInputs = state.pendingInputs.filter((input) => input.id !== event.pendingInputId);
      if (deleted?.mergeGroupId === state.localMergeGroupId) {
        state.localMergeGroupId = null;
      }
      if (deleted?.mergeGroupId === state.localFallbackMergeGroupId) {
        state.localFallbackMergeGroupId = null;
      }
      return;
    }

    if (event.type === "pendingInputAccepted") {
      state.acceptedPendingInputIds.add(event.pendingInputId);
      state.pendingInputs = state.pendingInputs.filter((input) => input.id !== event.pendingInputId);
      state.localMergeGroupId = null;
      state.localFallbackMergeGroupId = null;
      return;
    }

    if (event.type === "userMessage" && shouldDeferUserMessage(state, event)) {
      deferUserMessage(state, event);
      return;
    }

    if (event.type === "compactStart") {
      state.compactQueued = false;
    }

    const submittedUserMessage = event.type === "cancelled"
      ? state.submittedUserMessagesByRun.get(event.runId) ?? null
      : null;
    const mutations = reduceStreamEvent(state, event);
    traceEmbeddedOrder(state, "streamEventMutationBatch", {
      event: traceEmbeddedStreamEvent(event),
      mutationCount: mutations.length,
      mutations: mutations.map(traceEmbeddedStreamMutation),
    });
    for (const mutation of mutations) {
      applyMutation(state, mutation);
    }
    if (event.type === "toolCallRoundDone") {
      flushDeferredUserMessages(state, event.runId);
    }

    if (event.type === "error") {
      flushDeferredUserMessages(state, event.runId);
      state.error = normalizeAppError(event.error).message;
      state.currentRunId = null;
      state.pendingRun = false;
      state.isCancelling = false;
      state.compactQueued = false;
      state.submittedUserMessagesByRun.delete(event.runId);
      void reloadSessionState(state, event.sessionId);
      return;
    }

    if (event.type === "done" || event.type === "cancelled") {
      flushDeferredUserMessages(state, event.runId);
      let queuedRequest: EmbeddedChatRequest | null = null;
      const followUpMergeGroupId = event.type === "cancelled"
        ? state.localMergeGroupId
        : state.localFallbackMergeGroupId;
      if ((event.type === "done" || event.type === "cancelled") && followUpMergeGroupId) {
        const queued = state.pendingInputs.find((input) =>
          input.runId === event.runId && input.mergeGroupId === followUpMergeGroupId);
        if (queued) {
          state.pendingInputs = state.pendingInputs.filter((input) => input.id !== queued.id);
          state.localMergeGroupId = null;
          state.localFallbackMergeGroupId = null;
          queuedRequest = {
            text: queued.text,
            displayText: queued.displayText,
            mode: queued.mode ?? null,
            userIntent: queued.userIntent ?? null,
            images: queued.images ?? null,
            assetRefs: queued.assetRefs ?? null,
          };
        }
      }
      if (event.type === "cancelled") {
        state.pendingInputs = state.pendingInputs.filter((input) => input.runId !== event.runId);
        state.compactQueued = false;
        if (!queuedRequest) {
          state.localMergeGroupId = null;
          state.localFallbackMergeGroupId = null;
        }
      }
      state.currentRunId = null;
      state.pendingRun = false;
      state.isCancelling = false;
      state.resumeAvailable = false;
      if (event.type === "done") state.latestCompletedRunId = event.runId;
      state.submittedUserMessagesByRun.delete(event.runId);
      if (event.type === "cancelled" && event.removedUserMessage && submittedUserMessage) {
        restoreEmbeddedSubmittedDraft(submittedUserMessage);
      }
      if (queuedRequest) {
        globalThis.setTimeout(() => {
          void send(queuedRequest);
        }, 0);
      }
    }
  }

  function resolveExecutionSelection(
    state: EmbeddedChatState,
    mode?: string | null,
  ): {
    modelId: string;
    effort: EffortLevel | null;
    fastMode: boolean;
    multiAgentEnabled: boolean;
  } | null {
    const selectedModelId = toValue(options.selectedModelId)?.trim() ?? "";
    let modelId = selectedModelId;
    if (mode === "plan") {
      const planModelId = modelStore.modelDefaults.planModel?.trim() ?? "";
      if (
        planModelId
        && modelStore.availableModels.some((candidate) => candidate.id === planModelId)
      ) {
        modelId = planModelId;
      }
    }
    if (!modelId) return null;

    const model = modelStore.availableModels.find((candidate) => candidate.id === modelId);
    const requestedEffort = toValue(options.effort) ?? null;
    const effort = modelId === selectedModelId
      ? (toValue(options.effortSupported) ? requestedEffort : null)
      : model?.supportedEfforts?.includes(requestedEffort ?? "none")
        ? requestedEffort
        : null;
    const paneFastMode = toValue(options.fastMode) ?? state.sessionFastMode ?? false;
    const fastMode = paneFastMode === true && !!model && modelSupportsFastMode(model);
    const multiAgentEnabled = toValue(options.multiAgentEnabled) ?? state.sessionMultiAgentEnabled;
    return { modelId, effort, fastMode, multiAgentEnabled };
  }

  async function send(requestOverride?: EmbeddedChatRequest | null) {
    const state = activeState.value;

    const input = activeDraft.value.value.trim();
    const request = requestOverride ?? (input ? options.buildRequest(input) : null);
    if (!request) return;
    if (!requestOverride && !input) return;

    const execution = resolveExecutionSelection(state, request.mode);
    if (!execution) {
      state.error = t("model.select");
      return;
    }

    const displayText = request.displayText ?? request.text;
    const staleChanged = updateProposalStatus(state, "stale");
    if (staleChanged && state.sessionId) {
      sessionService.staleKnowledgeProposals(state.sessionId).catch((error: unknown) => {
        console.warn("[embedded-chat] staleKnowledgeProposals failed:", error);
      });
    }

    if (state.isStreaming && state.sessionId && state.currentRunId) {
      const { state: chatInputSettings } = useChatInputSettings();
      const delivery = chatInputSettings.runningSendMode === "insert" ? "immediate" : "after_run";
      let mergeGroupId = state.localMergeGroupId;
      if (!mergeGroupId) {
        mergeGroupId = `embedded_user_${Date.now()}`;
        state.localMergeGroupId = mergeGroupId;
      }
      const userIntent = withClientMessageId(request.userIntent, mergeGroupId);
      try {
        const pending = await sessionService.queueChatInput({
          sessionId: state.sessionId,
          runId: state.currentRunId,
          mergeGroupId,
          text: request.text,
          displayText,
          images: request.images && request.images.length > 0 ? request.images : null,
          assetRefs: request.assetRefs && request.assetRefs.length > 0 ? request.assetRefs : null,
          mode: request.mode ?? null,
          userIntent,
          clientMessageId: mergeGroupId,
          delivery,
        });
        if (!state.isStreaming || state.currentRunId !== pending.runId) {
          if (!state.acceptedPendingInputIds.has(pending.id)) {
            state.pendingInputs = visiblePendingInputs(
              mergePendingInputList(state.pendingInputs, pending),
            );
          }
          return;
        }
        if (!state.acceptedPendingInputIds.has(pending.id)) {
          state.pendingInputs = visiblePendingInputs(
            mergePendingInputList(state.pendingInputs, pending),
          );
        }
        activeDraft.value.value = "";
        state.error = null;
      } catch (error) {
        const err = normalizeAppError(error);
        if (isPendingInputFallbackError(err.code)) {
          const existing = state.pendingInputs.find((input) =>
            input.runId === state.currentRunId && input.mergeGroupId === mergeGroupId);
          const now = Date.now() / 1000;
          const pending: PendingSessionInput = existing
            ? {
              ...existing,
              text: joinPendingText(existing.text, request.text),
              displayText: joinPendingText(existing.displayText, displayText),
              images: [...(existing.images ?? []), ...(request.images ?? [])],
              assetRefs: [...(existing.assetRefs ?? []), ...(request.assetRefs ?? [])],
              mode: existing.mode === "plan" || request.mode === "plan"
                ? "plan"
                : request.mode ?? existing.mode ?? null,
              userIntent: userIntent ?? existing.userIntent ?? null,
              clientMessageId: existing.clientMessageId ?? mergeGroupId,
              updatedAt: now,
            }
            : {
              id: mergeGroupId,
              sessionId: state.sessionId,
              runId: state.currentRunId,
              mergeGroupId,
              status: "queued",
              delivery: "after_run",
              text: request.text,
              displayText,
              images: request.images && request.images.length > 0 ? [...request.images] : undefined,
              assetRefs: request.assetRefs && request.assetRefs.length > 0 ? [...request.assetRefs] : undefined,
              mode: request.mode ?? null,
              userIntent,
              clientMessageId: mergeGroupId,
              messageId: null,
              createdAt: now,
              updatedAt: now,
            };
          state.pendingInputs = visiblePendingInputs(
            mergePendingInputList(state.pendingInputs, pending),
          );
          state.localFallbackMergeGroupId = mergeGroupId;
          activeDraft.value.value = "";
          state.error = null;
          if (!state.isStreaming || state.currentRunId !== pending.runId) {
            state.pendingInputs = state.pendingInputs.filter((input) => input.id !== pending.id);
            state.localMergeGroupId = null;
            globalThis.setTimeout(() => {
              void send({
                text: pending.text,
                displayText: pending.displayText,
                mode: pending.mode ?? null,
                userIntent: pending.userIntent ?? null,
                images: pending.images ?? null,
                assetRefs: pending.assetRefs ?? null,
              });
            }, 0);
          }
          return;
        }
        state.error = err.message;
      }
      return;
    }

    const pendingMessageId = `embedded_user_${Date.now()}`;
    const userIntent = withClientMessageId(request.userIntent, pendingMessageId);
    const userIntentSignature = JSON.stringify(userIntent);

    const pendingUserMessage: ChatMessage = {
      id: pendingMessageId,
      role: "user",
      content: displayText,
      createdAt: Date.now() / 1000,
      images: request.images && request.images.length > 0 ? request.images : undefined,
      assetRefs: request.assetRefs && request.assetRefs.length > 0 ? request.assetRefs : undefined,
      thinkingSignature: userIntentSignature,
      intentMeta: userIntent,
    };
    const submittedUserMessage: EmbeddedSubmittedUserMessage = {
      // The optimistic transcript uses displayText, which intentionally
      // reduces local-file attachments to their names. Build the recoverable
      // draft from the full request so an interrupted run restores attachment
      // objects and absolute paths.
      draft: buildUserMessageDraft({
        ...pendingUserMessage,
        content: request.text,
      }),
      ownerKey: toValue(options.sessionKey),
    };
    state.cancelPendingLaunch = false;
    state.compactPendingLaunch = false;
    state.compactQueued = false;
    const interruptedToolResultMessages = buildInterruptedTrailingToolResultMessages(state.messages);
    if (interruptedToolResultMessages.length > 0) {
      let nextMessages = state.messages;
      for (const message of interruptedToolResultMessages) {
        nextMessages = replaceMessageById(nextMessages, message);
      }
      state.messages = [...nextMessages, pendingUserMessage];
    } else {
      state.messages.push(pendingUserMessage);
    }

    activeDraft.value.value = "";
    state.error = null;
    state.pendingQuestion = null;
    state.pendingToolConfirms = [];
    state.streamSequence = 0;
    state.isCompacting = false;
    resetRoundState(state);
    state.isStreaming = true;
    state.pendingRun = true;
    state.isCancelling = false;
    state.resumeAvailable = false;

    const knowledgeFocus = toValue(options.knowledgeFocus) ?? null;
    const knowledgeMode = toValue(options.knowledgeMode) ?? null;
    const workspaceRef = captureWorkspaceRef();
    const requestSessionId = state.sessionId;
    const selectedAgentId = toValue(options.selectedAgentId) ?? null;

    try {
      const launch = await sessionService.chat({
        workspaceRef,
        sessionId: requestSessionId,
        text: request.text,
        sessionTitle: toValue(options.sessionTitle) ?? null,
        agentId: selectedAgentId,
        model: execution.modelId,
        effort: execution.effort,
        fastMode: execution.fastMode,
        multiAgentEnabled: execution.multiAgentEnabled,
        images: request.images && request.images.length > 0 ? request.images : null,
        assetRefs: request.assetRefs && request.assetRefs.length > 0 ? request.assetRefs : null,
        sessionType: options.sessionType ?? "chat",
        mode: request.mode ?? null,
        userIntent,
        subagentModels: Object.keys(modelStore.modelDefaults.subagentModels).length > 0
          ? modelStore.modelDefaults.subagentModels
          : null,
        subagentEfforts: Object.keys(modelStore.modelDefaults.subagentEfforts).length > 0
          ? modelStore.modelDefaults.subagentEfforts
          : null,
        subagentFastModes: Object.keys(modelStore.modelDefaults.subagentFastModes).length > 0
          ? modelStore.modelDefaults.subagentFastModes
          : null,
        knowledgeMode,
        knowledgeDocType: knowledgeFocus?.docType ?? null,
        knowledgeDocPath: knowledgeFocus?.path ?? null,
      });

      const boundState = bindSharedEmbeddedChatSession(state, launch.sessionId);
      const cancelAfterLaunch = state.cancelPendingLaunch;
      const compactAfterLaunch = state.compactPendingLaunch;
      state.cancelPendingLaunch = false;
      state.compactPendingLaunch = false;
      boundState.submittedUserMessagesByRun.set(launch.runId, submittedUserMessage);
      boundState.currentRunId = launch.runId;
      boundState.pendingRun = false;
      boundState.sessionAgentId = selectedAgentId;
      boundState.sessionModelId = execution.modelId;
      boundState.sessionEffort = execution.effort;
      boundState.sessionFastMode = execution.fastMode;
      boundState.sessionMultiAgentEnabled = execution.multiAgentEnabled;
      boundState.resumeAvailable = false;
      boundState.hydrated = true;
      if (request.mode === "plan") boundState.planModeActive = true;
      sessionStates.set(launch.sessionId, boundState);
      if (activeState.value === state && boundState !== state) {
        replaceActiveState(boundState);
      }
      replayBufferedSessionEvents(launch.sessionId);
      replayBufferedAsyncTaskUpdates(launch.sessionId);
      if (cancelAfterLaunch) {
        boundState.compactQueued = false;
        void sessionService.cancelChat(launch.sessionId).catch((error) => {
          boundState.error = normalizeAppError(error).message;
        });
      } else if (compactAfterLaunch) {
        void compact();
      }
    } catch (error) {
      state.isStreaming = false;
      state.pendingRun = false;
      state.isCompacting = false;
      const interruptedToolResultIds = new Set(
        interruptedToolResultMessages.map((message) => message.id),
      );
      state.messages = state.messages.filter((message) => (
        message.id !== pendingMessageId && !interruptedToolResultIds.has(message.id)
      ));
      state.cancelPendingLaunch = false;
      state.compactPendingLaunch = false;
      state.compactQueued = false;
      restoreEmbeddedSubmittedDraft(submittedUserMessage);
      resetRoundState(state);
      state.error = normalizeAppError(error).message;
    }
  }

  // Entry point for composer "send" events. The composer payload is structurally
  // compatible with EmbeddedChatRequest, so passing it straight to send() would
  // skip options.buildRequest and drop the pane's injected context (e.g. the
  // knowledge pane's current-document block). Wrap the payload text here, then
  // carry the composer's attachments and intent over to the built request.
  function sendComposerPayload(payload?: ChatComposerSendPayload | null) {
    if (!payload) {
      void send();
      return;
    }
    const built = options.buildRequest(payload.text);
    if (!built && !payload.text.trim() && payload.images.length === 0 && payload.assetRefs.length === 0) {
      return;
    }
    void send({
      text: built?.text ?? payload.text,
      displayText: payload.displayText,
      mode: payload.mode ?? built?.mode ?? null,
      userIntent: payload.userIntent ?? built?.userIntent ?? null,
      images: payload.images.length > 0 ? payload.images : built?.images ?? null,
      assetRefs: payload.assetRefs.length > 0 ? payload.assetRefs : built?.assetRefs ?? null,
    });
  }

  async function launchHiddenSessionRun(request: {
    mode: "build" | "compact";
    resume?: boolean;
    materializeInterruptedTools?: boolean;
  }): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || state.isStreaming) return false;

    const execution = resolveExecutionSelection(state, request.mode);
    if (!execution) {
      state.error = t("model.select");
      return false;
    }

    const workspaceRef = captureWorkspaceRef();
    const knowledgeFocus = toValue(options.knowledgeFocus) ?? null;
    const knowledgeMode = toValue(options.knowledgeMode) ?? null;
    const selectedAgentId = toValue(options.selectedAgentId) ?? null;
    const resumeAvailableBeforeLaunch = state.resumeAvailable;
    const interruptedToolResultMessages = request.materializeInterruptedTools
      ? buildInterruptedTrailingToolResultMessages(state.messages)
      : [];

    state.cancelPendingLaunch = false;
    state.compactPendingLaunch = false;
    state.compactQueued = false;
    state.error = null;
    state.pendingQuestion = null;
    state.pendingToolConfirms = [];
    state.streamSequence = 0;
    state.isCompacting = false;
    state.resumeAvailable = false;
    resetRoundState(state);
    state.isStreaming = true;
    state.pendingRun = true;
    state.isCancelling = false;

    try {
      const launch = await sessionService.chat({
        workspaceRef,
        sessionId: targetSessionId,
        text: "",
        resume: request.resume === true ? true : null,
        sessionTitle: toValue(options.sessionTitle) ?? null,
        agentId: selectedAgentId,
        model: execution.modelId,
        effort: execution.effort,
        fastMode: execution.fastMode,
        multiAgentEnabled: execution.multiAgentEnabled,
        images: null,
        assetRefs: null,
        sessionType: options.sessionType ?? "chat",
        mode: request.mode,
        userIntent: null,
        subagentModels: Object.keys(modelStore.modelDefaults.subagentModels).length > 0
          ? modelStore.modelDefaults.subagentModels
          : null,
        subagentEfforts: Object.keys(modelStore.modelDefaults.subagentEfforts).length > 0
          ? modelStore.modelDefaults.subagentEfforts
          : null,
        subagentFastModes: Object.keys(modelStore.modelDefaults.subagentFastModes).length > 0
          ? modelStore.modelDefaults.subagentFastModes
          : null,
        knowledgeMode,
        knowledgeDocType: knowledgeFocus?.docType ?? null,
        knowledgeDocPath: knowledgeFocus?.path ?? null,
      });

      if (state.sessionId !== targetSessionId) return false;
      const boundState = bindSharedEmbeddedChatSession(state, launch.sessionId);
      const cancelAfterLaunch = state.cancelPendingLaunch;
      const compactAfterLaunch = state.compactPendingLaunch;
      state.cancelPendingLaunch = false;
      state.compactPendingLaunch = false;
      if (interruptedToolResultMessages.length > 0) {
        let nextMessages = boundState.messages;
        for (const message of interruptedToolResultMessages) {
          nextMessages = replaceMessageById(nextMessages, message);
        }
        boundState.messages = nextMessages;
      }
      boundState.currentRunId = launch.runId;
      boundState.pendingRun = false;
      boundState.sessionAgentId = selectedAgentId;
      boundState.sessionModelId = execution.modelId;
      boundState.sessionEffort = execution.effort;
      boundState.sessionFastMode = execution.fastMode;
      boundState.sessionMultiAgentEnabled = execution.multiAgentEnabled;
      boundState.resumeAvailable = false;
      sessionStates.set(launch.sessionId, boundState);
      if (activeState.value === state && boundState !== state) {
        replaceActiveState(boundState);
      }
      replayBufferedSessionEvents(launch.sessionId);
      replayBufferedAsyncTaskUpdates(launch.sessionId);
      if (cancelAfterLaunch) {
        boundState.compactQueued = false;
        void sessionService.cancelChat(launch.sessionId).catch((error) => {
          boundState.error = normalizeAppError(error).message;
        });
      } else if (compactAfterLaunch) {
        void compact();
      }
      return true;
    } catch (error) {
      if (state.sessionId !== targetSessionId) return false;
      state.isStreaming = false;
      state.pendingRun = false;
      state.isCompacting = false;
      state.cancelPendingLaunch = false;
      state.compactPendingLaunch = false;
      state.compactQueued = false;
      state.resumeAvailable = request.resume === true ? resumeAvailableBeforeLaunch : false;
      resetRoundState(state);
      state.error = normalizeAppError(error).message;
      return false;
    }
  }

  async function resumeInterrupted(): Promise<boolean> {
    const state = activeState.value;
    if (!state.sessionId || state.isStreaming || !state.resumeAvailable) return false;
    return launchHiddenSessionRun({
      mode: "build",
      resume: true,
      materializeInterruptedTools: true,
    });
  }

  async function compact(retryQueueRace = true): Promise<boolean> {
    const state = activeState.value;
    if (state.isStreaming && (!state.sessionId || !state.currentRunId)) {
      state.compactPendingLaunch = true;
      return true;
    }
    if (!state.sessionId) return false;

    if (state.isStreaming && state.currentRunId) {
      const targetSessionId = state.sessionId;
      const targetRunId = state.currentRunId;
      if (state.compactQueued) return true;
      try {
        await sessionService.queueSessionCompact(targetSessionId, targetRunId);
        if (state.sessionId === targetSessionId && state.currentRunId === targetRunId) {
          state.compactQueued = true;
          state.error = null;
        }
        return true;
      } catch (error) {
        const err = normalizeAppError(error);
        const runClosed = err.code === "session.pending_compact.no_active_run"
          || err.code === "session.pending_compact.run_mismatch"
          || err.code === "session.pending_compact.run_closed";
        if (runClosed && retryQueueRace) {
          const reloaded = await reloadSessionState(state, targetSessionId);
          if (reloaded && activeState.value === state) {
            return compact(false);
          }
        }
        state.error = err.message;
        return false;
      }
    }

    if (state.isStreaming) return false;
    return launchHiddenSessionRun({ mode: "compact" });
  }

  async function insertQueuedFollowUp() {
    const state = activeState.value;
    if (!state.sessionId || !state.currentRunId) return false;
    const pending = visiblePendingInputs(state.pendingInputs).find((input) =>
      input.runId === state.currentRunId && pendingInputDelivery(input) !== "immediate");
    if (!pending) return false;

    try {
      const inserted = await sessionService.insertPendingChatInput(
        state.sessionId,
        state.currentRunId,
        pending.id,
      );
      if (!state.acceptedPendingInputIds.has(inserted.id)) {
        state.pendingInputs = visiblePendingInputs(
          mergePendingInputList(state.pendingInputs, inserted),
        );
      }
      return true;
    } catch (error) {
      state.error = normalizeAppError(error).message;
      return false;
    }
  }

  async function deleteQueuedFollowUp() {
    const state = activeState.value;
    const targets = visiblePendingInputs(state.pendingInputs);
    if (!state.sessionId || targets.length === 0) return false;

    try {
      const deleteResults = await Promise.all(
        targets.map((input) =>
          sessionService.deletePendingChatInput(
            input.sessionId,
            input.runId,
            input.id,
          )),
      );
      const allWithdrawn = targets.every((input, index) => (
        deleteResults[index] === true
        || input.mergeGroupId === state.localFallbackMergeGroupId
      ));
      const targetIds = new Set(targets.map((input) => input.id));
      state.pendingInputs = state.pendingInputs.filter((input) => !targetIds.has(input.id));
      for (const input of targets) {
        if (input.mergeGroupId === state.localMergeGroupId) {
          state.localMergeGroupId = null;
        }
        if (input.mergeGroupId === state.localFallbackMergeGroupId) {
          state.localFallbackMergeGroupId = null;
        }
      }
      return allWithdrawn;
    } catch (error) {
      state.error = normalizeAppError(error).message;
      return false;
    }
  }

  async function reEditQueuedFollowUp() {
    const state = activeState.value;
    const targets = visiblePendingInputs(state.pendingInputs);
    if (!state.sessionId || targets.length === 0) return null;

    const draft = buildPendingSessionInputDraft(targets);
    const deleted = await deleteQueuedFollowUp();
    if (!deleted) return null;

    activeDraft.value.value = draft.text;
    return draft;
  }

  async function cancel() {
    const state = activeState.value;
    if (state.pendingRun && !state.currentRunId) {
      state.cancelPendingLaunch = true;
      state.compactPendingLaunch = false;
    }
    if (!state.sessionId) return;
    const targetSessionId = state.sessionId;
    state.isCancelling = true;
    try {
      await sessionService.cancelChat(targetSessionId);
      await reloadSessionState(state, targetSessionId);
    } catch (error) {
      state.error = normalizeAppError(error).message;
    } finally {
      state.isCancelling = false;
    }
  }

  async function setPlanMode(active: boolean): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || state.isStreaming) return false;
    const workspaceRef = captureWorkspaceRef();
    try {
      const planState = await sessionService.setSessionPlanMode(
        targetSessionId,
        active,
        workspaceRef,
      );
      if (state.sessionId !== targetSessionId) return false;
      state.planModeActive = planState.active;
      state.planFilePath = planState.planFilePath?.trim() || null;
      state.error = null;
      return true;
    } catch (error) {
      if (state.sessionId === targetSessionId) {
        state.error = normalizeAppError(error).message;
      }
      return false;
    }
  }

  function exitPlanMode(): Promise<boolean> {
    return setPlanMode(false);
  }

  async function loadOlderSessionHistory(): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    const beforeRowId = state.historyOldestRowId;
    if (
      !targetSessionId
      || !state.historyHasMore
      || state.historyLoading
      || beforeRowId === null
    ) {
      return false;
    }

    state.historyLoading = true;
    try {
      const messageLimit = useDisplaySettings().state.sessionMessagePageSize;
      const page = await sessionService.loadSessionMessagePage(
        targetSessionId,
        beforeRowId,
        messageLimit,
      );
      if (
        activeState.value !== state
        || state.sessionId !== targetSessionId
        || state.historyOldestRowId !== beforeRowId
      ) {
        return false;
      }
      const existingIds = new Set(state.messages.map((message) => message.id));
      const olderMessages = hydrateChatMessagesIntent(page.messages)
        .filter((message) => !existingIds.has(message.id));
      if (olderMessages.length > 0) {
        state.messages = [...olderMessages, ...state.messages];
      }
      const olderUserMessageIds = olderMessages
        .filter((message) => message.role === "user")
        .map((message) => message.id)
        .filter((messageId) => !state.userMessageIds.includes(messageId));
      if (olderUserMessageIds.length > 0) {
        state.userMessageIds = [...olderUserMessageIds, ...state.userMessageIds];
      }
      state.historyOldestRowId = page.oldestMessageRowId ?? null;
      state.historyHasMore = page.hasMoreHistory;
      return olderMessages.length > 0;
    } catch (error) {
      console.warn("[embedded-chat] load_session_message_page failed:", error);
      return false;
    } finally {
      if (state.sessionId === targetSessionId) state.historyLoading = false;
    }
  }

  function loadOlderHistory(): Promise<boolean> {
    return loadOlderSessionHistory();
  }

  async function loadSessionHistoryThroughMessage(messageId: string): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || !messageId) return false;
    while (
      activeState.value === state
      && state.sessionId === targetSessionId
      && !state.messages.some((message) => message.id === messageId)
      && state.historyHasMore
    ) {
      const loaded = await loadOlderSessionHistory();
      if (!loaded) break;
    }
    return activeState.value === state
      && state.sessionId === targetSessionId
      && state.messages.some((message) => message.id === messageId);
  }

  async function loadSessionTurnPreview(messageId: string): Promise<SessionTurnPreview | null> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || !messageId) return null;
    try {
      const preview = await sessionService.loadSessionTurnPreview(targetSessionId, messageId);
      return activeState.value === state && state.sessionId === targetSessionId ? preview : null;
    } catch (error) {
      console.warn("[embedded-chat] load_session_turn_preview failed:", error);
      return null;
    }
  }

  function applyAsyncTaskUpdate(update: AsyncTaskUpdatedEvent): boolean {
    const state = sessionStates.get(update.sessionId);
    if (!state || state.sessionId !== update.sessionId) return false;
    applyAsyncTaskUpdateToState(state, update);
    return true;
  }

  function applyExecutionState(
    targetSessionId: string,
    modelId: string,
    effort: EffortLevel,
    fastMode: boolean,
    multiAgentEnabled?: boolean,
  ): boolean {
    const state = sessionStates.get(targetSessionId);
    if (!state || state.sessionId !== targetSessionId) return false;
    state.sessionModelId = modelId;
    state.sessionEffort = effort;
    state.sessionFastMode = fastMode;
    if (multiAgentEnabled !== undefined) state.sessionMultiAgentEnabled = multiAgentEnabled;
    return true;
  }

  function restoreComposerDraft(draft: UserMessageDraft): void {
    activeRestoredDraft.value.value = draft;
  }

  async function forkFromMessage(
    messageId: string,
    title?: string | null,
  ): Promise<string | null> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || !messageId || state.isStreaming) return null;
    try {
      return await sessionService.forkSessionFromMessage(targetSessionId, messageId, title);
    } catch (error) {
      if (state.sessionId === targetSessionId) state.error = normalizeAppError(error).message;
      return null;
    }
  }

  async function forkSession(title?: string | null): Promise<string | null> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || state.isStreaming) return null;
    try {
      return await sessionService.forkSession(targetSessionId, title);
    } catch (error) {
      if (state.sessionId === targetSessionId) state.error = normalizeAppError(error).message;
      return null;
    }
  }

  async function checkUndoConflicts(assistantMessageId: string) {
    const state = activeState.value;
    if (!state.sessionId || !assistantMessageId) return [];
    return undoService.undoCheckConflicts(state.sessionId, assistantMessageId);
  }

  async function checkUndoDirty(assistantMessageId: string) {
    const state = activeState.value;
    if (!state.sessionId || !assistantMessageId) return [];
    return undoService.undoCheckDirty(state.sessionId, assistantMessageId);
  }

  async function performUndo(
    assistantMessageId: string,
    undoOptions?: { force?: boolean; acceptDirty?: boolean },
  ): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || !assistantMessageId || state.isStreaming) return false;
    state.error = null;
    try {
      await undoService.undoPerform(
        targetSessionId,
        assistantMessageId,
        undoOptions?.force ?? false,
        undoOptions?.acceptDirty ?? false,
      );
      return reloadSessionState(state, targetSessionId);
    } catch (error) {
      if (state.sessionId === targetSessionId) state.error = normalizeAppError(error).message;
      return false;
    }
  }

  async function rollbackConversation(targetMessageId: string | null): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || state.isStreaming) return false;
    state.error = null;
    try {
      if (targetMessageId) {
        await sessionService.rollbackSessionToMessage(targetSessionId, targetMessageId);
      } else {
        await sessionService.undoLatestConversationTurn(targetSessionId);
      }
      return reloadSessionState(state, targetSessionId);
    } catch (error) {
      state.error = normalizeAppError(error).message;
      return false;
    }
  }

  async function rollbackFilesAndConversation(
    targetMessageId: string | null,
    assistantMessageId: string,
    acceptDirty: boolean,
  ): Promise<boolean> {
    const state = activeState.value;
    const targetSessionId = state.sessionId;
    if (!targetSessionId || !assistantMessageId || state.isStreaming) return false;
    state.error = null;
    try {
      if (targetMessageId) {
        await undoService.undoPerformToMessage(
          targetSessionId,
          assistantMessageId,
          targetMessageId,
          false,
          acceptDirty,
        );
      } else {
        await undoService.undoPerform(
          targetSessionId,
          assistantMessageId,
          false,
          acceptDirty,
        );
      }
      return reloadSessionState(state, targetSessionId);
    } catch (error) {
      state.error = normalizeAppError(error).message;
      return false;
    }
  }

  async function answerQuestion(answer: string) {
    const state = activeState.value;
    const question = state.pendingQuestion;
    if (!question) return;
    state.pendingQuestion = null;
    try {
      await sessionService.answerQuestion(question.questionId, answer);
    } catch (error) {
      state.error = normalizeAppError(error).message;
    }
  }

  async function answerToolConfirm(questionId: string, answer: string) {
    const state = activeState.value;
    const toolConfirm = state.pendingToolConfirms.find((item) => item.questionId === questionId);
    if (!toolConfirm) return;
    state.pendingToolConfirms = state.pendingToolConfirms.filter((item) => item.questionId !== questionId);
    try {
      await sessionService.answerQuestion(toolConfirm.questionId, answer);
    } catch (error) {
      state.error = normalizeAppError(error).message;
    }
  }

  async function answerAllToolConfirms(questionIds: string[], answer: string) {
    const state = activeState.value;
    const toolConfirms = state.pendingToolConfirms.filter((item) => questionIds.includes(item.questionId));
    if (toolConfirms.length === 0) return;
    state.pendingToolConfirms = state.pendingToolConfirms.filter((item) => !questionIds.includes(item.questionId));
    await Promise.all(
      toolConfirms.map((item) =>
        sessionService.answerQuestion(item.questionId, answer).catch((error) => {
          state.error = normalizeAppError(error).message;
        })),
    );
  }

  async function applyKnowledgeProposal(proposalId: string) {
    const state = activeState.value;
    if (!state.sessionId) return;
    updateProposalStatus(state, "applying", proposalId);
    try {
      await sessionService.applyKnowledgeProposal(state.sessionId, proposalId);
      updateProposalStatus(state, "applied", proposalId);
    } catch (error) {
      state.error = normalizeAppError(error).message;
      updateProposalStatus(state, "stale", proposalId);
    }
  }

  async function ignoreKnowledgeProposal(proposalId: string) {
    const state = activeState.value;
    if (!state.sessionId) return;
    updateProposalStatus(state, "invalidated", proposalId);
    try {
      await sessionService.ignoreKnowledgeProposal(state.sessionId, proposalId);
    } catch (error) {
      state.error = normalizeAppError(error).message;
      updateProposalStatus(state, "stale", proposalId);
    }
  }

  function resetSession() {
    const previous = activeState.value;
    if (previous.sessionId) sessionStates.delete(previous.sessionId);

    // A durable session state can be shared by several editor panes. Starting a
    // new session detaches only this editor key so the other panes keep their
    // existing transcript and runtime state.
    const key = toValue(options.sessionKey);
    const fresh = rememberSharedEmbeddedChatState(key, createState(key));
    statesByKey.set(key, fresh);
    replaceActiveState(fresh);
    activeDraft.value.value = "";
    activeRestoredDraft.value.value = null;
  }

  function setExecutionSelection(selection: {
    agentId?: string | null;
    modelId?: string | null;
    effort?: EffortLevel | null;
    fastMode?: boolean | null;
    multiAgentEnabled?: boolean;
  }): void {
    const state = activeState.value;
    state.sessionAgentId = selection.agentId ?? null;
    state.sessionModelId = selection.modelId ?? null;
    state.sessionEffort = selection.effort ?? null;
    state.sessionFastMode = selection.fastMode ?? null;
    if (selection.multiAgentEnabled !== undefined) state.sessionMultiAgentEnabled = selection.multiAgentEnabled;
  }

  const inputText = computed({
    get: () => activeDraft.value.value,
    set: (value: string) => {
      activeDraft.value.value = value;
    },
  });
  const restoredComposerDraft = computed(() => activeRestoredDraft.value.value);

  function clearRestoredComposerDraft(draft?: UserMessageDraft | null): void {
    if (draft && activeRestoredDraft.value.value !== draft) return;
    activeRestoredDraft.value.value = null;
  }

  const activeKey = computed(() => toValue(options.sessionKey));
  const messages = computed(() => activeState.value.messages);
  // Embedded sessions have no typewriter: raw deltas would otherwise propagate
  // as new prop values per event, re-rendering the host transcript each time.
  // Coalesce both streaming strings to the shared cadence.
  const streamingText = useThrottledStreamingText(() => activeState.value.streamingText).text;
  const thinkingText = useThrottledStreamingText(() => activeState.value.streamingThinking).text;
  const thinkingStream = computed(() => activeState.value.thinkingStream);
  const streamingTextOrder = computed(() => activeState.value.streamingTextOrder);
  const thinkingOrder = computed(() => activeState.value.thinkingOrder);
  const liveRenderParts = computed(() => activeState.value.liveRenderParts);
  const livePartStreams = computed(() => activeState.value.livePartStreams);
  const isStreaming = computed(() => activeState.value.isStreaming);
  const isCancelling = computed(() => activeState.value.isCancelling);
  const isCompacting = computed(() => activeState.value.isCompacting);
  const compactQueued = computed(() => activeState.value.compactQueued);
  const isThinking = computed(() => activeState.value.isThinking);
  const hasThinking = computed(() => (
    activeState.value.streamingThinking.length > 0
    || activeState.value.liveRenderParts.some((part) => part.kind === "thinking")
  ));
  const thinkingDuration = computed(() => activeState.value.thinkingDuration);
  const activeToolCalls = computed(() => activeState.value.activeToolCalls);
  const tokenUsage = computed(() => activeState.value.tokenUsage);
  const todos = computed(() => activeState.value.todos);
  const todoRunBoundaryId = computed(() => (
    activeState.value.isStreaming
      ? activeState.value.currentRunId
      : activeState.value.latestCompletedRunId
  ));
  const currentTodos = computed(() => (
    todoRunBoundaryId.value
    && activeState.value.latestTodoRunId === todoRunBoundaryId.value
      ? activeState.value.todos
      : []
  ));
  const visibleTodos = computed(() => currentTodos.value);
  const hasAnyTodos = computed(() => activeState.value.todos.length > 0);
  const todoCelebrationVersion = computed(() => (
    todoRunBoundaryId.value
    && activeState.value.latestTodoRunId === todoRunBoundaryId.value
      ? activeState.value.todoWriteVersion
      : 0
  ));
  const showTodoPanel = computed({
    get: () => activeState.value.showTodoPanel,
    set: (value: boolean) => {
      activeState.value.showTodoPanel = value;
    },
  });
  const undoableMessageIds = computed(() => activeState.value.undoableMessageIds);
  const pendingQuestion = computed(() => activeState.value.pendingQuestion);
  const pendingToolConfirms = computed(() => activeState.value.pendingToolConfirms);
  const queuedFollowUp = computed(() => {
    const inputs = visiblePendingInputs(activeState.value.pendingInputs);
    if (inputs.length === 0) return null;
    return {
      inputs,
      canInsert: inputs.some((input) => pendingInputDelivery(input) !== "immediate"),
      isInserting: inputs.every((input) => pendingInputDelivery(input) === "immediate"),
      images: inputs.flatMap((input) => input.images ?? []),
      displayText: inputs
        .map((input) => input.displayText || input.text)
        .filter((text) => text.trim().length > 0)
        .join("\n"),
    };
  });
  const errorMessage = computed(() => activeState.value.error);
  const isLoading = computed(() => activeState.value.loading);
  const sessionId = computed(() => activeState.value.sessionId);
  const currentRunId = computed(() => activeState.value.currentRunId);
  const sessionAgentId = computed(() => activeState.value.sessionAgentId);
  const sessionModelId = computed(() => activeState.value.sessionModelId);
  const sessionEffort = computed(() => activeState.value.sessionEffort);
  const sessionFastMode = computed(() => activeState.value.sessionFastMode);
  const sessionMultiAgentEnabled = computed(() => activeState.value.sessionMultiAgentEnabled);
  const parentSessionId = computed(() => activeState.value.parentSessionId);
  const latestCompletedRunId = computed(() => activeState.value.latestCompletedRunId);
  const planModeActive = computed(() => activeState.value.planModeActive);
  const planFilePath = computed(() => activeState.value.planFilePath);
  const resumeAvailable = computed(() => activeState.value.resumeAvailable);
  const canResumeInterrupted = computed(() => (
    !!activeState.value.sessionId
    && !activeState.value.isStreaming
    && activeState.value.resumeAvailable
  ));
  const sessionHistoryHasMore = computed(() => activeState.value.historyHasMore);
  const sessionHistoryLoading = computed(() => activeState.value.historyLoading);
  const sessionUserMessageIds = computed(() => activeState.value.userMessageIds);

  const requestedSessionId = computed(() => toValue(options.initialSessionId)?.trim() ?? "");

  // Subscribe synchronously during setup. The app bootstrap owns the Tauri
  // listeners; this in-memory subscription has no async registration window
  // while an editor is reparented between workbench panes.
  const unsubscribeStreamEvents = subscribeSessionStreamEventConsumer(
    ({ event, source }) => {
      if (!sessionStreamSourceMatchesWorkspace(source, toValue(options.workspaceRef))) return null;
      return resolveStateForEvent(event);
    },
    ({ event }) => handleStreamEvent(event),
  );
  const unsubscribeAsyncTaskUpdates = subscribeSessionAsyncTaskUpdateConsumer(
    (update) => sessionStates.get(update.sessionId) ?? null,
    (update) => applyAsyncTaskUpdate(update),
  );
  const unsubscribeExecutionState = subscribeSessionExecutionStateConsumer(
    (update) => sessionStates.get(update.sessionId) ?? null,
    (update) => applyExecutionState(
      update.sessionId,
      update.modelId,
      update.effort,
      update.fastMode,
      update.multiAgentEnabled,
    ),
  );

  watch([activeKey, requestedSessionId], ([key, requestedId]) => {
    syncActiveState(key, requestedId || null);
    if (requestedId) void hydrateExistingSession(activeState.value, requestedId);
  }, { immediate: true });

  onUnmounted(() => {
    unsubscribeStreamEvents();
    unsubscribeAsyncTaskUpdates();
    unsubscribeExecutionState();
    releaseSharedEmbeddedChatState(activeState.value);
  });

  return {
    inputText,
    restoredComposerDraft,
    clearRestoredComposerDraft,
    messages,
    streamingText,
    thinkingText,
    thinkingStream,
    streamingTextOrder,
    thinkingOrder,
    liveRenderParts,
    livePartStreams,
    isStreaming,
    isCancelling,
    isCompacting,
    compactQueued,
    isThinking,
    hasThinking,
    thinkingDuration,
    activeToolCalls,
    tokenUsage,
    todos,
    currentTodos,
    visibleTodos,
    hasAnyTodos,
    todoCelebrationVersion,
    showTodoPanel,
    undoableMessageIds,
    pendingQuestion,
    pendingToolConfirms,
    queuedFollowUp,
    errorMessage,
    isLoading,
    sessionId,
    currentRunId,
    sessionAgentId,
    sessionModelId,
    sessionEffort,
    sessionFastMode,
    sessionMultiAgentEnabled,
    parentSessionId,
    latestCompletedRunId,
    planModeActive,
    planFilePath,
    resumeAvailable,
    canResumeInterrupted,
    sessionHistoryHasMore,
    sessionHistoryLoading,
    sessionUserMessageIds,
    send,
    sendComposerPayload,
    resumeInterrupted,
    compact,
    insertQueuedFollowUp,
    deleteQueuedFollowUp,
    reEditQueuedFollowUp,
    cancel,
    setPlanMode,
    exitPlanMode,
    loadOlderSessionHistory,
    loadOlderHistory,
    loadSessionHistoryThroughMessage,
    loadSessionTurnPreview,
    applyAsyncTaskUpdate,
    applyExecutionState,
    restoreComposerDraft,
    forkFromMessage,
    forkSession,
    checkUndoConflicts,
    checkUndoDirty,
    performUndo,
    rollbackConversation,
    rollbackFilesAndConversation,
    resetSession,
    setExecutionSelection,
    answerQuestion,
    answerToolConfirm,
    answerAllToolConfirms,
    applyKnowledgeProposal,
    ignoreKnowledgeProposal,
  };
}
