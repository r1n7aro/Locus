<script setup lang="ts">
import { ChevronRight, Plus, Trash2 } from "lucide";
import { computed, onBeforeUnmount, ref, shallowRef, watch } from "vue";
import {
  createInspectorPropertyTreeBinding,
  createPropertyTree,
  type InspectorPropertyCommit,
  type InspectorPropertyDrawerInput,
  type InspectorManagedReferenceTypeOption,
  type InspectorProperty,
  type InspectorPropertySnapshot,
  type InspectorPropertyTreeBinding,
  type InspectorPropertyTreeBindingInput,
} from "../../services/propertyTree";
import UnityPropertyDraw from "./UnityPropertyDraw.vue";
import UnityPropertyEditor from "./UnityPropertyEditor.vue";
import type {
  UnitySerializedPropertyCommitEvent,
  UnitySerializedPropertySnapshot,
} from "./unitySerializedValue";
import {
  constrainUnityNumberValue,
  formatUnityNumberValue,
  isUnityIntegerPropertyType,
  tryParseUnitySerializedEditValue,
} from "./unitySerializedValue";
import LucideIcon from "../icons/LucideIcon.vue";

type ArrayDragEdge = "before" | "after";

interface ArrayDragState {
  sourceIndex: number;
  targetIndex: number;
  edge: ArrayDragEdge;
}

interface ArrayPointerDragState extends ArrayDragState {
  pointerId: number;
  startX: number;
  startY: number;
  dragging: boolean;
  listElement: HTMLElement;
}

interface ArrayRenderItem {
  child: InspectorProperty;
  sourceIndex: number;
  displayIndex: number;
}

const ARRAY_POINTER_DRAG_THRESHOLD_PX = 4;
const ARRAY_OPTIMISTIC_MOVE_TIMEOUT_MS = 2400;

const props = withDefaults(defineProps<{
  property?: UnitySerializedPropertySnapshot | null;
  source?: InspectorPropertyTreeBindingInput | null;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  hideRootObjectHeader?: boolean;
  propertyDrawers?: InspectorPropertyDrawerInput;
}>(), {
  property: null,
  source: null,
  disabled: false,
  readonly: false,
  compact: false,
  hideRootObjectHeader: false,
  propertyDrawers: undefined,
});

const emit = defineEmits<{
  commit: [event: UnitySerializedPropertyCommitEvent];
  preview: [event: UnitySerializedPropertyCommitEvent];
}>();

