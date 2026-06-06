<script setup lang="ts">
import { computed, ref, watch } from "vue";
import BaseButton from "../ui/BaseButton.vue";
import BaseContextMenu from "../ui/BaseContextMenu.vue";
import {
  groupUnityPropertyFenceItems,
  parseUnityPropertyFence,
  type UnityPropertyFenceBlock as UnityPropertyFenceGroup,
  type UnityPropertyFenceEntry,
  type UnityPropertyFenceIssue,
  unityPropertyFenceDuplicateObjectLabels,
  unityPropertyFenceObjectLabelKey,
  unityPropertyFenceUnitySelectionTarget,
} from "../../composables/unityPropertyFence";
import { t } from "../../i18n";
import { isUnityConnectionError, normalizeAppError } from "../../services/errors";
import {
  classifyUnitySceneObjectError,
  selectUnityAsset,
  selectUnitySceneObject,
} from "../../services/unity";
import {
  readUnitySerializedProperty,
  writeUnitySerializedProperty,
  type UnitySerializedPropertyTarget,
} from "../../services/unitySerializedProperty";
import { useNotificationStore } from "../../stores/notification";
import UnitySerializedPropertyTree from "./UnitySerializedPropertyTree.vue";
import type {
  UnitySerializedPropertyCommitEvent,
  UnitySerializedPropertySnapshot,
} from "./unitySerializedValue";

const props = defineProps<{
  source: string;
}>();

interface PropertyRow {
  entry: UnityPropertyFenceEntry;
  loading: boolean;
  saving: boolean;
  error: string;
  property: UnitySerializedPropertySnapshot | null;
}

interface PropertyRowContextMenu {
  x: number;
  y: number;
  rowId: string;
}

const notificationStore = useNotificationStore();
const rows = ref<PropertyRow[]>([]);
const issues = ref<UnityPropertyFenceIssue[]>([]);
const selectedRowId = ref("");
const rowContextMenu = ref<PropertyRowContextMenu | null>(null);
const loading = computed(() => rows.value.some((row) => row.loading));
const propertyBlocks = computed(() =>
  groupUnityPropertyFenceItems(rows.value, (row) => row.entry),
);
const duplicateObjectLabels = computed(() =>
  unityPropertyFenceDuplicateObjectLabels(propertyBlocks.value.map((block) => block.entry)),
);
const rowContextRow = computed(() =>
  rowContextMenu.value ? rowById(rowContextMenu.value.rowId) : null,
);
const rowContextUnitySelection = computed(() => {
  const row = rowContextRow.value;
  return row ? unityPropertyFenceUnitySelectionTarget(rowTarget(row)) : null;
});
const rowContextCanSelectInUnity = computed(() => rowContextUnitySelection.value !== null);
let loadRun = 0;

watch(
  () => props.source,
  () => {
    void reloadProperties();
  },
  { immediate: true },
);

async function reloadProperties() {
  const run = ++loadRun;
  const parsed = parseUnityPropertyFence(props.source);
  issues.value = parsed.issues;
  rows.value = parsed.entries.map((entry) => ({
    entry,
    loading: true,
    saving: false,
    error: "",
    property: null,
  }));

  await Promise.all(rows.value.map((row) => loadProperty(row.entry, run)));
}

async function loadProperty(entry: UnityPropertyFenceEntry, run = loadRun) {
  patchRow(entry.id, { loading: true, error: "" });
  try {
    const result = await readUnitySerializedProperty({
      bindingId: entry.id,
      target: entry.target,
      maxDepth: 2,
      maxArrayItems: 32,
    });
    if (run !== loadRun) return;
    if (!result.ok) throw new Error(result.message || "Failed to read Unity property.");
    patchRow(entry.id, {
      loading: false,
      property: snapshotWithTarget(result, entry.target),
    });
  } catch (error) {
    if (run !== loadRun) return;
    patchRow(entry.id, {
      loading: false,
      error: unityPropertyErrorMessage(error),
      property: null,
    });
  }
}

