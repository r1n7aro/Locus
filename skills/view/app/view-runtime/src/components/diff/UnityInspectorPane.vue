<script setup lang="ts">
import { ref, computed, watch, watchEffect } from "vue";
import { t } from "../../i18n";
import UnityInspectorFieldTree from "./UnityInspectorFieldTree.vue";
import OptimizedPanelRenderer from "./OptimizedPanelRenderer.vue";
import { getRendererConfig } from "./rendererRegistry";
import type { InspectorPanel, SemanticTargetInspector, SemanticDisplayMode } from "../../types";
import {
  cleanInspectorPanelTitle,
  getInspectorPanelDisplayTitle,
  getInspectorPanelInferenceBadge,
  getInspectorPanelInferenceTooltip,
  getInspectorPanelResolveReason,
} from "./inspectorPanelDisplay";
import {
  buildGameObjectHeaderSummary,
  parsePrefabSourceSummary,
} from "./unityInspectorHeader";

const props = withDefaults(
  defineProps<{
    inspector?: SemanticTargetInspector | null;
    loading: boolean;
    error?: string | null;
    includeUnchanged: boolean;
    hideToolbar?: boolean;
    displayMode?: SemanticDisplayMode;
    headerLayout?: "default" | "unity";
  }>(),
  {
    displayMode: "optimized",
    headerLayout: "default",
  },
);

const emit = defineEmits<{
  "toggle-unchanged": [];
  "toggle-display-mode": [];
}>();

const collapsedPanels = ref(new Set<number>());
const hoveredInferencePanel = ref<number | null>(null);
const warnedPanelKeys = new Set<string>();
const unityHeaderLayout = computed(() => props.headerLayout === "unity");
const gameObjectHeaderPanel = computed(() =>
  props.inspector?.panels.find((panel) => panel.panelKind === "gameObjectHeader") ?? null,
);
const gameObjectSummary = computed(() => buildGameObjectHeaderSummary(gameObjectHeaderPanel.value));
const prefabSource = computed(() => parsePrefabSourceSummary(props.inspector?.subtitle));
const panelEntries = computed(() => {
  const inspector = props.inspector;
  if (!inspector) return [];

  return inspector.panels
    .map((panel, index) => ({ panel, index }))
    .filter(({ panel }) => !(unityHeaderLayout.value && panel.panelKind === "gameObjectHeader"))
    .sort((left, right) => {
      const rankDiff = panelOrderRank(left.panel) - panelOrderRank(right.panel);
      return rankDiff !== 0 ? rankDiff : left.index - right.index;
    });
});
const objectDisplayName = computed(() =>
  gameObjectSummary.value.name ?? cleanTitle(props.inspector?.title ?? ""),
);
const displaySubtitle = computed(() => {
  const cleaned = cleanSubtitle(props.inspector?.subtitle);
  if (!cleaned) return null;
  return prefabSource.value ? null : cleaned;
});
const unitySubtitle = computed(() => {
  if (!unityHeaderLayout.value) return null;
  if (!displaySubtitle.value) return null;
  return displaySubtitle.value === objectDisplayName.value ? null : displaySubtitle.value;
});

/** Header is redundant when there's exactly one panel whose title matches the inspector title */
const headerRedundant = computed(() => {
  if (unityHeaderLayout.value) return false;
  const insp = props.inspector;
  if (!insp || insp.panels.length !== 1) return false;
  const panel = insp.panels[0];
  if (panel.componentInference) return false;
  const panelLabel = getInspectorPanelDisplayTitle(panel);
  return panelLabel === cleanTitle(insp.title);
});

watchEffect(() => {
  const inspector = props.inspector;
  if (!inspector) return;

  inspector.panels.forEach((panel, panelIndex) => {
    const reason = getInspectorPanelResolveReason(panel);
    if (!reason) return;

    const warnKey = `${inspector.targetId}:${panelIndex}:${reason}`;
    if (warnedPanelKeys.has(warnKey)) return;
    warnedPanelKeys.add(warnKey);

    console.warn("[UnityInspectorPane] component name resolution fallback", {
      targetId: inspector.targetId,
      inspectorTitle: inspector.title,
      panelIndex,
      panelTitle: panel.title,
      componentType: panel.componentType ?? null,
      componentClassId: panel.componentClassId ?? null,
      componentSource: panel.componentSource ?? null,
      reason,
    });
  });
});