const propertyTreeBinding = computed<InspectorPropertyTreeBinding>(() =>
  createInspectorPropertyTreeBinding({
    ...(props.source ?? {}),
    snapshots: props.source?.snapshots ?? props.property ?? null,
    disabled: props.disabled || props.source?.disabled === true,
    readonly: props.readonly || props.source?.readonly === true,
  }),
);
const rootSnapshot = computed<UnitySerializedPropertySnapshot | null>(() => {
  const snapshots = propertyTreeBinding.value.snapshots;
  if (Array.isArray(snapshots)) return (snapshots[0] as UnitySerializedPropertySnapshot | undefined) ?? null;
  return (snapshots as UnitySerializedPropertySnapshot | null) ?? null;
});
const tree = computed(() =>
  createPropertyTree(rootSnapshot.value, {
    id: propertyTreeBinding.value.id,
    targetId: propertyTreeBinding.value.targetId,
    disabled: propertyTreeBinding.value.disabled,
    readonly: propertyTreeBinding.value.readonly || !propertyTreeBinding.value.editable,
    propertyDrawers: props.propertyDrawers,
  }),
);
const inspectorProperty = computed(() => tree.value.rootProperty);
const children = computed(() => inspectorProperty.value?.children ?? []);
const propertyType = computed(() => inspectorProperty.value?.valueType || "String");
const canEdit = computed(() => inspectorProperty.value?.canEdit === true);
const editorDisabled = computed(() => propertyTreeBinding.value.disabled);
const editorReadonly = computed(() => propertyTreeBinding.value.readonly || !propertyTreeBinding.value.editable);
const hasCustomDraw = computed(() => inspectorProperty.value?.hasPropertyDrawer() === true);
const arrayItemCount = computed(() => {
  const property = inspectorProperty.value;
  if (!property) return children.value.length;
  return Math.max(0, property.arraySize >= 0 ? property.arraySize : children.value.length);
});
const hideObjectHeader = computed(() => {
  const property = inspectorProperty.value;
  return props.hideRootObjectHeader &&
    property?.depth === 0 &&
    children.value.length > 0 &&
    !property.isArray &&
    !property.isManagedReference;
});
const hideLeafLabel = computed(() => props.hideRootObjectHeader && inspectorProperty.value?.depth === 0);
const managedTypeQuery = ref("");
const selectedManagedType = computed(() => inspectorProperty.value?.managedReferenceFullTypename || "");
const selectedManagedTypeOption = computed(() => inspectorProperty.value?.selectedManagedReferenceType ?? null);
const arrayCollapsed = ref(false);
const arrayDrag = ref<ArrayDragState | null>(null);
const arrayPointerDrag = shallowRef<ArrayPointerDragState | null>(null);
const arrayOptimisticMove = ref<ArrayDragState | null>(null);
const arrayRenderItems = computed<ArrayRenderItem[]>(() => {
  const items = children.value.map((child, sourceIndex) => ({ child, sourceIndex }));
  const pointerDrag = arrayPointerDrag.value;
  const drag = pointerDrag?.dragging ? pointerDrag : arrayOptimisticMove.value;
  if (
    drag &&
    drag.sourceIndex >= 0 &&
    drag.sourceIndex < items.length &&
    drag.targetIndex >= 0 &&
    drag.targetIndex < items.length &&
    drag.sourceIndex !== drag.targetIndex
  ) {
    const [moved] = items.splice(drag.sourceIndex, 1);
    if (moved) items.splice(drag.targetIndex, 0, moved);
  }
  return items.map((item, displayIndex) => ({ ...item, displayIndex }));
});
const numberLabelDrag = shallowRef<{
  property: InspectorProperty;
  propertyPath: string;
  startX: number;
  startValue: number;
  latestValue: number;
  step: number;
  pointerId: number;
} | null>(null);
const numberLabelDragPreview = ref<{ propertyPath: string; value: number } | null>(null);
let numberLabelCommitFrame = 0;
let pendingNumberLabelCommit: { property: InspectorProperty; value: number } | null = null;
let arrayOptimisticMoveTimer = 0;
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

watch(
  () => inspectorProperty.value?.propertyPath,
  () => {
    arrayCollapsed.value = false;
    clearArrayOptimisticMove();
    stopArrayItemDrag();
  },
);

watch(
  () => children.value.map(arrayChildChangeKey).join("\u0000"),
  () => {
    if (arrayOptimisticMove.value) clearArrayOptimisticMove();
  },
);

watch(
  () => [inspectorProperty.value?.propertyPath, inspectorProperty.value?.value] as const,
  () => {
    if (!numberLabelDrag.value) numberLabelDragPreview.value = null;
  },
);

onBeforeUnmount(() => {
  stopArrayItemDragListeners();
  clearArrayOptimisticMove();
  stopNumberLabelDragListeners();
  if (numberLabelCommitFrame) {
    cancelAnimationFrame(numberLabelCommitFrame);
    numberLabelCommitFrame = 0;
  }
});

function propertyLabel(property: InspectorProperty): string {
  return property.label || property.propertyPath;
}

function childSnapshot(property: InspectorProperty): UnitySerializedPropertySnapshot {
  return property.snapshot as UnitySerializedPropertySnapshot;
}

function arrayChildChangeKey(property: InspectorProperty): string {
  const snapshot = childSnapshot(property);
  return [
    property.propertyPath,
    property.displayValue,
    String(snapshot.displayValue ?? ""),
    String(snapshot.value ?? ""),
  ].join("\u001f");
}

