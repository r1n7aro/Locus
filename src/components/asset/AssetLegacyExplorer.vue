<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import { isMetaFile } from "../../composables/useHideMeta";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import FileTreeList from "../explorer/FileTreeList.vue";

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
              <svg
                viewBox="0 0 16 16"
                width="9"
                height="9"
                fill="currentColor"
              >
                <path d="M6.22 3.22a.75.75 0 0 1 1.06 0l4.25 4.25a.75.75 0 0 1 0 1.06l-4.25 4.25a.75.75 0 0 1-1.06-1.06L9.94 8 6.22 4.28a.75.75 0 0 1 0-1.06z" />
              </svg>
            </span>
            <span v-else class="alx-branch-spacer" aria-hidden="true"></span>

            <span
              class="alx-kind-icon"
              :class="[entry.isFolder ? 'folder' : 'file', { open: entry.expanded }]"
              aria-hidden="true"
            >
              <svg
                v-if="entry.isFolder"
                viewBox="0 0 16 16"
                width="13"
                height="13"
                fill="none"
              >
                <path
                  v-if="!entry.expanded"
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
              <svg
                v-else
                viewBox="0 0 16 16"
                width="13"
                height="13"
                fill="none"
              >
                <path
                  d="M5 2.75h4.55c.3 0 .58.12.8.33l1.57 1.57c.21.22.33.5.33.8V12A1.25 1.25 0 0 1 11 13.25H5A1.25 1.25 0 0 1 3.75 12V4A1.25 1.25 0 0 1 5 2.75Z"
                  stroke="currentColor"
                  stroke-width="1.2"
                  stroke-linejoin="round"
                />
                <path
                  d="M9.5 2.9V5a.5.5 0 0 0 .5.5h2.1"
                  stroke="currentColor"
                  stroke-width="1.2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
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
            <span class="alx-kind-icon alx-kind-icon-muted" aria-hidden="true">
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
  color: color-mix(in srgb, var(--text-secondary) 82%, var(--text-color));
  transition: color 0.15s ease;
}

.alx-kind-icon.folder.open {
  color: var(--text-color);
}

.alx-kind-icon.file {
  color: color-mix(in srgb, var(--text-secondary) 84%, transparent);
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
