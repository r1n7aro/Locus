<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, ref, watch } from "vue";
import { useAssetState, type AssetExplorerNode } from "../composables/useAssetState";
import type { AssetSearchResult } from "../types";
import { useUiStore } from "../stores/ui";
import { t } from "../i18n";
import AssetExplorer from "./asset/AssetExplorer.vue";
import AssetLegacyExplorer from "./asset/AssetLegacyExplorer.vue";
import AssetDirectoryList from "./asset/AssetDirectoryList.vue";
import AssetSearchBar from "./asset/AssetSearchBar.vue";
import ExplorerSearchBar from "./explorer/ExplorerSearchBar.vue";
import AssetSearchResults from "./asset/AssetSearchResults.vue";
import BaseContextMenu from "./ui/BaseContextMenu.vue";
import ResourceFileMenuItems from "./explorer/ResourceFileMenuItems.vue";
import ExplorerResourceActions from "./explorer/ExplorerResourceActions.vue";
import { resourceRelativePath, type ExplorerResourceAction, type ExplorerResourceTarget } from "./explorer/explorerResourceActions";
import { explorerFilePath } from "../composables/useExplorerPathDisplay";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import AssetStatsView from "./asset/AssetStatsView.vue";
import WorkspaceRequiredState from "./WorkspaceRequiredState.vue";
import type { WorkspaceRef } from "../services/project";
import type { FileHistoryTarget } from "../services/fileHistory";
import { useInternalDragController } from "../composables/useInternalDrag";
import { useWorkspaceFileDrop } from "../composables/useWorkspaceFileDrop";
import {
  workbenchReferenceInternalDragSource,
} from "./workbench/workbenchReferenceDrag";
import {
  assetWorkspaceReferenceDragData,
  type AssetWorkspaceDragEntry,
} from "./asset/assetWorkspaceDrag";

const WorkspaceAssetPreview = defineAsyncComponent(
  () => import("./asset/WorkspaceAssetPreview.vue"),
);

const props = withDefaults(defineProps<{
  workingDir: string;
  workspaceRef?: WorkspaceRef | null;
  projectId?: string | null;
  active?: boolean;
  listOnly?: boolean;
}>(), { active: true });
const emit = defineEmits<{ openFile: [file: { path: string; name: string }]; resourceChanged: [target: ExplorerResourceTarget, newPath: string | null]; fileHistory: [target: FileHistoryTarget] }>();
type FileMenuSource = "tree" | "directory" | "search";
const fileMenu = ref<{ x: number; y: number; target: ExplorerResourceTarget; source: FileMenuSource; rowKey: string } | null>(null);
const resourceActions = ref<InstanceType<typeof ExplorerResourceActions> | null>(null);
const renameSource = ref<FileMenuSource | null>(null);
const workspaceContexts = useWorkspaceContextStore();
const fileDropRoot = ref<HTMLElement | null>(null);
const fileSearchRoot = ref<HTMLElement | null>(null);
const assetProjectId = computed(() => props.projectId
  ?? (props.workspaceRef ? workspaceContexts.checkoutsById[props.workspaceRef.checkoutId]?.projectId : undefined));
