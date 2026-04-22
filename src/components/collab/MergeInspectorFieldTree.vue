<script setup lang="ts">
import { computed, ref } from "vue";
import type { MergeField, MergeSide } from "../../types";
import { t } from "../../i18n";
import { parseDisplayValue, type ParsedDisplayValue } from "../diff/fieldUtils";
import {
  compactBaseLabel,
  compactMergeSideLabel,
  humanizeMergeSideLabel,
  mergeStatusLabel,
  sharedBaseLabel,
} from "./mergeUi";

type MergeSelectionState = MergeSide | "mixed" | null;

interface SourceColumn {
  side: MergeSide;
  compactLabel: string;
  fullLabel: string;
}

interface CompactTuple {
  label: string;
  value: string;
}

interface VectorMergeComponent {
  label: string;
  base: string;
  ours: string;
  theirs: string;
}

interface FieldChoiceStats {
  leafCount: number;
  resolvedLeafCount: number;
  conflictTotal: number;
  resolvedConflictCount: number;
  uniqueLeafSides: Set<MergeSide>;
}

const props = defineProps<{
  field: MergeField;
  depth?: number;
  leftLabel?: string;
  rightLabel?: string;
  showSharedBase?: boolean;
  resolutionMap?: ReadonlyMap<string, MergeSide>;
}>();

const emit = defineEmits<{
  accept: [fieldId: string, side: MergeSide, field?: MergeField];
}>();

const collapsed = ref(false);
const hasChildren = computed(() => props.field.children.length > 0);
const displayBaseLabel = computed(() => sharedBaseLabel());
const compactBase = computed(() => compactBaseLabel());
const displayLeftLabel = computed(() => humanizeMergeSideLabel(props.leftLabel, "left"));
const displayRightLabel = computed(() => humanizeMergeSideLabel(props.rightLabel, "right"));
const compactLeftLabel = computed(() => compactMergeSideLabel(props.leftLabel, "left"));
const compactRightLabel = computed(() => compactMergeSideLabel(props.rightLabel, "right"));

const VECTOR_LABELS = new Set(["x", "y", "z", "w", "r", "g", "b", "a"]);

function parseCompoundString(val: string | undefined): CompactTuple[] | null {
  if (!val) return null;
  const trimmed = val.trim();
  const inner = trimmed.match(/^\{(.+)\}$/)?.[1];
  if (!inner) return null;
  const pairs = inner.split(",").map((part) => part.trim());
  if (pairs.length < 2 || pairs.length > 4) return null;

  const result: CompactTuple[] = [];
  for (const pair of pairs) {
    const match = pair.match(/^(\w+):\s*(.+)$/);
    if (!match) return null;
    result.push({ label: match[1].toUpperCase(), value: match[2].trim() });
  }
  return result;
}

function fmtNum(val: string | undefined): string {
  if (!val || val === "") return "-";
  const num = Number.parseFloat(val);
  if (Number.isNaN(num)) return val;
  return Number.isInteger(num)
    ? String(num)
    : num.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
}

function detectMergeVector(field: MergeField): VectorMergeComponent[] | null {
  const children = field.children;
  if (children.length >= 2 && children.length <= 4) {
    const allVectorChildren = children.every(
      (child) => child.children.length === 0 && VECTOR_LABELS.has(child.label.toLowerCase()),
    );
    if (allVectorChildren) {
      return children.map((child) => ({
        label: child.label.toUpperCase(),
        base: child.base ?? "",
        ours: child.ours ?? "",
        theirs: child.theirs ?? "",
      }));
    }
  }

  if (children.length === 0) {
    const sample = field.ours ?? field.theirs ?? field.base;
    const parsed = parseCompoundString(sample);
    if (!parsed) return null;

    const baseParsed = parseCompoundString(field.base);
    const oursParsed = parseCompoundString(field.ours);
    const theirsParsed = parseCompoundString(field.theirs);

    return parsed.map((part, index) => ({
      label: part.label,
      base: baseParsed?.[index]?.value ?? "",
      ours: oursParsed?.[index]?.value ?? "",
      theirs: theirsParsed?.[index]?.value ?? "",
    }));
  }

  return null;
}

const vectorComponents = computed(() => detectMergeVector(props.field));
const isVector = computed(() => vectorComponents.value !== null);

function toggle() {
  if (hasChildren.value && !isVector.value) {
    collapsed.value = !collapsed.value;
  }
}