function patchRow(id: string, patch: Partial<Omit<PropertyRow, "entry">>) {
  rows.value = rows.value.map((row) =>
    row.entry.id === id ? { ...row, ...patch } : row,
  );
}

function snapshotWithTarget(
  property: UnitySerializedPropertySnapshot,
  target: UnitySerializedPropertyTarget,
): UnitySerializedPropertySnapshot {
  return {
    ...property,
    bindingTarget: property.bindingTarget ?? target,
    target: property.target ?? target,
  };
}

function targetWithPropertyPath(
  target: UnitySerializedPropertyTarget,
  propertyPath: string,
): UnitySerializedPropertyTarget {
  return {
    ...target,
    propertyPath,
  };
}

async function commitProperty(row: PropertyRow, event: UnitySerializedPropertyCommitEvent) {
  const propertyPath = event.propertyPath || row.entry.target.propertyPath || row.property?.propertyPath || "";
  if (!propertyPath) return;
  const target = targetWithPropertyPath(row.entry.target, propertyPath);

  patchRow(row.entry.id, { saving: true, error: "" });
  try {
    const result = await writeUnitySerializedProperty({
      bindingId: row.entry.id,
      target,
      value: event.value,
      writeMode: "commit",
    });
    if (!result.ok) throw new Error(result.message || "Failed to write Unity property.");
    patchRow(row.entry.id, {
      saving: false,
      property: snapshotWithTarget(result, target),
    });
  } catch (error) {
    patchRow(row.entry.id, {
      saving: false,
      error: unityPropertyErrorMessage(error),
    });
  }
}

function unityPropertyErrorMessage(error: unknown): string {
  const normalized = normalizeAppError(error);
  return isUnityConnectionError(normalized)
    ? t("asset.preview.unityConnectionRequired")
    : normalized.message;
}

function targetMeta(target: UnitySerializedPropertyTarget): string {
  if (target.kind === "component") {
    const index = Number.isFinite(target.componentIndex) && Number(target.componentIndex) > 0
      ? `[${target.componentIndex}]`
      : "";
    return `${shortTypeName(target.componentType || "Component")}${index}`;
  }
  if (target.kind === "gameObject") return "GameObject";
  const targetType = shortTypeName(target.targetTypeName || target.targetTypeFullName || "");
  if (targetType) return targetType;
  return target.kind || "Unity";
}

function rowTarget(row: PropertyRow): UnitySerializedPropertyTarget {
  return row.property?.target ?? row.property?.bindingTarget ?? row.entry.target;
}

function rowById(rowId: string): PropertyRow | null {
  return rows.value.find((row) => row.entry.id === rowId) ?? null;
}

function selectRow(row: PropertyRow) {
  selectedRowId.value = row.entry.id;
}

function openRowContextMenu(event: MouseEvent) {
  const target = event.target;
  if (!(target instanceof Element)) return;
  const rowElement = target.closest<HTMLElement>("[data-unity-property-row-id]");
  const rowId = rowElement?.dataset.unityPropertyRowId?.trim() ?? "";
  const row = rowId ? rowById(rowId) : null;
  if (!row) return;

  event.preventDefault();
  event.stopPropagation();
  selectRow(row);
  rowContextMenu.value = {
    x: event.clientX,
    y: event.clientY,
    rowId: row.entry.id,
  };
}

function closeRowContextMenu() {
  rowContextMenu.value = null;
  selectedRowId.value = "";
}

async function selectContextRowInUnity() {
  const row = rowContextRow.value;
  const selection = rowContextUnitySelection.value;
  if (!row || !selection) return;
  selectRow(row);
  closeRowContextMenu();

  try {
    if (selection.kind === "sceneObject") {
      await selectUnitySceneObject(selection.scenePath, selection.objectPath);
      return;
    }
    await selectUnityAsset(selection.path);
  } catch (error) {
    notifySelectInUnityError(error, selection);
  }
}