useWorkspaceFileDrop({
  root: () => fileDropRoot.value,
  resolve: hit => {
    if (!props.workspaceRef || !assetProjectId.value || props.active === false) return null;
    const row = hit.closest<HTMLElement>("[data-file-drop-directory]");
    const list = hit.closest<HTMLElement>(".file-tree-list");
    if (!row && !list) return null;
    return { projectId: assetProjectId.value, root: props.workingDir, workspaceRef: props.workspaceRef,
      directory: row?.dataset.fileDropDirectory ?? (hit.closest(".ax-pane-directory") ? selectedFolderPath.value ?? "." : "."),
      element: row ?? list! };
  },
  expand: target => {
    if (target.directory !== "." && !isPathExpanded(target.directory)) void togglePath(target.directory);
  },
});
function openFileMenu(file: AssetExplorerNode | AssetSearchResult, event: MouseEvent, source: FileMenuSource) {
  const isDirectory = ("kind" in file && file.kind === "folder") || ("isDirectory" in file && file.isDirectory === true);
  const projectId = props.projectId ?? (props.workspaceRef ? workspaceContexts.checkoutsById[props.workspaceRef.checkoutId]?.projectId : null);
  if (!projectId || !props.workspaceRef) return;
  event.preventDefault();
  event.stopPropagation();
  listSelectedPath.value = file.path;
  const rowKey = "isSubAsset" in file && file.isSubAsset
    ? file.objectKey || `${file.path}/${file.name.trim()}` : ("objectKey" in file && file.objectKey) || file.path;
  fileMenu.value = { x: event.clientX, y: event.clientY, source, rowKey, target: { projectId, root: props.workingDir,
    path: explorerFilePath(props.workingDir, file.path), isDirectory, workspaceRef: { ...props.workspaceRef } } };
}
function runFileAction(action: ExplorerResourceAction) {
  const menu = fileMenu.value;
  fileMenu.value = null;
  if (!menu) return;
  if (action === "rename") {
    if (resourceActions.value?.draft?.busy) return;
    renameSource.value = menu.source;
  }
  void resourceActions.value?.run(action, menu.target, menu.rowKey);
}
async function resourceChanged(target: ExplorerResourceTarget, newPath: string | null) {
  emit("resourceChanged", target, newPath);
  if (listSelectedPath.value === resourceRelativePath(target)) {
    listSelectedPath.value = newPath ? resourceRelativePath({ ...target, path: newPath }) : null;
  }
  await applyExplorerFileChange(resourceRelativePath(target), "delete");
  if (newPath) await applyExplorerFileChange(resourceRelativePath({ ...target, path: newPath }), "upsert");
  if (searchQuery.value.trim()) await runFilenameSearch(searchQuery.value);
}
const listSelectedPath = ref<string | null>(null);
const uiStore = useUiStore();
const internalDrag = useInternalDragController();

const {
  error,
  sidebarWidth,
  directoryPaneWidth,
  explorerTree,
  isUnityWorkspace,
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
  openAssetPath,
  revealPath,
  visibleDirectoryEntries,
  currentFolderLoading,
  currentFolderLoaded,
  currentFolderHasMore,
  previewPayload,
  previewNode,
  previewLoading,
  previewError,
  previewFocusLine,
  dbOverview,
  dbLoading,
  triggerRescan,
  watcherTuning,
  watcherTuningSaving,
  updateWatcherTuning,
  applyExplorerFileChange,
  onResizeStart,
  onDirectoryResizeStart,
} = useAssetState({
  get workingDir() { return props.workingDir; },
  get workspaceRef() { return props.workspaceRef; },
  get active() { return props.active; },
  get includeFilesInTree() { return props.listOnly || layoutMode.value === "single"; },
});

if (props.listOnly) updateSearchScope("global");

function selectListNode(node: AssetExplorerNode) {
  listSelectedPath.value = node.path;
  if (node.kind === "folder") {
    void selectNode(node);
    return;
  }
  emit("openFile", { path: node.path, name: node.name });
}

const listExplorerRef = ref<InstanceType<typeof AssetLegacyExplorer> | null>(null);

/** Select a workspace path in the list-only tree and scroll it into view. */
async function revealListPath(path: string): Promise<boolean> {
  if (!props.listOnly) return false;
  if (searchQuery.value.trim()) runFilenameSearch("");
  const listedPath = await revealPath(path);
  if (!listedPath) return false;
  listSelectedPath.value = listedPath;
  await nextTick();
  return listExplorerRef.value?.scrollToPath(listedPath) ?? false;
}

defineExpose({ revealPath: revealListPath });

function selectListSearchResult(result: AssetSearchResult) {
  listSelectedPath.value = result.path;
  if (result.isDirectory) {
    void selectFromSearchResult(result);
    runFilenameSearch("");
    return;
  }
  emit("openFile", { path: result.path, name: result.name });
}

function normalizeProjectPath(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/g, "").toLowerCase();
}

watch(
  () => [uiStore.pendingAssetOpen?.id ?? null, props.workingDir] as const,
  async () => {
    const pending = uiStore.pendingAssetOpen;
    if (!pending || !props.workingDir.trim()) return;
    if (normalizeProjectPath(pending.projectPath) !== normalizeProjectPath(props.workingDir)) return;
    const opened = await openAssetPath(pending.assetPath, pending.line);
    if (opened) uiStore.clearPendingAssetOpen(pending.id);
  },
  { immediate: true },
);

type AssetLayoutMode = "single" | "double";

