<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import ChatTranscript from "../components/chat/ChatTranscript.vue";
import RichChatInput from "../components/chat/RichChatInput.vue";
import AskUserCard from "../components/chat/AskUserCard.vue";
import ToolConfirmCard from "../components/chat/ToolConfirmCard.vue";
import ChatStatusIndicators from "../components/chat/ChatStatusIndicators.vue";
import BackgroundTasksIndicator from "../components/chat/BackgroundTasksIndicator.vue";
import ModelEffortSelector from "../components/ModelEffortSelector.vue";
import type { IntentCommandType } from "../composables/chatInputIntents";
import {
  buildChatMessageClipboardPayload,
  copyableChatMessageText,
  writeChatMessageClipboard,
} from "../composables/chatMessageDraft";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { useEmbeddedChatSession } from "../composables/useEmbeddedChatSession";
import { useKnowledgeAccessMode } from "../composables/useKnowledgeAccessMode";
import { useSkills } from "../composables/useSkills";
import { useWorkspaceAssetDbStatus } from "../composables/useWorkspaceAssetDbStatus";
import { useWorkspaceUnityStatus } from "../composables/useWorkspaceUnityStatus";
import { listAgents, listWorkspaceAgents } from "../services/agent";
import { openRemoteReference } from "../services/remoteClient";
import { saveSessionExecutionState } from "../services/session";
import { markSessionAttentionRead, sessionAttention } from "../services/sessionAttention";
import type { WorkspaceRef } from "../services/project";
import { useModelStore } from "../stores/model";
import { useSelectionMemoryStore } from "../stores/selectionMemory";
import type {
  AgentInfo,
  AppErrorPayload,
  ChatComposerSendPayload,
  ChatMessage,
  EffortLevel,
  PendingToolConfirm,
  QuestionAnswerPayload,
} from "../types";
import { t } from "../i18n";
import { checkoutDescriptor } from "./api";
import { prepareImageAttachment } from "./images";
import { loadModelCatalog } from "./models";
import { referenceFromElement } from "./references";

const props = defineProps<{
  sessionKey: string;
  sessionId: string | null;
  workspaceRef: WorkspaceRef | null;
  title: string;
  canApproveTools: boolean;
  connected: boolean;
  /** The screen is on top of the navigation stack (others stay mounted). */
  active: boolean;
}>();

const emit = defineEmits<{
  back: [];
  sessionCreated: [sessionId: string];
  openImage: [src: string];
  /** A fork of this session was created. */
  openSession: [payload: { sessionId: string; title: string }];
  /** `/clear`: continue in a new conversation of the same workspace. */
  newSession: [];
}>();

const modelStore = useModelStore();
const selectionMemory = useSelectionMemoryStore();
const { state: displaySettings } = useDisplaySettings();
const { state: knowledgeAccessState, setMode: setKnowledgeAccessMode } = useKnowledgeAccessMode();
const launchError = ref<string | null>(null);
const statusError = ref<string | null>(null);

// ----- workspace facts for the status row (from the project list).
const checkout = computed(() => checkoutDescriptor(props.workspaceRef?.checkoutId));
const workingDir = computed(() => checkout.value?.root ?? "");
const projectServices = computed(() => checkout.value?.runtime?.detectedServices ?? []);
const isUnityWorkspace = computed(() => (
  projectServices.value.some((serviceId) => serviceId.trim().toLowerCase() === "unity")
));
const knowledgeEnabled = computed(() => (
  (checkout.value?.knowledgeEnabled ?? checkout.value?.runtime?.knowledgeEnabled) !== false
));
const knowledgeAccessMode = computed(() => (knowledgeEnabled.value ? knowledgeAccessState.mode : "disabled"));

// ----- Agent, model and effort. Same rules as a desktop session tab: values
// the session or the user chose win; until then a new session follows the
// workspace's remembered choice. Choices are not written back to that memory
// (desktop only); an existing session saves its own execution state.
const agents = ref<AgentInfo[]>([]);
const agentsLoaded = ref(false);
const explicitAgentId = ref("");
const explicitModelId = ref<string | null>(null);
const explicitEffort = ref<EffortLevel | null>(null);
const explicitFastMode = ref<boolean | null>(null);
const multiAgentEnabled = ref(false);

