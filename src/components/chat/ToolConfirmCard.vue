
<script setup lang="ts">
import { computed, ref } from "vue";
import { ExternalLink } from "lucide";
import type {
  BasicToolConfirmDisplay,
  KnowledgeToolConfirmPreview,
  PendingToolConfirm,
  PlanApprovalConfirmDisplay,
  UnityEditorStatusChangeToolConfirmDisplay,
} from "../../types";
import { t } from "../../i18n";
import { useDisplaySettings } from "../../composables/useDisplaySettings";
import { openPlanViewWindow } from "../../services/planViewWindow";
import LucideIcon from "../icons/LucideIcon.vue";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import BaseButton from "../ui/BaseButton.vue";
import KnowledgeToolConfirmCard from "./KnowledgeToolConfirmCard.vue";
import ToolConfirmFeedbackForm from "./ToolConfirmFeedbackForm.vue";
import { encodeToolConfirmFeedback } from "./toolConfirmAnswer";
import {
  editorStatusLabelForToolConfirm,
  titleForUnityEditorStatusChange,
} from "./toolConfirmLabels";
import UnityRunStatesPreview from "../tool-previews/UnityRunStatesPreview.vue";
import { parseUnityRunStatesArguments } from "../../composables/unityRunStatesPreview";
import { parseMcpToolName, toolCallDisplayName } from "../toolCallSummary";

const props = defineProps<{
  toolConfirm: PendingToolConfirm;
}>();

const emit = defineEmits<{
  answer: [answer: string];
}>();

function isKnowledgePreview(
  display: PendingToolConfirm["display"],
): display is KnowledgeToolConfirmPreview {
  return display.kind === "knowledge";
}

function isBasicDisplay(
  display: PendingToolConfirm["display"],
): display is BasicToolConfirmDisplay {
  return display.kind === "basic";
}

function isUnityEditorStatusChangeDisplay(
  display: PendingToolConfirm["display"],
): display is UnityEditorStatusChangeToolConfirmDisplay {
  return display.kind === "unityEditorStatusChange";
}

function isPlanApprovalDisplay(
  display: PendingToolConfirm["display"],
): display is PlanApprovalConfirmDisplay {
  return display.kind === "planApproval";
}

const knowledgeDisplay = computed(() =>
  isKnowledgePreview(props.toolConfirm.display) ? props.toolConfirm.display : null,
);

const basicDisplay = computed(() =>
  isBasicDisplay(props.toolConfirm.display) ? props.toolConfirm.display : null,
);

const unityRunStatesPreview = computed(() => {
  const display = basicDisplay.value;
  if (!display || display.toolName !== "unity_run_states") return null;
  return parseUnityRunStatesArguments(display.arguments);
});

// MCP wire names shed their prefix for display, but approval must keep the
// external-server origin visible — that is what the user is trusting.
const basicMcpParts = computed(() =>
  basicDisplay.value ? parseMcpToolName(basicDisplay.value.toolName) : null,
);

const basicToolDisplayName = computed(() =>
  basicDisplay.value ? toolCallDisplayName(basicDisplay.value.toolName) : "",
);

const unityStatusChangeDisplay = computed(() =>
  isUnityEditorStatusChangeDisplay(props.toolConfirm.display)
    ? props.toolConfirm.display
    : null,
);

const planApprovalDisplay = computed(() =>
  isPlanApprovalDisplay(props.toolConfirm.display) ? props.toolConfirm.display : null,
);

const title = computed(() => {
  if (unityStatusChangeDisplay.value) {
    return titleForUnityEditorStatusChange(unityStatusChangeDisplay.value.requestedStatus);
  }
  if (planApprovalDisplay.value) return t("chat.plan.approvalTitle");
  return t("chat.toolConfirm.title");
});

const allowLabel = computed(() => {
  if (unityStatusChangeDisplay.value) return t("chat.toolConfirm.unityStatus.confirm");
  if (planApprovalDisplay.value) return t("chat.plan.approve");
  return t("chat.toolConfirm.allow");
});

const denyLabel = computed(() => {
  if (unityStatusChangeDisplay.value) return t("chat.toolConfirm.unityStatus.cancel");
  if (planApprovalDisplay.value) return t("chat.plan.reject");
  return t("chat.toolConfirm.deny");
});

function formatToolArgs(raw: string): string {
  try {
    const obj = JSON.parse(raw);
    const pretty = JSON.stringify(obj, null, 2);
    return pretty.length > 500 ? pretty.slice(0, 500) + "\n..." : pretty;
  } catch {
    return raw.length > 500 ? raw.slice(0, 500) + "..." : raw;
  }
}

