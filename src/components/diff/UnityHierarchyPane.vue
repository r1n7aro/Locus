<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import type { MergeSide, SemanticTreeNode } from "../../types";
import { t } from "../../i18n";
import { compactMergeSideLabel } from "../collab/mergeUi";

const props = defineProps<{
  nodes: SemanticTreeNode[];
  selectedId?: string | null;
  leftLabel?: string;
  rightLabel?: string;
  showTargetActions?: boolean;
  changeKindOverrides?: Record<string, string>;
  hideTitle?: boolean;
  showCollapseAll?: boolean;
  autoCollapseWhenOverflow?: boolean;
}>();

const emit = defineEmits<{
  select: [id: string];
  acceptTarget: [targetId: string, side: MergeSide];
}>();

const compactLeft = computed(() => compactMergeSideLabel(props.leftLabel, "left"));
const compactRight = computed(() => compactMergeSideLabel(props.rightLabel, "right"));

interface HierarchyRow {
  id: string;
  depth: number;
  node: SemanticTreeNode;
  expanded: boolean;
}

const paneRef = ref<HTMLElement | null>(null);
const collapsedIds = ref(new Set<string>());
const paneHeight = ref(0);
const AUTO_COLLAPSE_ROW_HEIGHT = 28;
const HEADER_HEIGHT = 34;
let resizeObserver: ResizeObserver | null = null;

const collapsibleIds = computed(() => (
  props.nodes
    .filter((node) => hasChildren(node))
    .map((node) => node.id)
));

const canCollapseAll = computed(() => collapsibleIds.value.length > 0);
const isAllCollapsed = computed(() => (
  canCollapseAll.value
  && collapsibleIds.value.every((id) => collapsedIds.value.has(id))
));

const rows = computed<HierarchyRow[]>(() => {
  const byParent = new Map<string | null, SemanticTreeNode[]>();
  for (const node of props.nodes) {
    const key = node.parentId ?? null;
    const list = byParent.get(key) ?? [];
    list.push(node);
    byParent.set(key, list);
  }

  const knownIds = new Set(props.nodes.map((node) => node.id));
  const roots = props.nodes.filter((node) => !node.parentId || !knownIds.has(node.parentId));
  const out: HierarchyRow[] = [];

  function walk(node: SemanticTreeNode, depth: number) {
    const expanded = !collapsedIds.value.has(node.id);
    out.push({ id: node.id, depth, node, expanded });
    if (!expanded) return;

    const children = byParent.get(node.id) ?? [];
    for (const child of children) {
      walk(child, depth + 1);
    }
  }

  for (const root of roots) {
    walk(root, 0);
  }

  return out;
});

function hasChildren(node: SemanticTreeNode): boolean {
  return node.childIds.length > 0;
}

function updatePaneHeight() {
  paneHeight.value = paneRef.value?.clientHeight ?? 0;
}

function changeIcon(kind: string): string {
  switch (kind) {
    case "added": return "A";
    case "removed": return "D";
    case "modified": return "M";
    case "conflict": return "!";
    case "partial": return "!";
    case "resolved": return "\u2713";
    case "autoResolved": return "\u2713";
    case "oursOnly":
    case "theirsOnly": return "+";
    default: return "";
  }
}

function toggleNode(node: SemanticTreeNode) {
  if (!hasChildren(node)) return;
  const next = new Set(collapsedIds.value);
  if (next.has(node.id)) next.delete(node.id);
  else next.add(node.id);
  collapsedIds.value = next;
}

function collapseAll() {
  collapsedIds.value = new Set(collapsibleIds.value);
}

function selectNode(node: SemanticTreeNode) {
  if (!node.hasInspector) return;
  emit("select", node.id);
}

function displayChangeKind(node: SemanticTreeNode): string {
  return props.changeKindOverrides?.[node.id] ?? node.changeKind;
}

const nodesSignature = computed(() => props.nodes.map((node) => node.id).join("|"));
const autoCollapseHandledFor = ref<string | null>(null);

