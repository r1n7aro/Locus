<script setup lang="ts">
import { computed } from "vue";
import { t } from "../../i18n";
import type { AssetExplorerNode } from "../../composables/useAssetState";
import FileTreeList from "../explorer/FileTreeList.vue";

type AssetFolderNode = Extract<AssetExplorerNode, { kind: "folder" }>;

const props = defineProps<{
  items: AssetExplorerNode[];
  selectedPath: string | null;
  loading: boolean;
  loaded: boolean;
  hasMore: boolean;
  emptyLabel: string;
}>();

const emit = defineEmits<{
  (e: "select", node: AssetExplorerNode): void;
  (e: "loadMore"): void;
}>();

type VisibleEntry =
  | {
      key: string;
      kind: "row";
      node: AssetExplorerNode;
    }
  | {
      key: string;
      kind: "loadMore";
    };

const visibleRows = computed<VisibleEntry[]>(() => {
  const rows: VisibleEntry[] = props.items.map((node) => ({
    key: node.path,
    kind: "row",
    node,
  }));
  if ((props.loading || props.hasMore) && (props.loaded || props.items.length > 0)) {
    rows.push({
      key: "__load-more__",
      kind: "loadMore",
    });
  }
  return rows;
});

function handleVisibleRangeChange(payload: { start: number; end: number }) {
  if (payload.end < payload.start) return;
  const loadMoreVisible = visibleRows.value
    .slice(payload.start, payload.end + 1)
    .some((entry) => entry.kind === "loadMore");
  if (loadMoreVisible && !props.loading && props.hasMore) {
    emit("loadMore");
  }
}

function asVisibleEntry(item: { key: string }): VisibleEntry {
  return item as VisibleEntry;
}

function isFolder(node: AssetExplorerNode): node is AssetFolderNode {
  return node.kind === "folder";
}
</script>

<template>
  <div class="adl-root">
    <FileTreeList
      v-if="visibleRows.length"
      class="adl-list"
      :items="visibleRows"
      :row-height="32"
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
            class="adl-row"
            :class="{ selected: selectedPath === entry.node.path }"
            :title="entry.node.path"
            @click="emit('select', entry.node)"
          >
            <span
              v-if="isFolder(entry.node)"
              class="adl-kind-icon folder"
              aria-hidden="true"
            >
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
                fill="none"
              >
                <path
                  d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                  fill="currentColor"
                />
              </svg>
            </span>
            <span
              v-else
              class="adl-kind-icon file"
              aria-hidden="true"
            >
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
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

            <span class="adl-name">
              {{ entry.node.name }}<span v-if="isFolder(entry.node)" class="adl-folder-suffix">/</span>
            </span>
          </button>

          <div v-else class="adl-load-row">
            <span class="adl-load-icon" aria-hidden="true">
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
                fill="none"
              >
                <path
                  d="M2.25 4.5A1.25 1.25 0 0 1 3.5 3.25h2.1c.32 0 .62.13.84.36l.8.82c.14.15.34.23.55.23h4.71A1.25 1.25 0 0 1 13.75 5.9v5.6a1.25 1.25 0 0 1-1.25 1.25H3.5a1.25 1.25 0 0 1-1.25-1.25V4.5Z"
                  fill="currentColor"
                />
              </svg>
            </span>
            <span class="adl-load-label">
              {{ loading ? t("asset.explorer.loadingMore") : t("asset.explorer.loadMore") }}
            </span>
          </div>
        </template>
      </template>
    </FileTreeList>

    <div v-else class="adl-empty">
      {{ loading && !loaded ? t("common.loading") : emptyLabel }}
    </div>
  </div>
</template>

<style scoped>
.adl-root {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

.adl-list {
  flex: 1;
}

.adl-row {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  min-height: 28px;
  padding: 4px 12px;
  border: none;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
  background: transparent;
  color: var(--text-color);
  font: inherit;
  text-align: left;
  cursor: pointer;
}

.adl-row:hover {
  background: var(--hover-bg);
}

.adl-row.selected,
.adl-row.selected:hover {
  background: var(--active-bg);
}

.adl-row:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -2px;
}

.adl-kind-icon,
.adl-load-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  min-width: 16px;
  height: 16px;
  flex-shrink: 0;
}

.adl-kind-icon.folder {
  color: color-mix(in srgb, var(--accent-color) 40%, var(--text-secondary) 60%);
}

.adl-kind-icon.file {
  color: color-mix(in srgb, var(--text-secondary) 84%, transparent);
}

.adl-load-icon {
  color: color-mix(in srgb, var(--text-secondary) 52%, transparent);
}

.adl-name {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}

.adl-folder-suffix {
  opacity: 0.68;
}

.adl-load-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 28px;
  padding: 4px 12px;
  color: var(--text-secondary);
  font-size: 11px;
  border-bottom: 1px solid color-mix(in srgb, var(--border-color) 78%, transparent);
}

.adl-load-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.adl-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px 16px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 12px;
}
</style>