const checkoutId = computed(() => props.workspaceRef?.checkoutId ?? null);
const selectedAgentId = computed(() => {
  const current = explicitAgentId.value.trim();
  // The session's Agent counts even when the catalog could not be read.
  if (current && (agents.value.length === 0 || agents.value.some((agent) => agent.id === current))) return current;
  if (!agentsLoaded.value) return "";
  return selectionMemory.resolveAgentId(checkoutId.value, agents.value, { forNewSession: !props.sessionId });
});
const selectedAgent = computed(() => agents.value.find((agent) => agent.id === selectedAgentId.value) ?? null);
const resolvedExecution = computed(() => selectionMemory.resolveExecution(
  checkoutId.value,
  selectedAgent.value ?? (selectedAgentId.value ? { id: selectedAgentId.value } : null),
));
const selectedModelId = computed(() => (
  explicitModelId.value ?? (resolvedExecution.value.modelId || modelStore.selectedModelId)
));
const efforts = computed<EffortLevel[]>(() => modelStore.effortsForModel(selectedModelId.value));
const effortSupported = computed(() => efforts.value.length > 0);
const effort = computed<EffortLevel>(() => selectionMemory.clampEffort(
  selectedModelId.value,
  explicitEffort.value ?? resolvedExecution.value.effort,
));
const fastModeAvailable = computed(() => {
  const model = modelStore.availableModels.find((candidate) => candidate.id === selectedModelId.value);
  return model?.provider === "openai_codex" || model?.additionalSpeedTiers?.includes("fast") === true;
});
const fastMode = computed(() => (
  fastModeAvailable.value && (explicitFastMode.value ?? resolvedExecution.value.fastMode)
));

const chat = useEmbeddedChatSession({
  sessionKey: () => props.sessionKey,
  initialSessionId: () => props.sessionId,
  workspaceRef: () => props.workspaceRef,
  sessionType: "chat",
  sessionTitle: null,
  selectedModelId: () => selectedModelId.value,
  selectedAgentId: () => selectedAgentId.value || null,
  effort: () => effort.value,
  effortSupported: () => effortSupported.value,
  fastMode: () => fastMode.value,
  multiAgentEnabled: () => multiAgentEnabled.value,
  knowledgeMode: () => knowledgeAccessMode.value,
  buildRequest: (input) => ({ text: input, displayText: input }),
});

const {
  inputText,
  messages,
  streamingText,
  streamingTextOrder,
  thinkingOrder,
  liveRenderParts,
  livePartStreams,
  isStreaming,
  isCancelling,
  isCompacting,
  isThinking,
  claudeCodePhase,
  hasThinking,
  thinkingDuration,
  activeToolCalls,
  pendingQuestion,
  pendingToolConfirms,
  queuedFollowUp,
  errorMessage,
  isLoading,
  sessionId,
  planModeActive,
  restoredComposerDraft,
  clearRestoredComposerDraft,
  canResumeInterrupted,
  sessionHistoryHasMore,
  sessionHistoryLoading,
} = chat;

// The loaded session (and every launch) reports what it runs with.
watch(
  [chat.sessionAgentId, chat.sessionModelId, chat.sessionEffort, chat.sessionFastMode, chat.sessionMultiAgentEnabled] as const,
  ([agentId, modelId, sessionEffort, sessionFastMode, sessionMultiAgent]) => {
    if (agentId) explicitAgentId.value = agentId;
    if (modelId) explicitModelId.value = modelId;
    if (sessionEffort) explicitEffort.value = sessionEffort;
    if (sessionFastMode != null) explicitFastMode.value = sessionFastMode;
    multiAgentEnabled.value = sessionMultiAgent;
  },
  { immediate: true },
);

// A brand-new session gets its id from the first launch.
watch(sessionId, (id) => {
  if (id && id !== props.sessionId) emit("sessionCreated", id);
});

let agentLoadSeq = 0;
async function loadAgentCatalog() {
  const seq = ++agentLoadSeq;
  const workspaceRef = props.workspaceRef ? { ...props.workspaceRef } : null;
  try {
    const [list] = await Promise.all([
      workspaceRef ? listWorkspaceAgents(workspaceRef) : listAgents(),
      // The remembered choice must be ready before a new session resolves.
      selectionMemory.load(workspaceRef),
    ]);
    if (seq === agentLoadSeq) agents.value = list;
  } catch (error) {
    console.warn("[remote] agent list failed", error);
  } finally {
    if (seq === agentLoadSeq) agentsLoaded.value = true;
  }
}