watch(
  () => props.inspector?.targetId ?? null,
  () => {
    hoveredInferencePanel.value = null;
  },
);

function togglePanel(index: number) {
  if (collapsedPanels.value.has(index)) {
    collapsedPanels.value.delete(index);
  } else {
    collapsedPanels.value.add(index);
  }
}

/** Map internal panelKind to a user-friendly label */
function panelKindLabel(kind: string): string {
  switch (kind) {
    case "gameObjectHeader": return "GameObject";
    case "component": return "Component";
    case "assetRoot": return "Asset";
    case "subObject": return "Sub-Object";
    default: return kind;
  }
}

/** Strip fileID from labels like "StateSO (fileID:11400000)" */
function cleanTitle(title: string): string {
  return cleanInspectorPanelTitle(title);
}

/** Strip fileID from subtitles — if subtitle IS just "fileID:xxx", return null */
function cleanSubtitle(subtitle?: string): string | null {
  if (!subtitle) return null;
  const cleaned = subtitle.replace(/\s*\(?\s*fileID:-?\d+\s*\)?\s*/g, "").trim();
  return cleaned || null;
}

function changeIcon(kind: string): string {
  switch (kind) {
    case "added": return "A";
    case "removed": return "D";
    case "modified": return "M";
    default: return "";
  }
}

function panelDisplayTitle(panel: InspectorPanel): string {
  return getInspectorPanelDisplayTitle(panel);
}

function panelOrderRank(panel: InspectorPanel): number {
  if (panel.panelKind === "gameObjectHeader") return 0;
  if (isTransformPanel(panel)) return 1;
  return 2;
}

function isTransformPanel(panel: InspectorPanel): boolean {
  if (panel.panelKind !== "component") return false;
  const componentType = panel.componentType ?? getInspectorPanelDisplayTitle(panel);
  return componentType === "Transform" || componentType === "RectTransform";
}

function panelInferenceBadge(panel: InspectorPanel): string {
  return getInspectorPanelInferenceBadge(panel);
}

function panelInferenceTooltip(panel: InspectorPanel): string {
  return getInspectorPanelInferenceTooltip(panel);
}

function showInferenceTooltip(index: number) {
  hoveredInferencePanel.value = index;
}

function hideInferenceTooltip(index: number) {
  if (hoveredInferencePanel.value === index) {
    hoveredInferencePanel.value = null;
  }
}

function sourceLabel(kind: NonNullable<ReturnType<typeof parsePrefabSourceSummary>>["kind"]): string {
  switch (kind) {
    case "prefab":
      return t("diff.inspector.parentPrefab");
    case "fbx":
      return t("diff.inspector.parentFbx");
    case "model":
      return t("diff.inspector.sourceModel");
    default:
      return t("diff.inspector.parentAsset");
  }
}

function gameObjectFieldLabel(propertyPath: string, fallback: string): string {
  return gameObjectHeaderPanel.value?.fields.find((field) => field.propertyPath === propertyPath)?.label ?? fallback;
}
</script>

