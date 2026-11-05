<script setup lang="ts">
import { computed, ref } from "vue";
import { ChevronDown, ChevronRight } from "lucide";
import { t } from "../../i18n";
import { explorerFileKey, explorerFilePath, useExplorerPathDisplay } from "../../composables/useExplorerPathDisplay";
import { isMetaFile } from "../../composables/useHideMeta";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import FileTreeList from "../explorer/FileTreeList.vue";
import WorkspaceTree from "../explorer/WorkspaceTree.vue";
import ResourceRenameInput from "../explorer/ResourceRenameInput.vue";
import type { ExplorerResourceRename } from "../explorer/explorerResourceActions";
import { WORKSPACE_FILE_ROW_HEIGHT, workspaceFileEntries, type WorkspaceFileEntry } from "../explorer/workspaceFileTree";
import { workspaceFileParent } from "../explorer/workspaceFileMove";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
  unityFolderIconClass,
  unityFolderIconNode,
} from "../icons/unityAssetIcons";

type AssetFolderNode = Extract<AssetExplorerNode, { kind: "folder" }>;

const props = defineProps<{
  workingDir?: string;
  resourceRename?: ExplorerResourceRename | null;
  tree: AssetExplorerNode[];
  selectedPath: string | null;
  isPathExpanded: (path: string) => boolean;
  dragEnabled?: boolean;
  workspaceStyle?: boolean;
  searchable?: boolean;
}>();

const emit = defineEmits<{
  (e: "fileContextmenu", node: AssetExplorerNode, event: MouseEvent): void;
  (e: "select", node: AssetExplorerNode): void;
  (e: "toggle", path: string): void;
  (e: "loadMore", path: string): void;
  (e: "dragPointerDown", node: AssetExplorerNode, event: PointerEvent): void;
}>();

const { showsFullPath } = useExplorerPathDisplay();
function displayName(node: AssetExplorerNode): string {
  const path = explorerFilePath(props.workingDir ?? "", node.path);
  return showsFullPath(explorerFileKey(path)) ? path : node.name;
}

type VisibleEntry =
  | {
      key: string;
      kind: "row";
      node: AssetExplorerNode;
      isFolder: boolean;
      expanded: boolean;
    }
  | {
      key: string;
      kind: "loadMore";
      folder: AssetFolderNode;
      depth: number;
    };

const visibleRows = computed<VisibleEntry[]>(() => {
  const out: VisibleEntry[] = [];

  function walk(nodes: AssetExplorerNode[]) {
    for (const node of nodes) {
      if (node.kind === "file" && isMetaFile(node.name)) continue;
      const isFolder = node.kind === "folder";
      const expanded = isFolder ? props.isPathExpanded(node.path) : false;

      out.push({
        key: node.path,
        kind: "row",
        node,
        isFolder,
        expanded,
      });

      if (!isFolder || !expanded) continue;
      if (node.children.length > 0) {
        walk(node.children);
      }
      if (node.loading || node.hasMore) {
        out.push({
          key: `${node.path}::load-more`,
          kind: "loadMore",
          folder: node,
          depth: node.depth + 1,
        });
      }
    }
  }

  walk(props.tree);
  return out;
});

const workspaceRows = computed(() => workspaceFileEntries(props.tree, props.isPathExpanded, t("asset.explorer.loadMore"))
  .map(entry => ({ ...entry, treeRow: { ...entry.treeRow!,
    name: entry.loadMore ? entry.treeRow!.name : displayName(entry.node),
    title: explorerFilePath(props.workingDir ?? "", entry.node.path),
    selected: !entry.loadMore && props.selectedPath === entry.node.path,
    dragEnabled: !entry.loadMore && props.dragEnabled,
    classes: { "is-open": !entry.loadMore && props.selectedPath === entry.node.path },
  } })));

const workspaceTreeRef = ref<InstanceType<typeof WorkspaceTree> | null>(null);

/** Scroll a listed path into view; only the workspace-style tree supports it. */
function scrollToPath(path: string): boolean {
  const index = workspaceRows.value.findIndex((entry) => !entry.loadMore && entry.node.path === path);
  if (index < 0 || !workspaceTreeRef.value) return false;
  workspaceTreeRef.value.scrollToIndex(index, { align: "center" });
  return true;
}

