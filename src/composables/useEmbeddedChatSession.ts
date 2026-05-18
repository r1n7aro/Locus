import {
  computed,
  onMounted,
  onUnmounted,
  reactive,
  shallowRef,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from "vue";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import * as sessionService from "../services/session";
import { hydrateChatMessagesIntent, withClientMessageId } from "./chatInputIntents";
import { useChatInputSettings } from "./useChatInputSettings";
import {
  buildToolResultMessages,
  mergeUserMessage,
  reduceStreamEvent,
  type StreamMutation,
  type StreamState,
} from "./useStreamReducer";
import type {
  ChatMessage,
  EffortLevel,
  ImageAttachment,
  AssetRefAttachment,
  PendingQuestion,
  PendingToolConfirm,
  StreamEvent,
  TokenUsage,
  ToolCallDisplay,
  UserIntentMeta,
  AssistantRenderPart,
  PendingSessionInput,
} from "../types";

export interface EmbeddedChatRequest {
  text: string;
  displayText?: string;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  images?: ImageAttachment[] | null;
  assetRefs?: AssetRefAttachment[] | null;
}

interface EmbeddedChatState extends StreamState {
  key: string;
  sessionId: string | null;
  currentRunId: string | null;
  inputText: string;
  error: string | null;
  pendingRun: boolean;
  pendingInputs: PendingSessionInput[];
  acceptedPendingInputIds: Set<string>;
  localMergeGroupId: string | null;
  localFallbackMergeGroupId: string | null;
}

export interface UseEmbeddedChatSessionOptions {
  sessionKey: MaybeRefOrGetter<string>;
  sessionType?: string;
  sessionTitle?: MaybeRefOrGetter<string | null | undefined>;
  selectedModelId: MaybeRefOrGetter<string>;
  selectedAgentId?: MaybeRefOrGetter<string | null | undefined>;
  effort?: MaybeRefOrGetter<EffortLevel | null | undefined>;
  effortSupported?: MaybeRefOrGetter<boolean | undefined>;
  buildRequest: (input: string) => EmbeddedChatRequest | null;
}

function emptyTokenUsage(): TokenUsage {
  return {
    totalInputTokens: 0,
    totalOutputTokens: 0,
    totalCacheReadTokens: 0,
    totalCacheWriteTokens: 0,
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
    currentRunId: null,
    inputText: "",
    error: null,
    pendingRun: false,
    pendingInputs: [],
    acceptedPendingInputIds: new Set<string>(),
    localMergeGroupId: null,
    localFallbackMergeGroupId: null,
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
  });
}

