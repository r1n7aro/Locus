pub(super) fn app_vue(_name: &str) -> String {
    r##"<script setup lang="ts">
import { onMounted, ref } from "vue";
import { property } from "@locus/view-runtime";
import type { UnitySerializedPropertySnapshot } from "@locus/view-runtime";
import { CanvasView, UnityPropertyEditor, UnitySerializedPropertyTree } from "@locus/components";
import type { CanvasViewExpose } from "@locus/components";

type PropertyTarget = Exclude<Parameters<typeof property.readProperty>[0], string>;

interface FieldBinding {
  id: string;
  label: string;
  target: PropertyTarget;
  valueType: string;
  value: unknown;
  displayValue: string;
  property: UnitySerializedPropertySnapshot | null;
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

function selectionNameField(): FieldBinding {
  return {
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
  };
}

const canvasRef = ref<CanvasViewExpose | null>(null);
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
    fields: [selectionNameField()],
  },
]);

function blockClass(_block: FieldBlock, selected: boolean) {
  return ["field-block", selected ? "selected" : ""];
}

function fitCanvas() {
  canvasRef.value?.fitContent();
}

function fieldTarget(field: FieldBinding, propertyPath = ""): PropertyTarget {
  return propertyPath ? { ...field.target, propertyPath } : field.target;
}

function applySnapshotToField(field: FieldBinding, snapshot: UnitySerializedPropertySnapshot) {
  field.valueType = String(snapshot.valueType || field.valueType);
  field.value = snapshot.value;
  field.displayValue = String(snapshot.displayValue ?? snapshot.value ?? "");
  field.property = snapshot;
  field.editable = snapshot.editable !== false;
}

async function readField(field: FieldBinding) {
  field.status = "Reading";
  field.error = "";
  try {
    const bound = await property.readProperty(fieldTarget(field));
    applySnapshotToField(field, bound.raw.snapshot as UnitySerializedPropertySnapshot);
    field.status = "Ready";
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
    const result = await property.write(fieldTarget(field, propertyPath), value);
    await readField(field);
    field.status = result.saved ? "Saved" : field.status;
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

function usesPropertyTree(field: FieldBinding): boolean {
  const snapshot = field.property;
  if (!snapshot) return false;
  return !!snapshot.isArray
    || !!snapshot.isManagedReference
    || !!snapshot.isFlagsEnum
    || (snapshot.children?.length ?? 0) > 0;
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
    fields: [selectionNameField()],
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
          <div v-for="field in item.fields" :key="field.id" class="field-row">
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
              :enum-options="field.property?.enumOptions ?? []"
              :is-flags-enum="!!field.property?.isFlagsEnum"
              :enum-value-index="field.property?.enumValueIndex ?? -1"
              :enum-value-flag="field.property?.enumValueFlag ?? 0"
              :reference-type-full-name="field.property?.referenceTypeFullName ?? ''"
              :reference-type-assembly="field.property?.referenceTypeAssembly ?? ''"
              :title="field.target.propertyPath || ''"
              @commit="commitField(field, $event)"
            />
            <small class="field-status" :class="{ error: !!field.error }">
              {{ field.error || field.status }}
            </small>
          </div>
        </div>
      </template>
    </CanvasView>
  </main>
</template>
"##
    .to_string()
}

pub(super) fn style_css() -> String {
    super::common::style_css(
        r#".field-blocks-view > .locus-canvas-view {
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
"#,
    )
}