watch(
  nodesSignature,
  () => {
    collapsedIds.value = new Set();
    autoCollapseHandledFor.value = null;
  },
  { immediate: true },
);

watch(
  [nodesSignature, paneHeight],
  async ([signature, height]) => {
    if (!props.autoCollapseWhenOverflow || !signature || autoCollapseHandledFor.value === signature) return;
    if (height <= 0) return;
    await nextTick();
    const availableHeight = Math.max(0, height - (props.hideTitle ? 0 : HEADER_HEIGHT));
    const maxVisibleRows = Math.floor(availableHeight / AUTO_COLLAPSE_ROW_HEIGHT);
    if (maxVisibleRows > 0 && props.nodes.length > maxVisibleRows) {
      collapseAll();
    }
    autoCollapseHandledFor.value = signature;
  },
  { immediate: true },
);

onMounted(() => {
  updatePaneHeight();
  if (typeof ResizeObserver === "undefined") return;
  resizeObserver = new ResizeObserver(() => updatePaneHeight());
  if (paneRef.value) resizeObserver.observe(paneRef.value);
});

onUnmounted(() => {
  resizeObserver?.disconnect();
  resizeObserver = null;
});
</script>

<template>
  <div ref="paneRef" class="unity-hierarchy-pane">
    <div v-if="!hideTitle" class="hierarchy-title">
      <span class="hierarchy-title-label">{{ t("merge.tree.header") }}</span>
      <button
        v-if="showCollapseAll && canCollapseAll"
        type="button"
        class="hierarchy-toolbar-btn"
        :disabled="isAllCollapsed"
        @click="collapseAll"
      >
        {{ t("merge.tree.collapseAll") }}
      </button>
    </div>
    <div
      v-for="row in rows"
      :key="row.id"
      class="hierarchy-row"
      :class="[
        displayChangeKind(row.node),
        {
          selected: row.id === selectedId,
          inspectable: row.node.hasInspector,
          'no-inspector': !row.node.hasInspector,
        },
      ]"
      :style="{ paddingLeft: `${10 + row.depth * 16}px` }"
      :title="row.node.hasInspector ? row.node.path : t('merge.tree.aggregateNode', row.node.path)"
      :role="row.node.hasInspector ? 'button' : undefined"
      :tabindex="row.node.hasInspector ? 0 : undefined"
      :aria-label="row.node.hasInspector ? t('merge.tree.selectNode', row.node.label) : undefined"
      @click="selectNode(row.node)"
      @keydown.enter.prevent="selectNode(row.node)"
      @keydown.space.prevent="selectNode(row.node)"
    >
      <span class="row-change-bar" :class="displayChangeKind(row.node)" />
      <button
        v-if="hasChildren(row.node)"
        type="button"
        class="row-toggle"
        :aria-expanded="row.expanded"
        :aria-label="row.expanded ? t('merge.tree.toggleCollapse', row.node.label) : t('merge.tree.toggleExpand', row.node.label)"
        @click.stop="toggleNode(row.node)"
      />
      <span v-else class="row-toggle-spacer" />

      <div class="row-select">
        <span class="row-label">{{ row.node.label }}</span>
        <span v-if="displayChangeKind(row.node) !== 'unchanged'" class="row-change-icon" :class="displayChangeKind(row.node)">
          {{ changeIcon(displayChangeKind(row.node)) }}
        </span>
      </div>
      <div v-if="showTargetActions && row.node.hasInspector && displayChangeKind(row.node) !== 'unchanged'" class="row-target-actions">
        <button class="row-action-btn" :title="compactLeft" @click.stop="emit('acceptTarget', row.id, 'ours')">{{ compactLeft }}</button>
        <button class="row-action-btn" :title="compactRight" @click.stop="emit('acceptTarget', row.id, 'theirs')">{{ compactRight }}</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.unity-hierarchy-pane {
  height: 100%;
  overflow: auto;
  border-right: 1px solid var(--border-color);
  background: var(--bg-secondary, rgba(255, 255, 255, 0.02));
  font-family: var(--font-ui);
}

