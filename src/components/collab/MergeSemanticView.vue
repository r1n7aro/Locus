<script setup lang="ts">
import { computed, ref, watch, nextTick, onUnmounted } from "vue";
import type { MergeField, MergeSide, MergeSessionPayload, MergeTargetInspector } from "../../types";
import { mergeSemanticTarget } from "../../services/merge";
import { normalizeAppError } from "../../services/errors";
import type { MergeResolutionState } from "../../composables/useMergeResolution";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import type { MergeDisplayStatus } from "./mergeUi";
import { compactMergeSideLabel, mergeStatusLabel, mergeStatusTone } from "./mergeUi";
import UnityHierarchyPane from "../diff/UnityHierarchyPane.vue";
import MergeInspectorPane from "./MergeInspectorPane.vue";

const props = defineProps<{
  session: MergeSessionPayload;
  resolution: MergeResolutionState;
  leftLabel?: string;
  rightLabel?: string;
  showConflictsOnly?: boolean;
}>();

const selectedTargetId = ref<string | null>(null);
const activeInspector = ref<MergeTargetInspector | null>(null);
const inspectorCache = ref<Map<string, MergeTargetInspector>>(new Map());
const inspectorLoading = ref(false);
const inspectorError = ref<string | null>(null);
let targetLoadGeneration = 0;

const compactLeft = computed(() => compactMergeSideLabel(props.leftLabel, "left"));
const compactRight = computed(() => compactMergeSideLabel(props.rightLabel, "right"));
const allTargets = computed(() => props.session.targets ?? []);
const allTree = computed(() => props.session.tree ?? []);
const isHierarchyLayout = computed(() => props.session.layout === "sceneHierarchyInspector");

function shouldShowTarget(targetId: string, mergeStatus: string): boolean {
  if (!props.showConflictsOnly) return true;
  // Always show targets with actual conflicts (not auto-resolved)
  if (mergeStatus === "hasConflicts") return true;
  // Always show targets where user has already made resolution choices
  const derivedStatus = targetStatusMap.value.get(targetId);
  if (derivedStatus === "stagedResolved" || derivedStatus === "stagedPartial") return true;
  if (props.resolution.targetResolutions.value.has(targetId)) return true;
  return false;
}

const targets = computed(() => {
  if (!props.showConflictsOnly) return allTargets.value;
  return allTargets.value.filter((t) => shouldShowTarget(t.id, t.mergeStatus));
});

const filteredTargetIds = computed(() => new Set(targets.value.map((t) => t.id)));

const tree = computed(() => {
  if (!props.showConflictsOnly) return allTree.value;
  // Collect nodes that match + all their ancestors to preserve hierarchy
  const keepIds = new Set<string>();
  const nodeMap = new Map(allTree.value.map((n) => [n.id, n]));
  for (const id of filteredTargetIds.value) {
    let cur: string | null | undefined = id;
    while (cur && !keepIds.has(cur)) {
      keepIds.add(cur);
      cur = nodeMap.get(cur)?.parentId;
    }
  }
  return allTree.value.filter((n) => keepIds.has(n.id));
});

const selectedNode = computed(() => {
  if (!selectedTargetId.value) return null;
  return tree.value.find(n => n.id === selectedTargetId.value) ?? null;
});

function collectConflictProgress(fields: MergeField[]): { total: number; resolved: number } {
  let total = 0;
  let resolved = 0;

  const walk = (field: MergeField) => {
    if (field.children.length === 0) {
      if (field.mergeState === "conflict") {
        total += 1;
        if (props.resolution.fieldResolutions.value.has(field.id)) {
          resolved += 1;
        }
      }
      return;
    }

    for (const child of field.children) {
      walk(child);
    }
  };

  for (const field of fields) {
    walk(field);
  }

  return { total, resolved };
}

function deriveTargetStatus(targetId: string, baseStatus: MergeDisplayStatus, conflictCount: number): MergeDisplayStatus {
  if (conflictCount <= 0) return baseStatus;
  if (props.resolution.targetResolutions.value.has(targetId)) return "stagedResolved";

  const inspector = inspectorCache.value.get(targetId)
    ?? (activeInspector.value?.targetId === targetId ? activeInspector.value : null);
  if (!inspector) return baseStatus;

  const progress = collectConflictProgress(inspector.panels.flatMap((panel) => panel.fields));
  if (progress.total <= 0) return baseStatus;
  if (progress.resolved >= progress.total) return "stagedResolved";
  if (progress.resolved > 0) return "stagedPartial";
  return baseStatus;
}

