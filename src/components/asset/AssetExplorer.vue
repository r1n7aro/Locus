<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import WorkspaceTree, {
  type WorkspaceTreeItem,
  type WorkspaceTreeRow,
} from "../explorer/WorkspaceTree.vue";
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
      treeRow: WorkspaceTreeRow;
    }
  | {
      key: string;
      kind: "loadMore";
      folder: AssetFolderNode;
      depth: number;
      treeRow: null;
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
        treeRow: {
          key: node.path,
          name: node.name,
          depth: node.depth,
          kind: "folder",
          expandable: canToggle,
          expanded,
          selected: props.selectedPath === node.path,
          title: node.path,
        },
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
          treeRow: null,
        });
      }
    }
  }

  walk(props.tree);
  return out;
});

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

function activateItem(item: WorkspaceTreeItem) {
  const entry = asVisibleEntry(item);
  if (entry.kind === "row") emit("select", entry.node.path);
}

function toggleItem(item: WorkspaceTreeItem) {
  const entry = asVisibleEntry(item);
  if (entry.kind === "row") emit("toggle", entry.node.path);
}
</script>

<template>
  <div class="ax-explorer">
    <WorkspaceTree
      class="ax-tree"
      :items="visibleRows"
      :row-height="30"
      @activate="activateItem"
      @toggle="toggleItem"
      @visible-range-change="handleVisibleRangeChange"
    >
      <template #icon="{ item }">
        <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
          <span
            v-if="entry.kind === 'row'"
            class="ax-kind-icon folder"
            :class="[
              { open: entry.folderOpen },
              unityFolderIconClass(entry.folderOpen),
            ]"
            aria-hidden="true"
          >
            <LucideIcon
              :icon="unityFolderIconNode(entry.folderOpen)"
              :size="13"
            />
          </span>
        </template>
      </template>

      <template #name="{ item }">
        <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
          <span
            v-if="entry.kind === 'row'"
            class="ax-name"
            :class="{ 'ax-name-root': entry.node.isRoot }"
          >
            {{ entry.node.name }}
          </span>
        </template>
      </template>

      <template #trailing="{ item }">
        <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
          <span v-if="entry.kind === 'row' && folderMeta(entry.node)" class="ax-count">
            {{ folderMeta(entry.node) }}
          </span>
        </template>
      </template>

      <template #custom="{ item }">
        <template v-for="entry in [asVisibleEntry(item)]" :key="entry.key">
          <div
            v-if="entry.kind === 'loadMore'"
            class="ax-load-row"
            :style="{ paddingLeft: `${loadMoreIndentPx(entry.depth)}px` }"
          >
            <span class="ax-branch-spacer" aria-hidden="true"></span>
            <span
              class="ax-kind-icon ax-kind-icon-muted"
              :class="unityFolderIconClass(false)"
              aria-hidden="true"
            >
              <LucideIcon :icon="unityFolderIconNode(false)" :size="13" />
            </span>
            <span class="ax-load-label">{{ loadMoreLabel(entry.folder) }}</span>
          </div>
        </template>
      </template>
    </WorkspaceTree>
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

.ax-count {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.7;
}

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