const hasWorkspace = computed(() => !!props.workingDir.trim());
const selectedAssetPath = computed(() => selectedNode.value?.path ?? null);
const workspaceDragEnabled = computed(() => (
  !!assetProjectId.value?.trim()
  && !!props.workspaceRef
  && !!props.workingDir.trim()
));
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

function startAssetWorkspaceDrag(entry: AssetWorkspaceDragEntry, event: PointerEvent): void {
  if (!assetProjectId.value || !props.workspaceRef) return;
  const data = assetWorkspaceReferenceDragData({
    projectId: assetProjectId.value,
    workspaceRef: props.workspaceRef,
    workspaceRoot: props.workingDir,
  }, entry);
  if (!data) return;
  internalDrag.start(event, workbenchReferenceInternalDragSource(
    data,
    event.currentTarget instanceof HTMLElement ? event.currentTarget : undefined,
  ));
}
</script>

<template>
  <div ref="fileDropRoot" class="asset-view" :class="{ 'is-list-only': props.listOnly }">
    <WorkspaceRequiredState
      v-if="!hasWorkspace"
      :description="t('workspace.required.assetDescription')"
    />

    <template v-else>
      <div v-if="error" class="ax-error" @click="error = ''">{{ error }}</div>

      <div v-if="props.listOnly" class="ax-workspace">
        <section ref="fileSearchRoot" class="ax-pane ax-pane-tree ax-secondary-tree" tabindex="-1">
          <ExplorerSearchBar
            :scope="fileSearchRoot"
            :active="props.active"
            :query="searchQuery"
            :searching="searching"
            :placeholder="t('asset.search.placeholder')"
            @update:query="runFilenameSearch"
            @clear="runFilenameSearch('')"
          />
          <div class="ax-pane-body">
            <AssetSearchResults :resource-rename="renameSource === 'search' ? resourceActions?.draft : null"
              v-if="searchQuery.trim()"
              :working-dir="workingDir"
              :results="searchResults"
              :query="searchQuery"
              :searching="searching"
              :has-fallback="searchHasFallback"
              :truncated="searchTruncated"
              :selected-path="listSelectedPath"
              :drag-enabled="workspaceDragEnabled"
              @select="selectListSearchResult"
              @file-contextmenu="(file, event) => openFileMenu(file, event, 'search')"
              @drag-pointer-down="startAssetWorkspaceDrag"
            />
            <AssetLegacyExplorer
              ref="listExplorerRef"
              :resource-rename="renameSource === 'tree' ? resourceActions?.draft : null"
              :working-dir="workingDir"
              @file-contextmenu="(file, event) => openFileMenu(file, event, 'tree')"
              v-else
              workspace-style
              :searchable="false"
              :tree="explorerTree"
              :selected-path="listSelectedPath"
              :is-path-expanded="isPathExpanded"
              :drag-enabled="workspaceDragEnabled"
              @select="selectListNode"
              @toggle="togglePath"
              @load-more="loadMoreFolder"
              @drag-pointer-down="startAssetWorkspaceDrag"
            />
          </div>
        </section>
      </div>
      <div v-else-if="layoutMode === 'single'" class="ax-workspace">
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
              :resource-rename="renameSource === 'tree' ? resourceActions?.draft : null"
              :working-dir="workingDir"
              @file-contextmenu="(file, event) => openFileMenu(file, event, 'tree')"
              :tree="explorerTree"
              :selected-path="legacySelectedPath"
              :is-path-expanded="isPathExpanded"
              :drag-enabled="workspaceDragEnabled"
              @select="selectNode"
              @toggle="togglePath"
              @load-more="loadMoreFolder"
              @drag-pointer-down="startAssetWorkspaceDrag"
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
              :resource-rename="renameSource === 'search' ? resourceActions?.draft : null"
              :working-dir="workingDir"
              @file-contextmenu="(file, event) => openFileMenu(file, event, 'search')"
              v-if="searchQuery.trim() && searchScope === 'global'"
              :results="searchResults"
              :query="searchQuery"
              :searching="searching"
              :has-fallback="searchHasFallback"
              :truncated="searchTruncated"
              :selected-path="selectedAssetPath"
              :selected-key="selectedSearchKey"
              :drag-enabled="workspaceDragEnabled"
              @select="selectFromSearchResult"
              @drag-pointer-down="startAssetWorkspaceDrag"
            />

            <WorkspaceAssetPreview
              v-else-if="viewMode === 'preview' && previewDisplayPath"
              :workspace-ref="workspaceRef ?? null"
              :path="previewDisplayPath"
              :title="previewDisplayName"
              :active="active !== false"
              :payload="previewPayload"
              :loading="previewLoading"
              :error="previewError"
              :focus-line="previewFocusLine"
              :auto-load-preview="false"
              show-close
              @close="closePreview"
            />

            <AssetStatsView
              v-else-if="isUnityWorkspace"
              :overview="dbOverview"
              :loading="dbLoading"
              :tuning="watcherTuning"
              :tuning-saving="watcherTuningSaving"
              :workspace-ref="workspaceRef"
              @rescan="triggerRescan"
              @update-tuning="updateWatcherTuning"
            />
            <div v-else class="ax-preview-empty">
              {{ t("asset.preview.selectFile") }}
            </div>
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
              :drag-enabled="workspaceDragEnabled"
              @select="selectFolder"
              @toggle="togglePath"
              @probe="probeFolderPath"
              @load-more="loadMoreFolder"
              @drag-pointer-down="startAssetWorkspaceDrag"
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
              :resource-rename="renameSource === 'search' ? resourceActions?.draft : null"
              :working-dir="workingDir"
              @file-contextmenu="(file, event) => openFileMenu(file, event, 'search')"
              v-if="searchQuery.trim() && searchScope === 'global'"
              :results="searchResults"
              :query="searchQuery"
              :searching="searching"
              :has-fallback="searchHasFallback"
              :truncated="searchTruncated"
              :selected-path="selectedAssetPath"
              :selected-key="selectedSearchKey"
              :drag-enabled="workspaceDragEnabled"
              @select="selectFromSearchResult"
              @drag-pointer-down="startAssetWorkspaceDrag"
            />

            <AssetDirectoryList
              :resource-rename="renameSource === 'directory' ? resourceActions?.draft : null"
              :working-dir="workingDir"
              @file-contextmenu="(file, event) => openFileMenu(file, event, 'directory')"
              v-else
              :items="visibleDirectoryEntries"
              :selected-path="selectedAssetPath"
              :loading="currentFolderLoading"
              :loaded="currentFolderLoaded"
              :has-more="currentFolderHasMore"
              :empty-label="directoryEmptyLabel"
              :drag-enabled="workspaceDragEnabled"
              @select="selectNode"
              @load-more="loadCurrentFolderMore"
              @drag-pointer-down="startAssetWorkspaceDrag"
            />
          </div>
        </section>

        <div class="ax-resize ax-resize-directory" @mousedown="onDirectoryResizeStart"></div>

        <section class="ax-pane ax-pane-preview">
          <div class="ax-pane-body ax-pane-preview-body">
            <WorkspaceAssetPreview
              v-if="viewMode === 'preview' && previewDisplayPath"
              :workspace-ref="workspaceRef ?? null"
              :path="previewDisplayPath"
              :title="previewDisplayName"
              :active="active !== false"
              :payload="previewPayload"
              :loading="previewLoading"
              :error="previewError"
              :focus-line="previewFocusLine"
              :auto-load-preview="false"
              show-close
              @close="closePreview"
            />

            <AssetStatsView
              v-else-if="isUnityWorkspace"
              :overview="dbOverview"
              :loading="dbLoading"
              :tuning="watcherTuning"
              :tuning-saving="watcherTuningSaving"
              :workspace-ref="workspaceRef"
              @rescan="triggerRescan"
              @update-tuning="updateWatcherTuning"
            />
            <div v-else class="ax-preview-empty">
              {{ t("asset.preview.selectFile") }}
            </div>
          </div>
        </section>
      </div>
    </template>
    <BaseContextMenu v-if="fileMenu" :x="fileMenu.x" :y="fileMenu.y" @close="fileMenu = null">
      <ResourceFileMenuItems :target="fileMenu.target" @action="runFileAction" @close="fileMenu = null" />
    </BaseContextMenu>
    <ExplorerResourceActions ref="resourceActions" @changed="resourceChanged" @history="emit('fileHistory', $event)" />
  </div>
</template>

<style scoped>
.is-list-only .ax-secondary-tree {
  flex: 1;
  width: 100%;
  min-width: 0;
  border-right: 0;
  background: var(--sidebar-bg);
}

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

.ax-preview-empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
  color: var(--text-secondary);
  font-size: 12px;
  text-align: center;
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
