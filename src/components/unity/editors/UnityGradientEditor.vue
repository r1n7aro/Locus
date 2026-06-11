<script setup lang="ts">
import { computed, ref, shallowRef, watch } from "vue";
import { t } from "../../../i18n";
import type { UnityGradientValue } from "../unitySerializedValue";

const props = withDefaults(defineProps<{
  value: UnityGradientValue | null;
  readonly?: boolean;
}>(), {
  readonly: false,
});

const emit = defineEmits<{
  change: [payload: Record<string, unknown>];
}>();

interface GradientColorStop {
  time: number;
  color: string;
}

interface GradientAlphaStop {
  time: number;
  alpha: number;
}

interface GradientEditorState {
  mode: string;
  colorKeys: GradientColorStop[];
  alphaKeys: GradientAlphaStop[];
}

type StopSelection = { type: "color" | "alpha"; index: number };

const MAX_KEYS = 8;

const state = ref<GradientEditorState>(stateFromValue(props.value));
const selection = ref<StopSelection>({ type: "color", index: 0 });
const stopDrag = shallowRef<{
  pointerId: number;
  selection: StopSelection;
  trackEl: HTMLElement;
} | null>(null);

watch(
  () => props.value,
  (value) => {
    state.value = stateFromValue(value);
    selection.value = { type: "color", index: 0 };
  },
);

const editable = computed(() => !props.readonly);
const selectedColorStop = computed(() =>
  selection.value.type === "color" ? state.value.colorKeys[selection.value.index] ?? null : null,
);
const selectedAlphaStop = computed(() =>
  selection.value.type === "alpha" ? state.value.alphaKeys[selection.value.index] ?? null : null,
);

function stateFromValue(value: UnityGradientValue | null | undefined): GradientEditorState {
  const colorKeys = (value?.colorKeys ?? []).map((key) => ({
    time: clamp01(key.time),
    color: /^#[0-9a-fA-F]{6}$/.test(key.color) ? key.color.toLowerCase() : "#ffffff",
  }));
  const alphaKeys = (value?.alphaKeys ?? []).map((key) => ({
    time: clamp01(key.time),
    alpha: clamp01(key.alpha),
  }));
  if (!colorKeys.length) colorKeys.push({ time: 0, color: "#ffffff" }, { time: 1, color: "#ffffff" });
  if (!alphaKeys.length) alphaKeys.push({ time: 0, alpha: 1 }, { time: 1, alpha: 1 });
  colorKeys.sort((a, b) => a.time - b.time);
  alphaKeys.sort((a, b) => a.time - b.time);
  return {
    mode: value?.mode === "Fixed" ? "Fixed" : "Blend",
    colorKeys,
    alphaKeys,
  };
}

function payload(): Record<string, unknown> {
  return {
    mode: state.value.mode,
    colorKeys: [...state.value.colorKeys]
      .sort((a, b) => a.time - b.time)
      .map((key) => ({ time: key.time, color: key.color })),
    alphaKeys: [...state.value.alphaKeys]
      .sort((a, b) => a.time - b.time)
      .map((key) => ({ time: key.time, alpha: key.alpha })),
  };
}

function emitChange() {
  emit("change", payload());
}

function clamp01(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(1, value));
}

function hexToRgb(hex: string): { r: number; g: number; b: number } {
  const match = /^#([0-9a-fA-F]{6})$/.exec(hex);
  const value = match ? match[1] : "ffffff";
  return {
    r: parseInt(value.slice(0, 2), 16),
    g: parseInt(value.slice(2, 4), 16),
    b: parseInt(value.slice(4, 6), 16),
  };
}

function sampleColor(time: number): { r: number; g: number; b: number } {
  const keys = state.value.colorKeys;
  if (!keys.length) return { r: 255, g: 255, b: 255 };
  if (state.value.mode === "Fixed") {
    const hold = keys.find((key) => time <= key.time) ?? keys[keys.length - 1];
    return hexToRgb(hold.color);
  }
  if (time <= keys[0].time) return hexToRgb(keys[0].color);
  const last = keys[keys.length - 1];
  if (time >= last.time) return hexToRgb(last.color);
  for (let index = 0; index < keys.length - 1; index += 1) {
    const left = keys[index];
    const right = keys[index + 1];
    if (time >= left.time && time <= right.time) {
      const span = right.time - left.time || 1;
      const mix = (time - left.time) / span;
      const a = hexToRgb(left.color);
      const b = hexToRgb(right.color);
      return {
        r: Math.round(a.r + (b.r - a.r) * mix),
        g: Math.round(a.g + (b.g - a.g) * mix),
        b: Math.round(a.b + (b.b - a.b) * mix),
      };
    }
  }
  return hexToRgb(last.color);
}

