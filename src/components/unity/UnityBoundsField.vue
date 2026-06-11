<script setup lang="ts">
import { computed, reactive, watch } from "vue";
import {
  buildUnityBoundsValue,
  unityBoundsKeysForType,
  unityBoundsValueVectors,
  type UnityBoundsVectorValue,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  propertyType?: string;
  disabled?: boolean;
  readonly?: boolean;
  title?: string;
  ariaLabel?: string;
}>(), {
  propertyType: "Bounds",
  disabled: false,
  readonly: false,
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: Record<string, UnityBoundsVectorValue>];
  edit: [value: Record<string, string>];
  commit: [value: Record<string, UnityBoundsVectorValue>];
}>();

const AXES = ["x", "y", "z"] as const;

const rowKeys = computed(() => unityBoundsKeysForType(props.propertyType));
const parts = reactive<Record<string, string>>({});

function partKey(row: string, axis: string): string {
  return `${row}.${axis}`;
}

function syncParts() {
  const [first, second] = unityBoundsValueVectors(props.propertyType, props.modelValue);
  const [firstKey, secondKey] = rowKeys.value;
  const vectors: Record<string, UnityBoundsVectorValue> = { [firstKey]: first, [secondKey]: second };
  for (const row of rowKeys.value) {
    for (const axis of AXES) {
      parts[partKey(row, axis)] = String(vectors[row][axis] ?? 0);
    }
  }
}

watch(
  () => [props.modelValue, props.propertyType] as const,
  syncParts,
  { immediate: true },
);

function parsedVector(row: string): UnityBoundsVectorValue {
  const vector = { x: 0, y: 0, z: 0 };
  for (const axis of AXES) {
    const numeric = Number(String(parts[partKey(row, axis)] ?? "").trim());
    if (!Number.isFinite(numeric)) throw new Error("Expected number value");
    vector[axis] = numeric;
  }
  return vector;
}

function parsedBounds(): Record<string, UnityBoundsVectorValue> {
  const [firstKey, secondKey] = rowKeys.value;
  return buildUnityBoundsValue(props.propertyType, parsedVector(firstKey), parsedVector(secondKey));
}

function updatePart(row: string, axis: string, event: Event) {
  const target = event.target as HTMLInputElement | null;
  parts[partKey(row, axis)] = target?.value ?? "";
  emit("edit", { ...parts });
  try {
    emit("update:modelValue", parsedBounds());
  } catch {
    // Keep partial edits local until every axis parses.
  }
}

function commitBounds() {
  if (props.disabled || props.readonly) return;
  try {
    const value = parsedBounds();
    emit("update:modelValue", value);
    emit("commit", value);
  } catch {
    // Invalid partial input stays editable.
  }
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}

function restorePartOnEscape(row: string, axis: string, event: KeyboardEvent) {
  const input = event.target as HTMLInputElement | null;
  syncParts();
  // Sync the DOM value before blurring so the change event does not fire.
  if (input) input.value = parts[partKey(row, axis)] ?? "";
  input?.blur();
}

function rowLabel(row: string): string {
  return row.charAt(0).toUpperCase() + row.slice(1);
}
</script>

<template>
  <div class="unity-bounds-field" :title="title || undefined" :aria-label="ariaLabel || undefined">
    <div v-for="row in rowKeys" :key="row" class="unity-bounds-row">
      <span class="unity-bounds-row-label">{{ rowLabel(row) }}</span>
      <label v-for="axis in AXES" :key="axis" class="unity-bounds-part">
        <span>{{ axis }}</span>
        <input
          type="text"
          inputmode="decimal"
          :value="parts[partKey(row, axis)]"
          :disabled="disabled"
          :readonly="readonly"
          @input="updatePart(row, axis, $event)"
          @change="commitBounds"
          @keydown.enter.prevent="blurOnEnter"
          @keydown.esc.prevent="restorePartOnEscape(row, axis, $event)"
        />
      </label>
    </div>
  </div>
</template>

<style scoped>
.unity-bounds-field {
  width: 100%;
  min-width: 0;
  display: grid;
  gap: 4px;
}

.unity-bounds-row {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(52px, auto) repeat(3, minmax(0, 1fr));
  align-items: center;
  gap: 4px;
}

.unity-bounds-row-label {
  color: var(--text-secondary);
  font-size: 11px;
}

.unity-bounds-part {
  min-width: 0;
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  align-items: center;
  gap: 4px;
}

.unity-bounds-part span {
  color: var(--text-secondary);
  font-size: 11px;
  text-transform: uppercase;
}

.unity-bounds-part input {
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

.unity-bounds-part input:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-bounds-part input:disabled,
.unity-bounds-part input:read-only {
  opacity: 0.65;
}
</style>
