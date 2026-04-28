<script setup lang="ts">
import { computed } from "vue";
import { ChevronRight } from "lucide";
import { t } from "../../i18n";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import FileTreeList from "../explorer/FileTreeList.vue";
import LucideIcon from "../icons/LucideIcon.vue";
import { unityFolderIconClass, unityFolderIconNode } from "../icons/unityAssetIcons";

type AssetFolderNode = Extract<AssetExplorerNode, { kind: "folder" }>;

const props = defineProps<{
  tree: AssetExplorerNode[];
  selectedPath: string | null;
  isPathExpanded: (path: string) => boolean;
}>();

const emit = defineEmits<{
  (e: "select", path: string): void;
  (e: "toggle", path: string): void;
  (e: "loadMore", path: string): void;
  (e: "probe", path: string): void;
}>();

type VisibleEntry =
  | {
      key: string;
      kind: "row";
      node: AssetFolderNode;
      canToggle: boolean;
      expanded: boolean;
      folderOpen: boolean;
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
      if (node.kind !== "folder") continue;
      const expanded = props.isPathExpanded(node.path);
      const canToggle = canToggleFolder(node);

      out.push({
        key: node.path,
        kind: "row",
        node,
        canToggle,
        expanded,
        folderOpen: expanded && canToggle,
      });

      if (!expanded) continue;
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

function indentPx(node: AssetFolderNode): number {
  if (node.depth <= 0) return 10;
  return 10 + node.depth * 14;
}

function loadMoreIndentPx(depth: number): number {
  if (depth <= 0) return 10;
  return 10 + depth * 14;
}

function folderMeta(folder: AssetFolderNode): string {
  if (folder.loading && !folder.loaded) return t("common.loading");
  if (folder.hasMore && folder.totalCount > 0) {
    return `${folder.children.length}/${folder.totalCount}`;
  }
  if (folder.totalCount > 0) {
    return String(folder.totalCount);
  }
  return "";
}

function loadMoreLabel(folder: AssetFolderNode): string {
  if (folder.loading && !folder.loaded) return t("common.loading");
  if (folder.loading) return t("asset.explorer.loadingMore");
  if (folder.hasMore) {
    const remaining = Math.max(0, folder.totalCount - folder.children.length);
    if (remaining > 0) return t("asset.explorer.loadMoreCount", remaining);
  }
  return t("asset.explorer.loadMore");
}

function hasFolderChildren(folder: AssetFolderNode): boolean {
  return folder.hasChildFolders;
}

function canToggleFolder(folder: AssetFolderNode): boolean {
  if (!folder.hasChildFoldersKnown) return false;
  return hasFolderChildren(folder);
}

function handleVisibleRangeChange(payload: { start: number; end: number }) {
  if (payload.end < payload.start) return;
  const pendingProbes = new Set<string>();
  const pendingLoadMore = new Set<string>();
  for (const entry of visibleRows.value.slice(payload.start, payload.end + 1)) {
    if (entry.kind === "row") {
      if (entry.node.hasChildFoldersKnown || entry.node.branchProbeLoading) continue;
      if (pendingProbes.has(entry.node.path)) continue;
      pendingProbes.add(entry.node.path);
      emit("probe", entry.node.path);
      continue;
    }
    if (entry.folder.loading || !entry.folder.hasMore) continue;
    if (pendingLoadMore.has(entry.folder.path)) continue;
    pendingLoadMore.add(entry.folder.path);
    emit("loadMore", entry.folder.path);
  }
}

function asVisibleEntry(item: { key: string }): VisibleEntry {
  return item as VisibleEntry;
}
</script>

<template>
  <div class="ax-explorer">
    <FileTreeList
      class="ax-tree"
      :items="visibleRows"
      :row-height="30"
      @visible-range-change="handleVisibleRangeChange"
    >
      <template #item="{ item }">
        <template
          v-for="entry in [asVisibleEntry(item)]"
          :key="entry.key"
        >
          <div
            v-if="entry.kind === 'row'"
            class="ax-row-shell"
            :class="{ selected: selectedPath === entry.node.path }"
          >
            <div
              class="ax-row"
              :style="{ paddingLeft: `${indentPx(entry.node)}px` }"
            >
              <button
                v-if="entry.canToggle"
                type="button"
                class="ax-branch-btn"
                :class="{ open: entry.folderOpen }"
                :aria-label="entry.expanded ? t('merge.tree.toggleCollapse', entry.node.name) : t('merge.tree.toggleExpand', entry.node.name)"
                @click.stop="emit('toggle', entry.node.path)"
              >
                <LucideIcon
                  class="ax-branch-icon"
                  :icon="ChevronRight"
                  :size="10"
                />
              </button>
              <span v-else class="ax-branch-spacer" aria-hidden="true"></span>

              <button
                type="button"
                class="ax-row-main"
                @click="emit('select', entry.node.path)"
              >
                <span
                  class="ax-kind-icon folder"
                  :class="[{ open: entry.folderOpen }, unityFolderIconClass(entry.folderOpen)]"
                  aria-hidden="true"
                >
                  <LucideIcon
                    :icon="unityFolderIconNode(entry.folderOpen)"
                    :size="13"
                  />
                </span>

                <span class="ax-name" :class="{ 'ax-name-root': entry.node.isRoot }">
                  {{ entry.node.name }}
                </span>
              </button>
            </div>

            <div
              v-if="folderMeta(entry.node)"
              class="ax-row-side"
            >
              <span class="ax-count">{{ folderMeta(entry.node) }}</span>
            </div>
          </div>

          <div
            v-else
            class="ax-load-row"
            :style="{ paddingLeft: `${loadMoreIndentPx(entry.depth)}px` }"
          >
            <span class="ax-branch-spacer" aria-hidden="true"></span>
            <span
              class="ax-kind-icon ax-kind-icon-muted"
              :class="unityFolderIconClass(false)"
              aria-hidden="true"
            >
              <LucideIcon
                :icon="unityFolderIconNode(false)"
                :size="13"
              />
            </span>
            <span class="ax-load-label">{{ loadMoreLabel(entry.folder) }}</span>
          </div>
        </template>
      </template>
    </FileTreeList>
  </div>
</template>

<style scoped>
.ax-explorer {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  overflow: hidden;
}

.ax-tree {
  padding: 4px 0;
}

.ax-row-shell {
  position: relative;
  display: flex;
  align-items: stretch;
  gap: 4px;
  width: 100%;
  min-width: 0;
  background: transparent;
  transition: background 0.1s;
}

.ax-row-shell:hover {
  background: var(--hover-bg);
}

.ax-row-shell.selected,
.ax-row-shell.selected:hover {
  background: var(--active-bg);
}

.ax-row {
  display: flex;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-height: 26px;
  padding: 2px 12px 2px 10px;
  overflow: hidden;
  min-width: 0;
}

.ax-row-main {
  display: flex;
  align-items: center;
  gap: 4px;
  flex: 1;
  min-width: 0;
  min-height: 26px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-color);
  font: inherit;
  font-size: 13px;
  text-align: left;
  cursor: pointer;
}

