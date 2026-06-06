<script setup lang="ts">
import { computed, ref } from "vue";
import UnitySerializedPropertyTree from "../unity/UnitySerializedPropertyTree.vue";
import {
  createInspectorPropertyTreeBinding,
  type InspectorPropertyDrawerInput,
  type InspectorPropertySnapshot,
  type InspectorPropertyTreeBinding,
  type InspectorPropertyTreeBindingInput,
} from "../../services/propertyTree";
import type { UnitySerializedPropertyCommitEvent } from "../unity/unitySerializedValue";
import {
  normalizeUnityObjectPreviewModel,
  type UnityObjectPreviewInput,
  type UnityObjectPreviewModel,
} from "./unityObjectPreview";
import UnityObjectIdentity from "./UnityObjectIdentity.vue";

const props = withDefaults(defineProps<{
  model: UnityObjectPreviewInput | UnityObjectPreviewModel;
  disabled?: boolean;
  readonly?: boolean;
  compact?: boolean;
  showHeader?: boolean;
  propertyTree?: InspectorPropertyTreeBindingInput | null;
  propertyDrawers?: InspectorPropertyDrawerInput;
}>(), {
  disabled: false,
  readonly: false,
  compact: false,
  showHeader: true,
  propertyTree: null,
  propertyDrawers: undefined,
});

const emit = defineEmits<{
  commit: [event: UnitySerializedPropertyCommitEvent];
  preview: [event: UnitySerializedPropertyCommitEvent];
  blocked: [model: UnityObjectPreviewModel];
}>();

const objectModel = computed(() => normalizeUnityObjectPreviewModel(props.model));
const propertyTreeBinding = computed<InspectorPropertyTreeBinding>(() =>
  createInspectorPropertyTreeBinding({
    ...(props.propertyTree ?? {}),
    snapshots: props.propertyTree?.snapshots ?? objectModel.value.propertyTree ?? null,
    disabled: props.disabled || props.propertyTree?.disabled === true,
    readonly: props.readonly || props.propertyTree?.readonly === true,
    editable: props.propertyTree?.editable ?? objectModel.value.capabilities.edit,
  }),
);
const canEdit = computed(() =>
  propertyTreeBinding.value.editable &&
  !propertyTreeBinding.value.disabled &&
  !propertyTreeBinding.value.readonly,
);
const propertyTrees = computed(() =>
  Array.isArray(propertyTreeBinding.value.snapshots)
    ? propertyTreeBinding.value.snapshots
    : propertyTreeBinding.value.snapshots
      ? [propertyTreeBinding.value.snapshots]
      : [],
);
const editorStateLabel = computed(() =>
  propertyTreeBinding.value.error
    || (propertyTreeBinding.value.loading ? "Loading properties..." : objectModel.value.readonlyReason || "No editable properties"),
);
const collapsedProperties = ref<Set<string>>(new Set());

function targetIdForProperty(property: InspectorPropertySnapshot): string {
  const target = property.bindingTarget ?? property.target;
  if (!target) return propertyTreeBinding.value.targetId;
  return [
    target.kind,
    target.path ?? "",
    target.scenePath ?? "",
    target.objectPath ?? "",
    target.componentType ?? "",
    target.componentIndex ?? "",
  ].join("|");
}

function propertyKey(property: InspectorPropertySnapshot, index: number): string {
  const targetId = targetIdForProperty(property);
  return targetId || property.propertyPath || `${property.displayName || property.name || "property"}:${index}`;
}

function propertyTitle(property: InspectorPropertySnapshot): string {
  return property.displayName || property.name || property.propertyPath || "Component";
}

function propertyTypeLabel(property: InspectorPropertySnapshot): string {
  const target = property.bindingTarget ?? property.target;
  const componentType = target?.componentType?.trim();
  const label = componentType
    ? componentType.split(".").filter(Boolean).pop() || componentType
    : property.valueType || property.type || "";
  const normalizedLabel = label.replace(/\s+/g, "").toLowerCase();
  const normalizedTitle = propertyTitle(property)
    .replace(/\(script\)/gi, "")
    .replace(/\s+/g, "")
    .toLowerCase();
  if (!label || label === "Object" || normalizedTitle.includes(normalizedLabel)) return "";
  return label;
}