function sampleAlpha(time: number): number {
  const keys = state.value.alphaKeys;
  if (!keys.length) return 1;
  if (state.value.mode === "Fixed") {
    const hold = keys.find((key) => time <= key.time) ?? keys[keys.length - 1];
    return hold.alpha;
  }
  if (time <= keys[0].time) return keys[0].alpha;
  const last = keys[keys.length - 1];
  if (time >= last.time) return last.alpha;
  for (let index = 0; index < keys.length - 1; index += 1) {
    const left = keys[index];
    const right = keys[index + 1];
    if (time >= left.time && time <= right.time) {
      const span = right.time - left.time || 1;
      return left.alpha + (right.alpha - left.alpha) * ((time - left.time) / span);
    }
  }
  return last.alpha;
}

function rgbaAt(time: number): string {
  const color = sampleColor(time);
  const alpha = sampleAlpha(time);
  return `rgba(${color.r}, ${color.g}, ${color.b}, ${Number(alpha.toFixed(3))})`;
}

const stripStyle = computed(() => {
  const times = new Set<number>([0, 1]);
  state.value.colorKeys.forEach((key) => times.add(clamp01(key.time)));
  state.value.alphaKeys.forEach((key) => times.add(clamp01(key.time)));
  const sorted = [...times].sort((a, b) => a - b);
  const stops: string[] = [];
  if (state.value.mode === "Fixed") {
    // Fixed mode steps: each segment holds one sample, with hard edges.
    for (let index = 0; index < sorted.length - 1; index += 1) {
      const start = sorted[index];
      const end = sorted[index + 1];
      const sampleTime = Math.min(end, start + (end - start) / 2 + 1e-6);
      const color = rgbaAt(sampleTime);
      stops.push(`${color} ${(start * 100).toFixed(2)}%`, `${color} ${(end * 100).toFixed(2)}%`);
    }
  } else {
    sorted.forEach((time) => {
      stops.push(`${rgbaAt(time)} ${(time * 100).toFixed(2)}%`);
    });
  }
  return { background: `linear-gradient(to right, ${stops.join(", ")})` };
});

function selectStop(type: StopSelection["type"], index: number) {
  selection.value = { type, index };
}

function keysForType(type: StopSelection["type"]): Array<GradientColorStop | GradientAlphaStop> {
  return type === "color" ? state.value.colorKeys : state.value.alphaKeys;
}