function notifySelectInUnityError(
  error: unknown,
  selection: NonNullable<ReturnType<typeof unityPropertyFenceUnitySelectionTarget>>,
) {
  const normalized = normalizeAppError(error);
  const message = selection.kind === "sceneObject"
    ? unitySceneObjectErrorMessage(error, selection.scenePath, selection.objectPath)
    : normalized.message || "Failed to select in Unity.";
  notificationStore.addNotice("warning", message, {
    code: normalized.code,
    operation: "unityPropertySelectInUnity",
    replaceOperation: true,
  });
}

function unitySceneObjectErrorMessage(error: unknown, scenePath: string, objectPath: string): string {
  const kind = classifyUnitySceneObjectError(error);
  if (kind === "sceneNotLoaded") return t("chat.sceneObject.sceneNotLoaded", scenePath);
  if (kind === "objectMissing") return t("chat.sceneObject.objectMissing", objectPath);
  return t("chat.sceneObject.openFailed", `${scenePath}/${objectPath}`);
}

function blockTarget(row: PropertyRow | undefined): UnitySerializedPropertyTarget {
  return row ? rowTarget(row) : { kind: "Unity" };
}

function blockSaving(blockRows: PropertyRow[]): boolean {
  return blockRows.some((row) => row.saving);
}

function blockObjectPath(block: UnityPropertyFenceGroup<PropertyRow>): string {
  const labelKey = unityPropertyFenceObjectLabelKey(block.entry.objectLabel);
  if (!duplicateObjectLabels.value.has(labelKey)) return "";
  return block.entry.objectTitle.trim();
}

function shortTypeName(typeName: string): string {
  const normalized = typeName.trim();
  if (!normalized) return "";
  const withoutAssembly = normalized.includes(",")
    ? normalized.slice(0, normalized.indexOf(",")).trim()
    : normalized;
  const dot = withoutAssembly.lastIndexOf(".");
  return dot >= 0 ? withoutAssembly.slice(dot + 1) : withoutAssembly;
}
</script>

<template>
  <section class="unity-property-fence" @contextmenu.capture="openRowContextMenu">
    <header class="unity-property-fence-header">
      <div class="unity-property-fence-title">
        <span>Unity Property</span>
        <span v-if="rows.length" class="unity-property-fence-count">{{ rows.length }}</span>
      </div>
      <BaseButton
        type="button"
        size="sm"
        :disabled="loading"
        @click="reloadProperties"
      >
        Refresh
      </BaseButton>
    </header>

    <div v-if="issues.length" class="unity-property-issues">
      <div v-for="issue in issues" :key="`${issue.line}:${issue.source}`" class="unity-property-state error">
        Line {{ issue.line }}: {{ issue.message }}
      </div>
    </div>

    <div v-if="!rows.length && !issues.length" class="unity-property-state">
      No Unity properties.
    </div>

    <div v-else class="unity-property-list">
      <article
        v-for="block in propertyBlocks"
        :key="block.id"
        class="unity-property-block"
        :class="{ saving: blockSaving(block.items) }"
      >
        <div
          v-for="(row, rowIndex) in block.items"
          :key="row.entry.id"
          class="unity-property-editor-row"
          :class="{ saving: row.saving, selected: selectedRowId === row.entry.id }"
          :data-unity-property-row-id="row.entry.id"
        >
          <div class="unity-property-context" :class="{ empty: rowIndex > 0 }">
            <template v-if="rowIndex === 0">
              <div class="unity-property-object" :title="block.entry.objectTitle || block.entry.objectLabel">
                {{ block.entry.objectLabel }}
              </div>
              <div class="unity-property-target" :title="targetMeta(blockTarget(block.items[0]))">
                {{ targetMeta(blockTarget(block.items[0])) }}
              </div>
              <div
                v-if="blockObjectPath(block)"
                class="unity-property-object-path"
                :title="blockObjectPath(block)"
              >
                {{ blockObjectPath(block) }}
              </div>
            </template>
          </div>

          <div class="unity-property-editor-cell">
            <div v-if="row.loading" class="unity-property-state">Loading...</div>
            <div v-else-if="row.error" class="unity-property-state error">{{ row.error }}</div>
            <UnitySerializedPropertyTree
              v-else-if="row.property"
              :property="row.property"
              compact
              @commit="commitProperty(row, $event)"
            />
          </div>
        </div>
      </article>
    </div>

    <BaseContextMenu
      v-if="rowContextMenu && rowContextRow"
      :x="rowContextMenu.x"
      :y="rowContextMenu.y"
      :min-width="176"
      @close="closeRowContextMenu"
    >
      <button
        type="button"
        class="unity-property-ctx-item"
        :disabled="!rowContextCanSelectInUnity"
        @click="selectContextRowInUnity"
      >
        {{ t("common.selectInUnity") }}
      </button>
    </BaseContextMenu>
  </section>
