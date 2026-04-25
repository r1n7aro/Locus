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
import {
  buildToolResultMessages,
  reduceStreamEvent,
  type StreamMutation,
  type StreamState,
} from "./useStreamReducer";
import type {
  ChatMessage,
  EffortLevel,
  ImageAttachment,
  PendingQuestion,
  PendingToolConfirm,
  StreamEvent,
  TokenUsage,
  ToolCallDisplay,
  UserIntentMeta,
} from "../types";

export interface EmbeddedChatRequest {
  text: string;
  displayText?: string;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  images?: ImageAttachment[] | null;
}

interface EmbeddedChatState extends StreamState {
  key: string;
  sessionId: string | null;
  currentRunId: string | null;
  inputText: string;
  error: string | null;
  pendingRun: boolean;
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
    messages: [] as ChatMessage[],
    streamingText: "",
    rawStreamText: "",
    streamingThinking: "",
    isStreaming: false,
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

function clearState(state: EmbeddedChatState) {
  state.sessionId = null;
  state.currentRunId = null;
  state.inputText = "";
  state.error = null;
  state.pendingRun = false;
  state.messages = [];
  state.pendingQuestion = null;
  state.pendingToolConfirms = [];
  state.tokenUsage = emptyTokenUsage();
  state.todos = [];
  state.showTodoPanel = false;
  state.undoableMessageIds = new Set<string>();
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
    case "pushMessage":
      state.messages.push(mutation.message);
      break;
    case "upsertMessage": {
      const index = state.messages.findIndex((item) => item.id === mutation.message.id);
      if (index >= 0) {
        const next = [...state.messages];
        next.splice(index, 1, mutation.message);
        state.messages = next;
      } else {
        state.messages = [...state.messages, mutation.message];
      }
      break;
    }
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
    case "pushToolResults":
      state.messages.push(...buildToolResultMessages(state.activeToolCalls));
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

    const mutations = reduceStreamEvent(state, event);
    for (const mutation of mutations) {
      applyMutation(state, mutation);
    }

    if (event.type === "error") {
      state.error = normalizeAppError(event.error).message;
      state.currentRunId = null;
      state.pendingRun = false;
      return;
    }

    if (event.type === "done" || event.type === "cancelled") {
      state.currentRunId = null;
      state.pendingRun = false;
    }
  }

  async function send(requestOverride?: EmbeddedChatRequest | null) {
    const state = activeState.value;
    if (state.isStreaming) return;

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

    state.messages.push({
      id: `embedded_user_${Date.now()}`,
      role: "user",
      content: displayText,
      createdAt: Date.now() / 1000,
      images: request.images && request.images.length > 0 ? request.images : undefined,
      thinkingSignature: request.userIntent ? JSON.stringify(request.userIntent) : undefined,
      intentMeta: request.userIntent ?? undefined,
    });

    state.inputText = "";
    state.error = null;
    state.pendingQuestion = null;
    state.pendingToolConfirms = [];
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
        sessionType: options.sessionType ?? "chat",
        mode: request.mode ?? null,
        userIntent: request.userIntent ?? null,
      });

      state.sessionId = launch.sessionId;
      state.currentRunId = launch.runId;
      state.pendingRun = false;
      sessionIdToKey.set(launch.sessionId, state.key);
    } catch (error) {
      state.isStreaming = false;
      state.pendingRun = false;
      resetRoundState(state);
      state.error = normalizeAppError(error).message;
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
  const isStreaming = computed(() => activeState.value.isStreaming);
  const isThinking = computed(() => activeState.value.isThinking);
  const thinkingDuration = computed(() => activeState.value.thinkingDuration);
  const activeToolCalls = computed(() => activeState.value.activeToolCalls);
  const pendingQuestion = computed(() => activeState.value.pendingQuestion);
  const pendingToolConfirms = computed(() => activeState.value.pendingToolConfirms);
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
    isStreaming,
    isThinking,
    thinkingDuration,
    activeToolCalls,
    pendingQuestion,
    pendingToolConfirms,
    errorMessage,
    sessionId,
    send,
    cancel,
    resetSession,
    answerQuestion,
    answerToolConfirm,
    answerAllToolConfirms,
    applyKnowledgeProposal,
    ignoreKnowledgeProposal,
  };
}