watch(() => props.workspaceRef?.checkoutId, () => void loadAgentCatalog(), { immediate: true });

const modelCatalogReady = loadModelCatalog().catch(() => {});

const isClaudeCode = computed(() => selectedModelId.value.startsWith("claude_code/"));
const notice = computed(() => launchError.value || statusError.value || errorMessage.value);
const selectorAgents = computed(() => (
  displaySettings.showAgentSelector && agents.value.length > 0 ? agents.value : undefined
));

// ----- selector actions (mirrors WorkbenchSessionEditor).
let executionStateSaveQueue: Promise<unknown> = Promise.resolve();

function persistExecutionSelection() {
  const targetSessionId = sessionId.value;
  if (!targetSessionId) return;
  const modelId = selectedModelId.value;
  const level = effort.value;
  const fast = fastMode.value;
  const multiAgent = multiAgentEnabled.value;
  executionStateSaveQueue = executionStateSaveQueue
    .catch(() => undefined)
    .then(() => saveSessionExecutionState(targetSessionId, modelId, level, fast, multiAgent))
    .catch((error: unknown) => console.warn("[remote] save_session_execution_state failed", error));
}

function selectAgent(agentId: string) {
  const agent = agents.value.find((item) => item.id === agentId);
  const rememberedModel = selectionMemory.snapshotFor(checkoutId.value)?.memory.agents[agentId];
  explicitAgentId.value = agentId;
  if (!sessionId.value && (explicitModelId.value === null || rememberedModel)) {
    // A new session switches to the model and effort this Agent last used here.
    explicitModelId.value = null;
    explicitEffort.value = null;
  } else {
    const fallbackEffort = selectionMemory.fallbackEffort(agent);
    if (efforts.value.includes(fallbackEffort)) explicitEffort.value = fallbackEffort;
  }
}

function selectModel(modelId: string) {
  if (!modelStore.availableModels.some((model) => model.id === modelId)) return;
  explicitModelId.value = modelId;
  if (!fastModeAvailable.value) explicitFastMode.value = false;
  persistExecutionSelection();
}

function selectEffort(level: EffortLevel) {
  if (!efforts.value.includes(level)) return;
  explicitEffort.value = level;
  persistExecutionSelection();
}

function selectFastMode(enabled: boolean) {
  explicitFastMode.value = fastModeAvailable.value && enabled;
  persistExecutionSelection();
}

function selectMultiAgent(enabled: boolean) {
  multiAgentEnabled.value = enabled;
  persistExecutionSelection();
}

// ----- status row: Unity and asset database state of Unity workspaces.
function reportStatusError(error: AppErrorPayload) {
  statusError.value = error.message;
}

const {
  scanPhase,
  lastScanStats,
  startScan,
} = useWorkspaceAssetDbStatus({
  workspaceRef: () => props.workspaceRef,
  enabled: isUnityWorkspace,
  onScanError: reportStatusError,
});
const {
  connected: unityConnected,
  connectionStatus: unityConnectionStatus,
  pluginStatus: unityPluginStatus,
  pluginInstalling: unityPluginInstalling,
  launching: unityLaunching,
  launchState: unityLaunchState,
  launch: launchUnity,
  installPlugin: installUnityPlugin,
} = useWorkspaceUnityStatus({
  workspaceRef: () => props.workspaceRef,
  enabled: isUnityWorkspace,
  onError: reportStatusError,
});

function runStatusAction(action: () => Promise<boolean>) {
  statusError.value = null;
  void action();
}

// Periodic status reads stop while another screen is on top or the app is
// in the background.
const pageHidden = ref(typeof document !== "undefined" && document.visibilityState === "hidden");
function onVisibilityChange() {
  pageHidden.value = document.visibilityState === "hidden";
}
onMounted(() => document.addEventListener("visibilitychange", onVisibilityChange));
onBeforeUnmount(() => document.removeEventListener("visibilitychange", onVisibilityChange));
const statusPollingPaused = computed(() => !props.active || pageHidden.value);

// ----- unread results: the session on screen is read, on every client.
const readFailures = new Set<string>();
const readTarget = computed(() => {
  const id = sessionId.value;
  const entry = id ? sessionAttention.value.sessions[id] : undefined;
  if (!id || !entry || entry.sequence <= entry.readSequence) return null;
  if (!props.active || pageHidden.value || !props.connected || isLoading.value) return null;
  return { id, sequence: entry.sequence, key: `${id}\u0000${entry.sequence}` };
});
watch(() => readTarget.value?.key, () => {
  const target = readTarget.value;
  // A refused acknowledgement is not retried for the same result.
  if (!target || readFailures.has(target.key)) return;
  void markSessionAttentionRead(target.id, target.sequence).catch(() => readFailures.add(target.key));
}, { immediate: true });