function childSource(property: InspectorProperty): InspectorPropertyTreeBindingInput {
  const binding = propertyTreeBinding.value;
  return {
    id: binding.id,
    targetId: binding.targetId,
    snapshots: childSnapshot(property) as InspectorPropertySnapshot,
    loading: binding.loading,
    error: binding.error,
    disabled: binding.disabled,
    readonly: binding.readonly,
    editable: binding.editable,
    commit: binding.commit,
  };
}

function toUnityCommitEvent(
  commit: InspectorPropertyCommit,
  writeMode: UnitySerializedPropertyCommitEvent["writeMode"] = "commit",
): UnitySerializedPropertyCommitEvent {
  const target = (commit.property.root.snapshot as UnitySerializedPropertySnapshot).bindingTarget
    ?? (commit.property.root.snapshot as UnitySerializedPropertySnapshot).target
    ?? (commit.snapshot as UnitySerializedPropertySnapshot).bindingTarget
    ?? (commit.snapshot as UnitySerializedPropertySnapshot).target
    ?? null;
  return {
    propertyPath: commit.propertyPath,
    value: commit.value,
    property: commit.snapshot as UnitySerializedPropertySnapshot,
    target,
    writeMode,
  };
}

function commitProperty(property: InspectorProperty, value: unknown) {
  const commit = property.createCommit(value);
  void Promise.resolve(propertyTreeBinding.value.commit(commit));
  emit("commit", toUnityCommitEvent(commit));
}

function commitDrawEvent(event: UnitySerializedPropertyCommitEvent) {
  const property = tree.value.getProperty(event.propertyPath) ?? inspectorProperty.value;
  if (!property) return;
  commitProperty(property, event.value);
}

function emitCommit(property: InspectorProperty, value: unknown) {
  commitProperty(property, value);
}

function emitPreview(property: InspectorProperty, value: unknown) {
  const commit = property.createCommit(value);
  emit("preview", toUnityCommitEvent(commit, "preview"));
}

function commitLeaf(value: unknown) {
  const property = inspectorProperty.value;
  if (!property) return;
  emitCommit(property, value);
}

function previewLeaf(value: unknown) {
  const property = inspectorProperty.value;
  if (!property) return;
  emitPreview(property, value);
}

function leafModelValue(property: InspectorProperty): unknown {
  const preview = numberLabelDragPreview.value;
  return preview?.propertyPath === property.propertyPath ? preview.value : property.value;
}

function canDragNumberLabel(property: InspectorProperty): boolean {
  return property.drawer.kind === "number" && property.canEdit;
}

function numberConstraints(property: InspectorProperty) {
  return {
    hasRange: property.hasRange,
    rangeMin: property.rangeMin,
    rangeMax: property.rangeMax,
  };
}

function numberValue(property: InspectorProperty, rawValue: unknown): number | null {
  const parsed = tryParseUnitySerializedEditValue(property.valueType, rawValue);
  if (!parsed.ok || typeof parsed.value !== "number") return null;
  return constrainUnityNumberValue(property.valueType, parsed.value, numberConstraints(property));
}

function numberDragStep(property: InspectorProperty, startValue: number): number {
  if (property.numberStep > 0) return property.numberStep;
  if (isUnityIntegerPropertyType(property.valueType)) return 1;
  if (property.hasRange && Number.isFinite(property.rangeMin) && Number.isFinite(property.rangeMax)) {
    const range = Math.abs(property.rangeMax - property.rangeMin);
    if (range > 0) return range / 200;
  }
  const magnitude = Math.abs(startValue);
  if (magnitude >= 1000) return 10;
  if (magnitude >= 100) return 1;
  if (magnitude >= 10) return 0.1;
  return 0.01;
}

function numberDragMultiplier(event: PointerEvent | KeyboardEvent): number {
  if (event.shiftKey) return 10;
  if (event.altKey || event.ctrlKey || event.metaKey) return 0.1;
  return 1;
}

