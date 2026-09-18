<script setup lang="ts">
import { computed, nextTick, ref } from "vue";
import { Archive, ArchiveRestore, File, Folder, FolderOpen, MessageSquare, Package, Star } from "lucide";
import { t } from "../../i18n";
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
  starred?: boolean;
  pinned?: boolean;
  pinnedSectionEnd?: boolean;
  focused?: boolean;
  editing?: boolean;
  dragEnabled?: boolean;
  disabled?: boolean;
  domId?: string;
  title?: string;
  ariaLabel?: string;
  classes?: Record<string, boolean>;
  activity?: { status: string; statusLabel: string; animated: boolean };
  session?: {
    branch: string;
    branchTitle?: string;
    updatedTime: string;
    unread: boolean;
    animated: boolean;
    pending: boolean;
    status?: string | null;
    statusLabel: string;
    archived?: boolean;
    actionDisabled?: boolean;
    renameValue?: string;
  };
}

export interface WorkspaceTreeItem {
  key: string;
  treeRow?: WorkspaceTreeRow | null;
}

const props = withDefaults(defineProps<{
  items: WorkspaceTreeItem[];
  rowHeight?: number;
  baseIndent?: number;
  indentSize?: number;
  rowTabIndex?: 0 | -1;
}>(), {
  rowHeight: 30,
  baseIndent: 10,
  indentSize: 14,
  rowTabIndex: -1,
});

const pinnedSectionEndIndex = computed(() => props.items.findIndex((item) => item.treeRow?.pinnedSectionEnd));