// ----- composer: mentions (@), skills and commands (/), images.
const composerRef = ref<InstanceType<typeof RichChatInput> | null>(null);
const { skillItems, loadSkills } = useSkills(() => props.workspaceRef);
/** Actions that need the desktop's windows, dialogs or Unity view. */
const HIDDEN_COMMANDS: readonly IntentCommandType[] = [
  "undo",
  "export-context",
  "review-context",
  "garbage-collection",
  "capture-scene",
  "capture-game",
];

async function beforeSend(): Promise<boolean> {
  launchError.value = null;
  statusError.value = null;
  // Sent right after opening the page: the default model may still be loading.
  if (!selectedModelId.value) await modelCatalogReady;
  if (!selectedModelId.value) {
    launchError.value = t("mobile.session.noModel");
    return false;
  }
  if (!props.sessionId && !props.workspaceRef) {
    launchError.value = t("mobile.session.workspaceClosed");
    return false;
  }
  return true;
}

function sendComposerPayload(payload: ChatComposerSendPayload) {
  stickToBottom.value = true;
  chat.sendComposerPayload(payload);
}

async function forkCurrentSession() {
  const title = t("chat.session.forkTitle", props.title || t("mobile.session.untitled"));
  const forkedId = await chat.forkSession(title);
  if (forkedId) emit("openSession", { sessionId: forkedId, title });
}

// A run cancelled before any reply gives its message back as a draft.
watch(restoredComposerDraft, async (draft) => {
  if (!draft) return;
  await nextTick();
  await composerRef.value?.applyDraftPrefill(draft);
  clearRestoredComposerDraft(draft);
}, { flush: "post" });

const imageInput = ref<HTMLInputElement | null>(null);
const imagesPending = ref(false);

function pickImages() {
  imageInput.value?.click();
}

async function onImagesPicked(event: Event) {
  const input = event.target as HTMLInputElement;
  const files = [...(input.files ?? [])];
  input.value = "";
  if (files.length === 0) return;
  imagesPending.value = true;
  try {
    for (const file of files) {
      let image;
      try {
        image = await prepareImageAttachment(file);
      } catch (error) {
        console.warn("[remote] image could not be read", error);
        showToast(t("mobile.session.imageFailed"));
        continue;
      }
      if (!composerRef.value?.attachImage(image)) {
        showToast(t("capture.imageLimit"));
        break;
      }
    }
  } finally {
    imagesPending.value = false;
  }
}

function answerQuestion(answer: QuestionAnswerPayload, questionId: string) {
  void chat.answerQuestion(answer, questionId);
}

function answerToolConfirm(confirm: PendingToolConfirm, answer: string) {
  void chat.answerToolConfirm(confirm.questionId, answer);
}

function confirmLabel(confirm: PendingToolConfirm): string {
  return confirm.display.kind === "basic" ? confirm.display.toolName : t("mobile.session.knowledgeChange");
}

/** File, folder and knowledge references in messages open on the phone. The
 * capture phase runs before the renderer's own link handling. */
function onTranscriptClick(event: MouseEvent) {
  if (!(event.target instanceof Element)) return;
  const reference = referenceFromElement(event.target);
  if (!reference) return;
  event.preventDefault();
  event.stopPropagation();
  openRemoteReference({ ...reference, workspaceRef: props.workspaceRef });
}

// ----- toast
const toast = ref<string | null>(null);
let toastTimer: ReturnType<typeof setTimeout> | null = null;

function showToast(text: string) {
  toast.value = text;
  if (toastTimer) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { toast.value = null; }, 1800);
}

onBeforeUnmount(() => {
  if (toastTimer) clearTimeout(toastTimer);
});

// ----- message actions: a long press on a message opens a sheet (copy the
// message, or select part of its text in a plain view).
const sheetMessage = ref<ChatMessage | null>(null);
const selectionText = ref<string | null>(null);
const overlayOpen = computed(() => sheetMessage.value !== null || selectionText.value !== null);
const HOLD_MS = 550;
const HOLD_SLOP_PX = 10;
let holdTimer: ReturnType<typeof setTimeout> | null = null;
let holdOrigin: { x: number; y: number } | null = null;

