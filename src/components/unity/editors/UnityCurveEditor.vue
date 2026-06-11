<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { t } from "../../../i18n";
import {
  createAnimationFrameResizeObserver,
  type ResizeObserverHandle,
} from "../../../composables/resizeObserver";
import type { UnityAnimationCurveValue } from "../unitySerializedValue";
import {
  applyCurveTangentMode,
  clampCurveKeyTime,
  curveEditorStateFromValue,
  curveValuePayload,
  evaluateUnityCurve,
  makeCurveKey,
  refreshManagedTangentsAround,
  sampleUnityCurve,
  UNITY_CURVE_PRESETS,
  type UnityCurveEditorKey,
  type UnityCurveEditorState,
  type UnityCurveTangentMode,
} from "./curveMath";

const props = withDefaults(defineProps<{
  value: UnityAnimationCurveValue | null;
  readonly?: boolean;
}>(), {
  readonly: false,
});

const emit = defineEmits<{
  change: [payload: Record<string, unknown>];
}>();

const PLOT_PADDING = 14;
const CURVE_SAMPLES = 180;
const TIME_TICK_TARGET_PX = 76;
const VALUE_TICK_TARGET_PX = 46;
/** Margin (fraction of span) kept between a chased point and the view edge. */
const DOMAIN_EXPAND_MARGIN = 0.04;

const state = ref<UnityCurveEditorState>(curveEditorStateFromValue(props.value));
const selectedIndex = ref(0);
const plotEl = ref<SVGSVGElement | null>(null);
// Measured CSS size of the plot; the viewBox mirrors it so SVG units map
// 1:1 to pixels and resizing never distorts circles, strokes, or text.
const plotWidth = ref(640);
const plotHeight = ref(340);
const keyDrag = shallowRef<{
  pointerId: number;
  index: number;
} | null>(null);
let plotResizeObserver: ResizeObserverHandle | null = null;

interface PlotDomain {
  timeMin: number;
  timeMax: number;
  valueMin: number;
  valueMax: number;
}

interface PlotTick {
  position: number;
  label: string;
}

function fitDomainForKeys(all: readonly UnityCurveEditorKey[]): PlotDomain {
  let timeMin = Math.min(...all.map((key) => key.time));
  let timeMax = Math.max(...all.map((key) => key.time));
  if (!Number.isFinite(timeMin) || !Number.isFinite(timeMax)) {
    timeMin = 0;
    timeMax = 1;
  }
  if (timeMax - timeMin < 1e-3) timeMax = timeMin + 1;
  const sampled = sampleUnityCurve(all, timeMin, timeMax, 60).map((point) => point.value);
  let valueMin = Math.min(...sampled, ...all.map((key) => key.value));
  let valueMax = Math.max(...sampled, ...all.map((key) => key.value));
  if (!Number.isFinite(valueMin) || !Number.isFinite(valueMax)) {
    valueMin = 0;
    valueMax = 1;
  }
  if (valueMax - valueMin < 1e-3) {
    valueMin -= 0.5;
    valueMax += 0.5;
  }
  const timePad = (timeMax - timeMin) * 0.06;
  const valuePad = (valueMax - valueMin) * 0.12;
  return {
    timeMin: timeMin - timePad,
    timeMax: timeMax + timePad,
    valueMin: valueMin - valuePad,
    valueMax: valueMax + valuePad,
  };
}

// The view domain is persistent: it frames the content when a value loads,
// when a preset replaces the keys, or via the fit button — never as a side
// effect of editing, so the value space stays put while keys move.
const viewDomain = ref<PlotDomain>(fitDomainForKeys(state.value.keys));

function fitView() {
  viewDomain.value = fitDomainForKeys(state.value.keys);
}

