<script setup lang="ts">
import { computed, defineAsyncComponent, ref } from "vue";
import { useAssetState } from "../composables/useAssetState";
import { t } from "../i18n";
import AssetExplorer from "./asset/AssetExplorer.vue";
import AssetLegacyExplorer from "./asset/AssetLegacyExplorer.vue";
import AssetDirectoryList from "./asset/AssetDirectoryList.vue";
import AssetSearchBar from "./asset/AssetSearchBar.vue";
import AssetSearchResults from "./asset/AssetSearchResults.vue";
import AssetStatsView from "./asset/AssetStatsView.vue";
import WorkspaceRequiredState from "./WorkspaceRequiredState.vue";

const AssetPreviewHost = defineAsyncComponent(
  () => import("./asset/AssetPreviewHost.vue"),
);

const props = defineProps<{
  workingDir: string;
}>();

const {
  error,
  sidebarWidth,
  directoryPaneWidth,
  explorerTree,
  selectedFolderPath,
  selectedNode,
  isPathExpanded,
  selectFolder,
  togglePath,
  probeFolderPath,
  loadMoreFolder,
  loadCurrentFolderMore,
  selectNode,
  closePreview,
  viewMode,
  searchQuery,
  searchScope,
  searchResults,
  searchTruncated,
  searchHasFallback,
  searching,
  selectedSearchKey,
  runFilenameSearch,
  updateSearchScope,
  selectFromSearchResult,
  visibleDirectoryEntries,
  currentFolderLoading,
  currentFolderLoaded,
  currentFolderHasMore,
  previewPayload,
  previewNode,
  previewLoading,
  previewError,
  activeTargetId,
  targetCache,
  targetLoading,
  loadTarget,
  dbOverview,
  dbLoading,
  triggerRescan,
  watcherTuning,
  watcherTuningSaving,
  updateWatcherTuning,
  onResizeStart,
  onDirectoryResizeStart,
} = useAssetState(props);

type AssetLayoutMode = "single" | "double";

const hasWorkspace = computed(() => !!props.workingDir.trim());
const selectedAssetPath = computed(() => selectedNode.value?.path ?? null);
const legacySelectedPath = computed(() => selectedAssetPath.value ?? selectedFolderPath.value);
const previewDisplayName = computed(() => previewNode.value?.name ?? selectedNode.value?.name ?? "");
const previewDisplayPath = computed(() => previewNode.value?.path ?? selectedNode.value?.path ?? "");
const directoryEmptyLabel = computed(() => (
  searchQuery.value.trim() && searchScope.value === "folder"
    ? t("asset.directory.emptySearch")
    : t("asset.directory.empty")
));
const searchScopeOptions = computed(() => [
  { value: "folder", label: t("asset.search.scope.folder") },
  { value: "global", label: t("asset.search.scope.global") },
]);
const layoutMode = ref<AssetLayoutMode>("double");
const doubleModeSearchScope = ref<"folder" | "global">("folder");

function handleSearchScopeUpdate(value: string) {
  if (value === "folder" || value === "global") {
    doubleModeSearchScope.value = value;
    updateSearchScope(value);
  }
}

function toggleLayoutMode() {
  if (layoutMode.value === "double") {
    doubleModeSearchScope.value = searchScope.value;
    layoutMode.value = "single";
    if (searchScope.value !== "global") {
      updateSearchScope("global");
    }
    return;
  }

  layoutMode.value = "double";
  updateSearchScope(doubleModeSearchScope.value);
}

const layoutToggleTitle = computed(() => (
  layoutMode.value === "double"
    ? t("asset.layout.toggleToSingle")
    : t("asset.layout.toggleToDouble")
));
</script>

