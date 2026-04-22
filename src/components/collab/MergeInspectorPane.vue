<script setup lang="ts">
import { computed, ref } from "vue";
import type { MergeField, MergePanel, MergeTargetInspector, SemanticBadgeCounts } from "../../types";
import type { MergeResolutionState } from "../../composables/useMergeResolution";
import { t } from "../../i18n";
import {
  getInspectorPanelDisplayTitle,
  getInspectorPanelInferenceBadge,
  getInspectorPanelInferenceTooltip,
} from "../diff/inspectorPanelDisplay";
import {
  type MergeDisplayStatus,
  compactBaseLabel,
  compactMergeSideLabel,
  hierarchyBadgeLabel,
  humanizeMergeSideLabel,
  mergeStatusLabel,
  sharedBaseLabel,
} from "./mergeUi";
import MergeInspectorFieldTree from "./MergeInspectorFieldTree.vue";

const collapsedPanels = ref(new Set<number>());
const showSharedBase = ref(false);

function togglePanel(index: number) {
  const next = new Set(collapsedPanels.value);
  if (next.has(index)) next.delete(index);
  else next.add(index);
  collapsedPanels.value = next;
}

const props = defineProps<{
  inspector?: MergeTargetInspector | null;
  loading: boolean;
  error?: string | null;
  resolution: MergeResolutionState;
  leftLabel?: string;
  rightLabel?: string;
  badgeCounts?: SemanticBadgeCounts | null;
}>();

const badgeEntries = computed(() => {
  const counts = props.badgeCounts;
  if (!counts) return [];
  return [
    counts.added ? { label: hierarchyBadgeLabel("added", counts.added), kind: "added" } : null,
    counts.removed ? { label: hierarchyBadgeLabel("removed", counts.removed), kind: "removed" } : null,
    counts.modified ? { label: hierarchyBadgeLabel("modified", counts.modified), kind: "modified" } : null,
    counts.componentsChanged ? { label: hierarchyBadgeLabel("comp", counts.componentsChanged), kind: "comp" } : null,
  ].filter(Boolean) as { label: string; kind: string }[];
});

const displayLeftLabel = computed(() => humanizeMergeSideLabel(props.leftLabel, "left"));
const displayRightLabel = computed(() => humanizeMergeSideLabel(props.rightLabel, "right"));
const compactLeftLabel = computed(() => compactMergeSideLabel(props.leftLabel, "left"));
const compactRightLabel = computed(() => compactMergeSideLabel(props.rightLabel, "right"));
const displayBaseLabel = computed(() => sharedBaseLabel());
const compactBase = computed(() => compactBaseLabel());

function collectConflictProgress(fields: MergeField[]): { total: number; resolved: number } {
  let total = 0;
  let resolved = 0;

  const walk = (field: MergeField) => {
    if (field.children.length === 0) {
      if (field.mergeState === "conflict") {
        total += 1;
        if (props.resolution.fieldResolutions.value.has(field.id)) {
          resolved += 1;
        }
      }
      return;
    }
    for (const child of field.children) {
      walk(child);
    }
  };

  for (const field of fields) {
    walk(field);
  }

  return { total, resolved };
}

function panelDisplayStatus(panelIndex: number): MergeDisplayStatus {
  if (!props.inspector) return "unchanged";
  const panel = props.inspector.panels[panelIndex];
  if (!panel) return "unchanged";
  if (props.resolution.targetResolutions.value.has(props.inspector.targetId)) {
    return "stagedResolved";
  }

  const progress = collectConflictProgress(panel.fields);
  if (progress.total <= 0) return panel.mergeStatus;
  if (progress.resolved >= progress.total) return "stagedResolved";
  if (progress.resolved > 0) return "stagedPartial";
  return panel.mergeStatus;
}

function panelDisplayTitle(panel: MergePanel): string {
  return getInspectorPanelDisplayTitle(panel);
}

function panelInferenceBadge(panel: MergePanel): string {
  return getInspectorPanelInferenceBadge(panel);
}

function panelInferenceTooltip(panel: MergePanel): string {
  return getInspectorPanelInferenceTooltip(panel);
}
</script>