/** Grows the view (never shrinks) so an edited key cannot escape the plot. */
function expandViewToInclude(time: number, value: number) {
  const domain = viewDomain.value;
  const timeMargin = (domain.timeMax - domain.timeMin) * DOMAIN_EXPAND_MARGIN;
  const valueMargin = (domain.valueMax - domain.valueMin) * DOMAIN_EXPAND_MARGIN;
  let { timeMin, timeMax, valueMin, valueMax } = domain;
  if (time < timeMin) timeMin = time - timeMargin;
  if (time > timeMax) timeMax = time + timeMargin;
  if (value < valueMin) valueMin = value - valueMargin;
  if (value > valueMax) valueMax = value + valueMargin;
  if (
    timeMin !== domain.timeMin
    || timeMax !== domain.timeMax
    || valueMin !== domain.valueMin
    || valueMax !== domain.valueMax
  ) {
    viewDomain.value = { timeMin, timeMax, valueMin, valueMax };
  }
}

watch(
  () => props.value,
  (value) => {
    state.value = curveEditorStateFromValue(value);
    selectedIndex.value = Math.min(selectedIndex.value, state.value.keys.length - 1);
    fitView();
  },
);

const editable = computed(() => !props.readonly);
const keys = computed(() => state.value.keys);
const selectedKey = computed(() => keys.value[selectedIndex.value] ?? null);

function plotX(time: number): number {
  const domain = viewDomain.value;
  const span = domain.timeMax - domain.timeMin || 1;
  return PLOT_PADDING + ((time - domain.timeMin) / span) * (plotWidth.value - PLOT_PADDING * 2);
}

function plotY(value: number): number {
  const domain = viewDomain.value;
  const span = domain.valueMax - domain.valueMin || 1;
  return plotHeight.value - PLOT_PADDING
    - ((value - domain.valueMin) / span) * (plotHeight.value - PLOT_PADDING * 2);
}

function domainFromClient(clientX: number, clientY: number): { time: number; value: number } {
  const rect = plotEl.value?.getBoundingClientRect();
  const domain = viewDomain.value;
  if (!rect || rect.width <= 0 || rect.height <= 0) {
    return { time: domain.timeMin, value: domain.valueMin };
  }
  const px = ((clientX - rect.left) / rect.width) * plotWidth.value;
  const py = ((clientY - rect.top) / rect.height) * plotHeight.value;
  const innerWidth = Math.max(1, plotWidth.value - PLOT_PADDING * 2);
  const innerHeight = Math.max(1, plotHeight.value - PLOT_PADDING * 2);
  // Intentionally unclamped: dragging past an edge extrapolates beyond the
  // domain and expandViewToInclude() chases the pointer by growing the view.
  const time = domain.timeMin
    + ((px - PLOT_PADDING) / innerWidth) * (domain.timeMax - domain.timeMin);
  const value = domain.valueMin
    + ((innerHeight - (py - PLOT_PADDING)) / innerHeight) * (domain.valueMax - domain.valueMin);
  return { time, value };
}

const curvePath = computed(() => {
  const domain = viewDomain.value;
  const points = sampleUnityCurve(keys.value, domain.timeMin, domain.timeMax, CURVE_SAMPLES);
  if (!points.length) return "";
  return points
    .map((point, index) => `${index === 0 ? "M" : "L"}${plotX(point.time).toFixed(2)},${plotY(point.value).toFixed(2)}`)
    .join(" ");
});

/** Largest of 1/2/2.5/5 × 10^n that still yields at most maxTicks ticks. */
function niceTickStep(span: number, maxTicks: number): number {
  if (!(span > 0) || !(maxTicks > 0)) return 1;
  const rough = span / maxTicks;
  const magnitude = 10 ** Math.floor(Math.log10(rough));
  for (const multiple of [1, 2, 2.5, 5, 10]) {
    const step = magnitude * multiple;
    if (step >= rough) return step;
  }
  return magnitude * 10;
}

function tickValues(min: number, max: number, step: number): number[] {
  if (!(step > 0) || !(max > min)) return [];
  const values: number[] = [];
  const first = Math.ceil(min / step - 1e-6);
  const last = Math.floor(max / step + 1e-6);
  for (let index = first; index <= last && values.length < 100; index += 1) {
    values.push(index * step);
  }
  return values;
}