<template>
  <div class="unity-inspector-pane">
    <!-- Toolbar -->
    <div v-if="!hideToolbar" class="inspector-toolbar">
      <div class="toolbar-actions">
        <button class="toggle-btn" @click="emit('toggle-unchanged')">
          {{ includeUnchanged ? t('diff.fields.hideUnchanged') : t('diff.fields.showUnchanged') }}
        </button>
        <button class="toggle-btn" :class="{ active: displayMode === 'optimized' }" @click="emit('toggle-display-mode')">
          {{ displayMode === 'optimized' ? t('diff.mode.optimized') : t('diff.mode.full') }}
        </button>
      </div>
    </div>

    <!-- States -->
    <div v-if="loading" class="inspector-state">Loading inspector…</div>
    <div v-else-if="error" class="inspector-state error">{{ error }}</div>
    <div v-else-if="!inspector" class="inspector-state">Select an object to inspect changes</div>

    <!-- Body -->
    <div v-else class="inspector-body">
      <!-- Inspector header — hidden when redundant with single panel -->
      <div v-if="!headerRedundant" class="inspector-header" :class="{ 'unity-layout': unityHeaderLayout }">
        <template v-if="unityHeaderLayout">
          <div class="unity-header-shell">
            <div class="unity-header-main">
              <span
                class="unity-active-toggle"
                :class="{
                  inactive: gameObjectSummary.active === false,
                  unknown: gameObjectSummary.active === null,
                }"
                aria-hidden="true"
              >
                <span v-if="gameObjectSummary.active === true" class="unity-active-check">✓</span>
              </span>

              <div class="unity-header-name-block">
                <div class="unity-header-name-row">
                  <div
                    class="unity-name-field"
                    :class="{ inactive: gameObjectSummary.active === false }"
                  >
                    {{ objectDisplayName }}
                  </div>
                  <span v-if="gameObjectSummary.active === false" class="unity-header-flag">
                    {{ t("diff.inspector.inactive") }}
                  </span>
                  <span v-if="gameObjectSummary.isStatic" class="unity-header-flag">
                    {{ t("diff.inspector.static") }}
                  </span>
                </div>
                <div v-if="unitySubtitle" class="unity-header-caption">{{ unitySubtitle }}</div>
              </div>
            </div>

            <div
              v-if="gameObjectSummary.tag || gameObjectSummary.layer"
              class="unity-inline-meta"
            >
              <div v-if="gameObjectSummary.tag" class="unity-inline-field">
                <span class="unity-inline-label">{{ gameObjectFieldLabel("m_TagString", "Tag") }}</span>
                <span class="unity-inline-value">{{ gameObjectSummary.tag }}</span>
              </div>
              <div v-if="gameObjectSummary.layer" class="unity-inline-field">
                <span class="unity-inline-label">{{ gameObjectFieldLabel("m_Layer", "Layer") }}</span>
                <span class="unity-inline-value">{{ gameObjectSummary.layer }}</span>
              </div>
            </div>

            <div v-if="prefabSource" class="unity-inline-field unity-inline-field-wide">
              <span class="unity-inline-label">{{ sourceLabel(prefabSource.kind) }}</span>
              <span class="unity-inline-value unity-inline-value-path" :title="prefabSource.path">
                {{ prefabSource.path }}
              </span>
            </div>
          </div>
        </template>

        <div v-else class="inspector-meta">
          <div class="inspector-icon">■</div>
          <div class="inspector-meta-text">
            <div class="inspector-title">{{ cleanTitle(inspector.title) }}</div>
            <div v-if="displaySubtitle" class="inspector-subtitle">{{ displaySubtitle }}</div>
          </div>
        </div>
      </div>

      <!-- Panels (components) -->
      <div
        v-for="entry in panelEntries"
        :key="`${inspector.targetId}-${entry.index}`"
        class="panel-card"
        :class="{ collapsed: collapsedPanels.has(entry.index) }"
      >
        <!-- Panel header — hidden when redundant with inspector header -->
        <div
          v-if="!headerRedundant"
          class="panel-header"
          :class="entry.panel.changeKind"
          @click="togglePanel(entry.index)"
        >
          <!-- Change bar on left edge -->
          <span
            v-if="entry.panel.changeKind !== 'unchanged'"
            class="panel-change-bar"
            :class="entry.panel.changeKind"
          />

          <span class="panel-fold" :class="{ open: !collapsedPanels.has(entry.index) }">▶</span>
          <span class="panel-title">{{ panelDisplayTitle(entry.panel) }}</span>
          <span
            v-if="panelInferenceBadge(entry.panel)"
            class="panel-inference-anchor"
            @mouseenter="showInferenceTooltip(entry.index)"
            @mouseleave="hideInferenceTooltip(entry.index)"
            @focusin="showInferenceTooltip(entry.index)"
            @focusout="hideInferenceTooltip(entry.index)"
            @click.stop.prevent
          >
            <span
              class="panel-inference-badge"
              :aria-label="panelInferenceTooltip(entry.panel)"
              tabindex="0"
            >
              {{ panelInferenceBadge(entry.panel) }}
            </span>
            <span
              v-if="hoveredInferencePanel === entry.index"
              class="panel-inference-tooltip"
              role="note"
            >
              {{ panelInferenceTooltip(entry.panel) }}
            </span>
          </span>
          <span class="panel-kind-badge">{{ panelKindLabel(entry.panel.panelKind) }}</span>
          <span
            v-if="entry.panel.changeKind !== 'unchanged'"
            class="panel-change-badge"
            :class="entry.panel.changeKind"
          >
            {{ changeIcon(entry.panel.changeKind) }}
          </span>
        </div>

        <!-- Panel body -->
        <div v-if="!collapsedPanels.has(entry.index)" class="panel-body">
          <div v-if="entry.panel.fields.length === 0" class="panel-empty">No visible field changes</div>
          <OptimizedPanelRenderer
            v-else-if="displayMode === 'optimized' && getRendererConfig(entry.panel)"
            :panel="entry.panel"
            :config="getRendererConfig(entry.panel)!"
          />
          <template v-else>
            <UnityInspectorFieldTree
              v-for="field in entry.panel.fields"
              :key="field.id"
              :field="field"
            />
          </template>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.unity-inspector-pane {
  height: 100%;
  overflow: auto;
  background: var(--panel-bg);
  --unity-inspector-surface: color-mix(in srgb, var(--panel-bg) 92%, var(--bg-color) 8%);
  --unity-inspector-divider: color-mix(in srgb, var(--border-color) 78%, transparent);
  --unity-component-row-bg: color-mix(in srgb, var(--hover-bg) 72%, var(--sidebar-bg) 28%);
  --unity-component-row-hover-bg: color-mix(in srgb, var(--hover-bg) 84%, var(--active-bg) 16%);
  --unity-field-section-bg: color-mix(in srgb, var(--sidebar-bg) 62%, var(--panel-bg) 38%);
  --unity-field-row-bg: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  --unity-field-group-row-bg: color-mix(in srgb, var(--panel-bg) 78%, var(--bg-color) 22%);
  --unity-field-row-hover-bg: color-mix(in srgb, var(--hover-bg) 64%, var(--panel-bg) 36%);
}