.ax-row-main:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.ax-row-side {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  min-width: 30px;
  padding-right: 8px;
  flex-shrink: 0;
}

.ax-count {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.7;
}

.ax-branch-btn,
.ax-branch-spacer,
.ax-kind-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  min-width: 14px;
  height: 16px;
  flex-shrink: 0;
  align-self: center;
}

.ax-branch-btn {
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.12s ease, color 0.12s ease;
}

.ax-branch-btn:hover {
  background: color-mix(in srgb, var(--hover-bg) 85%, transparent);
  color: var(--text-color);
}

.ax-branch-btn:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -1px;
}

.ax-branch-btn.open .ax-branch-icon {
  transform: rotate(90deg);
}

.ax-branch-icon {
  opacity: 0.72;
  transition: transform 0.15s ease;
}

.ax-kind-icon {
  transition: color 0.15s ease;
}

.ax-kind-icon-muted {
  color: color-mix(in srgb, var(--text-secondary) 50%, transparent);
}

.ax-name {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono-identifier);
  font-size: 12px;
  color: var(--text-color);
}

.ax-name-root {
  color: var(--text-secondary);
  font-weight: 600;
}

.ax-load-row {
  display: flex;
  align-items: center;
  gap: 4px;
  min-height: 26px;
  padding: 2px 12px 2px 10px;
  color: var(--text-secondary);
  font-size: 11px;
}

.ax-load-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
</style>