function startStopDrag(type: StopSelection["type"], index: number, event: PointerEvent) {
  selectStop(type, index);
  if (!editable.value || event.button !== 0) return;
  const trackEl = (event.currentTarget as HTMLElement | null)?.closest<HTMLElement>(".unity-gradient-track");
  if (!trackEl) return;
  event.preventDefault();
  event.stopPropagation();
  (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
  stopDrag.value = { pointerId: event.pointerId, selection: { type, index }, trackEl };
  window.addEventListener("pointermove", handleStopDragMove);
  window.addEventListener("pointerup", stopStopDrag);
  window.addEventListener("pointercancel", stopStopDrag);
}

function handleStopDragMove(event: PointerEvent) {
  const drag = stopDrag.value;
  if (!drag || event.pointerId !== drag.pointerId) return;
  event.preventDefault();
  const rect = drag.trackEl.getBoundingClientRect();
  if (rect.width <= 0) return;
  const time = clamp01((event.clientX - rect.left) / rect.width);
  const keys = keysForType(drag.selection.type);
  const stop = keys[drag.selection.index];
  if (!stop) return;
  stop.time = time;
  emitChange();
}

function stopStopDrag(event?: PointerEvent) {
  const drag = stopDrag.value;
  if (drag && event && event.pointerId !== drag.pointerId) return;
  stopDrag.value = null;
  window.removeEventListener("pointermove", handleStopDragMove);
  window.removeEventListener("pointerup", stopStopDrag);
  window.removeEventListener("pointercancel", stopStopDrag);
}

function addStop(type: StopSelection["type"], event: MouseEvent) {
  if (!editable.value) return;
  const trackEl = event.currentTarget as HTMLElement | null;
  if (!trackEl) return;
  const keys = keysForType(type);
  if (keys.length >= MAX_KEYS) return;
  const rect = trackEl.getBoundingClientRect();
  if (rect.width <= 0) return;
  const time = clamp01((event.clientX - rect.left) / rect.width);
  if (type === "color") {
    const color = sampleColor(time);
    state.value.colorKeys.push({
      time,
      color: `#${[color.r, color.g, color.b]
        .map((channel) => channel.toString(16).padStart(2, "0"))
        .join("")}`,
    });
    state.value.colorKeys.sort((a, b) => a.time - b.time);
    selectStop("color", state.value.colorKeys.findIndex((key) => key.time === time));
  } else {
    state.value.alphaKeys.push({ time, alpha: sampleAlpha(time) });
    state.value.alphaKeys.sort((a, b) => a.time - b.time);
    selectStop("alpha", state.value.alphaKeys.findIndex((key) => key.time === time));
  }
  emitChange();
}

function removeSelectedStop() {
  if (!editable.value) return;
  const keys = keysForType(selection.value.type);
  if (keys.length <= 1) return;
  keys.splice(selection.value.index, 1);
  selection.value = {
    type: selection.value.type,
    index: Math.max(0, Math.min(selection.value.index, keys.length - 1)),
  };
  emitChange();
}

function commitSelectedColor(event: Event) {
  const stop = selectedColorStop.value;
  if (!stop || !editable.value) return;
  const value = String((event.target as HTMLInputElement | null)?.value ?? "").trim().toLowerCase();
  if (!/^#[0-9a-f]{6}$/.test(value)) return;
  stop.color = value;
  emitChange();
}

function commitSelectedAlpha(event: Event) {
  const stop = selectedAlphaStop.value;
  if (!stop || !editable.value) return;
  const numeric = Number((event.target as HTMLInputElement | null)?.value ?? "");
  if (!Number.isFinite(numeric)) return;
  stop.alpha = clamp01(numeric);
  emitChange();
}

function commitSelectedLocation(event: Event) {
  if (!editable.value) return;
  const numeric = Number((event.target as HTMLInputElement | null)?.value ?? "");
  if (!Number.isFinite(numeric)) return;
  const stop = selection.value.type === "color" ? selectedColorStop.value : selectedAlphaStop.value;
  if (!stop) return;
  stop.time = clamp01(numeric / 100);
  emitChange();
}

function setMode(event: Event) {
  if (!editable.value) return;
  const value = (event.target as HTMLSelectElement | null)?.value === "Fixed" ? "Fixed" : "Blend";
  state.value.mode = value;
  emitChange();
}

function selectedLocationPercent(): string {
  const stop = selection.value.type === "color" ? selectedColorStop.value : selectedAlphaStop.value;
  return stop ? String(Number((stop.time * 100).toFixed(1))) : "0";
}

function alphaSwatchStyle(stop: GradientAlphaStop) {
  const channel = Math.round(stop.alpha * 255);
  return { background: `rgb(${channel}, ${channel}, ${channel})` };
}
</script>

<template>
  <div class="unity-gradient-editor">
    <div class="unity-gradient-track unity-gradient-alpha-track" @dblclick="addStop('alpha', $event)">
      <button
        v-for="(stop, index) in state.alphaKeys"
        :key="`alpha-${index}`"
        type="button"
        class="unity-gradient-stop unity-gradient-stop-alpha"
        :class="{ selected: selection.type === 'alpha' && selection.index === index }"
        :style="{ left: `${stop.time * 100}%`, ...alphaSwatchStyle(stop) }"
        :title="`a=${Number(stop.alpha.toFixed(3))} @ ${(stop.time * 100).toFixed(1)}%`"
        @pointerdown="startStopDrag('alpha', index, $event)"
      />
    </div>

    <div class="unity-gradient-strip-frame">
      <div class="unity-gradient-strip" :style="stripStyle" />
    </div>

    <div class="unity-gradient-track unity-gradient-color-track" @dblclick="addStop('color', $event)">
      <button
        v-for="(stop, index) in state.colorKeys"
        :key="`color-${index}`"
        type="button"
        class="unity-gradient-stop unity-gradient-stop-color"
        :class="{ selected: selection.type === 'color' && selection.index === index }"
        :style="{ left: `${stop.time * 100}%`, background: stop.color }"
        :title="`${stop.color} @ ${(stop.time * 100).toFixed(1)}%`"
        @pointerdown="startStopDrag('color', index, $event)"
      />
    </div>
    <div class="unity-gradient-track-hint">{{ t("unity.valueEditor.gradient.hint") }}</div>

    <div class="unity-gradient-selected">
      <template v-if="selection.type === 'color' && selectedColorStop">
        <label class="unity-gradient-field">
          <span>{{ t("unity.valueEditor.gradient.color") }}</span>
          <input
            type="color"
            class="unity-gradient-color-input"
            :value="selectedColorStop.color"
            :disabled="!editable"
            @input="commitSelectedColor"
            @change="commitSelectedColor"
          />
        </label>
      </template>
      <template v-else-if="selectedAlphaStop">
        <label class="unity-gradient-field">
          <span>{{ t("unity.valueEditor.gradient.alpha") }}</span>
          <input
            type="text"
            inputmode="decimal"
            class="unity-gradient-number-input"
            :value="String(Number(selectedAlphaStop.alpha.toFixed(3)))"
            :disabled="!editable"
            @change="commitSelectedAlpha"
            @keydown.enter.prevent="($event.target as HTMLElement | null)?.blur()"
          />
        </label>
      </template>
      <label class="unity-gradient-field">
        <span>{{ t("unity.valueEditor.gradient.location") }}</span>
        <input
          type="text"
          inputmode="decimal"
          class="unity-gradient-number-input"
          :value="selectedLocationPercent()"
          :disabled="!editable"
          @change="commitSelectedLocation"
          @keydown.enter.prevent="($event.target as HTMLElement | null)?.blur()"
        />
        <span class="unity-gradient-unit">%</span>
      </label>
      <button
        type="button"
        class="unity-gradient-remove"
        :disabled="!editable || keysForType(selection.type).length <= 1"
        @click="removeSelectedStop"
      >
        {{ t("unity.valueEditor.gradient.deleteStop") }}
      </button>
      <label class="unity-gradient-field unity-gradient-mode">
        <span>{{ t("unity.valueEditor.gradient.mode") }}</span>
        <select :value="state.mode" :disabled="!editable" @change="setMode">
          <option value="Blend">Blend</option>
          <option value="Fixed">Fixed</option>
        </select>
      </label>
    </div>
  </div>
</template>

<style scoped>
.unity-gradient-editor {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.unity-gradient-track {
  position: relative;
  height: 22px;
  border-radius: 5px;
}

.unity-gradient-track-hint {
  color: color-mix(in srgb, var(--text-secondary) 80%, transparent);
  font-size: 11px;
}

.unity-gradient-stop {
  position: absolute;
  top: 2px;
  width: 14px;
  height: 18px;
  margin-left: -7px;
  padding: 0;
  border: 1px solid var(--border-strong);
  border-radius: 4px;
  cursor: grab;
  box-sizing: border-box;
}

.unity-gradient-stop.selected {
  outline: 2px solid var(--accent-color);
  outline-offset: 1px;
}

.unity-gradient-strip-frame {
  height: 34px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
  background-image:
    linear-gradient(45deg, rgba(128, 128, 128, 0.32) 25%, transparent 25%, transparent 75%, rgba(128, 128, 128, 0.32) 75%),
    linear-gradient(45deg, rgba(128, 128, 128, 0.32) 25%, transparent 25%, transparent 75%, rgba(128, 128, 128, 0.32) 75%);
  background-size: 12px 12px;
  background-position: 0 0, 6px 6px;
}

.unity-gradient-strip {
  width: 100%;
  height: 100%;
}

.unity-gradient-selected {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  margin-top: 4px;
}

.unity-gradient-field {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-gradient-color-input {
  width: 44px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
}

.unity-gradient-number-input {
  width: 72px;
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

.unity-gradient-number-input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-gradient-unit {
  color: var(--text-secondary);
}

.unity-gradient-remove {
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

.unity-gradient-remove:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.unity-gradient-remove:disabled {
  opacity: 0.55;
  cursor: default;
}

.unity-gradient-mode select {
  min-height: 26px;
  padding: 0 6px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  font-size: 11px;
}
</style>