function componentClass(property: InspectorPropertySnapshot): string {
  const target = property.bindingTarget ?? property.target;
  if (!target?.componentType) return "game-object";
  if (propertyTitle(property).toLowerCase().includes("(script)")) return "script";
  return "component";
}

function propertyPathLeaf(propertyPath: string | undefined): string {
  const normalized = (propertyPath || "").trim();
  if (!normalized) return "";
  const dot = normalized.lastIndexOf(".");
  return dot >= 0 ? normalized.slice(dot + 1) : normalized;
}

function isComponentEnableProperty(property: InspectorPropertySnapshot): boolean {
  if ((property.valueType || property.type) !== "Boolean") return false;
  const leaf = propertyPathLeaf(property.propertyPath);
  return leaf === "m_Enabled" || leaf === "m_IsActive";
}

function componentEnableProperty(property: InspectorPropertySnapshot): InspectorPropertySnapshot | null {
  return (property.children ?? []).find(isComponentEnableProperty) ?? null;
}

function componentBodySnapshot(property: InspectorPropertySnapshot): InspectorPropertySnapshot {
  const enableProperty = componentEnableProperty(property);
  const children = property.children ?? [];
  if (!enableProperty || !children.length) return property;
  const bodyChildren = children.filter((child) => child.propertyPath !== enableProperty.propertyPath);
  return {
    ...property,
    children: bodyChildren,
    hasChildren: bodyChildren.length > 0,
  };
}

function componentEnabled(property: InspectorPropertySnapshot): boolean {
  const enableProperty = componentEnableProperty(property);
  if (!enableProperty) return true;
  if (typeof enableProperty.value === "boolean") return enableProperty.value;
  return String(enableProperty.displayValue ?? enableProperty.value ?? "").trim().toLowerCase() === "true";
}

function canEditComponentEnable(property: InspectorPropertySnapshot): boolean {
  const enableProperty = componentEnableProperty(property);
  return canEdit.value && enableProperty?.editable !== false;
}

function componentEnableTitle(property: InspectorPropertySnapshot): string {
  const enableProperty = componentEnableProperty(property);
  const leaf = propertyPathLeaf(enableProperty?.propertyPath);
  if (leaf === "m_IsActive") return "Active";
  return "Enabled";
}

function commitComponentEnable(property: InspectorPropertySnapshot, event: Event) {
  const enableProperty = componentEnableProperty(property);
  if (!enableProperty || !canEditComponentEnable(property)) return;
  const value = (event.target as HTMLInputElement | null)?.checked === true;
  emit("commit", {
    propertyPath: enableProperty.propertyPath,
    value,
    property: enableProperty as UnitySerializedPropertyCommitEvent["property"],
    target: property.bindingTarget ?? property.target ?? enableProperty.bindingTarget ?? enableProperty.target ?? null,
  });
}

function isCollapsed(key: string): boolean {
  return collapsedProperties.value.has(key);
}

function toggleProperty(key: string) {
  const next = new Set(collapsedProperties.value);
  if (next.has(key)) {
    next.delete(key);
  } else {
    next.add(key);
  }
  collapsedProperties.value = next;
}

function sourceForProperty(property: InspectorPropertySnapshot): InspectorPropertyTreeBindingInput {
  const binding = propertyTreeBinding.value;
  const targetId = targetIdForProperty(property);
  return {
    id: targetId ? `${binding.id}:${targetId}` : binding.id,
    targetId,
    snapshots: componentBodySnapshot(property),
    loading: binding.loading,
    error: binding.error,
    disabled: binding.disabled,
    readonly: binding.readonly,
    editable: binding.editable,
    commit: binding.commit,
  };
}

function handleCommit(event: UnitySerializedPropertyCommitEvent) {
  emit("commit", event);
}