function accept(side: MergeSide) {
  emit("accept", props.field.id, side, props.field);
}

function sideLabel(side: MergeSide, compact = false): string {
  switch (side) {
    case "base":
      return compact ? compactBase.value : displayBaseLabel.value;
    case "ours":
      return compact ? compactLeftLabel.value : displayLeftLabel.value;
    case "theirs":
      return compact ? compactRightLabel.value : displayRightLabel.value;
  }
}

const sourceColumns = computed<SourceColumn[]>(() => {
  const columns: SourceColumn[] = [];
  if (props.showSharedBase) {
    columns.push({
      side: "base",
      compactLabel: compactBase.value,
      fullLabel: displayBaseLabel.value,
    });
  }
  columns.push(
    {
      side: "ours",
      compactLabel: compactLeftLabel.value,
      fullLabel: displayLeftLabel.value,
    },
    {
      side: "theirs",
      compactLabel: compactRightLabel.value,
      fullLabel: displayRightLabel.value,
    },
  );
  return columns;
});

function collectChoiceStats(field: MergeField): FieldChoiceStats {
  const stats: FieldChoiceStats = {
    leafCount: 0,
    resolvedLeafCount: 0,
    conflictTotal: 0,
    resolvedConflictCount: 0,
    uniqueLeafSides: new Set<MergeSide>(),
  };

  const walk = (node: MergeField) => {
    if (node.children.length === 0) {
      stats.leafCount += 1;
      const chosenSide = props.resolutionMap?.get(node.id);
      if (chosenSide) {
        stats.resolvedLeafCount += 1;
        stats.uniqueLeafSides.add(chosenSide);
      }
      if (node.mergeState === "conflict") {
        stats.conflictTotal += 1;
        if (chosenSide) stats.resolvedConflictCount += 1;
      }
      return;
    }

    for (const child of node.children) {
      walk(child);
    }
  };

  walk(field);
  return stats;
}

const choiceStats = computed(() => collectChoiceStats(props.field));

const directSelectedSide = computed<MergeSide | null>(
  () => props.resolutionMap?.get(props.field.id) ?? props.field.manualChoice ?? null,
);

const effectiveSelectedSide = computed<MergeSelectionState>(() => {
  if (!hasChildren.value || isVector.value) {
    return directSelectedSide.value ?? props.field.autoChoice ?? (
      props.field.mergeState === "unchanged" ? "base" : null
    );
  }

  if (choiceStats.value.resolvedLeafCount === 0) {
    return null;
  }

  if (
    choiceStats.value.resolvedLeafCount < choiceStats.value.leafCount
    || choiceStats.value.uniqueLeafSides.size > 1
  ) {
    return "mixed";
  }

  return Array.from(choiceStats.value.uniqueLeafSides)[0] ?? null;
});

const fieldStatus = computed(() => {
  const selectedSide = effectiveSelectedSide.value;
  const hasConflict = props.field.mergeState === "conflict" || choiceStats.value.conflictTotal > 0;

  if (hasConflict) {
    if (selectedSide === "mixed") {
      return { label: t("merge.fields.partialChoice"), tone: "mixed" };
    }
    if (selectedSide) {
      return { label: t("merge.fields.selectedFrom", sideLabel(selectedSide)), tone: "selected" };
    }
    return { label: t("merge.fields.pendingChoice"), tone: "pending" };
  }

  if (props.field.mergeState === "auto" && props.field.autoChoice) {
    return { label: t("merge.fields.selectedFrom", sideLabel(props.field.autoChoice)), tone: "auto" };
  }

  if (props.field.mergeState === "auto") {
    return { label: mergeStatusLabel("autoResolved"), tone: "auto" };
  }

  return { label: mergeStatusLabel("unchanged"), tone: "unchanged" };
});

function rawValueForSide(side: MergeSide): string | undefined {
  switch (side) {
    case "base":
      return props.field.base;
    case "ours":
      return props.field.ours;
    case "theirs":
      return props.field.theirs;
  }
}

function parsedValueForSide(side: MergeSide): ParsedDisplayValue {
  return parseDisplayValue(rawValueForSide(side), props.field.valueType);
}

function vectorValue(side: MergeSide, component: VectorMergeComponent): string {
  switch (side) {
    case "base":
      return component.base;
    case "ours":
      return component.ours;
    case "theirs":
      return component.theirs;
  }
}

