<script setup lang="ts">
import { computed } from "vue";
import { ChevronRight } from "lucide";
import { t } from "../../i18n";
import { isMetaFile } from "../../composables/useHideMeta";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import FileTreeList from "../explorer/FileTreeList.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityAssetIconNodeForPath,
  unityFolderIconClass,
  unityFolderIconNode,
} from "../icons/unityAssetIcons";

type AssetFolderNode = Extract<AssetExplorerNode, { kind: "folder" }>;

const props = defineProps<{
  tree: AssetExplorerNode[];
  selectedPath: string | null;
  isPathExpanded: (path: string) => boolean;
}>();

const emit = defineEmits<{
  (e: "select", node: AssetExplorerNode): void;
  (e: "toggle", path: string): void;
  (e: "loadMore", path: string): void;
}>();

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
</script>

<template>
  <div class="alx-root">
    <FileTreeList
      class="alx-tree"
      :items="visibleRows"
      :row-height="28"
      @visible-range-change="handleVisibleRangeChange"
    >
      <template #item="{ item }">
        <template
          v-for="entry in [asVisibleEntry(item)]"
          :key="entry.key"
        >
          <button
            v-if="entry.kind === 'row'"
            type="button"
            class="alx-row"
            :class="{ selected: selectedPath === entry.node.path }"
            :style="{ paddingLeft: `${indentPx(entry.node)}px` }"
            @click="rowClick(entry)"
          >
            <span
              v-if="entry.isFolder"
              class="alx-branch"
              :class="{ open: entry.expanded }"
              aria-hidden="true"
            >
              <LucideIcon
                :icon="ChevronRight"
                :size="9"
              />
            </span>
            <span v-else class="alx-branch-spacer" aria-hidden="true"></span>

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

            <span class="alx-name" :class="{ 'alx-name-root': entry.node.kind === 'folder' && entry.node.isRoot }">
              {{ entry.node.name }}
            </span>
          </button>

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
  font-size: 13px;
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