<template>
  <div class="merge-inspector-pane" :class="{ 'show-base': showSharedBase }">
    <div v-if="loading" class="merge-inspector-state">{{ t("merge.panels.loadingTarget") }}</div>
    <div v-else-if="error" class="merge-inspector-state error">{{ error }}</div>
    <div v-else-if="!inspector" class="merge-inspector-state">{{ t("merge.panels.selectTarget") }}</div>

    <template v-else>
      <div class="merge-inspector-topbar">
        <div class="merge-inspector-meta">
          <span class="merge-inspector-title">{{ inspector.title }}</span>
          <span v-if="inspector.path && inspector.path !== inspector.title" class="merge-inspector-path">{{ inspector.path }}</span>
          <template v-if="badgeEntries.length">
            <span v-for="badge in badgeEntries" :key="badge.label" class="inspector-badge-chip" :class="badge.kind">{{ badge.label }}</span>
          </template>
        </div>
        <button
          class="merge-mini-action merge-view-toggle"
          :class="{ active: showSharedBase }"
          type="button"
          :aria-pressed="showSharedBase"
          :title="showSharedBase ? t('merge.actions.hideSharedBase') : t('merge.actions.showSharedBase')"
          @click="showSharedBase = !showSharedBase"
        >
          {{ showSharedBase ? t('merge.actions.hideSharedBase') : t('merge.actions.showSharedBase') }}
        </button>
      </div>

      <div class="merge-inspector-table-wrap">
        <div class="merge-inspector-column-header">
          <span class="col-label">{{ t("merge.panels.field") }}</span>
          <span v-if="showSharedBase" class="col-value tone-base" :title="displayBaseLabel">
            <span class="col-chip">{{ compactBase }}</span>
          </span>
          <span class="col-value tone-ours" :title="displayLeftLabel">
            <span class="col-chip">{{ compactLeftLabel }}</span>
          </span>
          <span class="col-value tone-theirs" :title="displayRightLabel">
            <span class="col-chip">{{ compactRightLabel }}</span>
          </span>
        </div>

        <div class="merge-inspector-panels">
          <section
            v-for="(panel, panelIndex) in inspector.panels"
            :key="`${inspector.targetId}:${panelIndex}`"
            class="merge-panel-card"
            :class="{ collapsed: collapsedPanels.has(panelIndex) }"
          >
            <div class="merge-panel-header" :class="panelDisplayStatus(panelIndex)">
              <span v-if="panelDisplayStatus(panelIndex) !== 'unchanged'" class="merge-panel-change-bar" :class="panelDisplayStatus(panelIndex)" />
              <button
                type="button"
                class="merge-panel-toggle"
                :aria-expanded="!collapsedPanels.has(panelIndex)"
                @click="togglePanel(panelIndex)"
              >
                <span class="merge-panel-fold" :class="{ open: !collapsedPanels.has(panelIndex) }">&#x25B6;</span>
                <span class="merge-panel-title-wrap">
                  <span class="merge-panel-title">{{ panelDisplayTitle(panel) }}</span>
                  <span
                    v-if="panelInferenceBadge(panel)"
                    class="merge-panel-inference-badge"
                    :title="panelInferenceTooltip(panel)"
                    @click.stop
                  >
                    {{ panelInferenceBadge(panel) }}
                  </span>
                  <span v-if="panel.scriptClass && panel.scriptClass !== panelDisplayTitle(panel)" class="merge-panel-subtitle">{{ panel.scriptClass }}</span>
                </span>
              </button>
              <div class="merge-panel-right">
                <span class="merge-panel-status" :class="panelDisplayStatus(panelIndex)">{{ mergeStatusLabel(panelDisplayStatus(panelIndex)) }}</span>
                <button class="merge-mini-action merge-staged-choice-btn" :title="t('merge.actions.choosePanelFrom', displayLeftLabel)" @click="resolution.acceptPanel(inspector.targetId, panelIndex, 'ours', inspector)">{{ compactLeftLabel }}</button>
                <button class="merge-mini-action merge-staged-choice-btn" :title="t('merge.actions.choosePanelFrom', displayRightLabel)" @click="resolution.acceptPanel(inspector.targetId, panelIndex, 'theirs', inspector)">{{ compactRightLabel }}</button>
                <button v-if="showSharedBase" class="merge-mini-action merge-staged-choice-btn" :title="t('merge.actions.choosePanelFrom', displayBaseLabel)" @click="resolution.acceptPanel(inspector.targetId, panelIndex, 'base', inspector)">{{ compactBase }}</button>
              </div>
            </div>

            <template v-if="!collapsedPanels.has(panelIndex)">
              <div v-if="panel.fields.length === 0" class="merge-panel-empty">{{ t("merge.panels.noFields") }}</div>
              <div v-else class="merge-panel-body">
                <MergeInspectorFieldTree
                  v-for="field in panel.fields"
                  :key="field.id"
                  :field="field"
                  :left-label="leftLabel"
                  :right-label="rightLabel"
                  :show-shared-base="showSharedBase"
                  :resolution-map="resolution.fieldResolutions.value"
                  @accept="(fieldId, side, fieldDef) => resolution.acceptField(fieldId, side, fieldDef)"
                />
              </div>
            </template>
          </section>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.merge-inspector-pane {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg-color);
  container-type: inline-size;
  --merge-field-columns: minmax(220px, 1.45fr) repeat(2, minmax(184px, 1fr));
  --merge-table-min-width: 620px;
}