const emit = defineEmits<{
  (e: "sessionAction", item: WorkspaceTreeItem): void;
  (e: "renameChange", value: string): void;
  (e: "renameSubmit"): void;
  (e: "renameCancel"): void;
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

function focusRenameInput(element: unknown) {
  if (element instanceof HTMLInputElement && element.ownerDocument.activeElement !== element) {
    void nextTick(() => { if (element.isConnected) { element.focus(); element.select(); } });
  }
}

function rowActivity(row: WorkspaceTreeRow) { return row.session ?? row.activity; }

function defaultIcon(row: WorkspaceTreeRow) {
  if (row.session) return MessageSquare;
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
    :gap-after-index="pinnedSectionEndIndex"
    :gap-height="16"
    @visible-range-change="emit('visibleRangeChange', $event)"
  >
    <template #gap>
      <div
        class="workspace-tree-divider"
        :data-pinned-tree-key="items[pinnedSectionEndIndex]?.key"
        role="separator"
        aria-orientation="horizontal"
      />
    </template>
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
            'is-starred': item.treeRow.starred,
            'is-pinned': item.treeRow.pinned,
            'is-pinned-section-end': item.treeRow.pinnedSectionEnd,
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
            :aria-label="item.treeRow.ariaLabel"
            :disabled="item.treeRow.editing ? undefined : item.treeRow.disabled"
            :tabindex="item.treeRow.disabled ? -1 : rowTabIndex"
            @pointerdown="item.treeRow.dragEnabled && !item.treeRow.editing && emit('dragPointerDown', item, $event)"
            @click="!item.treeRow.editing && emit('activate', item, $event)"
          >
            <span class="workspace-tree-icon" :class="`kind-${item.treeRow.kind}`" aria-hidden="true">
              <slot name="icon" :item="item" :row="item.treeRow" :index="index">
                <LucideIcon :icon="defaultIcon(item.treeRow)" :size="13" :stroke-width="2" />
              </slot>
            </span>

            <span v-if="item.treeRow.editing" class="workspace-tree-editor">
              <slot name="editor" :item="item" :row="item.treeRow" :index="index">
                <input v-if="item.treeRow.session" :ref="focusRenameInput"
                  :value="item.treeRow.session.renameValue" class="development-session-rename-input"
                  :aria-label="t('chat.session.rename')" autocomplete="off"
                  @input="emit('renameChange', ($event.target as HTMLInputElement).value)"
                  @pointerdown.stop @click.stop @keydown.enter.prevent="emit('renameSubmit')"
                  @keydown.esc.prevent.stop="emit('renameCancel')" @blur="emit('renameSubmit')" />
              </slot>
            </span>
            <span v-else class="workspace-tree-name">
              <span class="workspace-tree-name-text">
                <span v-if="item.treeRow.session" class="development-session-title"
                  :class="{ 'is-running': rowActivity(item.treeRow)?.animated }"
                  :data-title="rowActivity(item.treeRow)?.animated ? item.treeRow.name : undefined">{{ item.treeRow.name }}</span>
                <slot v-else name="name" :item="item" :row="item.treeRow" :index="index">
                  {{ item.treeRow.name }}
                </slot>
              </span>
              <span
                v-if="item.treeRow.starred"
                class="workspace-tree-star"
                role="img"
                :aria-label="t('development.starredItem')"
                :title="t('development.starredItem')"
              >
                <LucideIcon :icon="Star" :size="11" :stroke-width="1.8" />
              </span>
            </span>
          </component>

          <div v-if="$slots.trailing || item.treeRow.session || item.treeRow.activity" class="workspace-tree-trailing">
            <slot name="trailing" :item="item" :row="item.treeRow" :index="index"></slot>
            <template v-if="rowActivity(item.treeRow)">
          <span
            v-if="item.treeRow.session?.pending"
            class="development-session-spinner"
            :title="t('common.loading')"
            aria-hidden="true"
          />
          <span
            v-else-if="rowActivity(item.treeRow)?.status && !rowActivity(item.treeRow)?.animated"
            class="development-session-dot"
            :class="`is-${rowActivity(item.treeRow)?.status}`"
            :title="rowActivity(item.treeRow)?.statusLabel"
            aria-hidden="true"
          />
          <span
            v-if="rowActivity(item.treeRow)?.status && rowActivity(item.treeRow)?.status !== 'running'"
            class="development-session-status"
            :class="`is-${rowActivity(item.treeRow)?.status}`"
          >
            {{ rowActivity(item.treeRow)?.statusLabel }}
          </span>
            </template>
            <template v-if="item.treeRow.session">
          <span
            class="development-session-meta"
          >
            <template v-if="item.treeRow.session.branch">
              <span
                class="development-branch-label"
                :title="item.treeRow.session.branchTitle"
              >{{ item.treeRow.session.branch }}</span>
              <span class="development-session-meta-separator" aria-hidden="true">|</span>
            </template>
            <span
              v-if="item.treeRow.session.unread"
              class="development-session-unread-dot"
              role="img"
              :aria-label="t('chat.session.unread')"
              :title="t('chat.session.unread')"
            />
            <span v-else class="development-session-updated-time">
              {{ item.treeRow.session.updatedTime }}
            </span>
          </span>
          <button
            :disabled="item.treeRow.session.actionDisabled"
            type="button"
            class="development-session-archive-button"
            :title="t(item.treeRow.session.archived ? 'chat.session.unarchive' : 'chat.session.archive')"
            :aria-label="t(item.treeRow.session.archived ? 'chat.session.unarchive' : 'chat.session.archive')"
            @pointerdown.stop
            @click.stop="emit('sessionAction', item)"
          >
            <LucideIcon :icon="item.treeRow.session.archived ? ArchiveRestore : Archive" :size="12" :stroke-width="2" />
          </button>
            </template>
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

.workspace-tree-divider {
  position: relative;
  height: 16px;
}

.workspace-tree-divider::after {
  content: "";
  position: absolute;
  right: 10px;
  top: 8px;
  left: 10px;
  border-bottom: 1px solid var(--border-color);
  pointer-events: none;
}

.workspace-tree-row-shell.is-pin-drop-target {
  background: var(--accent-soft);
}

.workspace-tree-row-shell.is-pin-drop-before::before,
.workspace-tree-row-shell.is-pin-drop-after::after {
  content: "";
  position: absolute;
  left: 12px;
  right: 10px;
  height: 1px;
  background: var(--accent-color);
  pointer-events: none;
  z-index: 1;
}

.workspace-tree-row-shell.is-pin-drop-before::before {
  top: 0;
}

.workspace-tree-row-shell.is-pin-drop-after::after {
  bottom: 0;
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
  height: 18px;
  flex-shrink: 0;
}

.workspace-tree-icon :deep(svg) {
  display: block;
  width: 14px;
  height: 14px;
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
  line-height: 18px;
}

.workspace-tree-editor {
  overflow: visible;
}

.workspace-tree-name {
  display: flex;
  align-items: center;
  gap: 6px;
}

.workspace-tree-name-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.workspace-tree-star {
  display: inline-flex;
  align-items: center;
  flex-shrink: 0;
  color: var(--accent-color);
}

.workspace-tree-star :deep(svg) {
  fill: currentColor;
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

.development-session-rename-input {
  width: 100%;
  height: 22px;
  padding: 0 7px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: color-mix(in srgb, var(--panel-bg) 82%, var(--bg-color));
  color: var(--text-color);
  font: inherit;
  font-family: var(--font-ui);
  font-size: 12px;
}

.development-session-rename-input:focus {
  border-color: var(--accent-color);
  outline: none;
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.development-session-title {
  position: relative;
  display: block;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-session-title.is-running {
  color: color-mix(in srgb, var(--text-color) 62%, var(--text-secondary) 38%);
  user-select: none;
}

.development-session-unread-dot {
  width: 4px;
  height: 4px;
  flex: 0 0 auto;
  border-radius: 50%;
  background: var(--accent-color);
}

.development-session-title.is-running::after {
  content: attr(data-title);
  position: absolute;
  inset: 0;
  overflow: hidden;
  color: var(--text-color);
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
  -webkit-mask-image: linear-gradient(90deg, transparent 40%, currentColor 50%, transparent 60%);
  mask-image: linear-gradient(90deg, transparent 40%, currentColor 50%, transparent 60%);
  -webkit-mask-size: 220% 100%;
  mask-size: 220% 100%;
  -webkit-mask-repeat: no-repeat;
  mask-repeat: no-repeat;
  animation: development-session-title-scan 2s ease-in-out infinite;
}

.development-session-dot {
  width: 6px;
  height: 6px;
  flex: 0 0 auto;
  border-radius: 999px;
  background: var(--text-secondary);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--text-secondary) 24%, transparent);
}

.development-session-dot.is-waiting_input {
  background: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 28%, transparent);
}

.development-session-dot.is-queued,
.development-session-dot.is-starting,
.development-session-dot.is-cancelling {
  background: var(--status-warn-fg, var(--text-color));
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-warn-border, var(--border-color)) 58%, transparent);
}