<template>
  <div class="asset-view">
    <WorkspaceRequiredState
      v-if="!hasWorkspace"
      :description="t('workspace.required.assetDescription')"
    />

    <template v-else>
      <div v-if="error" class="ax-error" @click="error = ''">{{ error }}</div>

      <div v-if="layoutMode === 'single'" class="ax-workspace">
        <section class="ax-pane ax-pane-tree" :style="{ width: `${sidebarWidth}px` }">
          <div class="ax-pane-header">
            <span class="ax-pane-title">{{ t("asset.layout.directory") }}</span>
            <span class="ax-pane-spacer"></span>
            <button
              type="button"
              class="ax-layout-toggle"
              :title="layoutToggleTitle"
              :aria-label="layoutToggleTitle"
              @click="toggleLayoutMode"
            >
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                stroke-width="1.2"
                aria-hidden="true"
              >
                <rect x="2.25" y="3" width="11.5" height="10" rx="1.25" />
              </svg>
            </button>
          </div>
          <div class="ax-pane-body">
            <AssetLegacyExplorer
              :tree="explorerTree"
              :selected-path="legacySelectedPath"
              :is-path-expanded="isPathExpanded"
              @select="selectNode"
              @toggle="togglePath"
              @load-more="loadMoreFolder"
            />
          </div>
        </section>

        <div class="ax-resize" @mousedown="onResizeStart"></div>

        <section class="ax-pane ax-pane-preview ax-pane-single-main">
          <AssetSearchBar
            :query="searchQuery"
            :searching="searching"
            @update:query="runFilenameSearch"
            @clear="runFilenameSearch('')"
          />
          <div class="ax-pane-body ax-pane-preview-body">
            <AssetSearchResults
              v-if="searchQuery.trim() && searchScope === 'global'"
              :results="searchResults"
              :query="searchQuery"
              :searching="searching"
              :has-fallback="searchHasFallback"
              :truncated="searchTruncated"
              :selected-path="selectedAssetPath"
              :selected-key="selectedSearchKey"
              @select="selectFromSearchResult"
            />

            <AssetPreviewHost
              v-else-if="viewMode === 'preview' && previewDisplayPath"
              :payload="previewPayload"
              :loading="previewLoading"
              :error="previewError"
              :selected-name="previewDisplayName"
              :selected-path="previewDisplayPath"
              :active-target-id="activeTargetId"
              :target-cache="targetCache"
              :target-loading="targetLoading"
              :load-target="loadTarget"
              @close="closePreview"
            />

            <AssetStatsView
              v-else
              :overview="dbOverview"
              :loading="dbLoading"
              :tuning="watcherTuning"
              :tuning-saving="watcherTuningSaving"
              @rescan="triggerRescan"
              @update-tuning="updateWatcherTuning"
            />
          </div>
        </section>
      </div>

      <div v-else class="ax-workspace">
        <section class="ax-pane ax-pane-tree" :style="{ width: `${sidebarWidth}px` }">
          <div class="ax-pane-header">
            <span class="ax-pane-title">{{ t("asset.layout.directory") }}</span>
            <span class="ax-pane-spacer"></span>
            <button
              type="button"
              class="ax-layout-toggle"
              :title="layoutToggleTitle"
              :aria-label="layoutToggleTitle"
              @click="toggleLayoutMode"
            >
              <svg
                viewBox="0 0 16 16"
                width="14"
                height="14"
                fill="none"
                stroke="currentColor"
                stroke-width="1.2"
                aria-hidden="true"
              >
                <rect x="2.25" y="3" width="11.5" height="10" rx="1.25" />
                <path d="M5.9 3v10" />
              </svg>
            </button>
          </div>
          <div class="ax-pane-body">
            <AssetExplorer
              :tree="explorerTree"
              :selected-path="selectedFolderPath"
              :is-path-expanded="isPathExpanded"
              @select="selectFolder"
              @toggle="togglePath"
              @probe="probeFolderPath"
              @load-more="loadMoreFolder"
            />
          </div>
        </section>

        <div class="ax-resize" @mousedown="onResizeStart"></div>

        <section class="ax-pane ax-pane-directory" :style="{ width: `${directoryPaneWidth}px` }">
          <AssetSearchBar
            :query="searchQuery"
            :searching="searching"
            :scope="searchScope"
            :scope-options="searchScopeOptions"
            @update:query="runFilenameSearch"
            @update:scope="handleSearchScopeUpdate"
            @clear="runFilenameSearch('')"
          />
          <div class="ax-pane-body">
            <AssetSearchResults
              v-if="searchQuery.trim() && searchScope === 'global'"
              :results="searchResults"
              :query="searchQuery"
              :searching="searching"
              :has-fallback="searchHasFallback"
              :truncated="searchTruncated"
              :selected-path="selectedAssetPath"
              :selected-key="selectedSearchKey"
              @select="selectFromSearchResult"
            />

            <AssetDirectoryList
              v-else
              :items="visibleDirectoryEntries"
              :selected-path="selectedAssetPath"
              :loading="currentFolderLoading"
              :loaded="currentFolderLoaded"
              :has-more="currentFolderHasMore"
              :empty-label="directoryEmptyLabel"
              @select="selectNode"
              @load-more="loadCurrentFolderMore"
            />
          </div>
        </section>

        <div class="ax-resize ax-resize-directory" @mousedown="onDirectoryResizeStart"></div>

        <section class="ax-pane ax-pane-preview">
          <div class="ax-pane-body ax-pane-preview-body">
            <AssetPreviewHost
              v-if="viewMode === 'preview' && previewDisplayPath"
              :payload="previewPayload"
              :loading="previewLoading"
              :error="previewError"
              :selected-name="previewDisplayName"
              :selected-path="previewDisplayPath"
              :active-target-id="activeTargetId"
              :target-cache="targetCache"
              :target-loading="targetLoading"
              :load-target="loadTarget"
              @close="closePreview"
            />

            <AssetStatsView
              v-else
              :overview="dbOverview"
              :loading="dbLoading"
              :tuning="watcherTuning"
              :tuning-saving="watcherTuningSaving"
              @rescan="triggerRescan"
              @update-tuning="updateWatcherTuning"
            />
          </div>
        </section>
      </div>
    </template>
  </div>
