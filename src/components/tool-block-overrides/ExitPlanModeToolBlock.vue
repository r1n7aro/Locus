<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import { t } from "../../i18n";
import MarkdownRenderer from "../MarkdownRenderer.vue";
import { parseExitPlanModeBlock } from "../../composables/exitPlanModeBlock";
import type { ToolCallDisplay } from "../../types";

const props = withDefaults(defineProps<{
  toolCall: ToolCallDisplay;
  collapseEnabled?: boolean;
}>(), {
  collapseEnabled: true,
});

const emit = defineEmits<{
  (e: "toolViewportAnchorStart", anchor: HTMLElement): void;
  (e: "toolViewportAnchorEnd", anchor: HTMLElement): void;
}>();

const rootRef = ref<HTMLElement | null>(null);
const headerRef = ref<HTMLElement | null>(null);

const blockState = computed(() => parseExitPlanModeBlock(props.toolCall));

// The approved plan is the core artifact of plan mode — surface it expanded
// so the transcript keeps a readable copy after the approval card is gone.
const expanded = ref(blockState.value.kind === "approved");

watch(
  () => blockState.value.kind,
  (kind, previousKind) => {
    if (kind === "approved" && previousKind !== "approved") {
      expanded.value = true;
    }
  },
);

function runOnNextFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") {
    requestAnimationFrame(() => callback());
    return;
  }
  setTimeout(callback, 16);
}

const hasDetail = computed(() => {
  const state = blockState.value;
  if (state.kind === "approved") return state.plan.length > 0;
  if (state.kind === "rejected") return state.feedback.length > 0;
  if (state.kind === "error") return state.detail.length > 0;
  return false;
});

function setExpanded(value: boolean) {
  if (expanded.value === value) return;
  if (value && !hasDetail.value) return;
  const anchor = headerRef.value ?? rootRef.value;
  if (anchor) emit("toolViewportAnchorStart", anchor);
  expanded.value = value;
  if (anchor) {
    nextTick(() => {
      runOnNextFrame(() => emit("toolViewportAnchorEnd", anchor));
    });
  }
}

function toggleExpanded() {
  setExpanded(!expanded.value);
}

function expandFromBlockClick(event: MouseEvent) {
  if (expanded.value || !hasDetail.value) return;
  const target = event.target instanceof HTMLElement ? event.target : null;
  if (target?.closest("button, a, input, textarea, select, [role='button'], .ui-select-text")) {
    return;
  }
  setExpanded(true);
}

// The icon follows the block state, not the raw tool status: a rejected plan
// is the user steering the loop ("keep planning"), not a failure, so it must
// not render with the error treatment.
const statusIcon = computed(() => {
  if (blockState.value.kind === "awaiting") return "spinner";
  if (blockState.value.kind === "error") return "error";
  return "check";
});

const summary = computed(() => {
  switch (blockState.value.kind) {
    case "awaiting": return t("chat.plan.blockAwaiting");
    case "approved": return t("chat.plan.blockApproved");
    case "rejected": return t("chat.plan.blockRejected");
    case "error": return "";
  }
});

const approvedPlan = computed(() =>
  blockState.value.kind === "approved" ? blockState.value.plan : "",
);
const rejectedFeedback = computed(() =>
  blockState.value.kind === "rejected" ? blockState.value.feedback : "",
);
const errorDetail = computed(() =>
  blockState.value.kind === "error" ? blockState.value.detail : "",
);
</script>