function tickLabel(value: number): string {
  const normalized = Number(value.toFixed(6));
  return String(Math.abs(normalized) < 1e-9 ? 0 : normalized);
}

const timeTicks = computed<PlotTick[]>(() => {
  const domain = viewDomain.value;
  const inner = plotWidth.value - PLOT_PADDING * 2;
  if (inner <= 0) return [];
  const step = niceTickStep(
    domain.timeMax - domain.timeMin,
    Math.max(2, Math.floor(inner / TIME_TICK_TARGET_PX)),
  );
  return tickValues(domain.timeMin, domain.timeMax, step).map((value) => ({
    position: plotX(value),
    label: tickLabel(value),
  }));
});

const valueTicks = computed<PlotTick[]>(() => {
  const domain = viewDomain.value;
  const inner = plotHeight.value - PLOT_PADDING * 2;
  if (inner <= 0) return [];
  const step = niceTickStep(
    domain.valueMax - domain.valueMin,
    Math.max(2, Math.floor(inner / VALUE_TICK_TARGET_PX)),
  );
  return tickValues(domain.valueMin, domain.valueMax, step).map((value) => ({
    position: plotY(value),
    label: tickLabel(value),
  }));
});

const zeroAxes = computed(() => {
  const domain = viewDomain.value;
  const axes: Array<{ x1: number; y1: number; x2: number; y2: number }> = [];
  if (domain.timeMin < 0 && domain.timeMax > 0) {
    const x = plotX(0);
    axes.push({ x1: x, y1: PLOT_PADDING, x2: x, y2: plotHeight.value - PLOT_PADDING });
  }
  if (domain.valueMin < 0 && domain.valueMax > 0) {
    const y = plotY(0);
    axes.push({ x1: PLOT_PADDING, y1: y, x2: plotWidth.value - PLOT_PADDING, y2: y });
  }
  return axes;
});

const tangentModes: Array<{ id: UnityCurveTangentMode; label: string }> = [
  { id: "auto", label: t("unity.valueEditor.curve.tangent.auto") },
  { id: "linear", label: t("unity.valueEditor.curve.tangent.linear") },
  { id: "flat", label: t("unity.valueEditor.curve.tangent.flat") },
  { id: "constant", label: t("unity.valueEditor.curve.tangent.constant") },
];

const wrapModes = ["ClampForever", "Once", "Loop", "PingPong"];

function emitChange() {
  emit("change", curveValuePayload(state.value));
}

function formatNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  const rounded = Math.abs(value) >= 100 ? value.toFixed(0) : value.toFixed(2);
  return String(Number(rounded));
}

function selectKey(index: number) {
  selectedIndex.value = index;
}

function startKeyDrag(index: number, event: PointerEvent) {
  selectKey(index);
  if (!editable.value || event.button !== 0) return;
  event.preventDefault();
  event.stopPropagation();
  (event.currentTarget as Element | null)?.setPointerCapture?.(event.pointerId);
  keyDrag.value = {
    pointerId: event.pointerId,
    index,
  };
  window.addEventListener("pointermove", handleKeyDragMove);
  window.addEventListener("pointerup", stopKeyDrag);
  window.addEventListener("pointercancel", stopKeyDrag);
}

function handleKeyDragMove(event: PointerEvent) {
  const drag = keyDrag.value;
  if (!drag || event.pointerId !== drag.pointerId) return;
  event.preventDefault();
  const { time, value } = domainFromClient(event.clientX, event.clientY);
  const list = state.value.keys;
  const key = list[drag.index];
  if (!key) return;
  key.time = clampCurveKeyTime(list, drag.index, time);
  key.value = value;
  expandViewToInclude(key.time, key.value);
  refreshManagedTangentsAround(list, drag.index);
  emitChange();
}

function stopKeyDrag(event?: PointerEvent) {
  const drag = keyDrag.value;
  if (drag && event && event.pointerId !== drag.pointerId) return;
  keyDrag.value = null;
  window.removeEventListener("pointermove", handleKeyDragMove);
  window.removeEventListener("pointerup", stopKeyDrag);
  window.removeEventListener("pointercancel", stopKeyDrag);
}