</template>

<style scoped>
.asset-view {
  flex: 1;
  display: flex;
  flex-direction: column;
  height: 100%;
  min-width: 0;
  background: var(--bg-color);
  overflow: hidden;
}

.ax-error {
  background: var(--status-danger-bg);
  color: var(--status-danger-fg);
  font-size: 12px;
  padding: 6px 12px;
  border-bottom: 1px solid var(--status-danger-border);
  cursor: pointer;
  flex-shrink: 0;
}

.ax-workspace {
  flex: 1;
  display: flex;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.ax-pane {
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
}

.ax-pane-tree {
  flex-shrink: 0;
  min-width: 220px;
}

.ax-pane-directory {
  flex-shrink: 0;
  min-width: 260px;
  border-left: 1px solid var(--border-color);
  border-right: 1px solid var(--border-color);
}

.ax-pane-preview {
  flex: 1;
}

.ax-pane-single-main {
  border-left: 1px solid var(--border-color);
}

.ax-pane-header {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 38px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 90%, var(--bg-color) 10%);
  flex-shrink: 0;
}

.ax-pane-title {
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.ax-pane-spacer {
  flex: 1;
}

.ax-layout-toggle {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  min-width: 28px;
  height: 26px;
  padding: 0;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
}

.ax-layout-toggle:hover {
  background: var(--hover-bg);
  border-color: var(--border-strong);
  color: var(--text-color);
}

.ax-layout-toggle:focus-visible {
  outline: 2px solid var(--accent-color);
  outline-offset: -1px;
}

.ax-pane-body {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.ax-pane-preview-body {
  background: var(--panel-bg);
}

.ax-resize {
  width: 4px;
  cursor: col-resize;
  background: transparent;
  flex-shrink: 0;
}

.ax-resize:hover {
  background: color-mix(in srgb, var(--accent-color) 30%, transparent);
}

.ax-resize-directory {
  border-right: 1px solid color-mix(in srgb, var(--border-color) 72%, transparent);
}
</style>