const targetStatusMap = computed(() => {
  const map = new Map<string, MergeDisplayStatus>();
  for (const target of allTargets.value) {
    map.set(target.id, deriveTargetStatus(target.id, target.mergeStatus, target.conflictCount));
  }
  return map;
});

const hierarchyChangeKindOverrides = computed<Record<string, string>>(() => {
  const overrides: Record<string, string> = {};
  for (const target of targets.value) {
    const status = targetStatusMap.value.get(target.id);
    if (status === "stagedResolved") overrides[target.id] = "resolved";
    else if (status === "stagedPartial") overrides[target.id] = "partial";
  }
  return overrides;
});

async function loadTarget(targetId: string) {
  const generation = ++targetLoadGeneration;
  selectedTargetId.value = targetId;
  inspectorError.value = null;

  const cached = inspectorCache.value.get(targetId);
  if (cached) {
    activeInspector.value = cached;
    inspectorLoading.value = false;
    props.resolution.registerConflictFields(cached);
    return;
  }

  activeInspector.value = null;
  inspectorLoading.value = true;
  try {
    const inspector = await mergeSemanticTarget({
      mergeKey: props.session.key,
      targetId,
    });
    if (generation !== targetLoadGeneration || selectedTargetId.value !== targetId) return;
    inspectorCache.value.set(targetId, inspector);
    activeInspector.value = inspector;
    props.resolution.registerConflictFields(inspector);
  } catch (e) {
    if (generation !== targetLoadGeneration || selectedTargetId.value !== targetId) return;
    activeInspector.value = null;
    inspectorError.value = normalizeAppError(e).message;
  } finally {
    if (generation !== targetLoadGeneration || selectedTargetId.value !== targetId) return;
    inspectorLoading.value = false;
  }
}

function onSelectTarget(targetId: string) {
  void loadTarget(targetId);
}

function onAcceptTarget(targetId: string, side: MergeSide) {
  const cached = inspectorCache.value.get(targetId);
  props.resolution.acceptTarget(targetId, side, cached ?? undefined);
}

// ── Sidebar resize ──────────────────────────────────────────────
const viewRef = ref<HTMLElement | null>(null);
const sidebarWidth = ref(
  (() => { try { return Number(localStorage.getItem("locus:merge-sidebar-w")) || 240; } catch { return 240; } })(),
);
const isDragging = ref(false);
let releaseSelectionLock: (() => void) | null = null;

function onSplitterDown(e: MouseEvent) {
  e.preventDefault();
  isDragging.value = true;
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
  document.addEventListener("mousemove", onSplitterMove);
  document.addEventListener("mouseup", onSplitterUp);
}

function onSplitterMove(e: MouseEvent) {
  if (!isDragging.value || !viewRef.value) return;
  const rect = viewRef.value.getBoundingClientRect();
  const x = e.clientX - rect.left;
  sidebarWidth.value = Math.max(160, Math.min(x, rect.width * 0.45));
}

function onSplitterUp() {
  isDragging.value = false;
  document.removeEventListener("mousemove", onSplitterMove);
  document.removeEventListener("mouseup", onSplitterUp);
  releaseSelectionLock?.();
  releaseSelectionLock = null;
  try { localStorage.setItem("locus:merge-sidebar-w", String(Math.round(sidebarWidth.value))); } catch {}
}

onUnmounted(() => {
  document.removeEventListener("mousemove", onSplitterMove);
  document.removeEventListener("mouseup", onSplitterUp);
  releaseSelectionLock?.();
  releaseSelectionLock = null;
});

watch(
  () => props.session.key,
  () => {
    targetLoadGeneration += 1;
    selectedTargetId.value = props.session.defaultTargetId ?? props.session.targets?.[0]?.id ?? null;
    activeInspector.value = null;
    inspectorCache.value = new Map();
    inspectorLoading.value = false;
    inspectorError.value = null;
    if (selectedTargetId.value) {
      void loadTarget(selectedTargetId.value);
    }
  },
  { immediate: true },
);

