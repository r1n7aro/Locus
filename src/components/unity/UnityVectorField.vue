<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import {
  isUnityQuaternionPropertyType,
  parseUnityVectorValue,
  tryParseUnitySerializedEditValue,
  unityVectorKeysForType,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  propertyType?: string;
  disabled?: boolean;
  readonly?: boolean;
  displayValue?: string;
  title?: string;
  ariaLabel?: string;
}>(), {
  propertyType: "Vector3",
  disabled: false,
  readonly: false,
  displayValue: "",
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: Record<string, number | string>];
  edit: [value: Record<string, string>];
  commit: [value: Record<string, number | string>];
}>();

const keys = computed(() => unityVectorKeysForType(props.propertyType));
const parts = reactive<Record<string, string>>({});

function syncParts() {
  const parsed = isUnityQuaternionPropertyType(props.propertyType) && props.displayValue.trim()
    ? tryParseUnitySerializedEditValue(props.propertyType, props.displayValue)
    : tryParseUnitySerializedEditValue(props.propertyType, props.modelValue);
  const value = parsed.ok && parsed.value && typeof parsed.value === "object"
    ? parsed.value as Record<string, unknown>
    : {};
  for (const key of keys.value) {
    parts[key] = String(value[key] ?? 0);
  }
}

watch(
  () => [props.modelValue, props.propertyType, props.displayValue] as const,
  syncParts,
  { immediate: true },
);

function parsedVector() {
  return parseUnityVectorValue(props.propertyType, parts);
}

function updatePart(key: string, event: Event) {
  const target = event.target as HTMLInputElement | null;
  parts[key] = target?.value ?? "";
  emit("edit", { ...parts });
  try {
    emit("update:modelValue", parsedVector());
  } catch {
    // Keep partial vector edits local until all components are valid.
  }
}

function commitVector() {
  if (props.disabled || props.readonly) return;
  try {
    const value = parsedVector();
    emit("update:modelValue", value);
    emit("commit", value);
  } catch {
    // Invalid partial input remains editable.
  }
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}

function restorePartOnEscape(key: string, event: KeyboardEvent) {
  const input = event.target as HTMLInputElement | null;
  // Other axes are already committed (change fires on blur), so re-syncing
  // every part from the model only reverts the axis being edited.
  syncParts();
  // Sync the DOM value before blurring so the change event does not fire.
  if (input) input.value = parts[key] ?? "";
  input?.blur();
}
</script>

<template>
  <div
    class="unity-vector-field"
    :style="{ '--unity-vector-parts': String(keys.length || 1) }"
    :title="title || undefined"
    :aria-label="ariaLabel || undefined"
  >
    <label v-for="key in keys" :key="key" class="unity-vector-part">
      <span>{{ key }}</span>
      <input
        type="text"
        inputmode="decimal"
        :value="parts[key]"
        :disabled="disabled"
        :readonly="readonly"
        @input="updatePart(key, $event)"
        @change="commitVector"
        @keydown.enter.prevent="blurOnEnter"
        @keydown.esc.prevent="restorePartOnEscape(key, $event)"
      />
    </label>
  </div>
</template>

<style scoped>
.unity-vector-field {
  width: 100%;
  min-width: 0;
  min-height: var(--unity-property-row-height, 26px);
  display: grid;
  grid-template-columns: repeat(var(--unity-vector-parts, 3), minmax(0, 1fr));
  gap: 4px;
}

.unity-vector-part {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 4px;
}

.unity-vector-part span {
  color: var(--text-secondary);
  font-size: 11px;
  text-transform: uppercase;
}

.unity-vector-part input {
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

.unity-vector-part input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-vector-part input:disabled,
.unity-vector-part input:read-only {
  opacity: 0.65;
}
</style>