/** Same resolution as the desktop's message menu: an assistant turn is
 * represented by its last message. */
function messageAt(target: EventTarget | null): ChatMessage | null {
  if (!(target instanceof Element)) return null;
  const messageElement = target.closest("[data-chat-message-id]") as HTMLElement | null;
  const hitId = messageElement?.dataset.chatMessageId?.trim();
  const group = messageElement?.closest("[data-chat-message-group-end-id]") as HTMLElement | null;
  const groupEndId = group?.dataset.chatMessageGroupEndId?.trim();
  const id = group?.dataset.chatMessageGroupRole === "assistant" && groupEndId ? groupEndId : hitId;
  return id ? messages.value.find((message) => message.id === id) ?? null : null;
}

function openMessageSheet(message: ChatMessage) {
  if (overlayOpen.value) return;
  sheetMessage.value = message;
  // One history entry for the overlay, so the back gesture closes it.
  history.pushState({ ...(history.state ?? {}), locusOverlay: props.sessionKey }, "");
}

function cancelHold() {
  if (holdTimer) clearTimeout(holdTimer);
  holdTimer = null;
  holdOrigin = null;
}

function startHold(event: TouchEvent) {
  cancelHold();
  if (event.touches.length !== 1 || !messageAt(event.target)) return;
  const touch = event.touches[0]!;
  const target = event.target;
  holdOrigin = { x: touch.clientX, y: touch.clientY };
  holdTimer = setTimeout(() => {
    holdTimer = null;
    // A text selection the WebView started itself keeps its own menu.
    if (window.getSelection()?.isCollapsed === false) return;
    const message = messageAt(target);
    if (message) openMessageSheet(message);
  }, HOLD_MS);
}

function moveHold(event: TouchEvent) {
  if (!holdOrigin || event.touches.length !== 1) return;
  const touch = event.touches[0]!;
  if (Math.abs(touch.clientX - holdOrigin.x) > HOLD_SLOP_PX || Math.abs(touch.clientY - holdOrigin.y) > HOLD_SLOP_PX) {
    cancelHold();
  }
}

/** Android reports a long press as `contextmenu`; the sheet replaces the
 * WebView's own selection and link menus on messages. */
function onTranscriptContextMenu(event: MouseEvent) {
  const message = messageAt(event.target);
  if (!message) return;
  event.preventDefault();
  cancelHold();
  openMessageSheet(message);
}

function closeOverlay() {
  if (!overlayOpen.value) return;
  sheetMessage.value = null;
  selectionText.value = null;
  if (history.state?.locusOverlay === props.sessionKey) history.back();
}

function onPopState() {
  if (overlayOpen.value && history.state?.locusOverlay !== props.sessionKey) {
    sheetMessage.value = null;
    selectionText.value = null;
  }
}

onMounted(() => window.addEventListener("popstate", onPopState));
onBeforeUnmount(() => {
  window.removeEventListener("popstate", onPopState);
  cancelHold();
});

const sheetCanCopy = computed(() => !!sheetMessage.value && copyableChatMessageText(sheetMessage.value).trim().length > 0);

async function copySheetMessage() {
  const message = sheetMessage.value;
  if (!message) return;
  try {
    await writeChatMessageClipboard(buildChatMessageClipboardPayload(message));
    showToast(t("common.copied"));
  } catch (error) {
    console.warn("[remote] copy failed", error);
    showToast(t("chat.messageMenu.copyFailed"));
  }
  closeOverlay();
}

function selectSheetText() {
  const message = sheetMessage.value;
  if (!message) return;
  // Stays on the overlay's history entry.
  sheetMessage.value = null;
  selectionText.value = copyableChatMessageText(message);
}

// ----- scrolling: follow new content only while the reader is at the bottom.
const transcriptRef = ref<InstanceType<typeof ChatTranscript> | null>(null);
const stickToBottom = ref(true);
const BOTTOM_SLACK_PX = 96;

function updateStickToBottom(element: HTMLElement) {
  stickToBottom.value = element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM_SLACK_PX;
}

function onTranscriptScroll(event: Event) {
  const element = event.target as HTMLElement | null;
  if (element) updateStickToBottom(element);
}

function followContent() {
  if (!stickToBottom.value) return;
  void nextTick(() => transcriptRef.value?.scrollToBottom());
}

