<script setup lang="ts">
import { computed, ref } from "vue";
import type { InspectorField } from "../../types";
import { shouldAutoCollapseField } from "./unityInspectorFieldState";
import {
  detectVectorComponents,
  formatQuaternionEulerTuple,
  formatVectorTuple,
  isFullyModifiedVector,
  isQuaternionField,
} from "./vectorDisplay";

defineOptions({ name: "UnityInspectorFieldTree" });

const props = defineProps<{
  field: InspectorField;
  depth?: number;
}>();

const hasChildren = !!props.field.children?.length;
const collapsed = ref(shouldAutoCollapseField(props.field));

function toggle() {
  if (hasChildren) collapsed.value = !collapsed.value;
}

/** Parsed value for structured rendering: asset name highlighted, path in gray */
interface ParsedValue {
  isRef: boolean;
  name: string;
  dir?: string;
}

/** Format a bool-like value (0/1) as True/False when the field is known to be bool */
function formatBoolValue(val: string): string {
  const trimmed = val.trim();
  if (trimmed === "0" || trimmed.toLowerCase() === "false") return "False";
  if (trimmed === "1" || trimmed.toLowerCase() === "true") return "True";
  return val;
}

/** Is this field a boolean? Prefer fieldType from C# source; fall back to inferred valueType only when no fieldType is available */
const isBoolField = props.field.fieldType === "bool"
  || (!props.field.fieldType && props.field.valueType === "bool" && !props.field.children?.length);

/** Stale reference: path resolved from current workspace for a snapshot side */
const isStaleRef = props.field.reference?.stale === true;
const staleRefTitle = isStaleRef
  ? "Resolved from current workspace — may not match the version at this commit"
  : undefined;

// Log unresolved GUID diagnostics once on mount (not per render)
if (props.field.reference?.guid && !props.field.reference.path) {
  const hint = props.field.reference.resolveHint ?? "unknown reason";
  console.warn(
    `[UnityInspector] Unresolved GUID: ${props.field.reference.guid}\n` +
    `  field: ${props.field.propertyPath}\n` +
    `  reason: ${hint}`
  );
}

function parseValue(val: string | undefined): ParsedValue {
  if (val == null || val === "") return { isRef: false, name: "(empty)" };

  // "fileID:0" or "(fileID:0)" → null reference
  if (/^\(?\s*fileID:\s*0\s*\)?$/.test(val.trim())) return { isRef: false, name: "None" };

  // Bool display: convert 0/1 to False/True
  if (isBoolField) return { isRef: false, name: formatBoolValue(val) };

  // Strip all (fileID:xxx) tokens
  let cleaned = val.replace(/\s*\(fileID:\d+\)/g, "").trim();

  // Asset path like "Assets/.../ParentDir/FileName.asset"
  const pathMatch = cleaned.match(/^Assets\/(.+)\/([^/]+)$/);
  if (pathMatch) {
    const segments = pathMatch[1].split("/");
    const parentDir = segments[segments.length - 1] ?? "";
    const fileName = pathMatch[2];
    const nameNoExt = fileName.replace(/\.(asset|prefab|mat|controller|anim|unity|meta)$/i, "");
    return { isRef: true, name: nameNoExt, dir: parentDir ? `${parentDir}/` : undefined };
  }

  // Bare "fileID:xxx" leftover
  if (/^fileID:\d+$/.test(cleaned)) return { isRef: false, name: "None" };

  if (cleaned === "") return { isRef: false, name: "(empty)" };
  return { isRef: false, name: cleaned };
}

function parseSingleValue(field: InspectorField): ParsedValue {
  return parseValue(field.after ?? field.before);
}

/** ── Color detection ── */
interface ParsedColor {
  r: number;
  g: number;
  b: number;
  a: number;
}

const COLOR_RE = /^\{?\s*r:\s*([\d.]+),\s*g:\s*([\d.]+),\s*b:\s*([\d.]+),\s*a:\s*([\d.]+)\s*\}?$/;

function parseColor(val: string | undefined): ParsedColor | null {
  if (!val) return null;
  const m = COLOR_RE.exec(val.trim());
  if (!m) return null;
  return { r: parseFloat(m[1]), g: parseFloat(m[2]), b: parseFloat(m[3]), a: parseFloat(m[4]) };
}