.merge-inspector-pane.show-base {
  --merge-field-columns: minmax(210px, 1.3fr) repeat(3, minmax(156px, 0.95fr));
  --merge-table-min-width: 760px;
}

.merge-inspector-state {
  padding: 20px;
  color: var(--text-secondary);
  font-size: 13px;
}

.merge-inspector-state.error {
  color: #e15759;
}

.merge-inspector-topbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 10px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--bg-color) 94%, var(--hover-bg));
}

.merge-inspector-meta {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex: 1;
  flex-wrap: wrap;
}

.merge-inspector-title {
  font-size: 13px;
  font-weight: 700;
  color: var(--text-color);
  white-space: nowrap;
}

.merge-inspector-path {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.inspector-badge-chip {
  padding: 1px 6px;
  border-radius: 4px;
  font-size: 10px;
  line-height: 16px;
  font-weight: 600;
  color: var(--text-secondary);
  background: rgba(255, 255, 255, 0.08);
  white-space: nowrap;
}

.inspector-badge-chip.added {
  color: #68d391;
  background: rgba(56, 161, 105, 0.1);
}

.inspector-badge-chip.removed {
  color: #fc8181;
  background: rgba(229, 62, 62, 0.1);
}

.inspector-badge-chip.modified {
  color: #ecc94b;
  background: rgba(214, 158, 46, 0.1);
}

.inspector-badge-chip.comp {
  color: #90cdf4;
  background: rgba(66, 153, 225, 0.1);
}

.merge-view-toggle.active {
  color: var(--text-color);
  border-color: var(--accent-color);
  background: color-mix(in srgb, var(--accent-color) 8%, transparent);
}

.merge-inspector-helper {
  padding: 6px 10px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--bg-color) 97%, var(--hover-bg));
  color: var(--text-secondary);
  font-size: 10.5px;
  line-height: 1.35;
}

.merge-inspector-column-header {
  display: grid;
  grid-template-columns: var(--merge-field-columns);
  gap: 6px;
  padding: 7px 10px 6px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--bg-color) 96%, var(--hover-bg));
  color: var(--text-secondary);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.02em;
  min-width: var(--merge-table-min-width);
  position: sticky;
  top: 0;
  z-index: 1;
}

.col-label,
.col-value {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.col-value {
  display: flex;
  align-items: center;
}

.col-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 0;
  padding: 2px 8px;
  border-radius: var(--radius-badge);
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, transparent);
  background: color-mix(in srgb, var(--bg-color) 84%, var(--hover-bg));
  color: var(--text-color);
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.04em;
}

.tone-base .col-chip {
  background: rgba(100, 116, 139, 0.12);
  color: #526075;
  border-color: rgba(100, 116, 139, 0.22);
}

.tone-ours .col-chip {
  background: rgba(57, 97, 255, 0.12);
  color: #3558d8;
  border-color: rgba(57, 97, 255, 0.22);
}