watch(() => messages.value.length, followContent);
watch(() => liveRenderParts.value.length, followContent);
watch(() => activeToolCalls.value.length, followContent);
watch(() => [pendingQuestion.value?.questionId, pendingToolConfirms.value.length], followContent);
watch(isLoading, (loading) => {
  if (!loading) {
    stickToBottom.value = true;
    followContent();
  }
});

// ----- earlier history: with the transcript already at its top, pulling it
// further down loads the previous page once the pull passes PULL_TRIGGER_PX.
// A fling that only coasts to the top never loads; the finger has to keep
// pulling. The indicator above the transcript holds at PULL_TRIGGER_PX while
// the page loads.
const PULL_DAMPING = 0.5;
const PULL_TRIGGER_PX = 44;
const pullDistance = ref(0);
/** A pull released before the trigger eases back instead of snapping. */
const pullSettling = ref(false);
let pullOriginY: number | null = null;

const canPullOlder = computed(
  () => sessionHistoryHasMore.value && !sessionHistoryLoading.value && messages.value.length > 0,
);
const pullIndicatorHeight = computed(() =>
  sessionHistoryLoading.value ? PULL_TRIGGER_PX : pullDistance.value,
);

function transcriptAtTop(): boolean {
  const element = transcriptRef.value?.getScrollElement();
  return !!element && element.scrollTop <= 0;
}

function resetPull() {
  pullOriginY = null;
  pullDistance.value = 0;
}

function onTranscriptTouchStart(event: TouchEvent) {
  startHold(event);
  pullSettling.value = false;
  resetPull();
  if (event.touches.length === 1 && canPullOlder.value && transcriptAtTop()) {
    pullOriginY = event.touches[0].clientY;
  }
}

function onTranscriptTouchEnd() {
  cancelHold();
  pullSettling.value = pullDistance.value > 0;
  resetPull();
}

function onTranscriptTouchMove(event: TouchEvent) {
  moveHold(event);
  if (event.touches.length !== 1 || !canPullOlder.value) {
    resetPull();
    return;
  }
  const y = event.touches[0].clientY;
  if (pullOriginY === null || y <= pullOriginY) {
    // The pull is measured from where the finger was when the transcript
    // reached its top, including a drag that scrolled it there.
    pullOriginY = transcriptAtTop() ? y : null;
    pullDistance.value = 0;
    return;
  }
  pullDistance.value = (y - pullOriginY) * PULL_DAMPING;
  if (pullDistance.value >= PULL_TRIGGER_PX) {
    resetPull();
    void loadOlder();
  }
}

async function loadOlder() {
  const element = transcriptRef.value?.getScrollElement();
  const previousHeight = element?.scrollHeight ?? 0;
  // A short transcript counts as "at the bottom"; without this the new
  // messages would be followed straight past.
  stickToBottom.value = false;
  await chat.loadOlderHistory();
  await nextTick();
  // Keep the message the reader was looking at in place. The loading
  // indicator collapses in the same frame and moves the transcript up by its
  // height, so the new messages take over the indicator's space.
  if (!element) return;
  element.scrollTop += element.scrollHeight - previousHeight - PULL_TRIGGER_PX;
  updateStickToBottom(element);
}
</script>

