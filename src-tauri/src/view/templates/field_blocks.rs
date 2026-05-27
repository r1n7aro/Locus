pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { onMounted, ref } from "vue";
import { CanvasView, view } from "@locus/view-runtime";
import { UnityPropertyEditor, UnitySerializedPropertyTree } from "@locus/components";

interface FieldBinding {
  id: string;
  label: string;
  target: Record<string, unknown>;
  valueType: string;
  value: unknown;
  displayValue: string;
  property: Record<string, unknown> | null;
  editable: boolean;
  status: string;
  error: string;
}

interface FieldBlock {
  id: string;
  title: string;
  subtitle: string;
  x: number;
  y: number;
  width: number;
  height: number;
  fields: FieldBinding[];
}

const canvasRef = ref(null);
const selectedBlockId = ref("selection");
const loading = ref(false);
const statusText = ref("Ready");
const blocks = ref<FieldBlock[]>([
  {
    id: "selection",
    title: "Selection",
    subtitle: "Active Unity object",
    x: 80,
    y: 80,
    width: 330,
    height: 164,
    fields: [
      {
        id: "name",
        label: "Name",
        target: { kind: "selection", propertyPath: "m_Name" },
        valueType: "String",
        value: "",
        displayValue: "",
        property: null,
        editable: true,
        status: "Idle",
        error: "",
      },
    ],
  },
]);

function blockClass(_block: FieldBlock, selected: boolean) {
  return ["field-block", selected ? "selected" : ""];
}

function fitCanvas() {
  canvasRef.value?.fitContent?.();
}

async function readField(field: FieldBinding) {
  field.status = "Reading";
  field.error = "";
  try {
    const result = await view.binding.read({ target: field.target });
    field.valueType = result.valueType || field.valueType;
    field.value = result.value;
    field.displayValue = result.displayValue || String(result.value ?? "");
    field.property = result as unknown as Record<string, unknown>;
    field.editable = !!result.editable;
    field.status = result.message || "Ready";
  } catch (error) {
    field.status = "Error";
    field.error = error instanceof Error ? error.message : String(error);
    console.error("[field-blocks] Read failed", error);
  }
}

async function refreshAll() {
  loading.value = true;
  statusText.value = "Reading";
  try {
    for (const block of blocks.value) {
      for (const field of block.fields) {
        await readField(field);
      }
    }
    statusText.value = "Ready";
  } finally {
    loading.value = false;
  }
}

async function commitField(field: FieldBinding, value: unknown, propertyPath = "") {
  if (!field.editable) return;
  field.status = "Saving";
  field.error = "";
  try {
    const target = propertyPath
      ? { ...field.target, propertyPath }
      : field.target;
    const result = await view.binding.write({
      target,
      value,
    });
    field.valueType = result.valueType || field.valueType;
    field.value = result.value;
    field.displayValue = result.displayValue || String(result.value ?? "");
    field.property = propertyPath
      ? (await view.binding.read({ target: field.target }) as unknown as Record<string, unknown>)
      : result as unknown as Record<string, unknown>;
    field.editable = !!result.editable;
    field.status = result.saved ? "Saved" : result.message || "Ready";
    statusText.value = "Saved";
  } catch (error) {
    field.status = "Error";
    field.error = error instanceof Error ? error.message : String(error);
    console.error("[field-blocks] Write failed", error);
    statusText.value = "Write failed";
  }
}

async function commitFieldProperty(field: FieldBinding, event: { propertyPath: string; value: unknown }) {
  await commitField(field, event.value, event.propertyPath);
}

function usesPropertyTree(field: FieldBinding) {
  const property = field.property;
  if (!property) return false;
  return !!property.isArray
    || !!property.isManagedReference
    || !!property.isFlagsEnum
    || (Array.isArray(property.children) && property.children.length > 0);
}

function propertyArray(field: FieldBinding, key: string): unknown[] {
  const value = field.property?.[key];
  return Array.isArray(value) ? value : [];
}

function propertyNumber(field: FieldBinding, key: string, fallback = 0): number {
  const value = Number(field.property?.[key]);
  return Number.isFinite(value) ? value : fallback;
}