function startNumberLabelDrag(property: InspectorProperty, event: PointerEvent) {
  if (event.button !== 0 || !canDragNumberLabel(property)) return;
  const startValue = numberValue(property, leafModelValue(property));
  if (startValue == null) return;
  event.preventDefault();
  event.stopPropagation();
  const target = event.currentTarget as HTMLElement | null;
  target?.setPointerCapture?.(event.pointerId);
  numberLabelDrag.value = {
    property,
    propertyPath: property.propertyPath,
    startX: event.clientX,
    startValue,
    latestValue: startValue,
    step: numberDragStep(property, startValue),
    pointerId: event.pointerId,
  };
  numberLabelDragPreview.value = { propertyPath: property.propertyPath, value: startValue };
  window.addEventListener("pointermove", handleNumberLabelDragMove);
  window.addEventListener("pointerup", stopNumberLabelDrag);
  window.addEventListener("pointercancel", stopNumberLabelDrag);
}

function handleNumberLabelDragMove(event: PointerEvent) {
  const drag = numberLabelDrag.value;
  if (!drag || event.pointerId !== drag.pointerId) return;
  event.preventDefault();
  const delta = event.clientX - drag.startX;
  const rawValue = drag.startValue + delta * drag.step * numberDragMultiplier(event);
  const nextValue = constrainUnityNumberValue(drag.property.valueType, rawValue, numberConstraints(drag.property));
  if (Object.is(nextValue, drag.latestValue)) return;
  drag.latestValue = nextValue;
  numberLabelDragPreview.value = { propertyPath: drag.propertyPath, value: nextValue };
  queueNumberLabelCommit(drag.property, nextValue);
}

function stopNumberLabelDrag() {
  const drag = numberLabelDrag.value;
  flushNumberLabelCommit();
  if (drag) emitCommit(drag.property, drag.latestValue);
  numberLabelDrag.value = null;
  stopNumberLabelDragListeners();
}

function stopNumberLabelDragListeners() {
  window.removeEventListener("pointermove", handleNumberLabelDragMove);
  window.removeEventListener("pointerup", stopNumberLabelDrag);
  window.removeEventListener("pointercancel", stopNumberLabelDrag);
}

function queueNumberLabelCommit(property: InspectorProperty, value: number) {
  pendingNumberLabelCommit = { property, value };
  if (numberLabelCommitFrame) return;
  numberLabelCommitFrame = requestAnimationFrame(() => {
    numberLabelCommitFrame = 0;
    flushNumberLabelCommit();
  });
}

function flushNumberLabelCommit() {
  if (numberLabelCommitFrame) {
    cancelAnimationFrame(numberLabelCommitFrame);
    numberLabelCommitFrame = 0;
  }
  const pending = pendingNumberLabelCommit;
  pendingNumberLabelCommit = null;
  if (!pending) return;
  emitPreview(pending.property, pending.value);
}

function nudgeNumberLabel(property: InspectorProperty, direction: 1 | -1, event: KeyboardEvent) {
  if (!canDragNumberLabel(property)) return;
  const currentValue = numberValue(property, leafModelValue(property));
  if (currentValue == null) return;
  event.preventDefault();
  const rawValue = currentValue + direction * numberDragStep(property, currentValue) * numberDragMultiplier(event);
  const nextValue = constrainUnityNumberValue(property.valueType, rawValue, numberConstraints(property));
  numberLabelDragPreview.value = { propertyPath: property.propertyPath, value: nextValue };
  emitCommit(property, nextValue);
}

function numberLabelTitle(property: InspectorProperty): string {
  const label = property.tooltip || property.propertyPath;
  if (!property.hasRange) return label;
  const min = formatUnityNumberValue(property.valueType, property.rangeMin, numberConstraints(property));
  const max = formatUnityNumberValue(property.valueType, property.rangeMax, numberConstraints(property));
  return `${label} (${min}..${max})`;
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
    index: arrayItemCount.value,
  });
}

function removeArrayElement(index: number) {
  const property = inspectorProperty.value;
  if (!canEdit.value) return;
  if (property) emitCommit(property, { action: "delete", index });
}

function moveArrayElement(index: number, toIndex: number) {
  const property = inspectorProperty.value;
  if (!canEdit.value || !property || toIndex < 0 || toIndex >= arrayItemCount.value || index === toIndex) return;
  emitCommit(property, { action: "move", index, toIndex });
}