.tone-theirs .col-chip {
  background: rgba(214, 144, 25, 0.16);
  color: #b87400;
  border-color: rgba(214, 144, 25, 0.24);
}

.merge-inspector-table-wrap {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

.merge-inspector-panels {
  display: flex;
  flex-direction: column;
  gap: 0;
  padding: 0;
  background: var(--bg-color);
  min-width: var(--merge-table-min-width);
}

.merge-panel-card {
  background: var(--bg-color);
  min-width: var(--merge-table-min-width);
}

.merge-panel-header {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  border-bottom: 1px solid var(--border-color);
  background: var(--bg-color);
}

.merge-panel-header:hover {
  background: var(--hover-bg);
}

.merge-panel-toggle {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
  padding: 0;
  border: none;
  background: transparent;
  color: inherit;
  text-align: left;
  cursor: pointer;
}

.merge-panel-toggle:focus-visible,
.merge-mini-action:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: 1px;
}

.merge-panel-change-bar {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.merge-panel-change-bar.hasConflicts {
  background: #d69e2e;
}

.merge-panel-change-bar.stagedPartial {
  background: #d69e2e;
}

.merge-panel-change-bar.stagedResolved,
.merge-panel-change-bar.autoResolved,
.merge-panel-change-bar.addedOurs,
.merge-panel-change-bar.addedTheirs {
  background: #38a169;
}

.merge-panel-change-bar.removedOurs,
.merge-panel-change-bar.removedTheirs {
  background: #e53e3e;
}

.merge-panel-fold {
  font-size: 8px;
  color: var(--text-secondary);
  transition: transform 0.15s ease;
  transform: rotate(0deg);
  flex-shrink: 0;
  width: 12px;
  text-align: center;
}

.merge-panel-fold.open {
  transform: rotate(90deg);
}

.merge-panel-title-wrap {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  flex-wrap: wrap;
}

.merge-panel-title {
  font-size: 12.5px;
  font-weight: 700;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}

.merge-panel-inference-badge {
  flex-shrink: 0;
  padding: 1px 5px;
  border-radius: 4px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  background: color-mix(in srgb, var(--accent-color) 8%, var(--bg-color));
  color: var(--text-secondary);
  font-size: 10px;
  font-weight: 700;
  line-height: 16px;
}

.merge-panel-subtitle {
  font-size: 11px;
  color: var(--text-secondary);
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.merge-panel-right {
  display: flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
  margin-left: auto;
}

.merge-panel-status {
  padding: 0 6px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 700;
  line-height: 18px;
}

.merge-panel-status.hasConflicts {
  background: rgba(210, 155, 0, 0.18);
  color: #d29b00;
}

.merge-panel-status.stagedPartial {
  background: rgba(210, 155, 0, 0.18);
  color: #d29b00;
}

.merge-panel-status.stagedResolved,
.merge-panel-status.autoResolved,
.merge-panel-status.addedOurs,
.merge-panel-status.addedTheirs,
.merge-panel-status.removedOurs,
.merge-panel-status.removedTheirs {
  background: rgba(46, 160, 67, 0.16);
  color: #3fb950;
}

.merge-mini-action {
  min-width: 34px;
  padding: 3px 7px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10px;
  font-weight: 700;
  cursor: pointer;
}

.merge-mini-action:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
  background: var(--hover-bg);
}

.merge-panel-body {
  border-bottom: 1px solid var(--border-color);
}

.merge-panel-empty {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
  border-bottom: 1px solid var(--border-color);
}

@container (max-width: 1280px) {
  .merge-inspector-pane {
    --merge-field-columns: minmax(196px, 1.28fr) repeat(2, minmax(164px, 0.98fr));
    --merge-table-min-width: 560px;
  }

  .merge-inspector-pane.show-base {
    --merge-field-columns: minmax(184px, 1.18fr) repeat(3, minmax(142px, 0.94fr));
    --merge-table-min-width: 700px;
  }

  .merge-inspector-topbar {
    padding: 5px 10px;
  }

  .merge-inspector-title {
    font-size: 12px;
  }
}

@container (max-width: 1080px) {
  .merge-inspector-topbar {
    flex-wrap: wrap;
  }
}
</style>
