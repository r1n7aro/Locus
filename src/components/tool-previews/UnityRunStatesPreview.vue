<script setup lang="ts">
import hljs from "../../hljs";
import { t } from "../../i18n";
import type { UnityRunStatesArgsPreview } from "../../composables/unityRunStatesPreview";

withDefaults(defineProps<{
  preview: UnityRunStatesArgsPreview;
  dense?: boolean;
}>(), {
  dense: false,
});

function escapeHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function highlightCSharp(code: string): string {
  try {
    return hljs.highlight(code, { language: "csharp" }).value;
  } catch {
    return escapeHtml(code);
  }
}
</script>

<template>
  <div class="unity-run-states-preview" :class="{ dense }">
    <div class="unity-run-summary">
      <div class="unity-run-summary-item">
        <span class="unity-run-summary-label">{{ t("tool.unityRunStates.requestStatus") }}</span>
        <code class="unity-run-summary-value">{{ preview.requestEditorStatus || "-" }}</code>
      </div>
      <div class="unity-run-summary-item">
        <span class="unity-run-summary-label">{{ t("tool.unityRunStates.initialState") }}</span>
        <code class="unity-run-summary-value">{{ preview.initialState || "-" }}</code>
      </div>
      <div class="unity-run-summary-item">
        <span class="unity-run-summary-label">{{ t("tool.unityRunStates.stateCount") }}</span>
        <code class="unity-run-summary-value">{{ preview.states.length }}</code>
      </div>
    </div>

    <div class="unity-run-states-list">
      <section v-for="state in preview.states" :key="state.name" class="unity-run-state">
        <div class="unity-run-state-header">
          <span class="unity-run-state-name">{{ state.name }}</span>
          <span v-if="state.isInitial" class="unity-run-state-meta">{{ t("tool.unityRunStates.initial") }}</span>
        </div>
        <div class="unity-run-phases">
          <div v-for="phase in state.phases" :key="phase.key" class="unity-run-phase">
            <div class="unity-run-phase-label">{{ phase.key }}</div>
            <pre v-if="!phase.empty" class="unity-run-phase-code hljs ui-select-text" v-html="highlightCSharp(phase.code)"></pre>
            <div v-else class="unity-run-phase-empty">{{ t("tool.unityRunStates.emptyPhase") }}</div>
          </div>
        </div>
      </section>
    </div>
  </div>
</template>

<style scoped>
.unity-run-states-preview {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.unity-run-summary {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 6px 10px;
  padding: 6px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
  border-radius: 6px;
  background: var(--hover-bg);
}

.unity-run-summary-item {
  min-width: 0;
  display: grid;
  gap: 2px;
}

.unity-run-summary-label {
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.3;
}

.unity-run-summary-value {
  min-width: 0;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  line-height: 1.4;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-run-states-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
}

.dense .unity-run-states-list {
  max-height: 360px;
  overflow: auto;
  scrollbar-gutter: stable;
}

.unity-run-state {
  min-width: 0;
  overflow: hidden;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
}

.unity-run-state-header {
  min-height: 28px;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 8px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg) 70%, transparent);
}

.unity-run-state-name {
  min-width: 0;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  font-weight: 600;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-run-state-meta {
  margin-left: auto;
  color: var(--text-secondary);
  font-size: 11px;
  flex: none;
}

.unity-run-phases {
  display: flex;
  flex-direction: column;
}

.unity-run-phase {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  min-width: 0;
  border-top: 1px solid color-mix(in srgb, var(--border-color) 66%, transparent);
}

.unity-run-phase:first-child {
  border-top: 0;
}

.unity-run-phase-label {
  padding: 7px 8px;
  border-right: 1px solid color-mix(in srgb, var(--border-color) 66%, transparent);
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  line-height: 1.5;
  background: color-mix(in srgb, var(--sidebar-bg) 46%, transparent);
}

.unity-run-phase-code {
  min-width: 0;
  max-height: 280px;
  overflow: auto;
  margin: 0;
  padding: 7px 8px;
  color: var(--text-color);
  background: transparent;
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  scrollbar-gutter: stable;
}

.unity-run-phase-empty {
  min-width: 0;
  padding: 7px 8px;
  color: var(--text-secondary);
  font-family: var(--font-mono-block);
  font-size: 12px;
  line-height: 1.5;
}

@media (max-width: 720px) {
  .unity-run-summary {
    grid-template-columns: minmax(0, 1fr);
  }

  .unity-run-phase {
    grid-template-columns: minmax(0, 1fr);
  }

  .unity-run-phase-label {
    border-right: 0;
    border-bottom: 1px solid color-mix(in srgb, var(--border-color) 66%, transparent);
  }
}
</style>