function addKeyAt(event: MouseEvent) {
  if (!editable.value) return;
  const { time, value } = domainFromClient(event.clientX, event.clientY);
  const list = state.value.keys;
  const key = makeCurveKey(time, value, "auto");
  let index = list.findIndex((existing) => existing.time > time);
  if (index < 0) index = list.length;
  list.splice(index, 0, key);
  expandViewToInclude(key.time, key.value);
  refreshManagedTangentsAround(list, index);
  selectedIndex.value = index;
  emitChange();
}

function removeSelectedKey() {
  const list = state.value.keys;
  if (!editable.value || list.length <= 1) return;
  list.splice(selectedIndex.value, 1);
  selectedIndex.value = Math.max(0, Math.min(selectedIndex.value, list.length - 1));
  refreshManagedTangentsAround(list, selectedIndex.value);
  emitChange();
}

function handlePlotKeydown(event: KeyboardEvent) {
  if ((event.key === "Delete" || event.key === "Backspace") && editable.value) {
    event.preventDefault();
    removeSelectedKey();
  }
}

function commitSelectedTime(event: Event) {
  const key = selectedKey.value;
  if (!key || !editable.value) return;
  const numeric = Number((event.target as HTMLInputElement | null)?.value ?? "");
  if (!Number.isFinite(numeric)) return;
  key.time = clampCurveKeyTime(state.value.keys, selectedIndex.value, numeric);
  expandViewToInclude(key.time, key.value);
  refreshManagedTangentsAround(state.value.keys, selectedIndex.value);
  emitChange();
}

function commitSelectedValue(event: Event) {
  const key = selectedKey.value;
  if (!key || !editable.value) return;
  const numeric = Number((event.target as HTMLInputElement | null)?.value ?? "");
  if (!Number.isFinite(numeric)) return;
  key.value = numeric;
  expandViewToInclude(key.time, key.value);
  refreshManagedTangentsAround(state.value.keys, selectedIndex.value);
  emitChange();
}

function setSelectedTangentMode(mode: UnityCurveTangentMode) {
  if (!editable.value || !selectedKey.value) return;
  applyCurveTangentMode(state.value.keys, selectedIndex.value, mode);
  emitChange();
}

function applyPreset(presetId: string) {
  if (!editable.value) return;
  const preset = UNITY_CURVE_PRESETS.find((entry) => entry.id === presetId);
  if (!preset) return;
  state.value = {
    ...state.value,
    keys: preset.build(),
  };
  selectedIndex.value = 0;
  fitView();
  emitChange();
}

function setWrapMode(side: "preWrapMode" | "postWrapMode", event: Event) {
  if (!editable.value) return;
  const value = (event.target as HTMLSelectElement | null)?.value || "ClampForever";
  state.value[side] = value;
  emitChange();
}

function keyTitle(index: number): string {
  const key = keys.value[index];
  if (!key) return "";
  return `t=${formatNumber(key.time)} v=${formatNumber(key.value)}`;
}

function selectedValueAt(time: number): string {
  return formatNumber(evaluateUnityCurve(keys.value, time));
}

onMounted(() => {
  plotResizeObserver = createAnimationFrameResizeObserver((entries) => {
    const rect = entries[entries.length - 1]?.contentRect;
    if (!rect || rect.width <= 0 || rect.height <= 0) return;
    plotWidth.value = rect.width;
    plotHeight.value = rect.height;
  });
  if (plotEl.value) plotResizeObserver?.observe(plotEl.value);
});

onBeforeUnmount(() => {
  stopKeyDrag();
  plotResizeObserver?.disconnect();
  plotResizeObserver = null;
});
</script>

