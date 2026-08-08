<script setup lang="ts">
import { ref } from "vue";
import { File, Folder, FolderOpen, Package } from "lucide";
import FileTreeList from "./FileTreeList.vue";
import LucideIcon from "../icons/LucideIcon.vue";

const listRef = ref<InstanceType<typeof FileTreeList> | null>(null);

export type WorkspaceTreeRowKind = "folder" | "file" | "package";

export interface WorkspaceTreeRow {
  key: string;
  name: string;
  depth: number;
  kind: WorkspaceTreeRowKind;
  expandable?: boolean;
  expanded?: boolean;
  selected?: boolean;
  focused?: boolean;
  editing?: boolean;
  draggable?: boolean;
  disabled?: boolean;
  domId?: string;
  title?: string;
  classes?: Record<string, boolean>;
}

export interface WorkspaceTreeItem {
  key: string;
  treeRow?: WorkspaceTreeRow | null;
}

withDefaults(defineProps<{
  items: WorkspaceTreeItem[];
  rowHeight?: number;
  baseIndent?: number;
  indentSize?: number;
}>(), {
  rowHeight: 30,
  baseIndent: 10,
  indentSize: 14,
});

const emit = defineEmits<{
  (e: "activate", item: WorkspaceTreeItem, event: MouseEvent): void;
  (e: "toggle", item: WorkspaceTreeItem): void;
  (e: "contextmenu", item: WorkspaceTreeItem, event: MouseEvent): void;
  (e: "dragstart", item: WorkspaceTreeItem, event: DragEvent): void;
  (e: "dragend", item: WorkspaceTreeItem, event: DragEvent): void;
  (e: "dragover", item: WorkspaceTreeItem, event: DragEvent): void;
  (e: "dragleave", item: WorkspaceTreeItem, event: DragEvent): void;
  (e: "drop", item: WorkspaceTreeItem, event: DragEvent): void;
  (e: "visibleRangeChange", payload: { start: number; end: number }): void;
}>();

function rowIndent(row: WorkspaceTreeRow, baseIndent: number, indentSize: number): string {
  return `${baseIndent + Math.max(0, row.depth) * indentSize}px`;
}

function defaultIcon(row: WorkspaceTreeRow) {
  if (row.kind === "package") return Package;
  if (row.kind === "folder") return row.expanded ? FolderOpen : Folder;
  return File;
}

function toggleBranch(item: WorkspaceTreeItem, event: MouseEvent) {
  if (event.detail >= 2) return;
  emit("toggle", item);
}

function scrollToIndex(index: number, options?: { align?: "auto" | "center" }) {
  listRef.value?.scrollToIndex(index, options);
}

defineExpose({ scrollToIndex });
</script>

<template>
  <FileTreeList
    ref="listRef"
    class="workspace-tree"
    :items="items"
    :row-height="rowHeight"
    @visible-range-change="emit('visibleRangeChange', $event)"
  >
    <template #empty>
      <slot name="empty"></slot>
    </template>

    <template #item="{ item, index }">
      <template v-if="item.treeRow" :key="item.key">
        <div
          :id="item.treeRow.domId"
          class="workspace-tree-row-shell"
          :class="{
            selected: item.treeRow.selected,
            focused: item.treeRow.focused,
            editing: item.treeRow.editing,
            ...item.treeRow.classes,
          }"
          :draggable="item.treeRow.draggable && !item.treeRow.editing"
          :data-tree-key="item.treeRow.key"
          role="treeitem"
          :aria-level="item.treeRow.depth + 1"
          :aria-expanded="item.treeRow.expandable ? item.treeRow.expanded : undefined"
          :aria-selected="item.treeRow.selected"
          @contextmenu="emit('contextmenu', item, $event)"
          @dragstart="emit('dragstart', item, $event)"
          @dragend="emit('dragend', item, $event)"
          @dragover="emit('dragover', item, $event)"
          @dragleave="emit('dragleave', item, $event)"
          @drop="emit('drop', item, $event)"
        >
          <component
            :is="item.treeRow.editing ? 'div' : 'button'"
            :type="item.treeRow.editing ? undefined : 'button'"
            class="workspace-tree-row"
            :class="{ disabled: item.treeRow.disabled }"
            :style="{
              paddingLeft: rowIndent(item.treeRow, baseIndent, indentSize),
            }"
            :title="item.treeRow.title"
            :disabled="item.treeRow.editing ? undefined : item.treeRow.disabled"
            tabindex="-1"
            @click="!item.treeRow.editing && emit('activate', item, $event)"
          >
            <button
              v-if="item.treeRow.expandable"
              type="button"
              class="workspace-tree-branch"
              :class="{ open: item.treeRow.expanded }"
              tabindex="-1"
              :aria-label="item.treeRow.expanded ? 'Collapse' : 'Expand'"
              @click.stop="toggleBranch(item, $event)"
            >
              <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true">
                <path d="M4 2.5 7.5 6 4 9.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </button>
            <span v-else class="workspace-tree-branch-spacer" aria-hidden="true"></span>

            <span class="workspace-tree-icon" :class="`kind-${item.treeRow.kind}`" aria-hidden="true">
              <slot name="icon" :item="item" :row="item.treeRow" :index="index">
                <LucideIcon :icon="defaultIcon(item.treeRow)" :size="13" :stroke-width="2" />
              </slot>
            </span>

            <span v-if="item.treeRow.editing" class="workspace-tree-editor">
              <slot name="editor" :item="item" :row="item.treeRow" :index="index"></slot>
            </span>
            <span v-else class="workspace-tree-name">
              <slot name="name" :item="item" :row="item.treeRow" :index="index">
                {{ item.treeRow.name }}
              </slot>
            </span>
          </component>

          <div v-if="$slots.trailing" class="workspace-tree-trailing">
            <slot name="trailing" :item="item" :row="item.treeRow" :index="index"></slot>
          </div>
        </div>
      </template>
      <slot v-else name="custom" :item="item" :index="index"></slot>
    </template>
  </FileTreeList>
</template>

<style scoped>
.workspace-tree {
  flex: 1;
  min-height: 0;
  padding: 4px 0;
}

.workspace-tree-row-shell {
  position: relative;
  display: flex;
  align-items: stretch;
  width: 100%;
  min-width: 0;
  background: transparent;
  transition: background 0.1s ease;
}

.workspace-tree-row-shell:hover {
  background: var(--hover-bg);
}

.workspace-tree-row-shell.selected,
.workspace-tree-row-shell.selected:hover {
  background: var(--active-bg);
}

.workspace-tree-row-shell.focused {
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 64%, transparent);
}

.workspace-tree-row {
  flex: 1;
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-width: 0;
  min-height: 30px;
  padding: 2px 8px 2px 10px;
  border: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
}

.workspace-tree-row.disabled {
  cursor: default;
  opacity: 0.56;
}

.workspace-tree-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.workspace-tree-branch,
.workspace-tree-branch-spacer,
.workspace-tree-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
}

.workspace-tree-branch {
  padding: 0;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
}

.workspace-tree-branch:hover {
  background: color-mix(in srgb, var(--hover-bg) 85%, transparent);
  color: var(--text-color);
}

.workspace-tree-branch svg {
  opacity: 0.72;
  transition: transform 0.15s ease;
}

.workspace-tree-branch.open svg {
  transform: rotate(90deg);
}

.workspace-tree-name,
.workspace-tree-editor {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  line-height: 1.4;
}

.workspace-tree-editor {
  overflow: visible;
}

.workspace-tree-trailing {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 4px;
  min-width: 0;
  padding-right: 8px;
  flex-shrink: 0;
  pointer-events: none;
}
</style>
