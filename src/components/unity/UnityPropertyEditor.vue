<script setup lang="ts">
import { computed, ref, watch } from "vue";
import UnityBoolField from "./UnityBoolField.vue";
import UnityBoundsField from "./UnityBoundsField.vue";
import UnityColorField from "./UnityColorField.vue";
import UnityColorHdrField from "./UnityColorHdrField.vue";
import UnityCurveField from "./UnityCurveField.vue";
import UnityEnumField from "./UnityEnumField.vue";
import UnityFlagsField from "./UnityFlagsField.vue";
import UnityGradientField from "./UnityGradientField.vue";
import UnityLayerMaskField from "./UnityLayerMaskField.vue";
import UnityNumberField from "./UnityNumberField.vue";
import UnityObjectReferenceField from "./UnityObjectReferenceField.vue";
import UnityVectorField from "./UnityVectorField.vue";
import {
  isUnityBoundsPropertyType,
  isUnityHdrColorValue,
  isUnityNumberPropertyType,
  isUnityVectorPropertyType,
  normalizeUnityPropertyType,
  tryParseUnitySerializedEditValue,
  unitySerializedValueToEditText,
  type UnitySelectOption,
  type UnitySerializedPropertyTargetSnapshot,
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
  tooltip?: string;
  hasRange?: boolean;
  rangeMin?: number;
  rangeMax?: number;
  numberStep?: number;
  multiline?: boolean;
  minLines?: number;
  maxLines?: number;
  referenceTypeFullName?: string;
  referenceTypeAssembly?: string;
  bindingTarget?: UnitySerializedPropertyTargetSnapshot | null;
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
  tooltip: "",
  hasRange: false,
  rangeMin: 0,
  rangeMax: 0,
  numberStep: 0,
  multiline: false,
  minLines: 0,
  maxLines: 0,
  referenceTypeFullName: "",
  referenceTypeAssembly: "",
  bindingTarget: null,
});

const emit = defineEmits<{
  "update:modelValue": [value: unknown];
  edit: [value: unknown];
  preview: [value: unknown];
  commit: [value: unknown];
}>();

const text = ref(unitySerializedValueToEditText(props.propertyType, props.modelValue, props.displayValue));

const propertyType = computed(() => normalizeUnityPropertyType(props.propertyType));
const disabled = computed(() => props.disabled || !props.editable);
const readonly = computed(() => props.readonly || !props.editable);
const effectiveTitle = computed(() => props.tooltip || props.title);
const hdrColor = computed(() => propertyType.value === "Color" && isUnityHdrColorValue(props.modelValue));
const objectReferencePlaceholder = computed(() => {
  if (props.placeholder) return props.placeholder;
  if (props.referenceTypeFullName) return props.referenceTypeFullName;
  return "Assets/...";
});

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

function emitPreview(value: unknown) {
  emit("update:modelValue", value);
  emit("preview", value);
}

function updateText(event: Event) {
  const target = event.target as HTMLInputElement | HTMLTextAreaElement | null;
  text.value = target?.value ?? "";
  emit("edit", text.value);
  emit("update:modelValue", text.value);
}

function commitText() {
  if (disabled.value || readonly.value) return;
  const parsed = tryParseUnitySerializedEditValue(propertyType.value, text.value);
  if (!parsed.ok) return;
  emitCommit(parsed.value);
}

function blurOnEnter(event: KeyboardEvent) {
  (event.target as HTMLElement | null)?.blur();
}

function restoreTextOnEscape(event: KeyboardEvent) {
  const input = event.target as HTMLInputElement | HTMLTextAreaElement | null;
  const original = unitySerializedValueToEditText(props.propertyType, props.modelValue, props.displayValue);
  text.value = original;
  // Sync the DOM value before blurring so the change event does not fire.
  if (input) input.value = original;
  input?.blur();
}
</script>