function isSourceSelected(side: MergeSide): boolean {
  return effectiveSelectedSide.value === side;
}

function sourceTitle(column: SourceColumn): string {
  return t("merge.fields.useSource", column.fullLabel, props.field.label);
}

function sourceSummaryPrimary(side: MergeSide): string {
  if (hasChildren.value && !isVector.value) {
    return t("merge.fields.childFieldScope", choiceStats.value.leafCount);
  }

  if (isVector.value && vectorComponents.value) {
    return vectorComponents.value
      .map((component) => `${component.label}:${fmtNum(vectorValue(side, component))}`)
      .join("  ");
  }

  return parsedValueForSide(side).primary;
}

function sourceSummarySecondary(side: MergeSide): string | null {
  if (hasChildren.value && !isVector.value) {
    if (choiceStats.value.conflictTotal <= 0) return null;
    return t(
      "merge.fields.selectionProgress",
      choiceStats.value.resolvedConflictCount,
      choiceStats.value.conflictTotal,
    );
  }

  if (isVector.value) return null;
  return parsedValueForSide(side).secondary ?? null;
}
</script>

<template>
  <div class="merge-field-node">
    <div
      class="merge-field-grid"
      :class="[
        field.mergeState,
        {
          group: hasChildren && !isVector,
          collapsed,
          pending: choiceStats.conflictTotal > choiceStats.resolvedConflictCount,
        },
      ]"
      :style="{ '--depth': String(depth ?? 0) }"
      :data-field-id="field.id"
    >
      <div class="merge-field-label-cell">
        <button
          v-if="hasChildren && !isVector"
          type="button"
          class="merge-field-toggle"
          :aria-expanded="!collapsed"
          :aria-label="collapsed ? t('merge.fields.toggleExpand', field.label) : t('merge.fields.toggleCollapse', field.label)"
          @click="toggle"
        >
          {{ collapsed ? "+" : "-" }}
        </button>
        <span v-else class="merge-field-fold-spacer" />

        <span class="merge-field-label-main">
          <span class="merge-field-title-row">
            <span class="merge-field-label">{{ field.label }}</span>
            <span v-if="field.fieldType" class="merge-field-type">{{ field.fieldType }}</span>
            <span class="merge-field-status-chip" :class="fieldStatus.tone">{{ fieldStatus.label }}</span>
          </span>
        </span>
      </div>

      <div
        v-for="source in sourceColumns"
        :key="`${field.id}:${source.side}`"
        role="button"
        tabindex="0"
        class="merge-field-source-btn"
        :class="[`side-${source.side}`, { selected: isSourceSelected(source.side) }]"
        :aria-pressed="isSourceSelected(source.side)"
        :title="sourceTitle(source)"
        @click.stop="accept(source.side)"
        @keydown.enter.prevent.stop="accept(source.side)"
        @keydown.space.prevent.stop="accept(source.side)"
      >
        <span class="merge-field-source-value">
          <span class="value-primary">{{ sourceSummaryPrimary(source.side) }}</span>
          <span v-if="sourceSummarySecondary(source.side)" class="value-secondary">{{ sourceSummarySecondary(source.side) }}</span>
        </span>
        <span class="merge-field-source-picked" aria-hidden="true" />
      </div>
    </div>

    <div v-if="hasChildren && !isVector && !collapsed" class="merge-field-children">
      <MergeInspectorFieldTree
        v-for="child in field.children"
        :key="child.id"
        :field="child"
        :depth="(depth ?? 0) + 1"
        :left-label="leftLabel"
        :right-label="rightLabel"
        :show-shared-base="showSharedBase"
        :resolution-map="resolutionMap"
        @accept="(fieldId, side, fieldDef) => emit('accept', fieldId, side, fieldDef)"
      />
    </div>
  </div>
</template>

<style scoped>
.merge-field-node {
  display: flex;
  flex-direction: column;
}

.merge-field-grid {
  position: relative;
  display: grid;
  grid-template-columns: var(--merge-field-columns, minmax(220px, 1.35fr) repeat(2, minmax(170px, 1fr)));
  gap: 6px;
  align-items: center;
  padding: 5px 10px 5px calc(10px + var(--depth) * 12px);
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 76%, transparent);
  background: color-mix(in srgb, var(--bg-color) 99%, transparent);
}

