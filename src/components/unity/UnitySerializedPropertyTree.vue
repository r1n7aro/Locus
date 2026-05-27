<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  createPropertyTree,
  type InspectorPropertyDrawComponentsInput,
  type InspectorManagedReferenceTypeOption,
  type InspectorProperty,
} from "../../services/propertyTree";
import UnityPropertyDraw from "./UnityPropertyDraw.vue";
import UnityPropertyEditor from "./UnityPropertyEditor.vue";
import type {
  UnitySerializedPropertyCommitEvent,
  UnitySerializedPropertySnapshot,
} from "./unitySerializedValue";

const props = withDefaults(defineProps<{
  property: UnitySerializedPropertySnapshot;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  drawComponents?: InspectorPropertyDrawComponentsInput;
}>(), {
  disabled: false,
  readonly: false,
  compact: false,
  drawComponents: undefined,
});

const emit = defineEmits<{
  commit: [event: UnitySerializedPropertyCommitEvent];
}>();

const tree = computed(() =>
  createPropertyTree(props.property, {
    disabled: props.disabled,
    readonly: props.readonly,
    drawComponents: props.drawComponents,
  }),
);
const inspectorProperty = computed(() => tree.value.rootProperty);
const children = computed(() => inspectorProperty.value?.children ?? []);
const propertyType = computed(() => inspectorProperty.value?.valueType || "String");
const canEdit = computed(() => inspectorProperty.value?.canEdit === true);
const hasCustomDraw = computed(() => inspectorProperty.value?.hasDrawComponent() === true);
const managedTypeQuery = ref("");
const selectedManagedType = computed(() => inspectorProperty.value?.managedReferenceFullTypename || "");
const selectedManagedTypeOption = computed(() => inspectorProperty.value?.selectedManagedReferenceType ?? null);
const managedTypeOptions = computed<InspectorManagedReferenceTypeOption[]>(() => {
  const property = inspectorProperty.value;
  if (!property) return [];
  const options = property.searchManagedReferenceTypes(managedTypeQuery.value, { limit: 80 });
  const selected = selectedManagedTypeOption.value;
  if (!selected || options.some((option) => option.value === selected.value)) return options;
  return managedTypeQuery.value.trim() ? [...options, selected] : [selected, ...options];
});

watch(selectedManagedType, () => {
  managedTypeQuery.value = "";
});

function propertyLabel(property: InspectorProperty): string {
  return property.label || property.propertyPath;
}

function childSnapshot(property: InspectorProperty): UnitySerializedPropertySnapshot {
  return property.snapshot as UnitySerializedPropertySnapshot;
}

function emitCommit(property: InspectorProperty, value: unknown) {
  emit("commit", {
    propertyPath: property.propertyPath,
    value,
    property: property.snapshot as UnitySerializedPropertySnapshot,
  });
}

function commitLeaf(value: unknown) {
  const property = inspectorProperty.value;
  if (!property) return;
  emitCommit(property, value);
}

function commitArraySize(event: Event) {
  const property = inspectorProperty.value;
  if (!canEdit.value) return;
  const target = event.target as HTMLInputElement | null;
  const size = Math.max(0, Number.parseInt(target?.value || "0", 10) || 0);
  if (property) emitCommit(property, { action: "resize", size });
}

function addArrayElement() {
  const property = inspectorProperty.value;
  if (!canEdit.value) return;
  if (!property) return;
  emitCommit(property, {
    action: "insert",
    index: Math.max(0, property.arraySize >= 0 ? property.arraySize : children.value.length),
  });
}

function removeArrayElement(index: number) {
  const property = inspectorProperty.value;
  if (!canEdit.value) return;
  if (property) emitCommit(property, { action: "delete", index });
}

function moveArrayElement(index: number, toIndex: number) {
  const property = inspectorProperty.value;
  if (!canEdit.value || !property || toIndex < 0 || toIndex >= (property.arraySize ?? children.value.length)) return;
  emitCommit(property, { action: "move", index, toIndex });
}

function updateManagedTypeQuery(event: Event) {
  managedTypeQuery.value = (event.target as HTMLInputElement | null)?.value || "";
}

function commitFirstManagedType() {
  const property = inspectorProperty.value;
  if (!canEdit.value || !property) return;
  const option = managedTypeOptions.value[0];
  if (option) emitCommit(property, property.createManagedReferenceTypeCommit(option).value);
}

function commitManagedType(event: Event) {
  const property = inspectorProperty.value;
  if (!canEdit.value) return;
  const value = (event.target as HTMLSelectElement | null)?.value || "";
  if (!property) return;
  const option = property.managedReferenceTypes.find((item) => item.value === value) ?? value;
  emitCommit(property, property.createManagedReferenceTypeCommit(option).value);
}
</script>