function colorToCSS(c: ParsedColor): string {
  const r = Math.round(Math.min(c.r, 1) * 255);
  const g = Math.round(Math.min(c.g, 1) * 255);
  const b = Math.round(Math.min(c.b, 1) * 255);
  return `rgba(${r}, ${g}, ${b}, ${c.a})`;
}

function colorTooltip(c: ParsedColor): string {
  return `R: ${fmt(c.r)}  G: ${fmt(c.g)}  B: ${fmt(c.b)}  A: ${fmt(c.a)}`;
}

function fmt(n: number): string {
  return Number.isInteger(n) ? n.toString() : n.toFixed(3).replace(/0+$/, "").replace(/\.$/, "");
}

/** Is this field a color value? (check both before/after) */
function fieldIsColor(field: InspectorField): boolean {
  return parseColor(field.before) !== null || parseColor(field.after) !== null;
}

function arraySizeHint(field: InspectorField): string | null {
  if (!field.children?.length) return null;
  if (field.valueType === "array" || /\[\d+\]$/.test(field.children[0]?.propertyPath ?? "")) {
    return `[${field.children.length}]`;
  }
  return null;
}

function changeLabel(kind: string): string {
  switch (kind) {
    case "added": return "A";
    case "removed": return "D";
    case "modified": return "M";
    default: return "";
  }
}

/** ── Inline vector detection ── */
const vectorComponents = hasChildren ? detectVectorComponents(props.field) : null;
const isVector = vectorComponents !== null;
const rotationDisplayMode = ref<"euler" | "quaternion">("euler");
const isQuaternionVector = vectorComponents !== null && isQuaternionField(props.field, vectorComponents);
const canShowQuaternionEuler = computed(() =>
  isQuaternionVector
  && vectorComponents !== null
  && (
    formatQuaternionEulerTuple(vectorComponents, "before", fmt) !== null
    || formatQuaternionEulerTuple(vectorComponents, "after", fmt) !== null
  ),
);

function toggleRotationDisplayMode() {
  rotationDisplayMode.value = rotationDisplayMode.value === "euler" ? "quaternion" : "euler";
}

function handleRowClick() {
  if (canShowQuaternionEuler.value) {
    toggleRotationDisplayMode();
    return;
  }
  if (!isVector) toggle();
}

function rawVectorTuple(side: "before" | "after"): string {
  return formatVectorTuple(vectorComponents ?? [], side, (value) => parseValue(value).name);
}

const compactVectorDisplay = computed<{
  kind: InspectorField["changeKind"];
  before?: string;
  after?: string;
  single?: string;
} | null>(() => {
  if (!vectorComponents) return null;

  if (isQuaternionVector && rotationDisplayMode.value === "euler") {
    const before = formatQuaternionEulerTuple(vectorComponents, "before", fmt) ?? undefined;
    const after = formatQuaternionEulerTuple(vectorComponents, "after", fmt) ?? undefined;
    if (props.field.changeKind === "modified" && before && after) {
      return { kind: "modified", before, after };
    }
    if (props.field.changeKind === "added" && after) {
      return { kind: "added", single: after };
    }
    if (props.field.changeKind === "removed" && before) {
      return { kind: "removed", single: before };
    }
    if (before || after) {
      return { kind: props.field.changeKind, single: after ?? before };
    }
    return null;
  }

  if (isFullyModifiedVector(vectorComponents)) {
    return {
      kind: "modified",
      before: rawVectorTuple("before"),
      after: rawVectorTuple("after"),
    };
  }

  return null;
});

/** Effective type hint to display: prefer C# fieldType, fall back to inferred valueType for numbers */
const displayTypeHint = (() => {
  if (props.field.fieldType) return props.field.fieldType;
  if (hasChildren) return undefined;
  if (props.field.valueType === "number") {
    // Distinguish int vs float from actual value
    const val = (props.field.after ?? props.field.before ?? "").trim();
    if (val && !val.includes(".")) return "Int";
    return "Float";
  }
  return undefined;
})();
</script>