function handlePreview(event: UnitySerializedPropertyCommitEvent) {
  emit("preview", event);
}
</script>

<template>
  <section class="unity-object-editor-panel" :class="{ compact }">
    <header v-if="showHeader" class="unity-object-editor-header">
      <UnityObjectIdentity
        :model="objectModel"
        mode="row"
        :draggable="false"
      />
    </header>

    <div class="unity-object-editor-body">
      <template v-if="propertyTrees.length">
        <section
          v-for="(property, index) in propertyTrees"
          :key="propertyKey(property, index)"
          class="unity-component-panel"
          :class="[
            `kind-${componentClass(property)}`,
            { collapsed: isCollapsed(propertyKey(property, index)) },
          ]"
        >
          <div class="unity-component-header">
            <button
              type="button"
              class="unity-component-fold-button"
              :aria-label="propertyTitle(property)"
              :aria-expanded="!isCollapsed(propertyKey(property, index))"
              @click="toggleProperty(propertyKey(property, index))"
            >
              <span class="unity-component-fold" aria-hidden="true">▶</span>
            </button>
            <input
              v-if="componentEnableProperty(property)"
              class="unity-component-enable"
              type="checkbox"
              :checked="componentEnabled(property)"
              :disabled="!canEditComponentEnable(property)"
              :title="componentEnableTitle(property)"
              :aria-label="`${propertyTitle(property)} ${componentEnableTitle(property)}`"
              @click.stop
              @keydown.stop
              @change="commitComponentEnable(property, $event)"
            />
            <span v-else class="unity-component-icon" aria-hidden="true" />
            <button
              type="button"
              class="unity-component-title-button"
              :aria-expanded="!isCollapsed(propertyKey(property, index))"
              @click="toggleProperty(propertyKey(property, index))"
            >
              <span class="unity-component-title">{{ propertyTitle(property) }}</span>
              <span v-if="propertyTypeLabel(property)" class="unity-component-type">{{ propertyTypeLabel(property) }}</span>
            </button>
          </div>
          <div v-if="!isCollapsed(propertyKey(property, index))" class="unity-component-body">
            <UnitySerializedPropertyTree
              :source="sourceForProperty(property)"
              :disabled="propertyTreeBinding.disabled"
              :readonly="!canEdit"
              :compact="compact"
              hide-root-object-header
              :property-drawers="propertyDrawers"
              @preview="handlePreview"
              @commit="handleCommit"
            />
          </div>
        </section>
      </template>
      <div v-else class="unity-object-editor-state">
        {{ editorStateLabel }}
      </div>
    </div>
  </section>
</template>

<style scoped>
.unity-object-editor-panel {
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-bg);
  border: 1px solid var(--border-color);
  border-radius: 8px;
  overflow: hidden;
}

.unity-object-editor-header {
  flex-shrink: 0;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 86%, var(--bg-color) 14%);
}

.unity-object-editor-body {
  min-width: 0;
  min-height: 0;
  padding: 10px;
  overflow: auto;
  display: grid;
  align-content: start;
  gap: 7px;
}

.unity-object-editor-panel.compact .unity-object-editor-body {
  padding: 8px;
  gap: 6px;
}

.unity-object-editor-state {
  padding: 12px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
}

.unity-component-panel {
  position: relative;
  min-width: 0;
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, var(--border-strong) 12%);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  overflow: visible;
}

.unity-component-panel:focus-within {
  z-index: 20;
}

.unity-component-header {
  width: 100%;
  min-height: 30px;
  display: grid;
  grid-template-columns: 18px 16px minmax(0, 1fr);
  align-items: center;
  gap: 5px;
  padding: 0 9px 0 6px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
  border-radius: 5px 5px 0 0;
  background: color-mix(in srgb, var(--sidebar-bg) 72%, var(--panel-bg) 28%);
  color: var(--text-color);
  font: inherit;
}

.unity-component-header:hover {
  background: color-mix(in srgb, var(--sidebar-bg) 62%, var(--hover-bg) 38%);
}