<template>
  <section class="m-screen m-session">
    <header class="m-topbar">
      <button class="m-icon-button" type="button" :aria-label="t('mobile.common.back')" @click="emit('back')">
        <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <div class="m-topbar-text">
        <span class="m-topbar-title">{{ title || t("mobile.session.untitled") }}</span>
        <span v-if="isStreaming" class="m-topbar-sub m-running">{{ t("mobile.session.running") }}</span>
      </div>
    </header>

    <div
      class="m-transcript"
      @click.capture="onTranscriptClick"
      @contextmenu="onTranscriptContextMenu"
      @touchstart.passive="onTranscriptTouchStart"
      @touchmove.passive="onTranscriptTouchMove"
      @touchend="onTranscriptTouchEnd"
      @touchcancel="onTranscriptTouchEnd"
    >
      <div v-if="isLoading && messages.length === 0" class="m-placeholder">{{ t("mobile.common.loading") }}</div>
      <div
        v-if="sessionHistoryHasMore || sessionHistoryLoading"
        class="m-pull-older"
        :class="{ 'is-settling': pullSettling }"
        :style="{
          height: `${pullIndicatorHeight}px`,
          opacity: sessionHistoryLoading ? 1 : Math.min(1, pullIndicatorHeight / PULL_TRIGGER_PX),
        }"
        :aria-hidden="pullIndicatorHeight === 0"
      >
        <span v-if="sessionHistoryLoading" class="m-pull-older-spinner" aria-hidden="true" />
        {{ sessionHistoryLoading ? t("mobile.common.loading") : t("mobile.session.pullOlder") }}
      </div>
      <ChatTranscript
        ref="transcriptRef"
        variant="session"
        :session-key="sessionKey"
        :session-id="sessionId"
        :workspace-ref="workspaceRef"
        :messages="messages"
        :streaming-text="streamingText"
        :has-streaming-text="streamingText.length > 0"
        :streaming-text-order="streamingTextOrder"
        :is-streaming="isStreaming"
        :is-compacting="isCompacting"
        :is-thinking="isThinking"
        :claude-code-mode="isClaudeCode"
        :claude-code-phase="claudeCodePhase"
        :show-thinking-blocks="!isClaudeCode"
        :has-thinking="hasThinking"
        :thinking-order="thinkingOrder"
        :thinking-duration="thinkingDuration"
        :live-render-parts="liveRenderParts"
        :live-part-streams="livePartStreams"
        :active-tool-calls="activeToolCalls"
        :empty-title="t('mobile.session.emptyTitle')"
        :empty-hint="t('mobile.session.emptyHint')"
        user-label="You"
        assistant-label="Locus"
        :handoff-label="t('chat.transcript.handoff')"
        :waiting-label="t('chat.transcript.waiting')"
        :compacting-label="t('chat.transcript.compacting')"
        :compacted-label="t('chat.transcript.compacted')"
        :thinking-active-label="t('chat.transcript.thinking')"
        :thought-duration-label="t('chat.transcript.thoughtDuration', '{0}')"
        :thought-moment-label="t('chat.transcript.thoughtMoment')"
        show-user-images
        @scroll="onTranscriptScroll"
        @stream-layout-change="followContent"
        @open-image="emit('openImage', $event)"
      />
    </div>

    <div v-if="pendingQuestion || pendingToolConfirms.length > 0" class="m-pending">
      <AskUserCard
        v-if="pendingQuestion"
        :question="pendingQuestion"
        @answer="answerQuestion"
      />
      <template v-for="confirm in pendingToolConfirms" :key="confirm.questionId">
        <ToolConfirmCard
          v-if="canApproveTools"
          :tool-confirm="confirm"
          :session-id="sessionId"
          @answer="answerToolConfirm(confirm, $event)"
        />
        <div v-else class="m-confirm-readonly">
          <strong>{{ confirmLabel(confirm) }}</strong>
          <span>{{ t("mobile.session.confirmOnDesktop") }}</span>
        </div>
      </template>
    </div>

    <div v-if="notice" class="m-notice" role="alert">{{ notice }}</div>

    <div v-if="queuedFollowUp" class="m-queued">
      <span class="m-queued-label">{{ t("mobile.session.queued") }}</span>
      <span class="m-queued-text">{{ queuedFollowUp.displayText }}</span>
      <button
        v-if="queuedFollowUp.canEdit"
        class="m-text-button"
        type="button"
        @click="chat.deleteQueuedFollowUp()"
      >
        {{ t("mobile.common.remove") }}
      </button>
    </div>

    <footer class="m-composer">
      <div class="m-composer-bar">
        <div class="m-composer-status">
          <button
            class="m-icon-button m-attach-button"
            type="button"
            :aria-label="t('mobile.session.attachImage')"
            :title="t('mobile.session.attachImage')"
            :disabled="!connected || imagesPending"
            @click="pickImages"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M4.5 7.5a2 2 0 0 1 2-2h11a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-11a2 2 0 0 1-2-2zM4.5 15l4-4 3.5 3.5 2.5-2.5 5 5M15.5 9.5h.01" />
            </svg>
          </button>
          <ChatStatusIndicators
            :session-id="sessionId"
            :selected-model-id="selectedModelId"
            :active="active"
            :polling-paused="statusPollingPaused"
            :is-streaming="isStreaming"
            :unity-connected="unityConnected"
            :unity-plugin-status="unityPluginStatus"
            :unity-plugin-installing="unityPluginInstalling"
            :unity-launching="unityLaunching"
            :unity-launch-state="unityLaunchState"
            :unity-connection-status="unityConnectionStatus"
            :workspace-ref="workspaceRef"
            :project-services="projectServices"
            :working-dir="workingDir"
            :scan-phase="scanPhase"
            :last-scan-stats="lastScanStats"
            :knowledge-access-mode="knowledgeAccessMode"
            :knowledge-enabled="knowledgeEnabled"
            :selected-agent-id="selectedAgentId"
            @start-scan="runStatusAction(startScan)"
            @install-plugin="runStatusAction(installUnityPlugin)"
            @launch-unity-project="runStatusAction(launchUnity)"
            @update-knowledge-access-mode="setKnowledgeAccessMode"
          >
            <BackgroundTasksIndicator :session-id="sessionId" />
          </ChatStatusIndicators>
        </div>
        <ModelEffortSelector
          class="m-model-selector"
          align="end"
          :agents="selectorAgents"
          :selected-agent-id="selectedAgentId"
          :agent-locked="false"
          :models="modelStore.availableModels"
          :selected-id="selectedModelId"
          :effort="effort"
          :efforts="efforts"
          :effort-supported="effortSupported"
          :multi-agent-enabled="multiAgentEnabled"
          :fast-mode-enabled="fastMode"
          :fast-mode-available="fastModeAvailable"
          :disabled="isStreaming || !connected"
          @select-agent="selectAgent"
          @select-model="selectModel"
          @select-effort="selectEffort"
          @select-multi-agent="selectMultiAgent"
          @select-fast-mode="selectFastMode"
        />
      </div>
      <RichChatInput
        ref="composerRef"
        v-model="inputText"
        class="m-rich-input"
        :selected-agent-id="selectedAgentId"
        :skills="skillItems"
        :placeholder="connected ? t('mobile.session.placeholder') : t('mobile.session.offline')"
        :disabled="!connected"
        :is-streaming="isStreaming"
        :cancelling="isCancelling"
        :can-resume="canResumeInterrupted && connected"
        :send-label="isStreaming ? t('mobile.session.queue') : t('mobile.session.send')"
        :cancel-label="t('mobile.session.stop')"
        :resume-label="t('mobile.session.resume')"
        :before-send="beforeSend"
        :allow-runtime-compact="true"
        :workspace-ref="workspaceRef"
        :workspace-root="workingDir"
        :plan-mode-active="planModeActive"
        :draft-state-key="sessionKey"
        :hidden-command-types="HIDDEN_COMMANDS"
        compact
        @focus="loadSkills()"
        @send="sendComposerPayload"
        @cancel="chat.cancel()"
        @resume="chat.resumeInterrupted()"
        @compact="chat.compact()"
        @fork="forkCurrentSession"
        @request-plan-mode="chat.setPlanMode($event)"
        @clear="emit('newSession')"
        @request-new-session="emit('newSession')"
      />
      <input
        ref="imageInput"
        class="m-hidden-input"
        type="file"
        accept="image/*"
        multiple
        @change="onImagesPicked"
      >
    </footer>

    <div v-if="sheetMessage" class="m-sheet-backdrop" @click.self="closeOverlay">
      <div class="m-sheet" role="menu">
        <button class="m-sheet-item" type="button" role="menuitem" :disabled="!sheetCanCopy" @click="copySheetMessage">
          {{ t("chat.messageMenu.copyMessage") }}
        </button>
        <button class="m-sheet-item" type="button" role="menuitem" :disabled="!sheetCanCopy" @click="selectSheetText">
          {{ t("mobile.session.selectText") }}
        </button>
        <button class="m-sheet-item m-sheet-cancel" type="button" @click="closeOverlay">
          {{ t("common.cancel") }}
        </button>
      </div>
    </div>

    <div v-if="selectionText !== null" class="m-select-text" role="dialog" :aria-label="t('mobile.session.selectText')">
      <header class="m-topbar">
        <button class="m-icon-button" type="button" :aria-label="t('common.close')" @click="closeOverlay">
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
        </button>
        <div class="m-topbar-text">
          <span class="m-topbar-title">{{ t("mobile.session.selectText") }}</span>
          <span class="m-topbar-sub">{{ t("mobile.session.selectTextHint") }}</span>
        </div>
      </header>
      <div class="m-select-text-body">{{ selectionText }}</div>
    </div>

    <div v-if="toast" class="m-toast" role="status">{{ toast }}</div>
  </section>
</template>
