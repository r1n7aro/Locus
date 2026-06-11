<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import {
  unityColorValueToRgba,
  unityRgbaToCssColor,
  type UnityColorRgba,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  disabled?: boolean;
  readonly?: boolean;
  title?: string;
  ariaLabel?: string;
}>(), {
  disabled: false,
  readonly: false,
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: UnityColorRgba];
  edit: [value: Record<string, string>];
  commit: [value: UnityColorRgba];
}>();

const CHANNELS = ["r", "g", "b", "a"] as const;

const parts = reactive<Record<string, string>>({ r: "0", g: "0", b: "0", a: "1" });

function currentRgba(): UnityColorRgba {
  return unityColorValueToRgba(props.modelValue) ?? { r: 0, g: 0, b: 0, a: 1 };
}

function syncParts() {
  const rgba = currentRgba();
  for (const channel of CHANNELS) {
    parts[channel] = String(rgba[channel]);
  }
}

watch(
  () => props.modelValue,
  syncParts,
  { immediate: true },
);

const swatchStyle = computed(() => {
  const rgba = unityColorValueToRgba({
    r: Number(parts.r),
    g: Number(parts.g),
    b: Number(parts.b),
    a: Number(parts.a),
  }) ?? currentRgba();
  return { background: unityRgbaToCssColor(rgba) };
});

function parsedRgba(): UnityColorRgba {
  const next = { r: 0, g: 0, b: 0, a: 1 };
  for (const channel of CHANNELS) {
    const numeric = Number(String(parts[channel] ?? "").trim());
    if (!Number.isFinite(numeric)) throw new Error("Expected number value");
    next[channel] = numeric;
  }
  // HDR allows intensities above 1 but never negative; alpha stays in [0,1].
  next.r = Math.max(0, next.r);
  next.g = Math.max(0, next.g);
  next.b = Math.max(0, next.b);
  next.a = Math.max(0, Math.min(1, next.a));
  return next;
}

function updateChannel(channel: string, event: Event) {
  const target = event.target as HTMLInputElement | null;
  parts[channel] = target?.value ?? "";
  emit("edit", { ...parts });
  try {
    emit("update:modelValue", parsedRgba());
  } catch {
    // Keep partial edits local until every channel parses.
  }
}

function commitColor() {
  if (props.disabled || props.readonly) return;
  try {
    const value = parsedRgba();
    for (const channel of CHANNELS) {
      parts[channel] = String(value[channel]);
    }
    emit("update:modelValue", value);
    emit("commit", value);
  } catch {
    // Invalid partial input stays editable.
  }
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}

function restoreChannelOnEscape(channel: string, event: KeyboardEvent) {
  const input = event.target as HTMLInputElement | null;
  syncParts();
  // Sync the DOM value before blurring so the change event does not fire.
  if (input) input.value = parts[channel] ?? "";
  input?.blur();
}
</script>

<template>
  <div class="unity-color-hdr-field" :title="title || undefined" :aria-label="ariaLabel || undefined">
    <span class="unity-color-hdr-swatch" :style="swatchStyle" />
    <label v-for="channel in CHANNELS" :key="channel" class="unity-color-hdr-part">
      <span>{{ channel }}</span>
      <input
        type="text"
        inputmode="decimal"
        :value="parts[channel]"
        :disabled="disabled"
        :readonly="readonly"
        @input="updateChannel(channel, $event)"
        @change="commitColor"
        @keydown.enter.prevent="blurOnEnter"
        @keydown.esc.prevent="restoreChannelOnEscape(channel, $event)"
      />
    </label>
    <span class="unity-color-hdr-badge">HDR</span>
  </div>
</template>

<style scoped>
.unity-color-hdr-field {
  width: 100%;
  min-width: 0;
  display: grid;
  grid-template-columns: 22px repeat(4, minmax(0, 1fr)) auto;
  align-items: center;
  gap: 4px;
}

.unity-color-hdr-swatch {
  width: 22px;
  height: 22px;
  display: block;
  border: 1px solid var(--border-color);
  border-radius: 5px;
}

.unity-color-hdr-part {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 4px;
}

.unity-color-hdr-part span {
  color: var(--text-secondary);
  font-size: 11px;
  text-transform: uppercase;
}

.unity-color-hdr-part input {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  color: var(--text-color);
  font: inherit;
  box-sizing: border-box;
}

.unity-color-hdr-part input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-color-hdr-part input:disabled,
.unity-color-hdr-part input:read-only {
  opacity: 0.65;
}

.unity-color-hdr-badge {
  flex-shrink: 0;
  padding: 1px 5px;
  border: 1px solid color-mix(in srgb, var(--status-good-fg) 38%, var(--border-color));
  border-radius: 4px;
  background: color-mix(in srgb, var(--status-good-bg) 24%, var(--panel-bg));
  color: var(--status-good-fg);
  font-family: var(--font-mono-identifier);
  font-size: 10px;
  line-height: 1.3;
}
</style>