function addSelectionNameBlock() {
  const index = blocks.value.length + 1;
  const id = `selection-${index}`;
  blocks.value.push({
    id,
    title: `Selection ${index}`,
    subtitle: "Bound field block",
    x: 120 + index * 38,
    y: 130 + index * 34,
    width: 330,
    height: 164,
    fields: [
      {
        id: "name",
        label: "Name",
        target: { kind: "selection", propertyPath: "m_Name" },
        valueType: "String",
        value: "",
        displayValue: "",
        property: null,
        editable: true,
        status: "Idle",
        error: "",
      },
    ],
  });
  selectedBlockId.value = id;
}

onMounted(() => {
  void refreshAll();
});
</script>

<template>
  <main class="view-shell field-blocks-view" data-locus-template="field-blocks">
    <header class="view-toolbar">
      <div class="toolbar-title">
        <span>Field Blocks</span>
        <small>{{ statusText }}</small>
      </div>
      <div class="toolbar-actions">
        <button type="button" :disabled="loading" @click="refreshAll">{{ loading ? "Reading" : "Refresh" }}</button>
        <button type="button" @click="fitCanvas">Fit</button>
        <button type="button" @click="addSelectionNameBlock">Add</button>
      </div>
    </header>

    <CanvasView
      ref="canvasRef"
      v-model:selected-item-id="selectedBlockId"
      :items="blocks"
      :item-class="blockClass"
    >
      <template #default="{ item }">
        <div class="field-block-header">
          <div class="field-block-title">
            <span>{{ item.title }}</span>
            <small>{{ item.subtitle }}</small>
          </div>
        </div>
        <div class="field-block-body">
          <label v-for="field in item.fields" :key="field.id" class="field-row">
            <span class="field-label">{{ field.label }}</span>
            <UnitySerializedPropertyTree
              v-if="usesPropertyTree(field)"
              :property="field.property"
              :disabled="field.status === 'Saving'"
              @commit="commitFieldProperty(field, $event)"
            />
            <UnityPropertyEditor
              v-else
              :model-value="field.value"
              :property-type="field.valueType"
              :display-value="field.displayValue"
              :editable="field.editable"
              :disabled="field.status === 'Saving'"
              :enum-options="propertyArray(field, 'enumOptions')"
              :is-flags-enum="!!field.property?.isFlagsEnum"
              :enum-value-index="propertyNumber(field, 'enumValueIndex', -1)"
              :enum-value-flag="propertyNumber(field, 'enumValueFlag')"
              :title="String(field.target.propertyPath || '')"
              @commit="commitField(field, $event)"
            />
            <small class="field-status" :class="{ error: !!field.error }">
              {{ field.error || field.status }}
            </small>
          </label>
        </div>
      </template>
    </CanvasView>
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    r#":root {
  color-scheme: light dark;
  font-family: var(--font-ui);
}

body {
  margin: 0;
  background: var(--bg-color);
  color: var(--text-color);
  font-family: var(--font-ui);
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
}

.view-shell {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--bg-color);
}

.view-toolbar {
  min-height: 42px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 0 10px 0 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
}

.toolbar-title {
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
}

.toolbar-title span {
  font-size: 13px;
  font-weight: 650;
}

.toolbar-title small {
  color: var(--text-secondary);
  font-size: 11px;
}

.toolbar-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

button {
  min-height: 28px;
  padding: 0 9px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--sidebar-bg) 28%);
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
}

button:disabled {
  opacity: 0.58;
}

.field-blocks-view > .locus-canvas-view {
  flex: 1;
  min-height: 0;
}

.field-block {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-strong);
  border-radius: 8px;
  background: var(--surface-elevated);
  color: var(--text-color);
  box-shadow: 0 1px 0 color-mix(in srgb, var(--border-color) 70%, transparent);
  overflow: hidden;
}

.field-block.selected {
  border-color: var(--accent-color);
}

.field-block-header {
  min-height: 38px;
  display: flex;
  align-items: center;
  padding: 0 10px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--surface-elevated) 82%, var(--sidebar-bg) 18%);
}

.field-block-title {
  min-width: 0;
  display: grid;
  gap: 1px;
}

.field-block-title span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 650;
}

.field-block-title small {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-size: 11px;
}

.field-block-body {
  display: grid;
  gap: 8px;
  padding: 10px;
}

.field-row {
  display: grid;
  grid-template-columns: 76px minmax(0, 1fr) 54px;
  align-items: center;
  gap: 8px;
  font-size: 12px;
}

.field-label,
.field-status {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
}

.field-status {
  text-align: right;
  font-size: 11px;
}

.field-status.error {
  color: var(--status-danger-fg);
}
"#
    .to_string()
}