function replaceMessageById(list: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = list.findIndex((item) => item.id === message.id);
  if (index < 0) return [...list, message];
  const next = [...list];
  next.splice(index, 1, message);
  return next;
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

function clearState(state: EmbeddedChatState) {
  state.sessionId = null;
  state.currentRunId = null;
  state.inputText = "";
  state.error = null;
  state.pendingRun = false;
  state.pendingInputs = [];
  state.acceptedPendingInputIds.clear();
  state.localMergeGroupId = null;
  state.localFallbackMergeGroupId = null;
  state.messages = [];
  state.pendingQuestion = null;
  state.pendingToolConfirms = [];
  state.tokenUsage = emptyTokenUsage();
  state.todos = [];
  state.showTodoPanel = false;
  state.undoableMessageIds = new Set<string>();
  state.streamSequence = 0;
  state.isCompacting = false;
  resetRoundState(state);
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
}

function applyMutation(state: EmbeddedChatState, mutation: StreamMutation) {
  switch (mutation.type) {
    case "appendRawText":
      state.rawStreamText += mutation.text;
      state.streamingText = state.rawStreamText;
      break;
    case "appendThinking":
      state.streamingThinking += mutation.text;
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
      state.liveRenderParts = state.liveRenderParts.map((part) => {
        if (part.id !== mutation.partId) return part;
        if (part.kind !== "thinking" && part.kind !== "text") return part;
        return { ...part, content: part.content + mutation.text };
      });
      break;
    case "deactivateLiveThinkingParts":
      state.liveRenderParts = state.liveRenderParts.map((part) =>
        part.kind === "thinking"
          ? { ...part, active: false, duration: mutation.duration ?? part.duration }
          : part,
      );
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
      break;
    case "upsertMessage": {
      state.messages = replaceMessageById(state.messages, mutation.message);
      break;
    }
    case "upsertUserMessage":
      state.messages = mergeUserMessage(state.messages, mutation.message);
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
        for (const message of buildToolResultMessages(sourceToolCalls)) {
          state.messages = replaceMessageById(state.messages, message);
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
      break;
    case "setTodos":
    case "addUndoable":
    case "canvasAutoOpen":
      break;
  }
}

export function useEmbeddedChatSession(options: UseEmbeddedChatSessionOptions) {
  const statesByKey = new Map<string, EmbeddedChatState>();
  const sessionIdToKey = new Map<string, string>();
  const activeState = shallowRef<EmbeddedChatState>(createState(toValue(options.sessionKey)));

  function ensureState(key: string) {
    const existing = statesByKey.get(key);
    if (existing) return existing;
    const created = createState(key);
    statesByKey.set(key, created);
    return created;
  }

  function syncActiveState(key: string) {
    activeState.value = ensureState(key);
  }

  function resolveStateForEvent(event: StreamEvent) {
    const mappedKey = sessionIdToKey.get(event.sessionId);
    if (mappedKey) {
      return statesByKey.get(mappedKey) ?? null;
    }
    if (event.type !== "runStart") return null;
    const pendingState = [...statesByKey.values()].find((state) => state.pendingRun && !state.sessionId);
    if (!pendingState) return null;
    pendingState.sessionId = event.sessionId;
    pendingState.currentRunId = event.runId;
    pendingState.pendingRun = false;
    sessionIdToKey.set(event.sessionId, pendingState.key);
    return pendingState;
  }

  async function reloadSessionMessagesAfterError(state: EmbeddedChatState, sessionId: string) {
    try {
      const detail = await sessionService.loadSession(sessionId);
      if (state.sessionId !== sessionId) return;
      state.messages = hydrateChatMessagesIntent(detail.messages);
      state.pendingInputs = visiblePendingInputs(detail.pendingInputs ?? []);
    } catch (error) {
      console.warn("[embedded-chat] loadSession after stream error failed:", error);
    }
  }

  function handleStreamEvent(event: StreamEvent) {
    const state = resolveStateForEvent(event);
    if (!state) return;

    if (state.currentRunId && event.runId !== state.currentRunId) return;
    if (!state.currentRunId) state.currentRunId = event.runId;

    if (event.type === "runStart") {
      state.isStreaming = true;
      state.error = null;
      return;
    }

    if (event.type === "pendingInputQueued") {
      if (state.acceptedPendingInputIds.has(event.input.id)) return;
      state.pendingInputs = visiblePendingInputs(
        mergePendingInputList(state.pendingInputs, event.input),
      );
      return;
    }

    if (event.type === "pendingInputAccepted") {
      state.acceptedPendingInputIds.add(event.pendingInputId);
      state.pendingInputs = state.pendingInputs.filter((input) => input.id !== event.pendingInputId);
      state.localMergeGroupId = null;
      state.localFallbackMergeGroupId = null;
      return;
    }

    const mutations = reduceStreamEvent(state, event);
    for (const mutation of mutations) {
      applyMutation(state, mutation);
    }

    if (event.type === "error") {
      state.error = normalizeAppError(event.error).message;
      state.currentRunId = null;
      state.pendingRun = false;
      void reloadSessionMessagesAfterError(state, event.sessionId);
      return;
    }

    if (event.type === "done" || event.type === "cancelled") {
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
        if (!queuedRequest) {
          state.localMergeGroupId = null;
          state.localFallbackMergeGroupId = null;
        }
      }
      state.currentRunId = null;
      state.pendingRun = false;
      if (queuedRequest) {
        globalThis.setTimeout(() => {
          void send(queuedRequest);
        }, 0);
      }
    }
  }

  async function send(requestOverride?: EmbeddedChatRequest | null) {
    const state = activeState.value;

    const input = state.inputText.trim();
    const request = requestOverride ?? (input ? options.buildRequest(input) : null);
    if (!request) return;
    if (!requestOverride && !input) return;

    const selectedModelId = toValue(options.selectedModelId)?.trim() ?? "";
    if (!selectedModelId) {
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
        state.inputText = "";
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
          state.inputText = "";
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

    state.messages.push({
      id: pendingMessageId,
      role: "user",
      content: displayText,
      createdAt: Date.now() / 1000,
      images: request.images && request.images.length > 0 ? request.images : undefined,
      assetRefs: request.assetRefs && request.assetRefs.length > 0 ? request.assetRefs : undefined,
      thinkingSignature: userIntentSignature,
      intentMeta: userIntent,
    });

    state.inputText = "";
    state.error = null;
    state.pendingQuestion = null;
    state.pendingToolConfirms = [];
    state.streamSequence = 0;
    state.isCompacting = false;
    resetRoundState(state);
    state.isStreaming = true;
    state.pendingRun = true;

    try {
      const launch = await sessionService.chat({
        sessionId: state.sessionId,
        text: request.text,
        sessionTitle: toValue(options.sessionTitle) ?? null,
        agentId: toValue(options.selectedAgentId) ?? null,
        model: selectedModelId,
        effort: toValue(options.effortSupported) ? (toValue(options.effort) ?? null) : null,
        images: request.images && request.images.length > 0 ? request.images : null,
        assetRefs: request.assetRefs && request.assetRefs.length > 0 ? request.assetRefs : null,
        sessionType: options.sessionType ?? "chat",
        mode: request.mode ?? null,
        userIntent,
      });

      state.sessionId = launch.sessionId;
      state.currentRunId = launch.runId;
      state.pendingRun = false;
      sessionIdToKey.set(launch.sessionId, state.key);
    } catch (error) {
      state.isStreaming = false;
      state.pendingRun = false;
      state.isCompacting = false;
      state.messages = state.messages.filter((message) => message.id !== pendingMessageId);
      resetRoundState(state);
      state.error = normalizeAppError(error).message;
    }
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

  async function cancel() {
    const state = activeState.value;
    if (!state.sessionId || !state.isStreaming) return;
    try {
      await sessionService.cancelChat(state.sessionId);
    } catch (error) {
      state.error = normalizeAppError(error).message;
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
    const state = activeState.value;
    if (state.sessionId) {
      sessionIdToKey.delete(state.sessionId);
    }
    clearState(state);
  }

  const inputText = computed({
    get: () => activeState.value.inputText,
    set: (value: string) => {
      activeState.value.inputText = value;
    },
  });

  const activeKey = computed(() => toValue(options.sessionKey));
  const messages = computed(() => activeState.value.messages);
  const streamingText = computed(() => activeState.value.streamingText);
  const thinkingText = computed(() => activeState.value.streamingThinking);
  const streamingTextOrder = computed(() => activeState.value.streamingTextOrder);
  const thinkingOrder = computed(() => activeState.value.thinkingOrder);
  const liveRenderParts = computed(() => activeState.value.liveRenderParts);
  const isStreaming = computed(() => activeState.value.isStreaming);
  const isCompacting = computed(() => activeState.value.isCompacting);
  const isThinking = computed(() => activeState.value.isThinking);
  const thinkingDuration = computed(() => activeState.value.thinkingDuration);
  const activeToolCalls = computed(() => activeState.value.activeToolCalls);
  const pendingQuestion = computed(() => activeState.value.pendingQuestion);
  const pendingToolConfirms = computed(() => activeState.value.pendingToolConfirms);
  const queuedFollowUp = computed(() => {
    const inputs = visiblePendingInputs(activeState.value.pendingInputs);
    if (inputs.length === 0) return null;
    return {
      inputs,
      canInsert: inputs.some((input) => pendingInputDelivery(input) !== "immediate"),
      isInserting: inputs.every((input) => pendingInputDelivery(input) === "immediate"),
      displayText: inputs
        .map((input) => input.displayText || input.text)
        .filter((text) => text.trim().length > 0)
        .join("\n"),
    };
  });
  const errorMessage = computed(() => activeState.value.error);
  const sessionId = computed(() => activeState.value.sessionId);

  watch(activeKey, (key) => {
    syncActiveState(key);
  }, { immediate: true });

  let unlisten: RuntimeUnsubscribe | null = null;
  let destroyed = false;

  onMounted(async () => {
    const release = await getLocusRuntime().subscribe<StreamEvent>("stream-event", (payload) => {
      handleStreamEvent(payload);
    });
    if (destroyed) {
      release();
      return;
    }
    unlisten = release;
  });

  onUnmounted(() => {
    destroyed = true;
    unlisten?.();
  });

  return {
    inputText,
    messages,
    streamingText,
    thinkingText,
    streamingTextOrder,
    thinkingOrder,
    liveRenderParts,
    isStreaming,
    isCompacting,
    isThinking,
    thinkingDuration,
    activeToolCalls,
    pendingQuestion,
    pendingToolConfirms,
    queuedFollowUp,
    errorMessage,
    sessionId,
    send,
    insertQueuedFollowUp,
    cancel,
    resetSession,
    answerQuestion,
    answerToolConfirm,
    answerAllToolConfirms,
    applyKnowledgeProposal,
    ignoreKnowledgeProposal,
  };
}