<template>
  <div
    ref="rootRef"
    class="exit-plan-tool-block"
    :class="[toolCall.status, `state-${blockState.kind}`, { 'is-expanded': expanded }]"
    @click="expandFromBlockClick"
  >
    <button
      ref="headerRef"
      type="button"
      class="tool-call-header ui-select-none"
      :aria-expanded="expanded && hasDetail"
      @click.stop="toggleExpanded"
    >
      <span class="tool-call-icon" :class="statusIcon">
        <span v-if="statusIcon === 'spinner'" class="spinner-anim"></span>
        <span v-else class="tool-call-status-dot"></span>
      </span>
      <span class="tool-call-name">exit_plan_mode</span>
      <span v-if="summary" class="tool-call-summary">{{ summary }}</span>
      <span v-if="blockState.kind === 'awaiting'" class="tool-call-inline-dots" aria-hidden="true"><span>.</span><span>.</span><span>.</span></span>
    </button>

    <div v-if="expanded && hasDetail" class="exit-plan-detail">
      <div v-if="approvedPlan" class="exit-plan-content">
        <MarkdownRenderer :content="approvedPlan" />
      </div>
      <div v-else-if="rejectedFeedback" class="exit-plan-feedback ui-select-text">
        {{ rejectedFeedback }}
      </div>
      <pre v-else-if="errorDetail" class="exit-plan-error ui-select-text">{{ errorDetail }}</pre>
    </div>
  </div>
</template>

<style scoped>
.exit-plan-tool-block {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  width: 100%;
  max-width: 100%;
  font-size: 13px;
}

.exit-plan-tool-block:not(.is-expanded) {
  cursor: pointer;
}

.tool-call-header {
  appearance: none;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  width: 100%;
  max-width: 100%;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 1px 4px;
  border-radius: 4px;
  cursor: pointer;
  user-select: none;
  min-height: 22px;
  text-align: left;
  transition: color 0.12s ease, background 0.12s ease;
}

.tool-call-header:hover {
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
}

.tool-call-header:focus-visible {
  outline: 1px solid color-mix(in srgb, var(--accent-color) 36%, transparent);
  outline-offset: 1px;
}

.tool-call-icon {
  width: 14px;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.tool-call-icon.spinner {
  color: var(--accent-color);
}

.tool-call-icon.check {
  color: var(--text-secondary);
}

.tool-call-icon.error {
  color: var(--status-danger-fg);
}

.tool-call-status-dot {
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: currentColor;
  opacity: 0.7;
}

.tool-call-icon.check .tool-call-status-dot {
  opacity: 0.46;
}

.tool-call-icon.error .tool-call-status-dot {
  width: 6px;
  height: 6px;
  opacity: 0.78;
}

.spinner-anim {
  width: 10px;
  height: 10px;
  border: 1.5px solid color-mix(in srgb, var(--accent-color) 18%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 50%;
  animation: exit-plan-spin 0.8s linear infinite;
  display: inline-block;
}

@keyframes exit-plan-spin {
  to { transform: rotate(360deg); }
}

.tool-call-name {
  font-weight: 600;
  font-family: var(--font-mono-identifier);
  color: var(--text-color);
  font-size: 12px;
  flex-shrink: 0;
}

.tool-call-summary {
  color: var(--text-secondary);
  font-size: 11px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.tool-call-inline-dots {
  display: inline-flex;
  width: 1.4em;
  margin-left: -4px;
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.4;
  flex-shrink: 0;
  opacity: 0.72;
}

.tool-call-inline-dots span {
  animation: exit-plan-inline-dot 1.2s infinite ease-in-out;
}

.tool-call-inline-dots span:nth-child(2) {
  animation-delay: 0.2s;
}

.tool-call-inline-dots span:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes exit-plan-inline-dot {
  0%, 20% { opacity: 0.22; }
  50% { opacity: 1; }
  100% { opacity: 0.22; }
}

.exit-plan-detail {
  align-self: stretch;
  margin-top: 6px;
  padding: 0 2px 0 20px;
}

.exit-plan-content {
  box-sizing: border-box;
  min-width: 0;
  max-width: 100%;
  max-height: 320px;
  overflow: auto;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--sidebar-bg) 14%);
  color: var(--text-color);
}

.exit-plan-content :deep(.markdown-body) {
  font-size: 12px;
  line-height: 1.6;
}

.exit-plan-feedback {
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.exit-plan-error {
  margin: 0;
  padding: 6px 8px;
  border-radius: 6px;
  background: var(--hover-bg);
  color: var(--status-danger-fg);
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.4;
  white-space: pre-wrap;
  word-break: break-word;
}
</style>