.hierarchy-title {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px;
  font-size: 11px;
  font-weight: 700;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
  border-bottom: 1px solid var(--border-color);
}

.hierarchy-title-label {
  min-width: 0;
}

.hierarchy-toolbar-btn {
  flex-shrink: 0;
  padding: 2px 8px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 600;
  text-transform: none;
  letter-spacing: normal;
  cursor: pointer;
  transition: background 0.15s, border-color 0.15s, color 0.15s;
}

.hierarchy-toolbar-btn:hover:not(:disabled) {
  color: var(--text-color);
  border-color: var(--text-secondary);
  background: var(--hover-bg);
}

.hierarchy-toolbar-btn:disabled {
  opacity: 0.5;
  cursor: default;
}

.hierarchy-row {
  position: relative;
  display: flex;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  padding: 4px 10px;
  cursor: default;
  border-bottom: 1px solid rgba(255, 255, 255, 0.03);
  font-size: 12.5px;
}

.hierarchy-row:hover {
  background: rgba(255, 255, 255, 0.04);
}

.hierarchy-row.no-inspector {
  color: var(--text-secondary);
}

.hierarchy-row.inspectable {
  cursor: pointer;
}

.hierarchy-row.selected {
  background: rgba(56, 161, 105, 0.14);
}

.row-change-bar {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 0;
  width: 3px;
}

.row-change-bar.added {
  background: #38a169;
}

.row-change-bar.removed {
  background: #e53e3e;
}

.row-change-bar.modified {
  background: #d69e2e;
}

.row-change-bar.conflict {
  background: #dd6b20;
}

.row-change-bar.partial {
  background: #d69e2e;
}

.row-change-bar.resolved,
.row-change-bar.autoResolved {
  background: #38a169;
}

.row-change-bar.oursOnly {
  background: #4299e1;
}

.row-change-bar.theirsOnly {
  background: #9f7aea;
}

.row-toggle,
.row-toggle-spacer {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
}

.row-toggle {
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 0;
}

.row-toggle:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
  background: var(--hover-bg);
}

.hierarchy-row:focus-visible,
.row-toggle:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.row-toggle::before {
  content: "▶";
  font-size: 9px;
}

.row-toggle[aria-expanded="true"]::before {
  content: "▼";
}

.row-select {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1;
  min-width: 0;
  color: inherit;
  text-align: left;
  cursor: inherit;
}

.row-label {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  color: var(--text-color);
}

.row-change-icon {
  flex-shrink: 0;
  width: 16px;
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 3px;
  font-size: 10px;
  font-weight: 700;
}

.row-change-icon.added {
  color: #38a169;
  background: rgba(56, 161, 105, 0.14);
}

.row-change-icon.removed {
  color: #e53e3e;
  background: rgba(229, 62, 62, 0.14);
}

.row-change-icon.modified {
  color: #d69e2e;
  background: rgba(214, 158, 46, 0.14);
}

.row-change-icon.conflict {
  color: #dd6b20;
  background: rgba(221, 107, 32, 0.14);
}

.row-change-icon.partial {
  color: #d69e2e;
  background: rgba(214, 158, 46, 0.14);
}

.row-change-icon.resolved,
.row-change-icon.autoResolved {
  color: #38a169;
  background: rgba(56, 161, 105, 0.14);
}

.row-change-icon.oursOnly {
  color: #4299e1;
  background: rgba(66, 153, 225, 0.14);
}

.row-change-icon.theirsOnly {
  color: #9f7aea;
  background: rgba(159, 122, 234, 0.14);
}

.row-target-actions {
  display: none;
  gap: 2px;
  flex-shrink: 0;
  margin-left: auto;
}

.hierarchy-row:hover .row-target-actions {
  display: flex;
}

.row-action-btn {
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

.row-action-btn:hover {
  color: var(--text-color);
  border-color: var(--text-secondary);
  background: var(--hover-bg);
}

</style>
