<script setup lang="ts">
import { computed, ref, watch } from "vue";
import BaseButton from "../ui/BaseButton.vue";
import {
  parseUnityPropertyFence,
  type UnityPropertyFenceEntry,
  type UnityPropertyFenceIssue,
} from "../../composables/unityPropertyFence";
import { normalizeAppError } from "../../services/errors";
import {
  readUnitySerializedProperty,
  writeUnitySerializedProperty,
  type UnitySerializedPropertyTarget,
} from "../../services/unitySerializedProperty";
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

const rows = ref<PropertyRow[]>([]);
const issues = ref<UnityPropertyFenceIssue[]>([]);
const loading = computed(() => rows.value.some((row) => row.loading));
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
      error: normalizeAppError(error).message,
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
      error: normalizeAppError(error).message,
    });
  }
}

function targetMeta(target: UnitySerializedPropertyTarget): string {
  if (target.kind === "component") {
    const index = Number.isFinite(target.componentIndex) && Number(target.componentIndex) > 0
      ? `[${target.componentIndex}]`
      : "";
    return `${target.componentType || "Component"}${index}`;
  }
  if (target.kind === "gameObject") return "GameObject";
  return target.kind || "Unity";
}
</script>

<template>
  <section class="unity-property-fence">
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
        v-for="row in rows"
        :key="row.entry.id"
        class="unity-property-row"
        :class="{ saving: row.saving }"
      >
        <div class="unity-property-context">
          <div class="unity-property-object" :title="row.entry.objectLabel">
            {{ row.entry.objectLabel }}
          </div>
          <div class="unity-property-target" :title="row.entry.target.propertyPath || row.entry.propertyLabel">
            {{ targetMeta(row.entry.target) }}
          </div>
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
      </article>
    </div>
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

.unity-property-row {
  min-width: 0;
  display: grid;
  grid-template-columns: minmax(150px, 0.34fr) minmax(0, 1fr);
  gap: 10px;
  padding: 8px 10px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}

.unity-property-row:last-child {
  border-bottom: 0;
}

.unity-property-row.saving {
  background: color-mix(in srgb, var(--hover-bg) 42%, transparent);
}

.unity-property-context {
  min-width: 0;
  display: grid;
  align-content: center;
  gap: 2px;
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
  .unity-property-row {
    grid-template-columns: minmax(0, 1fr);
    gap: 6px;
  }
}
</style>