/* ── Toolbar ── */
.inspector-toolbar {
  display: flex;
  justify-content: flex-end;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-left: auto;
}

.toggle-btn {
  padding: 4px 10px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 80%, var(--bg-color) 20%);
  color: var(--text-secondary);
  font-size: 11px;
  cursor: pointer;
}

.toggle-btn:hover {
  background: var(--bg-hover);
  color: var(--text-color);
}

.toggle-btn.active {
  border-color: color-mix(in srgb, var(--git-focus) 32%, var(--border-color));
  background: color-mix(in srgb, var(--git-focus) 12%, var(--bg-color));
  color: var(--text-color);
}

/* ── States ── */
.inspector-state {
  padding: 18px;
  color: var(--text-secondary);
  font-size: 13px;
}

.inspector-state.error {
  color: var(--status-danger-fg);
}

/* ── Body ── */
.inspector-body {
  display: flex;
  flex-direction: column;
  gap: 1px;
  background: var(--unity-inspector-divider);
}

/* ── Inspector header — like Unity's top bar showing the object ── */
.inspector-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
  background: var(--unity-inspector-surface);
  border-bottom: 1px solid var(--unity-inspector-divider);
}

.inspector-header.unity-layout {
  padding: 10px 12px 12px;
}

.inspector-icon {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  background: color-mix(in srgb, var(--sidebar-bg) 64%, var(--panel-bg) 36%);
  color: var(--text-secondary);
  font-size: 14px;
  flex-shrink: 0;
}

.inspector-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  flex: 1;
  min-width: 0;
}

.inspector-meta-text {
  flex: 1;
  min-width: 0;
}

