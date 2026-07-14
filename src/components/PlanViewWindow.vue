<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide";
import { t } from "../i18n";
import { normalizeAppError } from "../services/errors";
import { answerQuestion, getPlanFileContent } from "../services/session";
import {
  getPlanViewWindowPayload,
  PLAN_VIEW_RESOLVED_EVENT,
  PLAN_VIEW_WINDOW_EVENT,
  type PlanViewWindowPayload,
} from "../services/planViewWindow";
import { encodeToolConfirmFeedback } from "./chat/toolConfirmAnswer";
import LucideIcon from "./icons/LucideIcon.vue";
import MarkdownRenderer from "./MarkdownRenderer.vue";
import BaseButton from "./ui/BaseButton.vue";

const appWindow = getCurrentWindow();

const planFilePath = ref("");
const questionId = ref("");
const content = ref("");
const loading = ref(false);
const error = ref("");
const answering = ref(false);
const feedback = ref("");

let unlistenPayload: UnlistenFn | null = null;
let unlistenResolved: UnlistenFn | null = null;
let loadSeq = 0;

const hasApprovalActions = computed(() => questionId.value.length > 0);

async function loadPlan(path: string) {
  const seq = ++loadSeq;
  loading.value = true;
  error.value = "";
  try {
    const result = await getPlanFileContent(path);
    if (seq !== loadSeq) return;
    planFilePath.value = result.planFilePath;
    content.value = result.content;
  } catch (cause) {
    if (seq !== loadSeq) return;
    error.value = normalizeAppError(cause).message;
    content.value = "";
  } finally {
    if (seq === loadSeq) loading.value = false;
  }
}

function applyWindowPayload(payload: PlanViewWindowPayload) {
  questionId.value = payload.questionId ?? "";
  feedback.value = "";
  if (payload.planFilePath) {
    planFilePath.value = payload.planFilePath;
    void loadPlan(payload.planFilePath);
  }
}

async function closeWindow() {
  try {
    await appWindow.close();
    return;
  } catch {
    // fall through
  }
  await appWindow.destroy().catch(() => {});
}

async function submitAnswer(answer: string) {
  if (!questionId.value || answering.value) return;
  answering.value = true;
  try {
    await answerQuestion(questionId.value, answer);
    await closeWindow();
  } catch (cause) {
    error.value = normalizeAppError(cause).message;
  } finally {
    answering.value = false;
  }
}

function approvePlan() {
  void submitAnswer("allow");
}

function sendBackPlan() {
  const note = feedback.value.trim();
  void submitAnswer(note ? encodeToolConfirmFeedback(note) : "deny");
}

function handleFeedbackEnter(event: KeyboardEvent) {
  if (event.isComposing) return;
  if (!feedback.value.trim()) return;
  sendBackPlan();
}

onMounted(async () => {
  applyWindowPayload(getPlanViewWindowPayload());
  unlistenPayload = await listen<PlanViewWindowPayload>(
    PLAN_VIEW_WINDOW_EVENT,
    (event) => applyWindowPayload(event.payload),
  );
  // The approval settled elsewhere (approval card, batch card, run cancel):
  // the pending question is gone, so drop the action bar. Keep the window
  // open as a read-only view — closing it under the user would be jarring.
  unlistenResolved = await listen<{ questionId: string }>(
    PLAN_VIEW_RESOLVED_EVENT,
    (event) => {
      if (!questionId.value || event.payload.questionId !== questionId.value) return;
      questionId.value = "";
      feedback.value = "";
    },
  );
});

onUnmounted(() => {
  unlistenPayload?.();
  unlistenPayload = null;
  unlistenResolved?.();
  unlistenResolved = null;
  loadSeq += 1;
});
</script>

<template>
  <div class="plan-view-window-root">
    <div class="plan-view-titlebar">
      <div class="plan-view-title">
        <span class="plan-view-title-main">{{ t("chat.plan.windowTitle") }}</span>
        <span class="plan-view-title-path" :title="planFilePath">{{ planFilePath }}</span>
      </div>
      <button
        type="button"
        class="plan-view-close"
        :title="t('app.win.close')"
        @click="closeWindow"
      >
        <LucideIcon :icon="X" :size="14" />
      </button>
    </div>

    <div class="plan-view-body">
      <div v-if="error" class="plan-view-error">{{ error }}</div>
      <div v-else-if="loading && !content" class="plan-view-loading">{{ t("common.loading") }}</div>
      <div v-else class="plan-view-markdown">
        <MarkdownRenderer :content="content" />
      </div>
    </div>

    <div v-if="hasApprovalActions" class="plan-view-approval">
      <div class="plan-view-approval-hint">{{ t("chat.plan.approvalHint") }}</div>
      <input
        v-model="feedback"
        class="plan-view-feedback-input"
        :placeholder="t('chat.plan.feedbackPlaceholder')"
        :disabled="answering"
        @keydown.enter="handleFeedbackEnter"
      />
      <div class="plan-view-approval-actions">
        <BaseButton variant="primary" size="md" :disabled="answering" @click="approvePlan">
          {{ t("chat.plan.approve") }}
        </BaseButton>
        <BaseButton size="md" :disabled="answering" @click="sendBackPlan">
          {{ t("chat.plan.reject") }}
        </BaseButton>
      </div>
    </div>
  </div>
</template>

<style scoped>
.plan-view-window-root {
  width: 100vw;
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--panel-bg);
  color: var(--text-color);
  border: 1px solid var(--border-strong);
}

.plan-view-titlebar {
  -webkit-app-region: drag;
  min-height: 38px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 14px;
  background: var(--sidebar-bg);
  border-bottom: 1px solid var(--border-color);
}

.plan-view-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 8px;
}

.plan-view-title-main {
  flex-shrink: 0;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.plan-view-title-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
}

.plan-view-close {
  -webkit-app-region: no-drag;
  width: 28px;
  height: 28px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.plan-view-close:hover,
.plan-view-close:focus-visible {
  background: var(--hover-bg);
  border-color: var(--border-color);
  color: var(--text-color);
  outline: none;
}

.plan-view-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 18px 22px;
}

.plan-view-markdown :deep(.markdown-body) {
  font-size: 13px;
  line-height: 1.7;
  max-width: 860px;
  margin: 0 auto;
}

.plan-view-loading,
.plan-view-error {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 120px;
  color: var(--text-secondary);
  font-size: 13px;
}

.plan-view-error {
  color: var(--status-danger-fg);
}

.plan-view-approval {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 12px 22px 14px;
  border-top: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
}

.plan-view-approval-hint {
  color: var(--text-secondary);
  font-size: 11px;
}

.plan-view-feedback-input {
  width: 100%;
  box-sizing: border-box;
  min-height: 32px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--panel-bg));
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
}

.plan-view-feedback-input:focus {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color));
}

.plan-view-approval-actions {
  display: flex;
  gap: 8px;
}
</style>