// ── Conflict navigation ────────────────────────────────────────
let navIndex = -1;

function getUnresolvedFieldElements(): HTMLElement[] {
  if (!viewRef.value) return [];
  return Array.from(
    viewRef.value.querySelectorAll<HTMLElement>(".merge-field-grid.conflict"),
  ).filter((el) => {
    const fieldId = el.dataset.fieldId;
    return fieldId && !props.resolution.fieldResolutions.value.has(fieldId);
  });
}

function highlightElement(el: HTMLElement) {
  el.scrollIntoView({ behavior: "smooth", block: "center" });
  el.classList.remove("conflict-highlight");
  void el.offsetWidth;
  el.classList.add("conflict-highlight");
}

function orderedUnresolvedTargetIds(): string[] {
  return targets.value
    .map((t) => t.id)
    .filter((id) => props.resolution.isTargetUnresolved(id));
}

async function navigateConflict(direction: "prev" | "next") {
  // 1) try within current target
  const elements = getUnresolvedFieldElements();
  if (elements.length > 0) {
    if (direction === "next") {
      navIndex = navIndex + 1 >= elements.length ? 0 : navIndex + 1;
    } else {
      navIndex = navIndex - 1 < 0 ? elements.length - 1 : navIndex - 1;
    }
    // if wrapping around and there are other unresolved targets, jump to them instead
    const wrappedAround = direction === "next" ? navIndex === 0 : navIndex === elements.length - 1;
    if (!wrappedAround || orderedUnresolvedTargetIds().length <= 1) {
      highlightElement(elements[navIndex]);
      return;
    }
  }

  // 2) jump to next/prev unresolved target
  const unresolvedIds = orderedUnresolvedTargetIds();
  if (unresolvedIds.length === 0) return;

  const curIdx = selectedTargetId.value ? unresolvedIds.indexOf(selectedTargetId.value) : -1;
  let nextIdx: number;
  if (direction === "next") {
    nextIdx = curIdx + 1 >= unresolvedIds.length ? 0 : curIdx + 1;
  } else {
    nextIdx = curIdx - 1 < 0 ? unresolvedIds.length - 1 : curIdx - 1;
  }
  const nextTargetId = unresolvedIds[nextIdx];
  if (nextTargetId === selectedTargetId.value && elements.length > 0) {
    // same target, just highlight
    highlightElement(elements[direction === "next" ? 0 : elements.length - 1]);
    return;
  }

  await loadTarget(nextTargetId);
  await nextTick();
  navIndex = -1;

  const newElements = getUnresolvedFieldElements();
  if (newElements.length > 0) {
    navIndex = direction === "next" ? 0 : newElements.length - 1;
    highlightElement(newElements[navIndex]);
  }
}

function navigateTarget(direction: "prev" | "next") {
  const ids = targets.value.map((t) => t.id);
  if (ids.length === 0) return;
  const curIdx = selectedTargetId.value ? ids.indexOf(selectedTargetId.value) : -1;
  let nextIdx: number;
  if (direction === "next") {
    nextIdx = curIdx + 1 >= ids.length ? 0 : curIdx + 1;
  } else {
    nextIdx = curIdx - 1 < 0 ? ids.length - 1 : curIdx - 1;
  }
  void loadTarget(ids[nextIdx]);
}

defineExpose({ navigateConflict, navigateTarget });
</script>

