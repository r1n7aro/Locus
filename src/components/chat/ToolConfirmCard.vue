
<script setup lang="ts">
import { computed } from "vue";
import type {
  BasicToolConfirmDisplay,
  KnowledgeToolConfirmPreview,
  PendingToolConfirm,
  UnityEditorStatusChangeToolConfirmDisplay,
} from "../../types";
import { t } from "../../i18n";
import BaseButton from "../ui/BaseButton.vue";
import KnowledgeToolConfirmCard from "./KnowledgeToolConfirmCard.vue";
import ToolConfirmFeedbackForm from "./ToolConfirmFeedbackForm.vue";
import {
  editorStatusLabelForToolConfirm,
  titleForUnityEditorStatusChange,
} from "./toolConfirmLabels";
import UnityRunStatesPreview from "../tool-previews/UnityRunStatesPreview.vue";
import { parseUnityRunStatesArguments } from "../../composables/unityRunStatesPreview";

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

const unityStatusChangeDisplay = computed(() =>
  isUnityEditorStatusChangeDisplay(props.toolConfirm.display)
    ? props.toolConfirm.display
    : null,
);

const title = computed(() =>
  unityStatusChangeDisplay.value
    ? titleForUnityEditorStatusChange(unityStatusChangeDisplay.value.requestedStatus)
    : t("chat.toolConfirm.title"),
);

const allowLabel = computed(() =>
  unityStatusChangeDisplay.value
    ? t("chat.toolConfirm.unityStatus.confirm")
    : t("chat.toolConfirm.allow"),
);

const denyLabel = computed(() =>
  unityStatusChangeDisplay.value
    ? t("chat.toolConfirm.unityStatus.cancel")
    : t("chat.toolConfirm.deny"),
);

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
        <div class="tool-confirm-name">{{ basicDisplay.toolName }}</div>
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
    <ToolConfirmFeedbackForm v-if="basicDisplay" @submit="emit('answer', $event)" />
    <div class="tool-confirm-actions">
      <BaseButton class="tool-confirm-btn" variant="primary" size="md" @click="emit('answer', 'allow')">{{ allowLabel }}</BaseButton>
      <BaseButton class="tool-confirm-btn" size="md" @click="emit('answer', 'deny')">{{ denyLabel }}</BaseButton>
    </div>
  </div>
</template>

<style scoped>
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
