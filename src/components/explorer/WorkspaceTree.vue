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
  dragEnabled?: boolean;
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
  (e: "contextmenu", item: WorkspaceTreeItem, event: MouseEvent): void;
  (e: "dragPointerDown", item: WorkspaceTreeItem, event: PointerEvent): void;
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
          :style="{ '--workspace-tree-row-indent': rowIndent(item.treeRow, baseIndent, indentSize) }"
          :data-tree-key="item.treeRow.key"
          role="treeitem"
          :aria-level="item.treeRow.depth + 1"
          :aria-expanded="item.treeRow.expandable ? item.treeRow.expanded : undefined"
          :aria-selected="item.treeRow.selected"
          @contextmenu="emit('contextmenu', item, $event)"
          @dragover="emit('dragover', item, $event)"
          @dragleave="emit('dragleave', item, $event)"
          @drop="emit('drop', item, $event)"
        >
          <component
            :is="item.treeRow.editing ? 'div' : 'button'"
            :type="item.treeRow.editing ? undefined : 'button'"
            class="workspace-tree-row"
            :class="{
              disabled: item.treeRow.disabled,
              'drag-enabled': item.treeRow.dragEnabled && !item.treeRow.editing,
            }"
            :style="{
              paddingLeft: rowIndent(item.treeRow, baseIndent, indentSize),
            }"
            :title="item.treeRow.title"
            :disabled="item.treeRow.editing ? undefined : item.treeRow.disabled"
            tabindex="-1"
            @pointerdown="item.treeRow.dragEnabled && !item.treeRow.editing && emit('dragPointerDown', item, $event)"
            @click="!item.treeRow.editing && emit('activate', item, $event)"
          >
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
  color: color-mix(in srgb, var(--text-color) 78%, var(--text-secondary) 22%);
  font: inherit;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
}

.workspace-tree-row.disabled {
  cursor: default;
  opacity: 0.56;
}

.workspace-tree-row.drag-enabled {
  cursor: grab;
  touch-action: none;
}

.workspace-tree-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.workspace-tree-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
}

.workspace-tree-name,
.workspace-tree-editor {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-ui);
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