<template>
  <div ref="viewRef" class="merge-semantic-view" :class="{ 'sidebar-dragging': isDragging }">
    <aside class="merge-semantic-sidebar" :style="{ width: sidebarWidth + 'px' }">
      <div class="merge-semantic-sidebar-header">
        <span>Targets ({{ targets.length }})</span>
      </div>

      <UnityHierarchyPane
        v-if="isHierarchyLayout"
        :hide-title="true"
        :nodes="tree"
        :selected-id="selectedTargetId"
        :left-label="leftLabel"
        :right-label="rightLabel"
        :show-target-actions="true"
        :change-kind-overrides="hierarchyChangeKindOverrides"
        @select="onSelectTarget"
        @accept-target="onAcceptTarget"
      />

      <div v-else class="merge-semantic-target-list">
        <div
          v-for="target in targets"
          :key="target.id"
          class="merge-semantic-target-item"
          :class="{ selected: target.id === selectedTargetId }"
          @click="onSelectTarget(target.id)"
        >
          <span class="merge-target-meta">
            <span class="merge-target-label">{{ target.label }}</span>
            <span class="merge-target-path">{{ target.path }}</span>
          </span>
          <span
            v-if="mergeStatusTone(targetStatusMap.get(target.id) ?? target.mergeStatus)"
            class="merge-target-badge"
            :class="'badge-' + mergeStatusTone(targetStatusMap.get(target.id) ?? target.mergeStatus)"
          >
            {{ mergeStatusLabel(targetStatusMap.get(target.id) ?? target.mergeStatus) }}
          </span>
          <span class="merge-target-actions">
            <button class="merge-target-action-btn" @click.stop="onAcceptTarget(target.id, 'ours')">{{ compactLeft }}</button>
            <button class="merge-target-action-btn" @click.stop="onAcceptTarget(target.id, 'theirs')">{{ compactRight }}</button>
          </span>
        </div>
      </div>
    </aside>

    <div class="merge-sidebar-divider" @mousedown="onSplitterDown"></div>

    <section class="merge-semantic-main">
      <MergeInspectorPane
        :inspector="activeInspector"
        :loading="inspectorLoading"
        :error="inspectorError"
        :resolution="resolution"
        :left-label="leftLabel"
        :right-label="rightLabel"
        :badge-counts="selectedNode?.badgeCounts ?? null"
      />
    </section>
  </div>
</template>

<style scoped>
.merge-semantic-view {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
  background: var(--bg-color);
}

.merge-semantic-view.sidebar-dragging {
  cursor: col-resize;
}

.merge-semantic-sidebar {
  display: flex;
  flex-direction: column;
  min-width: 160px;
  flex-shrink: 0;
  background: var(--sidebar-bg);
}

.merge-sidebar-divider {
  width: 3px;
  flex-shrink: 0;
  background: var(--border-color);
  cursor: col-resize;
  transition: background 0.15s;
}

.merge-sidebar-divider:hover,
.merge-semantic-view.sidebar-dragging .merge-sidebar-divider {
  background: var(--accent-color);
}

.merge-semantic-sidebar-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 10px;
  border-bottom: 1px solid var(--border-color);
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.4px;
}

.merge-semantic-target-list {
  display: flex;
  flex-direction: column;
  padding: 4px 0;
  overflow-y: auto;
}

.merge-semantic-target-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border: none;
  border-left: 3px solid transparent;
  background: transparent;
  color: var(--text-color);
  text-align: left;
  cursor: pointer;
}

.merge-semantic-target-item:hover {
  background: var(--hover-bg);
}

.merge-semantic-target-item:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.merge-semantic-target-item.selected {
  border-left-color: var(--accent-color);
  background: var(--active-bg);
}

.merge-target-meta {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.merge-target-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.merge-target-path {
  font-size: 11px;
  color: var(--text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.merge-target-badge {
  flex-shrink: 0;
  padding: 2px 7px;
  border-radius: var(--radius-badge);
  font-size: 10px;
  font-weight: 700;
}

.merge-target-badge.badge-conflict {
  background: rgba(210, 155, 0, 0.16);
  color: #d29b00;
}

.merge-target-badge.badge-partial {
  background: rgba(210, 155, 0, 0.16);
  color: #d29b00;
}

.merge-target-badge.badge-resolved,
.merge-target-badge.badge-auto,
.merge-target-badge.badge-current,
.merge-target-badge.badge-incoming,
.merge-target-badge.badge-removed {
  background: rgba(46, 160, 67, 0.14);
  color: #3fb950;
}

.merge-target-actions {
  display: none;
  gap: 2px;
  flex-shrink: 0;
  margin-left: auto;
}

.merge-semantic-target-item:hover .merge-target-actions {
  display: flex;
}

.merge-target-action-btn {
  padding: 1px 6px;
  border: 1px solid var(--border-color);
  border-radius: 3px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 10px;
  font-weight: 700;
  cursor: pointer;
  white-space: nowrap;
  line-height: 16px;
}

.merge-target-action-btn:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
  background: var(--hover-bg);
}

.merge-semantic-main {
  flex: 1;
  min-width: 0;
  overflow: hidden;
}
</style>