defineExpose({ scrollToPath });

function activateWorkspaceEntry(entry: WorkspaceFileEntry) {
  if (entry.loadMore) emit("loadMore", entry.node.path);
  else if (entry.node.kind === "folder") emit("toggle", entry.node.path);
  else emit("select", entry.node);
}

function workspaceVisibleRange({ start, end }: { start: number; end: number }) {
  for (const entry of workspaceRows.value.slice(start, end + 1)) {
    if (entry.loadMore && entry.node.kind === "folder" && !entry.node.loading && entry.node.hasMore) {
      emit("loadMore", entry.node.path);
    }
  }
}

function rowClick(entry: Extract<VisibleEntry, { kind: "row" }>) {
  if (entry.isFolder) {
    emit("toggle", entry.node.path);
    return;
  }
  emit("select", entry.node);
}

function indentPx(node: AssetExplorerNode): number {
  if (node.depth <= 0) return 10;
  return 10 + node.depth * 14;
}

function loadMoreIndentPx(depth: number): number {
  if (depth <= 0) return 10;
  return 10 + depth * 14;
}

function handleVisibleRangeChange(payload: { start: number; end: number }) {
  if (payload.end < payload.start) return;
  const pending = new Set<string>();
  for (const entry of visibleRows.value.slice(payload.start, payload.end + 1)) {
    if (entry.kind !== "loadMore") continue;
    if (entry.folder.loading || !entry.folder.hasMore) continue;
    if (pending.has(entry.folder.path)) continue;
    pending.add(entry.folder.path);
    emit("loadMore", entry.folder.path);
  }
}

function asVisibleEntry(item: { key: string }): VisibleEntry {
  return item as VisibleEntry;
}

function fileIconClass(node: AssetExplorerNode) {
  return node.kind === "folder"
    ? unityFolderIconClass(false)
    : unityAssetIconClassForPath(node.path, { isFolder: false });
}

function beginDrag(node: AssetExplorerNode, event: PointerEvent) {
  if (!props.dragEnabled) return;
  emit("dragPointerDown", node, event);
}
</script>

