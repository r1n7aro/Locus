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
  unityGradientValue,
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
const gradient = computed(() => unityGradientValue(effectiveValue.value));
const keyCountLabel = computed(() => {
  const count = gradient.value?.colorKeys.length ?? 0;
  return `${count} ${count === 1 ? "key" : "keys"}`;
});
const canOpenEditor = computed(() =>
  props.editable && !!(props.bindingTarget?.propertyPath ?? "").trim(),
);
const stripStyle = computed(() => {
  const value = gradient.value;
  if (!value || !value.colorKeys.length) return null;
  const stops: string[] = [];
  if (value.mode.toLowerCase() === "fixed") {
    // Fixed mode: each color holds until its own key time.
    let previous = 0;
    value.colorKeys.forEach((key) => {
      const end = Math.max(previous, key.time * 100);
      stops.push(`${key.color} ${previous.toFixed(2)}%`, `${key.color} ${end.toFixed(2)}%`);
      previous = end;
    });
    const last = value.colorKeys[value.colorKeys.length - 1];
    if (previous < 100) stops.push(`${last.color} ${previous.toFixed(2)}%`, `${last.color} 100%`);
  } else {
    const first = value.colorKeys[0];
    if (first.time > 0) stops.push(`${first.color} 0%`);
    value.colorKeys.forEach((key) => {
      stops.push(`${key.color} ${(key.time * 100).toFixed(2)}%`);
    });
    const last = value.colorKeys[value.colorKeys.length - 1];
    if (last.time < 1) stops.push(`${last.color} 100%`);
  }
  return { background: `linear-gradient(to right, ${stops.join(", ")})` };
});
const tooltip = computed(() => {
  if (canOpenEditor.value) return t("unity.valueEditor.open");
  if (props.title) return props.title;
  const value = gradient.value;
  return value ? `${value.mode} · ${keyCountLabel.value}` : keyCountLabel.value;
});
const fallbackText = computed(() => props.displayValue || "Gradient");

function openEditor() {
  const target = props.bindingTarget;
  if (!canOpenEditor.value || !target) return;
  void openUnityValueEditorWindow({
    kind: "gradient",
    target: target as UnitySerializedPropertyTarget,
    label: props.label || undefined,
  }).catch((error) => {
    console.warn("[UnityGradientField] failed to open value editor:", error);
  });
}

function handleFieldKeydown(event: KeyboardEvent) {
  if (event.key !== "Enter" && event.key !== " ") return;
  event.preventDefault();
  openEditor();
}

onMounted(() => {
  void listenUnityValueEditorCommitted((event) => {
    if (event.kind !== "gradient" || !props.bindingTarget) return;
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
    class="unity-gradient-field"
    :class="{ editable: canOpenEditor }"
    :title="tooltip"
    :aria-label="ariaLabel || undefined"
    :role="canOpenEditor ? 'button' : undefined"
    :tabindex="canOpenEditor ? 0 : undefined"
    @click="openEditor"
    @keydown="canOpenEditor ? handleFieldKeydown($event) : undefined"
  >
    <span v-if="stripStyle" class="unity-gradient-strip" :style="stripStyle" />
    <span v-else class="unity-gradient-fallback">{{ fallbackText }}</span>
    <span class="unity-gradient-meta">{{ keyCountLabel }}</span>
    <span v-if="canOpenEditor" class="unity-gradient-edit" aria-hidden="true">
      <LucideIcon :icon="Pencil" :size="12" />
    </span>
  </div>
</template>

<style scoped>
.unity-gradient-field {
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

.unity-gradient-field.editable {
  cursor: pointer;
}

.unity-gradient-field.editable:hover,
.unity-gradient-field.editable:focus-visible {
  border-color: var(--accent-color);
  outline: none;
}

.unity-gradient-strip {
  width: 100%;
  height: 14px;
  display: block;
  border: 1px solid color-mix(in srgb, var(--border-color) 70%, transparent);
  border-radius: 4px;
}

.unity-gradient-fallback {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 12px;
}

.unity-gradient-meta {
  flex-shrink: 0;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.unity-gradient-edit {
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  color: var(--text-secondary);
}

.unity-gradient-field.editable:hover .unity-gradient-edit {
  color: var(--text-color);
}
</style>