const unityStatusRows = computed(() => {
  const display = unityStatusChangeDisplay.value;
  if (!display) return [];
  return [
    {
      label: t("chat.toolConfirm.unityStatus.current"),
      value: editorStatusLabelForToolConfirm(display.currentStatus),
    },
    {
      label: t("chat.toolConfirm.unityStatus.requested"),
      value: editorStatusLabelForToolConfirm(display.requestedStatus),
    },
  ];
});

// Plan approval collapses to TWO intents: approve, or send the plan back.
// The feedback field is an optional note attached to "send back" — not a
// third, separately-submitted action.
const planFeedback = ref("");

const { state: displaySettings } = useDisplaySettings();

// With the standalone-window preference the card stays as the in-transcript
// anchor (actions still work here) but stops duplicating the full plan text.
const planPrefersWindow = computed(
  () => !!planApprovalDisplay.value && displaySettings.planApprovalTarget === "window",
);

function openPlanWindow() {
  const display = planApprovalDisplay.value;
  if (!display) return;
  void openPlanViewWindow({
    planFilePath: display.planFilePath,
    questionId: props.toolConfirm.questionId,
  });
}

function submitPlanSendBack() {
  const feedback = planFeedback.value.trim();
  emit("answer", feedback ? encodeToolConfirmFeedback(feedback) : "deny");
}

// Enter only sends back when there is an actual note: an empty field must
// not reject the plan by accident, and an IME composition Enter (Chinese
// candidate confirm) must never submit.
function handlePlanFeedbackEnter(event: KeyboardEvent) {
  if (event.isComposing) return;
  if (!planFeedback.value.trim()) return;
  submitPlanSendBack();
}
</script>

<template>
  <KnowledgeToolConfirmCard
    v-if="knowledgeDisplay"
    :preview="knowledgeDisplay"
    @answer="emit('answer', $event)"
  />
  <div
    v-else
    class="ask-user-card tool-confirm-card"
    :class="{ 'is-unity-status-change': unityStatusChangeDisplay }"
  >
    <div class="tool-confirm-header">
      <span v-if="!unityStatusChangeDisplay" class="tool-confirm-icon">
        <svg viewBox="0 0 16 16" fill="currentColor" width="14" height="14">
          <path d="M8 1a3.5 3.5 0 0 0-3.5 3.5v1H3.25A1.25 1.25 0 0 0 2 6.75v7A1.25 1.25 0 0 0 3.25 15h9.5A1.25 1.25 0 0 0 14 13.75v-7A1.25 1.25 0 0 0 12.75 5.5H11.5v-1A3.5 3.5 0 0 0 8 1zm-2 4.5v-1a2 2 0 1 1 4 0v1H6z"/>
        </svg>
      </span>
      <span class="tool-confirm-title">{{ title }}</span>
    </div>
    <template v-if="basicDisplay">
      <div class="tool-confirm-body">
        <div class="tool-confirm-name">
          {{ basicToolDisplayName }}
          <span v-if="basicMcpParts" class="tool-confirm-mcp-origin">
            MCP · {{ basicMcpParts.serverId || "server" }}
          </span>
        </div>
        <UnityRunStatesPreview
          v-if="unityRunStatesPreview"
          :preview="unityRunStatesPreview"
          dense
        />
        <pre v-else class="tool-confirm-args">{{ formatToolArgs(basicDisplay.arguments) }}</pre>
      </div>
    </template>
    <template v-else-if="unityStatusChangeDisplay">
      <div class="tool-confirm-body">
        <div class="tool-confirm-name">{{ unityStatusChangeDisplay.toolName }}</div>
        <dl class="unity-status-change-details">
          <div
            v-for="row in unityStatusRows"
            :key="row.label"
            class="unity-status-change-row"
          >
            <dt class="unity-status-change-label">{{ row.label }}</dt>
            <dd class="unity-status-change-value">{{ row.value }}</dd>
          </div>
        </dl>
      </div>
    </template>
    <template v-else-if="planApprovalDisplay">
      <div class="tool-confirm-body">
        <div class="plan-approval-path-row">
          <span class="plan-approval-path">{{ planApprovalDisplay.planFilePath }}</span>
          <button
            type="button"
            class="plan-approval-open-window ui-select-none"
            :title="t('chat.plan.openInWindow')"
            @click="openPlanWindow"
          >
            <LucideIcon :icon="ExternalLink" :size="12" />
            <span>{{ t("chat.plan.openInWindow") }}</span>
          </button>
        </div>
        <button
          v-if="planPrefersWindow"
          type="button"
          class="plan-approval-window-notice ui-select-none"
          @click="openPlanWindow"
        >
          {{ t("chat.plan.openedInWindow") }} · {{ t("chat.plan.focusWindow") }}
        </button>
        <div v-else class="plan-approval-content">
          <MarkdownRenderer :content="planApprovalDisplay.plan" />
        </div>
        <div class="plan-approval-hint">{{ t("chat.plan.approvalHint") }}</div>
      </div>
    </template>
    <ToolConfirmFeedbackForm
      v-if="basicDisplay"
      @submit="emit('answer', $event)"
    />
    <input
      v-if="planApprovalDisplay"
      v-model="planFeedback"
      class="plan-approval-feedback-input"
      :placeholder="t('chat.plan.feedbackPlaceholder')"
      @keydown.enter="handlePlanFeedbackEnter"
    />
    <div class="tool-confirm-actions">
      <BaseButton class="tool-confirm-btn" variant="primary" size="md" @click="emit('answer', 'allow')">{{ allowLabel }}</BaseButton>
      <BaseButton
        v-if="planApprovalDisplay"
        class="tool-confirm-btn"
        size="md"
        @click="submitPlanSendBack"
      >
        {{ denyLabel }}
      </BaseButton>
      <BaseButton v-else class="tool-confirm-btn" size="md" @click="emit('answer', 'deny')">{{ denyLabel }}</BaseButton>
    </div>
  </div>