.unity-component-fold-button,
.unity-component-title-button {
  min-width: 0;
  min-height: 30px;
  padding: 0;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  cursor: pointer;
}

.unity-component-fold-button {
  width: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 4px;
}

.unity-component-title-button {
  display: grid;
  grid-template-columns: minmax(0, 1fr) minmax(0, max-content);
  align-items: center;
  gap: 6px;
  text-align: left;
}

.unity-component-fold-button:hover,
.unity-component-title-button:hover {
  color: var(--text-color);
}

.unity-component-fold-button:focus-visible,
.unity-component-title-button:focus-visible,
.unity-component-enable:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.unity-component-fold {
  width: 16px;
  color: var(--text-secondary);
  font-size: 10px;
  line-height: 1;
  transform: rotate(90deg);
  transition: transform 120ms ease;
}

.unity-component-panel.collapsed .unity-component-fold {
  transform: rotate(0deg);
}

.unity-component-icon {
  justify-self: center;
  width: 14px;
  height: 14px;
  border: 1px solid color-mix(in srgb, var(--accent-color) 38%, var(--border-color));
  border-radius: 3px;
  background:
    linear-gradient(135deg, transparent 0 46%, color-mix(in srgb, var(--accent-color) 70%, var(--text-color) 30%) 47% 53%, transparent 54%),
    color-mix(in srgb, var(--accent-color) 20%, var(--panel-bg) 80%);
}

.unity-component-panel.kind-script .unity-component-icon {
  border-color: color-mix(in srgb, var(--status-good-fg) 38%, var(--border-color));
  background:
    linear-gradient(90deg, transparent 0 42%, color-mix(in srgb, var(--status-good-fg) 64%, var(--text-color) 36%) 43% 57%, transparent 58%),
    color-mix(in srgb, var(--status-good-bg) 22%, var(--panel-bg) 78%);
}

.unity-component-panel.kind-game-object .unity-component-icon {
  border-color: color-mix(in srgb, var(--text-secondary) 44%, var(--border-color));
  background:
    linear-gradient(135deg, transparent 0 45%, color-mix(in srgb, var(--text-secondary) 70%, var(--text-color) 30%) 46% 54%, transparent 55%),
    color-mix(in srgb, var(--sidebar-bg) 76%, var(--panel-bg) 24%);
}

.unity-component-enable {
  justify-self: center;
  align-self: center;
  width: 13px;
  height: 13px;
  min-width: 13px;
  min-height: 13px;
  margin: 0;
  padding: 0;
  display: inline-grid;
  place-content: center;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 46%, var(--panel-bg) 54%);
  border-radius: 2px;
  background: color-mix(in srgb, var(--input-bg) 72%, var(--panel-bg) 28%);
  appearance: none;
}

.unity-component-enable::after {
  width: 7px;
  height: 4px;
  border-left: 2px solid var(--bg-color);
  border-bottom: 2px solid var(--bg-color);
  transform: translateY(-1px) rotate(-45deg);
  content: "";
  opacity: 0;
}

.unity-component-enable:checked {
  border-color: color-mix(in srgb, var(--text-secondary) 82%, var(--panel-bg) 18%);
  background: color-mix(in srgb, var(--text-secondary) 78%, var(--panel-bg) 22%);
}

.unity-component-enable:checked::after {
  opacity: 1;
}

.unity-component-enable:disabled {
  opacity: 0.55;
}

.unity-component-title {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
}

.unity-component-type {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  justify-self: end;
}

.unity-component-body {
  position: relative;
  padding: 8px 10px 9px;
  border-radius: 0 0 5px 5px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--bg-color) 10%);
}

.unity-object-editor-panel.compact .unity-component-header {
  min-height: 28px;
}

.unity-object-editor-panel.compact .unity-component-fold-button,
.unity-object-editor-panel.compact .unity-component-title-button {
  min-height: 28px;
}

.unity-object-editor-panel.compact .unity-component-body {
  padding: 7px 8px 8px;
}

.unity-component-panel.collapsed .unity-component-header {
  border-bottom-color: transparent;
  border-radius: 5px;
}
</style>