function keepArrayOptimisticMove(sourceIndex: number, targetIndex: number) {
  clearArrayOptimisticMove();
  if (sourceIndex === targetIndex) return;
  arrayOptimisticMove.value = { sourceIndex, targetIndex, edge: "before" };
  arrayOptimisticMoveTimer = window.setTimeout(() => {
    arrayOptimisticMoveTimer = 0;
    arrayOptimisticMove.value = null;
  }, ARRAY_OPTIMISTIC_MOVE_TIMEOUT_MS);
}

function clearArrayOptimisticMove() {
  if (arrayOptimisticMoveTimer) {
    window.clearTimeout(arrayOptimisticMoveTimer);
    arrayOptimisticMoveTimer = 0;
  }
  arrayOptimisticMove.value = null;
}

function toggleArrayCollapsed() {
  arrayCollapsed.value = !arrayCollapsed.value;
  if (arrayCollapsed.value) stopArrayItemDrag();
}

function canDragArrayItem(index: number): boolean {
  return canEdit.value && arrayItemCount.value > 1 && index >= 0 && index < arrayItemCount.value;
}

function canMoveArrayItem(index: number, direction: 1 | -1): boolean {
  const toIndex = index + direction;
  return canEdit.value && toIndex >= 0 && toIndex < arrayItemCount.value;
}

function arrayDropSlotFromPoint(
  listElement: HTMLElement,
  sourceIndex: number,
  clientY: number,
): { edge: ArrayDragEdge; slotIndex: number; rowIndex: number; targetIndex: number } {
  const rows = Array.from(listElement.querySelectorAll<HTMLElement>(":scope > .array-item"))
    .filter((row) => Number(row.dataset.arraySourceIndex) !== sourceIndex);
  if (!rows.length) {
    return { edge: "after", slotIndex: sourceIndex + 1, rowIndex: sourceIndex, targetIndex: sourceIndex };
  }

  for (let index = 0; index < rows.length; index += 1) {
    const rect = rows[index].getBoundingClientRect();
    if (clientY <= rect.top + rect.height / 2) {
      return {
        edge: "before",
        slotIndex: index,
        rowIndex: index,
        targetIndex: Math.max(0, Math.min(arrayItemCount.value - 1, index)),
      };
    }
  }

  const lastIndex = rows.length - 1;
  const slotIndex = arrayItemCount.value - 1;
  return {
    edge: "after",
    slotIndex,
    rowIndex: lastIndex,
    targetIndex: Math.max(0, Math.min(arrayItemCount.value - 1, slotIndex)),
  };
}

function startArrayItemDrag(index: number, event: PointerEvent) {
  if (!canDragArrayItem(index)) {
    event.preventDefault();
    return;
  }
  const row = (event.currentTarget as HTMLElement | null)?.closest<HTMLElement>(".array-item");
  const listElement = row?.parentElement;
  if (!listElement) return;
  event.preventDefault();
  event.stopPropagation();
  clearArrayOptimisticMove();
  (event.currentTarget as HTMLElement | null)?.setPointerCapture?.(event.pointerId);
  arrayPointerDrag.value = {
    sourceIndex: index,
    targetIndex: index,
    edge: "after",
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    dragging: false,
    listElement,
  };
  arrayDrag.value = { sourceIndex: index, targetIndex: index, edge: "after" };
  window.addEventListener("pointermove", handleArrayItemDragMove);
  window.addEventListener("pointerup", stopArrayItemDrag);
  window.addEventListener("pointercancel", stopArrayItemDrag);
}

function handleArrayItemDragMove(event: PointerEvent) {
  const drag = arrayPointerDrag.value;
  if (!drag || event.pointerId !== drag.pointerId) return;
  event.preventDefault();
  const distance = Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY);
  if (!drag.dragging && distance < ARRAY_POINTER_DRAG_THRESHOLD_PX) return;
  const next = arrayDropSlotFromPoint(drag.listElement, drag.sourceIndex, event.clientY);
  arrayPointerDrag.value = {
    ...drag,
    dragging: true,
    targetIndex: next.targetIndex,
    edge: next.edge,
  };
  arrayDrag.value = {
    sourceIndex: drag.sourceIndex,
    targetIndex: next.targetIndex === drag.sourceIndex ? -1 : next.rowIndex,
    edge: next.edge,
  };
}