<template>
  <div class="unity-curve-editor">
    <svg
      ref="plotEl"
      class="unity-curve-plot"
      :viewBox="`0 0 ${plotWidth} ${plotHeight}`"
      tabindex="0"
      role="application"
      :aria-label="t('unity.valueEditor.curve.plotLabel')"
      @dblclick="addKeyAt"
      @keydown="handlePlotKeydown"
    >
      <line
        v-for="(tick, index) in timeTicks"
        :key="`grid-time-${index}`"
        class="unity-curve-grid"
        :x1="tick.position"
        :y1="PLOT_PADDING"
        :x2="tick.position"
        :y2="plotHeight - PLOT_PADDING"
      />
      <line
        v-for="(tick, index) in valueTicks"
        :key="`grid-value-${index}`"
        class="unity-curve-grid"
        :x1="PLOT_PADDING"
        :y1="tick.position"
        :x2="plotWidth - PLOT_PADDING"
        :y2="tick.position"
      />
      <line
        v-for="(axis, index) in zeroAxes"
        :key="`axis-${index}`"
        class="unity-curve-zero-axis"
        :x1="axis.x1"
        :y1="axis.y1"
        :x2="axis.x2"
        :y2="axis.y2"
      />
      <text
        v-for="(tick, index) in timeTicks"
        :key="`label-time-${index}`"
        class="unity-curve-tick-label"
        text-anchor="middle"
        :x="tick.position"
        :y="plotHeight - 3.5"
      >{{ tick.label }}</text>
      <text
        v-for="(tick, index) in valueTicks"
        :key="`label-value-${index}`"
        class="unity-curve-tick-label"
        :x="PLOT_PADDING + 4"
        :y="tick.position - 3"
      >{{ tick.label }}</text>
      <path class="unity-curve-line" :d="curvePath" fill="none" />
      <circle
        v-for="(key, index) in keys"
        :key="`key-${index}`"
        class="unity-curve-key"
        :class="{ selected: index === selectedIndex }"
        :cx="plotX(key.time)"
        :cy="plotY(key.value)"
        r="5"
        :data-title="keyTitle(index)"
        @pointerdown="startKeyDrag(index, $event)"
      />
    </svg>
    <div class="unity-curve-axis">
      <span class="unity-curve-axis-hint">{{ t("unity.valueEditor.curve.hint") }}</span>
      <button
        type="button"
        class="unity-curve-fit"
        :title="t('unity.valueEditor.curve.fitTitle')"
        @click="fitView"
      >
        {{ t("unity.valueEditor.curve.fit") }}
      </button>
    </div>

    <div class="unity-curve-selected" v-if="selectedKey">
      <label class="unity-curve-field">
        <span>{{ t("unity.valueEditor.curve.time") }}</span>
        <input
          type="text"
          inputmode="decimal"
          :value="formatNumber(selectedKey.time)"
          :disabled="!editable"
          @change="commitSelectedTime"
          @keydown.enter.prevent="($event.target as HTMLElement | null)?.blur()"
        />
      </label>
      <label class="unity-curve-field">
        <span>{{ t("unity.valueEditor.curve.value") }}</span>
        <input
          type="text"
          inputmode="decimal"
          :value="formatNumber(selectedKey.value)"
          :disabled="!editable"
          @change="commitSelectedValue"
          @keydown.enter.prevent="($event.target as HTMLElement | null)?.blur()"
        />
      </label>
      <div class="unity-curve-tangents" role="group" :aria-label="t('unity.valueEditor.curve.tangentMode')">
        <button
          v-for="mode in tangentModes"
          :key="mode.id"
          type="button"
          class="unity-curve-tangent-button"
          :class="{ active: selectedKey.mode === mode.id }"
          :disabled="!editable"
          @click="setSelectedTangentMode(mode.id)"
        >
          {{ mode.label }}
        </button>
      </div>
      <button
        type="button"
        class="unity-curve-remove"
        :disabled="!editable || keys.length <= 1"
        @click="removeSelectedKey"
      >
        {{ t("unity.valueEditor.curve.deleteKey") }}
      </button>
    </div>

    <div class="unity-curve-footer">
      <div class="unity-curve-presets" role="group" :aria-label="t('unity.valueEditor.curve.presets')">
        <span class="unity-curve-footer-label">{{ t("unity.valueEditor.curve.presets") }}</span>
        <button
          v-for="preset in UNITY_CURVE_PRESETS"
          :key="preset.id"
          type="button"
          class="unity-curve-preset-button"
          :disabled="!editable"
          @click="applyPreset(preset.id)"
        >
          {{ preset.label }}
        </button>
      </div>
      <div class="unity-curve-wraps">
        <label class="unity-curve-wrap">
          <span>{{ t("unity.valueEditor.curve.preWrap") }}</span>
          <select :value="state.preWrapMode" :disabled="!editable" @change="setWrapMode('preWrapMode', $event)">
            <option v-for="mode in wrapModes" :key="mode" :value="mode">{{ mode }}</option>
          </select>
        </label>
        <label class="unity-curve-wrap">
          <span>{{ t("unity.valueEditor.curve.postWrap") }}</span>
          <select :value="state.postWrapMode" :disabled="!editable" @change="setWrapMode('postWrapMode', $event)">
            <option v-for="mode in wrapModes" :key="mode" :value="mode">{{ mode }}</option>
          </select>
        </label>
        <span class="unity-curve-eval" :title="t('unity.valueEditor.curve.valueAtOne')">
          f(1) = {{ selectedValueAt(1) }}
        </span>
      </div>
    </div>
  </div>