</template>

<style scoped>
.plan-approval-path-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 6px;
}

.plan-approval-path {
  min-width: 0;
  color: var(--text-secondary);
  font-size: 11px;
  font-family: var(--font-mono-identifier);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.plan-approval-open-window {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease, border-color 0.12s ease;
}

.plan-approval-open-window:hover {
  background: var(--hover-bg);
  border-color: color-mix(in srgb, var(--accent-color) 30%, var(--border-color));
  color: var(--text-color);
}

.plan-approval-window-notice {
  display: block;
  width: 100%;
  box-sizing: border-box;
  margin: 0 0 8px;
  padding: 10px 12px;
  border: 1px dashed color-mix(in srgb, var(--accent-color) 30%, var(--border-color));
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 92%, var(--accent-color) 8%);
  color: var(--text-secondary);
  font-size: 12px;
  text-align: left;
  cursor: pointer;
}

.plan-approval-window-notice:hover {
  color: var(--text-color);
}

.plan-approval-content {
  box-sizing: border-box;
  min-width: 0;
  max-width: 100%;
  max-height: 320px;
  overflow: auto;
  margin: 0 0 8px;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--sidebar-bg) 14%);
  color: var(--text-color);
}

.plan-approval-content :deep(.markdown-body) {
  font-size: 12px;
  line-height: 1.6;
}

.plan-approval-hint {
  color: var(--text-secondary);
  font-size: 11px;
}

.plan-approval-feedback-input {
  width: 100%;
  box-sizing: border-box;
  min-height: 32px;
  margin-bottom: 10px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--panel-bg));
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
}

.plan-approval-feedback-input:focus {
  outline: none;
  border-color: color-mix(in srgb, var(--accent-color) 42%, var(--border-color));
}

.tool-confirm-card.is-unity-status-change {
  border-color: color-mix(in srgb, var(--border-color) 86%, var(--accent-color) 14%);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
}

.tool-confirm-card.is-unity-status-change .tool-confirm-header {
  margin-bottom: 8px;
}

.tool-confirm-card.is-unity-status-change .tool-confirm-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
}

.tool-confirm-card.is-unity-status-change .tool-confirm-body {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 12px;
}

.tool-confirm-mcp-origin {
  margin-left: 6px;
  padding: 1px 7px;
  border: 1px solid var(--border-color);
  border-radius: 999px;
  font-size: 10px;
  font-weight: 500;
  color: var(--text-secondary);
  vertical-align: 1px;
  white-space: nowrap;
}

.tool-confirm-card.is-unity-status-change .tool-confirm-name {
  margin-bottom: 0;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 600;
}

.unity-status-change-details {
  display: grid;
  margin: 0;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--sidebar-bg) 14%);
}

.unity-status-change-row {
  display: grid;
  grid-template-columns: 88px minmax(0, 1fr);
  min-height: 32px;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 74%, transparent);
}

.unity-status-change-row:first-child {
  border-top: 0;
}

.unity-status-change-label,
.unity-status-change-value {
  display: flex;
  align-items: center;
  min-width: 0;
  margin: 0;
  padding: 6px 10px;
  font-size: 12px;
  line-height: 1.5;
}

.unity-status-change-label {
  border-right: 1px solid color-mix(in srgb, var(--border-color) 74%, transparent);
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--sidebar-bg) 46%, transparent);
}

.unity-status-change-value {
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}

.tool-confirm-card.is-unity-status-change .tool-confirm-actions {
  justify-content: flex-end;
}

@media (max-width: 720px) {
  .unity-status-change-row {
    grid-template-columns: minmax(0, 1fr);
  }

  .unity-status-change-label {
    border-right: 0;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 74%, transparent);
  }
}
</style>