function stopArrayItemDrag(event?: PointerEvent) {
  const drag = arrayPointerDrag.value;
  if (drag && (!event || event.pointerId === drag.pointerId)) {
    if (drag.dragging) {
      keepArrayOptimisticMove(drag.sourceIndex, drag.targetIndex);
      moveArrayElement(drag.sourceIndex, drag.targetIndex);
    }
    arrayPointerDrag.value = null;
  }
  arrayDrag.value = null;
  stopArrayItemDragListeners();
}

function stopArrayItemDragListeners() {
  window.removeEventListener("pointermove", handleArrayItemDragMove);
  window.removeEventListener("pointerup", stopArrayItemDrag);
  window.removeEventListener("pointercancel", stopArrayItemDrag);
}

function nudgeArrayElement(index: number, direction: 1 | -1, event: KeyboardEvent) {
  if (!canMoveArrayItem(index, direction)) return;
  event.preventDefault();
  moveArrayElement(index, index + direction);
}

function arrayItemClass(item: ArrayRenderItem): Record<string, boolean> {
  const pointerDrag = arrayPointerDrag.value;
  const drag = arrayDrag.value;
  const isMoving = pointerDrag?.dragging === true && pointerDrag.sourceIndex === item.sourceIndex;
  return {
    dragging: isMoving,
    "preview-moving": isMoving && pointerDrag.targetIndex !== pointerDrag.sourceIndex,
    "drop-before": drag?.targetIndex === item.displayIndex && drag.edge === "before",
    "drop-after": drag?.targetIndex === item.displayIndex && drag.edge === "after",
  };
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
    <div v-if="inspectorProperty.header" class="property-header-label">
      {{ inspectorProperty.header }}
    </div>
    <UnityPropertyDraw
      v-if="hasCustomDraw"
      :property="inspectorProperty"
      :property-drawers="propertyDrawers"
      :disabled="editorDisabled"
      :readonly="editorReadonly"
      :compact="compact"
      @commit="commitDrawEvent"
    />

    <div v-else-if="inspectorProperty.isArray" class="property-container array-container">
      <div class="property-container-header" :title="inspectorProperty.tooltip || inspectorProperty.propertyPath">
        <div class="array-header-title">
          <button
            type="button"
            class="array-fold-button"
            :class="{ collapsed: arrayCollapsed }"
            :aria-expanded="!arrayCollapsed"
            :aria-label="propertyLabel(inspectorProperty)"
            @click="toggleArrayCollapsed"
          >
            <LucideIcon :icon="ChevronRight" :size="13" />
          </button>
          <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
        </div>
        <div class="array-controls">
          <input
            class="array-size-input"
            type="number"
            min="0"
            :value="inspectorProperty.arraySize >= 0 ? inspectorProperty.arraySize : children.length"
            :disabled="!canEdit"
            :readonly="editorReadonly"
            :title="inspectorProperty.propertyPath"
            @change="commitArraySize"
          />
          <button type="button" class="array-add-button" :disabled="!canEdit" @click="addArrayElement">
            <LucideIcon :icon="Plus" :size="13" />
            <span>Add</span>
          </button>
        </div>
      </div>
      <div v-if="!arrayCollapsed" class="property-children">
        <div
          v-for="item in arrayRenderItems"
          :key="item.child.propertyPath"
          class="array-item"
          :class="arrayItemClass(item)"
          :data-array-source-index="item.sourceIndex"
        >
          <button
            type="button"
            class="array-index-handle"
            :disabled="!canDragArrayItem(item.sourceIndex)"
            :aria-label="`Move element ${item.displayIndex}`"
            title="Drag to reorder"
            @pointerdown="startArrayItemDrag(item.sourceIndex, $event)"
            @keydown.arrow-up="nudgeArrayElement(item.sourceIndex, -1, $event)"
            @keydown.arrow-down="nudgeArrayElement(item.sourceIndex, 1, $event)"
          >
            <span class="array-item-index">{{ item.displayIndex }}</span>
          </button>
          <div class="array-item-content">
            <UnitySerializedPropertyTree
              :source="childSource(item.child)"
              :disabled="editorDisabled"
              :readonly="editorReadonly"
              :property-drawers="propertyDrawers"
              hide-root-object-header
              compact
              @commit="$emit('commit', $event)"
              @preview="$emit('preview', $event)"
            />
          </div>
          <button
            type="button"
            class="array-icon-button array-remove-button"
            :disabled="!canEdit"
            :aria-label="`Remove element ${item.displayIndex}`"
            title="Remove"
            @click="removeArrayElement(item.sourceIndex)"
          >
            <LucideIcon :icon="Trash2" :size="13" />
          </button>
        </div>
      </div>
    </div>

    <div v-else-if="inspectorProperty.isManagedReference" class="property-container managed-reference-container">
      <div class="property-container-header" :title="inspectorProperty.tooltip || inspectorProperty.propertyPath">
        <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
        <div class="managed-type-control">
          <input
            class="managed-type-search"
            type="search"
            :value="managedTypeQuery"
            :disabled="!canEdit"
            :readonly="editorReadonly"
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
          :source="childSource(child)"
          :disabled="editorDisabled"
          :readonly="editorReadonly"
          :property-drawers="propertyDrawers"
          compact
          @commit="$emit('commit', $event)"
          @preview="$emit('preview', $event)"
        />
      </div>
    </div>

    <div
      v-else-if="children.length && inspectorProperty.drawer.container"
      class="property-container object-container"
      :class="{ 'hide-root-header': hideObjectHeader }"
    >
      <div
        v-if="!hideObjectHeader"
        class="property-container-header"
        :title="inspectorProperty.tooltip || inspectorProperty.propertyPath"
      >
        <span class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
        <span class="property-type">{{ propertyType }}</span>
      </div>
      <div class="property-children">
        <UnitySerializedPropertyTree
          v-for="child in children"
          :key="child.propertyPath"
          :source="childSource(child)"
          :disabled="editorDisabled"
          :readonly="editorReadonly"
          :property-drawers="propertyDrawers"
          compact
          @commit="$emit('commit', $event)"
          @preview="$emit('preview', $event)"
        />
      </div>
    </div>

    <div v-else class="property-leaf" :class="{ 'hide-leaf-label': hideLeafLabel }" :title="numberLabelTitle(inspectorProperty)">
      <button
        v-if="!hideLeafLabel && canDragNumberLabel(inspectorProperty)"
        type="button"
        class="property-name property-name-drag"
        :class="{ dragging: numberLabelDrag?.propertyPath === inspectorProperty.propertyPath }"
        :title="numberLabelTitle(inspectorProperty)"
        @pointerdown="startNumberLabelDrag(inspectorProperty, $event)"
        @keydown.arrow-left="nudgeNumberLabel(inspectorProperty, -1, $event)"
        @keydown.arrow-right="nudgeNumberLabel(inspectorProperty, 1, $event)"
      >{{ propertyLabel(inspectorProperty) }}</button>
      <span v-else-if="!hideLeafLabel" class="property-name">{{ propertyLabel(inspectorProperty) }}</span>
      <UnityPropertyEditor
        :model-value="leafModelValue(inspectorProperty)"
        :property-type="propertyType"
        :display-value="inspectorProperty.displayValue || ''"
        :editable="inspectorProperty.editable !== false"
        :disabled="editorDisabled"
        :readonly="editorReadonly"
        :enum-options="inspectorProperty.enumOptions"
        :is-flags-enum="inspectorProperty.isFlagsEnum"
        :enum-value-index="inspectorProperty.enumValueIndex"
        :enum-value-flag="inspectorProperty.enumValueFlag"
        :title="inspectorProperty.propertyPath"
        :tooltip="inspectorProperty.tooltip"
        :has-range="inspectorProperty.hasRange"
        :range-min="inspectorProperty.rangeMin"
        :range-max="inspectorProperty.rangeMax"
        :number-step="inspectorProperty.numberStep"
        :multiline="inspectorProperty.multiline"
        :min-lines="inspectorProperty.minLines"
        :max-lines="inspectorProperty.maxLines"
        :reference-type-full-name="inspectorProperty.referenceTypeFullName"
        :reference-type-assembly="inspectorProperty.referenceTypeAssembly"
        @preview="previewLeaf"
        @commit="commitLeaf"
      />
    </div>
  </div>
</template>

<style scoped>
.unity-property-tree {
  --unity-property-row-height: 26px;
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

.property-header-label {
  min-width: 0;
  padding: 4px 0 2px;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.property-container-header,
.property-leaf {
  min-width: 0;
  min-height: var(--unity-property-row-height);
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
}

.property-name {
  color: var(--text-color);
}

.property-type {
  color: var(--text-secondary);
}

.property-name-drag {
  width: 100%;
  min-height: var(--unity-property-row-height);
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: ew-resize;
  user-select: none;
}

.property-name-drag:hover,
.property-name-drag:focus-visible,
.property-name-drag.dragging {
  color: var(--text-color);
}

.property-name-drag:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 2px;
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

.object-container.hide-root-header {
  gap: 0;
}

.object-container.hide-root-header > .property-children {
  padding-left: 0;
  border-left: 0;
}

.array-container > .property-children {
  gap: 4px;
  padding-left: 0;
  border-left: 0;
}

.array-header-title {
  min-width: 0;
  display: grid;
  grid-template-columns: 22px minmax(0, 1fr);
  align-items: center;
  gap: 2px;
}

.array-fold-button {
  width: 22px;
  min-width: 22px;
  min-height: 22px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-color: transparent;
  background: transparent;
  color: var(--text-secondary);
}

.array-fold-button:not(.collapsed) svg {
  transform: rotate(90deg);
}

.array-fold-button:hover,
.array-fold-button:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
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

.property-leaf.hide-leaf-label {
  grid-template-columns: minmax(0, 1fr);
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
  position: relative;
  min-width: 0;
  display: grid;
  grid-template-columns: 42px minmax(0, 1fr) 24px;
  align-items: start;
  gap: 6px;
  padding: 2px 0;
  border-radius: 6px;
}

.array-item::before,
.array-item::after {
  content: "";
  position: absolute;
  left: 48px;
  right: 0;
  height: 2px;
  border-radius: 999px;
  background: var(--accent-color);
  opacity: 0;
  pointer-events: none;
}

.array-item::before {
  top: -2px;
}

.array-item::after {
  bottom: -2px;
}

.array-item.drop-before::before,
.array-item.drop-after::after {
  opacity: 0.82;
}

.array-item.dragging {
  opacity: 0.58;
}

.array-item.preview-moving {
  background: var(--hover-bg);
}

.array-item-content {
  min-width: 0;
}

.array-index-handle {
  min-width: 0;
  min-height: var(--unity-property-row-height);
  padding: 0 6px;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  border-color: transparent;
  background: transparent;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  cursor: grab;
  user-select: none;
}

.array-index-handle:hover:not(:disabled),
.array-index-handle:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
}

.array-index-handle:active {
  cursor: grabbing;
}

.array-index-handle:disabled {
  opacity: 1;
  cursor: default;
}

.array-item-index {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
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

button:focus-visible {
  outline: 1px solid var(--accent-color);
  outline-offset: 2px;
}

.array-add-button {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.array-icon-button {
  width: 24px;
  min-width: 24px;
  min-height: 24px;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-color: transparent;
  background: transparent;
  color: var(--text-secondary);
}

.array-icon-button:hover:not(:disabled),
.array-icon-button:focus-visible {
  border-color: var(--border-color);
  background: var(--hover-bg);
  color: var(--text-color);
}

.array-remove-button:hover:not(:disabled),
.array-remove-button:focus-visible {
  border-color: var(--status-danger-border);
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
}

button:disabled,
.array-size-input:disabled,
.managed-type-search:disabled,
.managed-type-select:disabled {
  opacity: 0.58;
}

button.property-name-drag {
  min-height: var(--unity-property-row-height);
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  font-size: inherit;
}
</style>