<template>
  <div class="alx-root" :class="{ 'is-workspace-tree': props.workspaceStyle }">
    <WorkspaceTree
      v-if="props.workspaceStyle"
      ref="workspaceTreeRef"
      :resource-rename="resourceRename"
      navigation
      :searchable="props.searchable"
      :row-height="WORKSPACE_FILE_ROW_HEIGHT"
      :items="workspaceRows"
      :base-indent="12"
      :row-tab-index="0"
      @activate="activateWorkspaceEntry($event as WorkspaceFileEntry)"
      @contextmenu="(item, event) => !((item as WorkspaceFileEntry).loadMore) && emit('fileContextmenu', (item as WorkspaceFileEntry).node, event)"
      @drag-pointer-down="(item, event) => !((item as WorkspaceFileEntry).loadMore) && beginDrag((item as WorkspaceFileEntry).node, event)"
      @visible-range-change="workspaceVisibleRange"
    >
      <template #icon="{ item, size }">
        <LucideIcon :class="(item as WorkspaceFileEntry).node.kind === 'file' ? fileIconClass((item as WorkspaceFileEntry).node) : undefined"
          :icon="(item as WorkspaceFileEntry).node.kind === 'folder'
          ? item.treeRow?.expanded ? ChevronDown : ChevronRight
          : unityAssetIconNodeForPath((item as WorkspaceFileEntry).node.path, { isFolder: false })" :size="size" />
      </template>
    </WorkspaceTree>
    <FileTreeList
      v-else
      class="alx-tree"
      :items="visibleRows"
      :search-text="props.searchable !== false ? (item) => item.node?.name ?? '' : undefined"
      :row-height="props.workspaceStyle ? 30 : 28"
      @visible-range-change="handleVisibleRangeChange"
    >
      <template #item="{ item, searchMatch }">
        <template
          v-for="entry in [asVisibleEntry(item)]"
          :key="entry.key"
        >
          <component
            v-if="entry.kind === 'row'"
            :is="resourceRename?.key === entry.key ? 'div' : 'button'"
            :type="resourceRename?.key === entry.key ? undefined : 'button'"
            class="alx-row"
            :data-file-drop-directory="entry.node.kind === 'folder' ? entry.node.path : workspaceFileParent(entry.node.path)"
            :class="{ selected: selectedPath === entry.node.path, 'is-search-current': searchMatch }"
            :style="{ paddingLeft: `${indentPx(entry.node)}px` }"
            @pointerdown="resourceRename?.key !== entry.key && beginDrag(entry.node, $event)"
            @click="resourceRename?.key !== entry.key && rowClick(entry)"
            :title="explorerFilePath(workingDir ?? '', entry.node.path)"
            @contextmenu="emit('fileContextmenu', entry.node, $event)"
          >
            <span
              v-if="entry.isFolder && !props.workspaceStyle"
              class="alx-branch"
              :class="{ open: entry.expanded }"
              aria-hidden="true"
            >
              <LucideIcon
                :icon="ChevronRight"
                :size="9"
              />
            </span>
            <span v-else-if="!props.workspaceStyle" class="alx-branch-spacer" aria-hidden="true"></span>

            <span
              class="alx-kind-icon"
              :class="[
                entry.isFolder ? 'folder' : 'file',
                { open: entry.expanded },
                entry.isFolder ? unityFolderIconClass(entry.expanded) : fileIconClass(entry.node),
              ]"
              aria-hidden="true"
            >
              <LucideIcon
                :icon="entry.isFolder ? unityFolderIconNode(entry.expanded) : unityAssetIconNodeForPath(entry.node.path, { isFolder: false })"
                :size="13"
              />
            </span>

            <ResourceRenameInput v-if="resourceRename?.key === entry.key" :rename="resourceRename" />
            <span v-else class="alx-name" :class="{ 'alx-name-root': entry.node.kind === 'folder' && entry.node.isRoot }">
              {{ displayName(entry.node) }}
            </span>
          </component>

          <div
            v-else
            class="alx-load-row"
            :style="{ paddingLeft: `${loadMoreIndentPx(entry.depth)}px` }"
          >
            <span class="alx-branch-spacer" aria-hidden="true"></span>
            <span
              class="alx-kind-icon alx-kind-icon-muted"
              :class="unityFolderIconClass(false)"
              aria-hidden="true"
            >
              <LucideIcon
                :icon="unityFolderIconNode(false)"
                :size="13"
              />
            </span>
            <span class="alx-load-label">{{ t("asset.explorer.loadMore") }}</span>
          </div>
        </template>
      </template>
    </FileTreeList>
  </div>
</template>

<style scoped>
.alx-root.is-workspace-tree {
  background: var(--sidebar-bg);
}

.alx-root {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  overflow: hidden;
}

.alx-tree {
  padding: 4px 0;
}

.alx-row {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-height: 26px;
  padding: 2px 12px 2px 10px;
  border: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 12px;
  text-align: left;
  cursor: pointer;
  overflow: hidden;
}

.alx-row:hover {
  background: var(--hover-bg);
}

.alx-row.selected,
.alx-row.selected:hover {
  background: var(--active-bg);
}

.alx-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.alx-branch,
.alx-branch-spacer,
.alx-kind-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
}

.alx-branch {
  color: var(--text-secondary);
  opacity: 0.72;
  transition: transform 0.15s ease;
}

.alx-branch.open {
  transform: rotate(90deg);
}

.alx-kind-icon.folder {
  transition: color 0.15s ease;
}

.alx-kind-icon-muted {
  color: color-mix(in srgb, var(--text-secondary) 50%, transparent);
}

.alx-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-color);
}

.alx-name-root {
  color: var(--text-secondary);
  font-weight: 600;
}

.alx-load-row {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 26px;
  padding: 2px 12px 2px 10px;
  color: var(--text-secondary);
  font-size: 11px;
}

.alx-load-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