</template>

<style scoped>
.unity-property-fence {
  width: min(760px, 100%);
  min-width: 0;
  margin: 4px 0 12px;
  border: 1px solid color-mix(in srgb, var(--border-color) 86%, transparent);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--sidebar-bg) 10%);
  color: var(--text-color);
  overflow: hidden;
}

.unity-property-fence-header {
  min-height: 34px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 4px 8px 4px 10px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 84%, transparent);
  background: color-mix(in srgb, var(--sidebar-bg) 64%, var(--panel-bg) 36%);
}

.unity-property-fence-title {
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 7px;
  color: var(--text-color);
  font-size: 12px;
  font-weight: 600;
  line-height: 1.2;
}

.unity-property-fence-count {
  color: var(--text-secondary);
  font-weight: 500;
}

.unity-property-issues {
  display: grid;
  gap: 1px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.unity-property-list {
  min-width: 0;
  display: grid;
}

.unity-property-block {
  min-width: 0;
  display: grid;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.unity-property-block:last-child {
  border-bottom: 0;
}

.unity-property-block.saving {
  background: color-mix(in srgb, var(--hover-bg) 42%, transparent);
}

.unity-property-context {
  min-width: 0;
  display: grid;
  align-content: center;
  gap: 2px;
}

.unity-property-context.empty {
  min-height: 1px;
}

.unity-property-object,
.unity-property-target {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.unity-property-object {
  color: var(--text-color);
  font-family: var(--font-mono-inline);
  font-size: 12px;
  line-height: 1.35;
}

.unity-property-target {
  color: var(--text-secondary);
  font-size: 11px;
  line-height: 1.25;
}

.unity-property-object-path {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-secondary);
  font-family: var(--font-mono-inline);
  font-size: 11px;
  line-height: 1.25;
}

.unity-property-editor-row {
  position: relative;
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(150px, 0.34fr) minmax(0, 1fr);
  align-items: center;
  gap: 10px;
  padding: 7px 10px;
}

.unity-property-editor-row + .unity-property-editor-row {
  border-top: 1px solid color-mix(in srgb, var(--border-color) 54%, transparent);
}

.unity-property-editor-row.saving {
  background: color-mix(in srgb, var(--hover-bg) 42%, transparent);
}

.unity-property-editor-row:hover,
.unity-property-editor-row.selected {
  background: color-mix(in srgb, var(--hover-bg) 48%, transparent);
}

.unity-property-editor-row.selected::before {
  content: "";
  position: absolute;
  left: 0;
  top: 5px;
  bottom: 5px;
  width: 2px;
  border-radius: 999px;
  background: var(--accent-color);
}

.unity-property-editor-cell {
  min-width: 0;
  align-self: center;
}

.unity-property-state {
  min-height: 26px;
  display: flex;
  align-items: center;
  padding: 0 8px;
  color: var(--text-secondary);
  font-size: 12px;
  line-height: 1.35;
}

.unity-property-state.error {
  color: var(--status-danger-fg);
}

@media (max-width: 720px) {
  .unity-property-editor-row {
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
  }

  .unity-property-context.empty {
    display: none;
  }

  .unity-property-editor-row {
    padding: 6px 10px 8px;
  }
}
</style>