<template>
  <div class="field-node">
    <div
      class="field-row"
      :class="[field.changeKind, { collapsible: hasChildren && !isVector, collapsed, 'mode-toggle-row': canShowQuaternionEuler }]"
      :style="{ paddingLeft: `${12 + (depth ?? 0) * 14}px` }"
      @click="handleRowClick"
    >
      <!-- Left color bar -->
      <span v-if="field.changeKind !== 'unchanged'" class="change-bar" :class="field.changeKind" />

      <!-- Collapse arrow (not for inline vectors) -->
      <span v-if="hasChildren && !isVector" class="fold-arrow" :class="{ open: !collapsed }">&#x25B6;</span>
      <span v-else class="fold-spacer" />

      <!-- Label -->
      <span class="field-label">{{ field.label }}</span>

      <!-- C# field type hint (or inferred type for numeric leaf fields) -->
      <span v-if="displayTypeHint" class="field-type-hint">{{ displayTypeHint }}</span>

      <!-- Array size hint -->
      <span v-if="arraySizeHint(field)" class="array-hint">{{ arraySizeHint(field) }}</span>

      <!-- Change badge (group rows, non-vector) -->
      <span v-if="hasChildren && !isVector && field.changeKind !== 'unchanged'" class="change-badge" :class="field.changeKind">
        {{ changeLabel(field.changeKind) }}
      </span>

      <!-- ── Inline vector display ── -->
      <template v-if="isVector">
        <span class="vector-inline">
          <template v-if="compactVectorDisplay">
            <span class="vector-compact" :class="compactVectorDisplay.kind">
              <template v-if="compactVectorDisplay.kind === 'modified'">
                <span class="val-name vec-before">{{ compactVectorDisplay.before }}</span>
                <span class="field-arrow">&rarr;</span>
                <span class="val-name highlight-mod">{{ compactVectorDisplay.after }}</span>
              </template>
              <template v-else-if="compactVectorDisplay.kind === 'added'">
                <span class="val-name highlight-add">{{ compactVectorDisplay.single }}</span>
              </template>
              <template v-else-if="compactVectorDisplay.kind === 'removed'">
                <span class="val-name strikethrough">{{ compactVectorDisplay.single }}</span>
              </template>
              <template v-else>
                <span class="val-name">{{ compactVectorDisplay.single }}</span>
              </template>
            </span>
          </template>
          <template v-else>
            <span v-for="vc in vectorComponents" :key="vc.label" class="vector-comp" :class="vc.changeKind">
              <span class="vector-label">{{ vc.label }}</span>
              <template v-if="vc.changeKind === 'unchanged'">
                <span class="val-name">{{ parseValue(vc.after ?? vc.before).name }}</span>
              </template>
              <template v-else-if="vc.changeKind === 'added'">
                <span class="val-name highlight-add">{{ parseValue(vc.after).name }}</span>
              </template>
              <template v-else-if="vc.changeKind === 'removed'">
                <span class="val-name strikethrough">{{ parseValue(vc.before).name }}</span>
              </template>
              <template v-else>
                <span class="val-name vec-before">{{ parseValue(vc.before).name }}</span>
                <span class="field-arrow">&rarr;</span>
                <span class="val-name highlight-mod">{{ parseValue(vc.after).name }}</span>
              </template>
            </span>
          </template>
          <button
            v-if="canShowQuaternionEuler"
            type="button"
            class="vector-mode-toggle"
            @click.stop="toggleRotationDisplayMode"
          >
            {{ rotationDisplayMode === 'euler' ? 'Euler' : 'Quat' }}
          </button>
        </span>
      </template>

      <!-- ── Values (leaf nodes only) ── -->
      <template v-if="!hasChildren">
        <!-- ── Color values ── -->
        <template v-if="fieldIsColor(field)">
          <!-- Unchanged color -->
          <template v-if="field.changeKind === 'unchanged'">
            <span class="value-cell single">
              <span class="color-swatch" :title="colorTooltip(parseColor(field.after ?? field.before)!)"><span class="color-fill" :style="{ background: colorToCSS(parseColor(field.after ?? field.before)!) }" /></span>
            </span>
          </template>
          <!-- Added color -->
          <template v-else-if="field.changeKind === 'added'">
            <span class="value-cell after">
              <span class="color-swatch" :title="colorTooltip(parseColor(field.after)!)"><span class="color-fill" :style="{ background: colorToCSS(parseColor(field.after)!) }" /></span>
            </span>
          </template>
          <!-- Removed color -->
          <template v-else-if="field.changeKind === 'removed'">
            <span class="value-cell before">
              <span class="color-swatch removed" :title="colorTooltip(parseColor(field.before)!)"><span class="color-fill" :style="{ background: colorToCSS(parseColor(field.before)!) }" /></span>
            </span>
          </template>
          <!-- Modified color: before → after -->
          <template v-else>
            <span class="value-cell before">
              <span class="color-swatch" :title="colorTooltip(parseColor(field.before)!)"><span class="color-fill" :style="{ background: colorToCSS(parseColor(field.before)!) }" /></span>
            </span>
            <span class="field-arrow">&rarr;</span>
            <span class="value-cell after">
              <span class="color-swatch" :title="colorTooltip(parseColor(field.after)!)"><span class="color-fill" :style="{ background: colorToCSS(parseColor(field.after)!) }" /></span>
            </span>
          </template>
        </template>

        <!-- ── Normal values ── -->
        <template v-else>
          <!-- Unchanged -->
          <template v-if="field.changeKind === 'unchanged'">
            <span class="value-cell single" :class="{ 'stale-ref': isStaleRef }" :title="staleRefTitle">
              <span class="val-name">{{ parseSingleValue(field).name }}</span>
              <span v-if="parseSingleValue(field).dir" class="val-dir">({{ parseSingleValue(field).dir }})</span>
              <span v-if="isStaleRef" class="stale-badge">?</span>
            </span>
          </template>

          <!-- Added -->
          <template v-else-if="field.changeKind === 'added'">
            <span class="value-cell after" :class="{ 'stale-ref': isStaleRef }" :title="staleRefTitle">
              <span class="val-name highlight-add">{{ parseValue(field.after).name }}</span>
              <span v-if="parseValue(field.after).dir" class="val-dir">({{ parseValue(field.after).dir }})</span>
              <span v-if="isStaleRef" class="stale-badge">?</span>
            </span>
          </template>

          <!-- Removed -->
          <template v-else-if="field.changeKind === 'removed'">
            <span class="value-cell before strikethrough" :class="{ 'stale-ref': isStaleRef }" :title="staleRefTitle">
              <span class="val-name">{{ parseValue(field.before).name }}</span>
              <span v-if="parseValue(field.before).dir" class="val-dir">({{ parseValue(field.before).dir }})</span>
              <span v-if="isStaleRef" class="stale-badge">?</span>
            </span>
          </template>

          <!-- Modified: before → after -->
          <template v-else>
            <span class="value-cell before" :class="{ 'stale-ref': isStaleRef }" :title="staleRefTitle">
              <span class="val-name">{{ parseValue(field.before).name }}</span>
              <span v-if="parseValue(field.before).dir" class="val-dir">({{ parseValue(field.before).dir }})</span>
            </span>
            <span class="field-arrow">&rarr;</span>
            <span class="value-cell after" :class="{ 'stale-ref': isStaleRef }" :title="staleRefTitle">
              <span class="val-name highlight-mod">{{ parseValue(field.after).name }}</span>
              <span v-if="parseValue(field.after).dir" class="val-dir">({{ parseValue(field.after).dir }})</span>
              <span v-if="isStaleRef" class="stale-badge">?</span>
            </span>
          </template>
        </template>
      </template>
    </div>

    <!-- Children (not rendered for inline vectors) -->
    <div v-if="hasChildren && !isVector && !collapsed" class="field-children">
      <UnityInspectorFieldTree
        v-for="child in field.children"
        :key="child.id"
        :field="child"
        :depth="(depth ?? 0) + 1"
      />
    </div>
  </div>
