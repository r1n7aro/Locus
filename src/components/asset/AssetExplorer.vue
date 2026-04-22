<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import FileTreeList from "../explorer/FileTreeList.vue";

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
                <svg
                  class="ax-branch-icon"
                  viewBox="0 0 16 16"
                  width="10"
                  height="10"
                  fill="currentColor"
                  aria-hidden="true"
                >
                  <path d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06z" />
                </svg>
              </button>
              <span v-else class="ax-branch-spacer" aria-hidden="true"></span>

              <button
                type="button"
                class="ax-row-main"
                @click="emit('select', entry.node.path)"
              >
                <span
                  class="ax-kind-icon folder"
                  :class="{ open: entry.folderOpen }"
                  aria-hidden="true"
                >
                  <svg
                    viewBox="0 0 16 16"
                    width="13"
                    height="13"
                    fill="none"
                  >
                    <path
                      v-if="!entry.folderOpen"
                      d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                      fill="currentColor"
                    />
                    <template v-else>
                      <path
                        d="M2.5 4.5a1.25 1.25 0 0 1 1.25-1.25h1.9c.28 0 .55.11.74.31l.98.98c.2.2.46.31.74.31h4.14a1.25 1.25 0 0 1 1.25 1.25v5.1a1.25 1.25 0 0 1-1.25 1.25h-8.5A1.25 1.25 0 0 1 2.5 11.2V4.5Z"
                        stroke="currentColor"
                        stroke-width="1.2"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                      />
                    </template>
                  </svg>
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
            <span class="ax-kind-icon ax-kind-icon-muted" aria-hidden="true">
              <svg
                viewBox="0 0 16 16"
                width="13"
                height="13"
                fill="none"
              >
                <path
                  d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                  fill="currentColor"
                />
              </svg>
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
  color: color-mix(in srgb, var(--accent-color) 38%, var(--text-secondary) 62%);
  transition: color 0.15s ease;
}

.ax-kind-icon.open {
  color: color-mix(in srgb, var(--accent-color) 54%, var(--text-secondary) 46%);
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
