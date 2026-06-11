<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Pencil } from "lucide";
import { t } from "../../i18n";
import LucideIcon from "../icons/LucideIcon.vue";
import { unityPropertyTargetKey } from "../../services/unityPropertyPath";
import type { UnitySerializedPropertyTarget } from "../../services/unitySerializedProperty";
import {
  listenUnityValueEditorCommitted,
  openUnityValueEditorWindow,
} from "../../services/unityValueEditorWindow";
import {
  unityAnimationCurveValue,
  type UnitySerializedPropertyTargetSnapshot,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  displayValue?: string;
  title?: string;
  ariaLabel?: string;
  editable?: boolean;
  bindingTarget?: UnitySerializedPropertyTargetSnapshot | null;
  label?: string;
}>(), {
  displayValue: "",
  title: "",
  ariaLabel: "",
  editable: false,
  bindingTarget: null,
  label: "",
});

const VIEW_WIDTH = 100;
const VIEW_HEIGHT = 32;
const VIEW_PADDING = 2;

// Edits committed by the Locus value editor window show immediately, even
// before the hosting surface re-reads the property.
const localOverride = ref<unknown>(null);
let unlistenCommitted: (() => void) | null = null;

watch(
  () => props.modelValue,
  () => {
    localOverride.value = null;
  },
);

const effectiveValue = computed(() => localOverride.value ?? props.modelValue);
const curve = computed(() => unityAnimationCurveValue(effectiveValue.value));
const keyCountLabel = computed(() => {
  const count = curve.value?.keyCount ?? 0;
  return `${count} ${count === 1 ? "key" : "keys"}`;
});
const canOpenEditor = computed(() =>
  props.editable && !!(props.bindingTarget?.propertyPath ?? "").trim(),
);
const fieldTitle = computed(() => {
  if (canOpenEditor.value) return t("unity.valueEditor.open");
  return props.title || keyCountLabel.value;
});
const polylinePoints = computed(() => {
  const value = curve.value;
  if (!value || value.samples.length < 2) return "";
  const range = value.maxValue - value.minValue;
  const span = range > 1e-9 ? range : 1;
  const innerHeight = VIEW_HEIGHT - VIEW_PADDING * 2;
  return value.samples
    .map((sample, index) => {
      const x = (index / (value.samples.length - 1)) * VIEW_WIDTH;
      const normalized = (sample - value.minValue) / span;
      const y = VIEW_HEIGHT - VIEW_PADDING - normalized * innerHeight;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .join(" ");
});
const fallbackText = computed(() => props.displayValue || "Curve");

function openEditor() {
  const target = props.bindingTarget;
  if (!canOpenEditor.value || !target) return;
  void openUnityValueEditorWindow({
    kind: "curve",
    target: target as UnitySerializedPropertyTarget,
    label: props.label || undefined,
  }).catch((error) => {
    console.warn("[UnityCurveField] failed to open value editor:", error);
  });
}

function handleFieldKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  openEditor();
}

onMounted(() => {
  void listenUnityValueEditorCommitted((event) => {
    if (event.kind !== "curve" || !props.bindingTarget) return;
    const ownKey = unityPropertyTargetKey(props.bindingTarget as UnitySerializedPropertyTarget);
    if (unityPropertyTargetKey(event.target) !== ownKey) return;
    localOverride.value = event.value;
  }).then((dispose) => {
    unlistenCommitted = dispose;
  });
});

onBeforeUnmount(() => {
  unlistenCommitted?.();
  unlistenCommitted = null;
});
</script>

<template>
  <div
    class="unity-curve-field"
    :class="{ editable: canOpenEditor }"
    :title="fieldTitle"
    :aria-label="ariaLabel || undefined"
    :role="canOpenEditor ? 'button' : undefined"
    :tabindex="canOpenEditor ? 0 : undefined"
    @click="openEditor"
    @keydown="canOpenEditor ? handleFieldKeydown($event) : undefined"
  >
    <svg
      v-if="polylinePoints"
      class="unity-curve-plot"
      :viewBox="`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`"
      preserveAspectRatio="none"
      role="img"
    >
      <polyline :points="polylinePoints" fill="none" vector-effect="non-scaling-stroke" />
    </svg>
    <span v-else class="unity-curve-fallback">{{ fallbackText }}</span>
    <span class="unity-curve-meta">{{ keyCountLabel }}</span>
    <span v-if="canOpenEditor" class="unity-curve-edit" aria-hidden="true">
      <LucideIcon :icon="Pencil" :size="12" />
    </span>
  </div>
</template>

<style scoped>
.unity-curve-field {
  width: 100%;
  min-width: 0;
  min-height: 26px;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 8px;
  padding: 2px 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg);
  box-sizing: border-box;
}

.unity-curve-field.editable {
  cursor: pointer;
}

.unity-curve-field.editable:hover,
.unity-curve-field.editable:focus-visible {
  border-color: var(--accent-color);
  outline: none;
}

.unity-curve-plot {
  width: 100%;
  height: 20px;
  display: block;
}

.unity-curve-plot polyline {
  stroke: var(--status-good-fg, var(--accent-color));
  stroke-width: 1.4;
}

.unity-curve-fallback {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 12px;
}

.unity-curve-meta {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.unity-curve-edit {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  color: var(--text-secondary);
}

.unity-curve-field.editable:hover .unity-curve-edit {
  color: var(--text-color);
}
</style>