</template>

<style scoped>
.field-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 26px;
  padding: 3px 12px;
  padding-right: 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 56%, transparent);
  background: var(--unity-field-row-bg, color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%));
  font-size: 12.5px;
  line-height: 1.4;
}

.field-row.collapsible {
  background: var(--unity-field-group-row-bg, color-mix(in srgb, var(--panel-bg) 78%, var(--bg-color) 22%));
  cursor: pointer;
}

.field-row.collapsible:hover {
  background: var(--unity-field-row-hover-bg, color-mix(in srgb, var(--hover-bg) 64%, var(--panel-bg) 36%));
}

.field-row.mode-toggle-row {
  cursor: pointer;
}

.field-row.mode-toggle-row:hover {
  background: var(--unity-field-row-hover-bg, color-mix(in srgb, var(--hover-bg) 64%, var(--panel-bg) 36%));
}

/* ── Left color bar ── */
.change-bar {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.change-bar.added { background: var(--git-status-added); }
.change-bar.removed { background: var(--git-status-deleted); }
.change-bar.modified { background: var(--git-status-modified); }

/* ── Fold arrow ── */
.fold-arrow {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  flex-shrink: 0;
  font-size: 8px;
  color: var(--text-secondary);
  transition: transform 0.15s ease;
  transform: rotate(0deg);
}

.fold-arrow.open {
  transform: rotate(90deg);
}

.fold-spacer {
  width: 14px;
  flex-shrink: 0;
}

/* ── Label ── */
.field-label {
  flex-shrink: 0;
  font-weight: 600;
  color: var(--text-color);
  white-space: nowrap;
}

/* ── Field type hint ── */
.field-type-hint {
  font-size: 10px;
  color: var(--text-secondary);
  opacity: 0.55;
  flex-shrink: 0;
  font-style: italic;
}

/* ── Array hint ── */
.array-hint {
  font-size: 11px;
  color: var(--text-secondary);
  flex-shrink: 0;
}

/* ── Change badge ── */
.change-badge {
  flex-shrink: 0;
  padding: 0 5px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 700;
  line-height: 16px;
}

.change-badge.added {
  color: var(--git-status-added);
  background: color-mix(in srgb, var(--git-status-added) 14%, transparent);
}

.change-badge.removed {
  color: var(--git-status-deleted);
  background: color-mix(in srgb, var(--git-status-deleted) 14%, transparent);
}

.change-badge.modified {
  color: var(--git-status-modified);
  background: color-mix(in srgb, var(--git-status-modified) 14%, transparent);
}

/* ── Value cell ── */
.value-cell {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-size: 12px;
}

.value-cell.single {
  margin-left: auto;
}

.value-cell.before .val-name {
  color: var(--text-secondary);
}

.value-cell.after .val-name {
  color: var(--text-color);
}

.value-cell.strikethrough .val-name {
  text-decoration: line-through;
  opacity: 0.65;
}

.val-name.strikethrough {
  text-decoration: line-through;
  opacity: 0.65;
}

/* Highlighted value names for actual changes */
.val-name.highlight-add {
  color: var(--text-color);
  font-weight: 700;
}

.val-name.highlight-mod {
  color: color-mix(in srgb, var(--git-status-modified) 82%, var(--text-color));
  font-weight: 700;
  background: color-mix(in srgb, var(--git-status-modified) 14%, transparent);
  padding: 1px 5px;
  border-radius: 3px;
}

/* Gray directory path */
.val-dir {
  color: var(--text-secondary);
  opacity: 0.55;
  font-size: 11px;
  flex-shrink: 1;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* ── Color swatch ── */
.color-swatch {
  display: inline-flex;
  width: 34px;
  height: 18px;
  border-radius: 3px;
  border: 2px solid #888;
  outline: 1px solid rgba(0, 0, 0, 0.6);
  outline-offset: -1px;
  cursor: default;
  vertical-align: middle;
  overflow: hidden;
}

.color-fill {
  display: block;
  width: 100%;
  height: 100%;
}

.color-swatch.removed {
  opacity: 0.55;
}

.field-arrow {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-size: 11px;
}

/* ── Inline vector ── */
.vector-inline {
  display: inline-flex;
  align-items: center;
  gap: 12px;
  margin-left: auto;
  font-size: 12px;
  white-space: nowrap;
}

.vector-comp {
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.vector-compact {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.vector-label {
  font-weight: 600;
  font-size: 10px;
  color: var(--text-secondary);
  opacity: 0.7;
  min-width: 10px;
}

.vector-comp .val-name.vec-before,
.vector-compact .val-name.vec-before {
  color: var(--text-secondary);
}

.vector-comp .field-arrow {
  margin: 0 1px;
}

.vector-mode-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 38px;
  height: 18px;
  padding: 0 6px;
  border: 1px solid transparent;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10px;
  line-height: 1;
  cursor: pointer;
}

.vector-mode-toggle:hover,
.vector-mode-toggle:focus-visible {
  border-color: var(--border-color);
  background: color-mix(in srgb, var(--hover-bg) 68%, transparent);
  color: var(--text-color);
  outline: none;
}

/* ── Stale reference indicator ── */
.stale-badge {
  font-size: 10px;
  color: var(--text-secondary);
  opacity: 0.5;
  flex-shrink: 0;
  margin-left: 2px;
}

/* Push values to the right */
.field-label ~ .value-cell {
  margin-left: auto;
}

/* When there's before → after, don't auto-margin the arrow or after cell */
.field-label ~ .value-cell.before {
  margin-left: auto;
}

.field-label ~ .value-cell.before ~ .field-arrow,
.field-label ~ .value-cell.before ~ .value-cell.after {
  margin-left: 0;
}
</style>