.inspector-title {
  font-size: 14px;
  font-weight: 700;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.inspector-subtitle {
  font-size: 11px;
  color: var(--text-secondary);
  margin-top: 1px;
}

.unity-header-shell {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.unity-header-main {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  min-width: 0;
}

.unity-active-toggle {
  width: 14px;
  height: 14px;
  margin-top: 6px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, var(--text-secondary));
  border-radius: 4px;
  background: color-mix(in srgb, var(--input-bg) 70%, var(--panel-bg) 30%);
}

.unity-active-toggle.inactive,
.unity-active-toggle.unknown {
  background: var(--panel-bg);
}

.unity-active-check {
  font-size: 10px;
  line-height: 1;
  color: var(--text-color);
}

.unity-header-name-block {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.unity-header-name-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.unity-name-field {
  min-width: 0;
  flex: 1;
  padding: 5px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, var(--bg-color));
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 72%, var(--panel-bg) 28%);
  font-size: 12.5px;
  font-weight: 600;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.unity-name-field.inactive {
  color: var(--text-secondary);
}

.unity-header-flag,
.unity-header-caption,
.unity-inline-label,
.unity-inline-value-path {
  font-size: 11px;
  color: var(--text-secondary);
}

.unity-header-flag {
  flex-shrink: 0;
  white-space: nowrap;
}

.unity-inline-meta {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px 12px;
}

.unity-inline-field {
  min-width: 0;
  display: grid;
  grid-template-columns: 56px minmax(0, 1fr);
  align-items: center;
  gap: 8px;
}

.unity-inline-field-wide {
  grid-template-columns: 72px minmax(0, 1fr);
}

.unity-inline-label {
  white-space: nowrap;
}

.unity-inline-value {
  min-width: 0;
  padding: 5px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, var(--bg-color));
  border-radius: 6px;
  background: color-mix(in srgb, var(--input-bg) 72%, var(--panel-bg) 28%);
  font-size: 12px;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

@media (max-width: 720px) {
  .unity-inline-meta {
    grid-template-columns: 1fr;
  }

  .unity-inline-field,
  .unity-inline-field-wide {
    grid-template-columns: 64px minmax(0, 1fr);
  }
}

/* ── Panel card ── */
.panel-card {
  background: var(--unity-inspector-surface);
}

/* ── Panel header — Unity gray component bar ── */
.panel-header {
  position: relative;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 12px;
  background: var(--unity-component-row-bg);
  border-bottom: 1px solid var(--unity-inspector-divider);
  cursor: pointer;
}

.panel-header:hover {
  background: var(--unity-component-row-hover-bg);
}

/* Left change bar on panel */
.panel-change-bar {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.panel-change-bar.added {
  background: var(--git-status-added);
}

.panel-change-bar.removed {
  background: var(--git-status-deleted);
}

.panel-change-bar.modified {
  background: var(--git-status-modified);
}

.panel-fold {
  font-size: 8px;
  color: var(--text-secondary);
  transition: transform 0.15s ease;
  transform: rotate(0deg);
  flex-shrink: 0;
  width: 12px;
  text-align: center;
}

.panel-fold.open {
  transform: rotate(90deg);
}

.panel-title {
  font-weight: 700;
  font-size: 12.5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.panel-inference-anchor {
  position: relative;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
}

.panel-inference-badge {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  border: 1px solid color-mix(in srgb, var(--git-status-modified) 55%, var(--border-color));
  background: color-mix(in srgb, var(--git-status-modified) 18%, var(--bg-color));
  color: color-mix(in srgb, var(--git-status-modified) 80%, var(--text-color));
  font-size: 10px;
  font-weight: 700;
  line-height: 1;
  cursor: help;
}

.panel-inference-badge:hover {
  background: color-mix(in srgb, var(--git-status-modified) 32%, var(--bg-color));
}

.panel-inference-badge:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--git-status-modified) 38%, var(--border-color));
  outline-offset: 2px;
}

.panel-inference-tooltip {
  position: absolute;
  top: calc(100% + 6px);
  left: -10px;
  z-index: 12;
  min-width: 240px;
  max-width: min(560px, calc(100vw - 48px));
  padding: 8px 10px;
  border: 1px solid color-mix(in srgb, var(--git-status-modified) 26%, var(--border-color));
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 96%, var(--bg-color));
  box-shadow: 0 10px 24px color-mix(in srgb, var(--text-color) 16%, transparent);
  color: var(--text-color);
  font-size: 12px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}

.panel-kind-badge {
  font-size: 10px;
  color: var(--text-secondary);
  padding: 1px 6px;
  border-radius: 4px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--hover-bg) 28%);
  flex-shrink: 0;
}

.panel-change-badge {
  margin-left: auto;
  flex-shrink: 0;
  padding: 0 6px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 700;
  line-height: 18px;
}

.panel-change-badge.added {
  color: var(--git-status-added);
  background: color-mix(in srgb, var(--git-status-added) 14%, var(--bg-color));
}

.panel-change-badge.removed {
  color: var(--git-status-deleted);
  background: color-mix(in srgb, var(--git-status-deleted) 14%, var(--bg-color));
}

.panel-change-badge.modified {
  color: var(--git-status-modified);
  background: color-mix(in srgb, var(--git-status-modified) 14%, var(--bg-color));
}

/* ── Panel body ── */
.panel-body {
  background: var(--unity-field-row-bg);
  border-bottom: 1px solid var(--unity-inspector-divider);
}

.panel-empty {
  padding: 10px 14px;
  color: var(--text-secondary);
  font-size: 12px;
}
</style>