.development-session-dot.is-error {
  background: var(--status-danger-fg);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--status-danger-border) 60%, transparent);
}

.development-session-status {
  max-width: 58px;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 10px;
  line-height: 1;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.development-session-status.is-finishing,
.development-session-status.is-waiting_input {
  color: var(--accent-color);
}

.development-session-status.is-queued,
.development-session-status.is-starting,
.development-session-status.is-cancelling {
  color: var(--status-warn-fg, var(--text-color));
}

.development-session-status.is-error {
  color: var(--status-danger-fg);
}

.development-session-spinner {
  width: 10px;
  height: 10px;
  flex: 0 0 auto;
  border: 1px solid color-mix(in srgb, var(--text-secondary) 34%, transparent);
  border-top-color: var(--accent-color);
  border-radius: 999px;
  animation: development-session-spin 0.8s linear infinite;
}

.development-session-updated-time {
  align-self: center;
  max-width: 88px;
  margin-right: 8px;
  overflow: hidden;
  color: var(--text-secondary);
  font-size: 11px;
  text-overflow: ellipsis;
  white-space: nowrap;
  pointer-events: none;
}

.development-session-updated-time {
  font-family: var(--font-ui);
  font-variant-numeric: tabular-nums;
}

.development-session-meta {
  display: inline-flex;
  align-items: center;
  align-self: center;
  gap: 6px;
  min-width: 0;
  margin-right: 8px;
  color: var(--text-secondary);
  font-family: var(--font-ui);
  font-size: 11px;
  line-height: 16px;
  white-space: nowrap;
}

.development-session-meta .development-branch-label,
.development-session-meta .development-session-updated-time {
  margin-right: 0;
  font-family: inherit;
  line-height: inherit;
}

.development-session-meta-separator,
.development-session-meta .development-session-updated-time {
  flex-shrink: 0;
}

.development-session-meta .development-branch-label { max-width: 88px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
@keyframes development-session-title-scan {
  0% {
    -webkit-mask-position: 100% 0;
    mask-position: 100% 0;
  }
  100% {
    -webkit-mask-position: 0 0;
    mask-position: 0 0;
  }
}

@keyframes development-session-spin {
  to { transform: rotate(360deg); }
}

.development-session-archive-button {
  position: absolute;
  top: 50%;
  right: 20px;
  z-index: 2;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  opacity: 0;
  pointer-events: none;
  transform: translateY(-50%);
  transition: opacity 0.1s ease, background 0.1s ease, border-color 0.1s ease, color 0.1s ease;
}

.workspace-tree-row-shell.editing .workspace-tree-trailing { display: none; }
.workspace-tree-row-shell:hover .development-session-archive-button,
.development-session-archive-button:focus-visible { opacity: 1; pointer-events: auto; }
.workspace-tree-row-shell:hover .development-session-meta,
.workspace-tree-row-shell:has(.development-session-archive-button:focus-visible) .development-session-meta { opacity: 0; }
.development-session-archive-button:hover,
.development-session-archive-button:focus-visible { border-color: var(--border-color); background: var(--hover-bg); color: var(--text-color); outline: none; }
.development-session-archive-button:disabled { opacity: 0.45; cursor: default; }


.workspace-tree-row-shell.is-session-row .workspace-tree-row { gap: 6px; cursor: default; }

.workspace-tree-row-shell.is-open,
.workspace-tree-row-shell.is-open:hover {
  background: var(--active-bg);
  box-shadow: inset 2px 0 0 var(--accent-color);
}
.workspace-tree-row-shell.has-active-session:not(.is-open) {
  background: color-mix(in srgb, var(--accent-color) 5%, transparent);
}
.workspace-tree-row-shell.has-active-session .workspace-tree-icon {
  color: color-mix(in srgb, var(--accent-color) 72%, var(--text-secondary) 28%);
}
</style>
