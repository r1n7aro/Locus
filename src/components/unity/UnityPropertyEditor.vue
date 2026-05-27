<script setup lang="ts">
import { computed, ref, watch } from "vue";
import UnityBoolField from "./UnityBoolField.vue";
import UnityColorField from "./UnityColorField.vue";
import UnityEnumField from "./UnityEnumField.vue";
import UnityFlagsField from "./UnityFlagsField.vue";
import UnityLayerMaskField from "./UnityLayerMaskField.vue";
import UnityNumberField from "./UnityNumberField.vue";
import UnityObjectReferenceField from "./UnityObjectReferenceField.vue";
import UnityVectorField from "./UnityVectorField.vue";
import {
  isUnityNumberPropertyType,
  isUnityVectorPropertyType,
  normalizeUnityPropertyType,
  parseUnitySerializedEditValue,
  unitySerializedValueToEditText,
  type UnitySelectOption,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  modelValue: unknown;
  propertyType?: string;
  displayValue?: string;
  editable?: boolean;
  disabled?: boolean;
  readonly?: boolean;
  enumOptions?: UnitySelectOption[];
  isFlagsEnum?: boolean;
  enumValueIndex?: number;
  enumValueFlag?: number;
  placeholder?: string;
  title?: string;
  ariaLabel?: string;
}>(), {
  propertyType: "String",
  displayValue: "",
  editable: true,
  disabled: false,
  readonly: false,
  enumOptions: () => [],
  isFlagsEnum: false,
  enumValueIndex: -1,
  enumValueFlag: 0,
  placeholder: "",
  title: "",
  ariaLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: unknown];
  edit: [value: unknown];
  commit: [value: unknown];
}>();

const text = ref(unitySerializedValueToEditText(props.propertyType, props.modelValue, props.displayValue));

const propertyType = computed(() => normalizeUnityPropertyType(props.propertyType));
const disabled = computed(() => props.disabled || !props.editable);
const readonly = computed(() => props.readonly || !props.editable);

watch(
  () => [props.modelValue, props.propertyType, props.displayValue] as const,
  () => {
    text.value = unitySerializedValueToEditText(props.propertyType, props.modelValue, props.displayValue);
  },
);

function emitUpdate(value: unknown) {
  emit("update:modelValue", value);
}

function emitEdit(value: unknown) {
  emit("edit", value);
}

function emitCommit(value: unknown) {
  emit("update:modelValue", value);
  emit("commit", value);
}

function updateText(event: Event) {
  const target = event.target as HTMLInputElement | null;
  text.value = target?.value ?? "";
  emit("edit", text.value);
  emit("update:modelValue", text.value);
}

function commitText() {
  if (disabled.value || readonly.value) return;
  emitCommit(parseUnitySerializedEditValue(propertyType.value, text.value));
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}
</script>

<template>
  <div class="unity-property-editor" :class="`type-${propertyType}`">
    <UnityBoolField
      v-if="propertyType === 'Boolean'"
      :model-value="modelValue"
      :disabled="disabled"
      :readonly="readonly"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @commit="emitCommit"
    />
    <UnityEnumField
      v-else-if="propertyType === 'Enum' && !isFlagsEnum"
      :model-value="modelValue"
      :enum-options="enumOptions"
      :enum-value-index="enumValueIndex"
      :disabled="disabled"
      :readonly="readonly"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @commit="emitCommit"
    />
    <UnityFlagsField
      v-else-if="propertyType === 'Enum' && isFlagsEnum"
      :model-value="modelValue"
      :enum-options="enumOptions"
      :enum-value-flag="enumValueFlag"
      :disabled="disabled"
      :readonly="readonly"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @commit="emitCommit"
    />
    <UnityLayerMaskField
      v-else-if="propertyType === 'LayerMask'"
      :model-value="modelValue"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityNumberField
      v-else-if="isUnityNumberPropertyType(propertyType)"
      :model-value="modelValue"
      :property-type="propertyType"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityVectorField
      v-else-if="isUnityVectorPropertyType(propertyType)"
      :model-value="modelValue"
      :property-type="propertyType"
      :disabled="disabled"
      :readonly="readonly"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityColorField
      v-else-if="propertyType === 'Color'"
      :model-value="modelValue"
      :disabled="disabled"
      :readonly="readonly"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityObjectReferenceField
      v-else-if="propertyType === 'ObjectReference'"
      :model-value="modelValue"
      :display-value="displayValue"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder || 'Assets/...'"
      :title="title"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <input
      v-else
      class="unity-text-field"
      type="text"
      :value="text"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :title="title || undefined"
      :aria-label="ariaLabel || undefined"
      @input="updateText"
      @change="commitText"
      @keydown.enter.prevent="blurOnEnter"
    />
  </div>
</template>

<style scoped>
.unity-property-editor {
  width: 100%;
  min-width: 0;
  display: flex;
  align-items: center;
}

.unity-property-editor.type-Boolean {
  justify-content: flex-start;
}

.unity-text-field {
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

.unity-text-field:focus {
  outline: none;
  border-color: var(--accent-color);
}

.unity-text-field:disabled,
.unity-text-field:read-only {
  opacity: 0.65;
}
</style>