<template>
  <div v-if="inspectorProperty" class="unity-property-tree" :class="{ compact }">
    <UnityPropertyDraw
      v-if="hasCustomDraw"
      :property="inspectorProperty"
      :draw-components="drawComponents"
      :disabled="disabled"
      :readonly="readonly"
      :compact="compact"
      @commit="$emit('commit', $event)"
    />

    <div v-else-if="inspectorProperty.isArray" class="property-container array-container">
      <div class="property-container-header">
        <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
        <div class="array-controls">
          <input
            class="array-size-input"
            type="number"
            min="0"
            :value="inspectorProperty.arraySize >= 0 ? inspectorProperty.arraySize : children.length"
            :disabled="!canEdit"
            :readonly="readonly"
            :title="inspectorProperty.propertyPath"
            @change="commitArraySize"
          />
          <button type="button" :disabled="!canEdit" @click="addArrayElement">Add</button>
        </div>
      </div>
      <div class="property-children">
        <div v-for="(child, index) in children" :key="child.propertyPath" class="array-item">
          <div class="array-item-toolbar">
            <span>{{ index }}</span>
            <button type="button" :disabled="!canEdit || index === 0" @click="moveArrayElement(index, index - 1)">Up</button>
            <button
              type="button"
              :disabled="!canEdit || index >= (property.arraySize ?? children.length) - 1"
              @click="moveArrayElement(index, index + 1)"
            >Down</button>
            <button type="button" :disabled="!canEdit" @click="removeArrayElement(index)">Remove</button>
          </div>
          <UnitySerializedPropertyTree
            :property="childSnapshot(child)"
            :disabled="disabled"
            :readonly="readonly"
            :draw-components="drawComponents"
            compact
            @commit="$emit('commit', $event)"
          />
        </div>
      </div>
    </div>

    <div v-else-if="inspectorProperty.isManagedReference" class="property-container managed-reference-container">
      <div class="property-container-header">
        <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
        <div class="managed-type-control">
          <input
            class="managed-type-search"
            type="search"
            :value="managedTypeQuery"
            :disabled="!canEdit"
            :readonly="readonly"
            placeholder="Search type"
            :title="inspectorProperty.managedReferenceFieldTypename || inspectorProperty.propertyPath"
            @input="updateManagedTypeQuery"
            @keydown.enter.prevent="commitFirstManagedType"
          />
          <select
            class="managed-type-select"
            :value="selectedManagedType"
            :disabled="!canEdit"
            :title="inspectorProperty.propertyPath"
            @change="commitManagedType"
          >
            <option value="">None</option>
            <option v-for="option in managedTypeOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </div>
      </div>
      <div v-if="children.length" class="property-children">
        <UnitySerializedPropertyTree
          v-for="child in children"
          :key="child.propertyPath"
          :property="childSnapshot(child)"
          :disabled="disabled"
          :readonly="readonly"
          :draw-components="drawComponents"
          compact
          @commit="$emit('commit', $event)"
        />
      </div>
    </div>

    <div v-else-if="children.length" class="property-container object-container">
      <div class="property-container-header">
        <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
        <span class="property-type">{{ propertyType }}</span>
      </div>
      <div class="property-children">
        <UnitySerializedPropertyTree
          v-for="child in children"
          :key="child.propertyPath"
          :property="childSnapshot(child)"
          :disabled="disabled"
          :readonly="readonly"
          :draw-components="drawComponents"
          compact
          @commit="$emit('commit', $event)"
        />
      </div>
    </div>

    <label v-else class="property-leaf">
      <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
      <UnityPropertyEditor
        :model-value="inspectorProperty.value"
        :property-type="propertyType"
        :display-value="inspectorProperty.displayValue || ''"
        :editable="inspectorProperty.editable !== false"
        :disabled="disabled"
        :readonly="readonly"
        :enum-options="inspectorProperty.enumOptions"
        :is-flags-enum="inspectorProperty.isFlagsEnum"
        :enum-value-index="inspectorProperty.enumValueIndex"
        :enum-value-flag="inspectorProperty.enumValueFlag"
        :title="inspectorProperty.propertyPath"
        @commit="commitLeaf"
      />
    </label>
  </div>
</template>

<style scoped>
.unity-property-tree {
  width: 100%;
  min-width: 0;
  display: grid;
  gap: 6px;
  font-size: 12px;
}

.property-container {
  min-width: 0;
  display: grid;
  gap: 6px;
}

.property-container-header,
.property-leaf {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(84px, 0.42fr) minmax(0, 1fr);
  align-items: center;
  gap: 8px;
}

.compact > .property-leaf,
.compact > .property-container > .property-container-header {
  grid-template-columns: minmax(72px, 0.34fr) minmax(0, 1fr);
}

.property-name,
.property-type {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
}

.property-type {
  font-family: var(--font-mono-identifier);
  font-size: 11px;
}

.property-children {
  min-width: 0;
  display: grid;
  gap: 6px;
  padding-left: 10px;
  border-left: 1px solid var(--border-color);
}

.array-controls {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(54px, 72px) auto;
  justify-content: start;
  gap: 6px;
}

.managed-type-control {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(96px, 0.34fr) minmax(0, 1fr);
  gap: 6px;
}

.array-size-input,
.managed-type-search,
.managed-type-select {
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

.managed-type-search,
.managed-type-select {
  font-family: var(--font-mono-identifier);
}

.array-size-input:focus,
.managed-type-search:focus,
.managed-type-select:focus {
  outline: none;
  border-color: var(--accent-color);
}

.array-item {
  min-width: 0;
  display: grid;
  gap: 5px;
}

.array-item-toolbar {
  display: flex;
  align-items: center;
  gap: 4px;
  color: var(--text-secondary);
  font-size: 11px;
}

button {
  min-height: 24px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
  font-size: 11px;
}

button:disabled,
.array-size-input:disabled,
.managed-type-search:disabled,
.managed-type-select:disabled {
  opacity: 0.58;
}
</style>