<template>
  <div class="unity-property-editor" :class="`type-${propertyType}`">
    <UnityBoolField
      v-if="propertyType === 'Boolean'"
      :model-value="modelValue"
      :disabled="disabled"
      :readonly="readonly"
      :title="effectiveTitle"
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
      :title="effectiveTitle"
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
      :title="effectiveTitle"
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
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @preview="emitPreview"
      @commit="emitCommit"
    />
    <UnityNumberField
      v-else-if="isUnityNumberPropertyType(propertyType)"
      :model-value="modelValue"
      :property-type="propertyType"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      :has-range="hasRange"
      :range-min="rangeMin"
      :range-max="rangeMax"
      :number-step="numberStep"
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
      :display-value="displayValue"
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityColorHdrField
      v-else-if="propertyType === 'Color' && hdrColor"
      :model-value="modelValue"
      :disabled="disabled"
      :readonly="readonly"
      :title="effectiveTitle"
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
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityBoundsField
      v-else-if="isUnityBoundsPropertyType(propertyType)"
      :model-value="modelValue"
      :property-type="propertyType"
      :disabled="disabled"
      :readonly="readonly"
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <UnityCurveField
      v-else-if="propertyType === 'AnimationCurve'"
      :model-value="modelValue"
      :display-value="displayValue"
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      :editable="!disabled && !readonly"
      :binding-target="bindingTarget"
      :label="title"
    />
    <UnityGradientField
      v-else-if="propertyType === 'Gradient'"
      :model-value="modelValue"
      :display-value="displayValue"
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      :editable="!disabled && !readonly"
      :binding-target="bindingTarget"
      :label="title"
    />
    <UnityObjectReferenceField
      v-else-if="propertyType === 'ObjectReference'"
      :model-value="modelValue"
      :display-value="displayValue"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="objectReferencePlaceholder"
      :title="effectiveTitle"
      :aria-label="ariaLabel"
      :reference-type-full-name="referenceTypeFullName"
      :reference-type-assembly="referenceTypeAssembly"
      @update:model-value="emitUpdate"
      @edit="emitEdit"
      @commit="emitCommit"
    />
    <textarea
      v-else-if="propertyType === 'String' && multiline"
      class="unity-text-field unity-text-area"
      :value="text"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :rows="Math.max(2, minLines || 3)"
      :style="{ maxHeight: maxLines > 0 ? `${Math.max(maxLines, minLines || 3) * 20}px` : undefined }"
      :title="effectiveTitle || undefined"
      :aria-label="ariaLabel || undefined"
      @input="updateText"
      @change="commitText"
      @keydown.esc.prevent="restoreTextOnEscape"
    />
    <input
      v-else-if="propertyType === 'Character'"
      class="unity-text-field unity-char-field"
      type="text"
      maxlength="1"
      :value="text"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :title="effectiveTitle || undefined"
      :aria-label="ariaLabel || undefined"
      @input="updateText"
      @change="commitText"
      @keydown.enter.prevent="blurOnEnter"
      @keydown.esc.prevent="restoreTextOnEscape"
    />
    <input
      v-else
      class="unity-text-field"
      type="text"
      :value="text"
      :disabled="disabled"
      :readonly="readonly"
      :placeholder="placeholder"
      :title="effectiveTitle || undefined"
      :aria-label="ariaLabel || undefined"
      @input="updateText"
      @change="commitText"
      @keydown.enter.prevent="blurOnEnter"
      @keydown.esc.prevent="restoreTextOnEscape"
    />
  </div>
</template>

<style scoped>
.unity-property-editor {
  width: 100%;
  min-width: 0;
  min-height: var(--unity-property-row-height, 26px);
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

.unity-text-area {
  min-height: 56px;
  padding-top: 5px;
  padding-bottom: 5px;
  line-height: 1.4;
  resize: vertical;
}

.unity-char-field {
  max-width: 64px;
}

.unity-text-field:disabled,
.unity-text-field:read-only {
  opacity: 0.65;
}
</style>