.merge-field-grid.group {
  padding-top: 6px;
  padding-bottom: 6px;
}

.merge-field-grid.conflict {
  background: rgba(210, 155, 0, 0.05);
}

.merge-field-grid.conflict::before,
.merge-field-grid.auto::before {
  content: "";
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.merge-field-grid.conflict::before {
  background: #d69e2e;
}

.merge-field-grid.auto {
  background: rgba(46, 160, 67, 0.03);
}

.merge-field-grid.auto::before {
  background: #38a169;
}

.merge-field-grid.pending {
  box-shadow: inset 0 0 0 1px rgba(210, 155, 0, 0.08);
}

.merge-field-label-cell {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

.merge-field-label-main {
  min-width: 0;
  display: flex;
  align-items: center;
  min-height: 30px;
}

.merge-field-title-row {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  flex-wrap: wrap;
  width: 100%;
}

.merge-field-toggle {
  width: 16px;
  height: 16px;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10px;
  line-height: 1;
  cursor: pointer;
}

.merge-field-toggle:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
  background: var(--hover-bg);
}

.merge-field-toggle:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: 1px;
}

.merge-field-fold-spacer {
  width: 16px;
  flex-shrink: 0;
}

.merge-field-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 11.5px;
  font-weight: 700;
}

.merge-field-type {
  color: var(--text-secondary);
  font-size: 9px;
  line-height: 14px;
  padding: 0 5px;
  border: 1px solid var(--border-color);
  border-radius: var(--radius-badge);
}

.merge-field-status-chip {
  display: inline-flex;
  align-items: center;
  width: fit-content;
  padding: 0 6px;
  border-radius: var(--radius-badge);
  font-size: 9px;
  font-weight: 700;
  letter-spacing: 0.02em;
  line-height: 15px;
  white-space: nowrap;
}

.merge-field-status-chip.pending {
  color: #b87400;
  background: rgba(214, 144, 25, 0.16);
}

.merge-field-status-chip.selected {
  color: #3558d8;
  background: rgba(57, 97, 255, 0.12);
}

.merge-field-status-chip.mixed {
  color: #7c3aed;
  background: rgba(124, 58, 237, 0.12);
}

.merge-field-status-chip.auto {
  color: #2f855a;
  background: rgba(46, 160, 67, 0.12);
}

.merge-field-status-chip.unchanged {
  color: #526075;
  background: rgba(100, 116, 139, 0.12);
}

.merge-field-source-btn {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 6px;
  min-height: 30px;
  padding: 4px 8px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--bg-color) 96%, var(--hover-bg));
  cursor: pointer;
  pointer-events: auto;
  transition: border-color 0.12s ease, background 0.12s ease, box-shadow 0.12s ease;
}

.merge-field-source-btn:hover {
  border-color: color-mix(in srgb, var(--text-secondary) 38%, var(--border-color));
}

.merge-field-source-btn:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: 1px;
}

.merge-field-source-btn.selected {
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 18%, transparent);
}

.merge-field-source-btn.side-base.selected {
  color: #526075;
  border-color: rgba(100, 116, 139, 0.26);
  background: rgba(100, 116, 139, 0.11);
}

.merge-field-source-btn.side-ours.selected {
  color: #3558d8;
  border-color: rgba(57, 97, 255, 0.3);
  background: rgba(57, 97, 255, 0.1);
}

.merge-field-source-btn.side-theirs.selected {
  color: #b87400;
  border-color: rgba(214, 144, 25, 0.32);
  background: rgba(214, 144, 25, 0.12);
}

.merge-field-source-value {
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  flex-wrap: wrap;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
  font-size: 10.5px;
  line-height: 1.2;
}

.value-primary,
.value-secondary {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.value-primary {
  color: var(--text-color);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

.value-secondary {
  color: var(--text-secondary);
  opacity: 0.86;
}

.merge-field-source-picked {
  width: 9px;
  height: 9px;
  flex-shrink: 0;
  border-radius: 999px;
  border: 1px solid transparent;
  background: transparent;
}

.merge-field-source-btn:hover .merge-field-source-picked {
  border-color: color-mix(in srgb, var(--text-secondary) 55%, transparent);
}

.merge-field-source-btn.selected .merge-field-source-picked {
  border-color: currentColor;
  background: currentColor;
}

.merge-field-children {
  display: flex;
  flex-direction: column;
}
</style>