</template>

<style scoped>
.unity-curve-editor {
  min-width: 0;
  min-height: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.unity-curve-plot {
  width: 100%;
  flex: 1;
  min-height: 220px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--sidebar-bg) 12%);
  touch-action: none;
}

.unity-curve-plot:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 2px;
}

.unity-curve-grid {
  stroke: color-mix(in srgb, var(--border-color) 55%, transparent);
  stroke-width: 1;
}

.unity-curve-zero-axis {
  stroke: color-mix(in srgb, var(--text-secondary) 65%, transparent);
  stroke-width: 1.2;
}

.unity-curve-tick-label {
  fill: color-mix(in srgb, var(--text-secondary) 80%, transparent);
  font-family: var(--font-mono-identifier);
  font-size: 9.5px;
  pointer-events: none;
  user-select: none;
}

.unity-curve-line {
  stroke: var(--status-good-fg, var(--accent-color));
  stroke-width: 1.8;
}

.unity-curve-key {
  fill: var(--panel-bg);
  stroke: var(--text-secondary);
  stroke-width: 1.4;
  cursor: grab;
}

.unity-curve-key:hover {
  stroke: var(--text-color);
}

.unity-curve-key.selected {
  fill: var(--accent-color);
  stroke: var(--accent-color);
}

.unity-curve-axis {
  display: flex;
  align-items: center;
  justify-content: space-between;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.unity-curve-axis-hint {
  color: color-mix(in srgb, var(--text-secondary) 80%, transparent);
  font-family: inherit;
}

.unity-curve-selected {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.unity-curve-field {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-curve-field input {
  width: 86px;
  min-height: 26px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-mono-identifier);
  box-sizing: border-box;
}

.unity-curve-field input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-curve-tangents {
  display: inline-flex;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
}

.unity-curve-tangent-button {
  min-height: 26px;
  padding: 0 9px;
  border: 0;
  border-right: 1px solid var(--border-color);
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}

.unity-curve-tangent-button:last-child {
  border-right: 0;
}

.unity-curve-tangent-button:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.unity-curve-tangent-button.active {
  background: color-mix(in srgb, var(--accent-color) 18%, transparent);
  color: var(--text-color);
}

.unity-curve-remove,
.unity-curve-preset-button,
.unity-curve-fit {
  min-height: 26px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}

.unity-curve-remove:hover:not(:disabled),
.unity-curve-preset-button:hover:not(:disabled),
.unity-curve-fit:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.unity-curve-remove:disabled,
.unity-curve-preset-button:disabled,
.unity-curve-tangent-button:disabled {
  opacity: 0.55;
  cursor: default;
}

.unity-curve-footer {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.unity-curve-presets {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}

.unity-curve-footer-label {
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-curve-wraps {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.unity-curve-wrap {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-curve-wrap select {
  min-height: 26px;
  padding: 0 6px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 11px;
}

.unity-curve-eval {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}
</style>
