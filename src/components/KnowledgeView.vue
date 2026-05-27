<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import type {
  KnowledgeDocumentPatch,
  ModelDefaults,
  KnowledgeDocumentSection,
  KnowledgeDocumentType,
} from "../types";
import KnowledgeExplorer from "./knowledge/KnowledgeExplorer.vue";
import KnowledgeOverviewPanel from "./knowledge/KnowledgeOverviewPanel.vue";
import KnowledgeDirectoryPreview from "./knowledge/KnowledgeDirectoryPreview.vue";
import KnowledgeRetrievalPanel from "./knowledge/KnowledgeRetrievalPanel.vue";
import KnowledgeInjectionPreviewPanel from "./knowledge/KnowledgeInjectionPreviewPanel.vue";
import KnowledgeSearchBar from "./knowledge/KnowledgeSearchBar.vue";
import KnowledgePreview from "./knowledge/KnowledgePreview.vue";
import KnowledgeSkillPackagePreview from "./knowledge/KnowledgeSkillPackagePreview.vue";
import WorkspaceRequiredState from "./WorkspaceRequiredState.vue";
import {
  useKnowledgeState,
  type ExplorerNode,
} from "../composables/useKnowledgeState";
import { t } from "../i18n";
import {
  openReferenceExternalImportWindow,
  type ReferenceExternalImportSource,
} from "../services/referenceExternalImportWindow";

const UNITY_REFERENCE_MANAGED_DIR = "unity-official-docs";

const props = defineProps<{
  workingDir: string;
  selectedModelId: string;
  modelDefaults: ModelDefaults;
}>();

const {
  sidebarWidth,
  loading,
  overview,
  overviewLoading,
  documents,
  visibleExplorerTree,
  rootDirectoryConfigs,
  referenceExternalDirectorySources,
  referenceManagedDirectoryStats,
  activeDirectoryCount,
  activeType,
  selectedPath,
  selectedDocument,
  selectedDocumentLoading,
  selectedPackageDocument,
  selectedDirectoryConfig,
  selectedDirectoryLoading,
  savingDocument,
  searchQuery,
  searchResults,
  searching,
  searchLatencyMs,
  searchMode,
  recentQueryTokens,
  selectedSearchContext,
  catalogStats,
  retrievalOverview,
  generalConfig,
  embeddingConfig,
  embeddingLocalModelCatalog,
  embeddingStatus,
  lexicalRebuildStatus,
  retrievalActionPending,
  isPathExpanded,
  togglePath,
  hasMoreRootDocuments,
  hasMoreDirectoryDocuments,
  hasLoadedDirectoryDocuments,
  isRootDocumentsLoading,
  isDirectoryDocumentsLoading,
  loadMoreRootDocuments,
  loadMoreDirectoryDocuments,
  clearSelection,
  clearSearch,
  beginExplorerDrag,
  endExplorerDrag,
  refreshKnowledgeData,
  saveGeneralConfigPatch,
  setSemanticSearchEnabled,
  setEmbeddingDevicePolicy,
  setEmbeddingDownloadSource,
  activateSemanticRuntime,
  rebuildLexicalIndex,
  refreshRetrievalState,
  selectType,
  selectDocument,
  selectPackage,
  selectDirectory,
  selectSearchResult,
  selectLocalEmbeddingModelOption,
  downloadSelectedLocalEmbeddingModel,
  deleteFeishuReferenceDocs,
  deleteUnityReferenceDocs,
  createDocumentAt,
  createFolder,
  updateSection,
  updateMeta,
  updatePackageConfig,
  importSkillPackageArchive,
  exportSkillPackageArchive,
  saveDirectoryConfig,
  deleteDocument,
  deleteExplorerNode,
  deleteExplorerNodes,
  renameExplorerFolder,
  renameExplorerDocument,
  copyExplorerRelativePath,
  openExplorerInFileSystem,
  moveExplorerNode,
} = useKnowledgeState(props);

const deleteDialog = ref<ExplorerNode[] | null>(null);
const deleteDialogBusy = ref(false);
const specialPage = ref<null | "retrieval" | "injection">(null);
const overviewDismissed = ref(false);

const hasWorkspace = computed(() => !!props.workingDir.trim());
const embeddingRuntimeLoading = computed(
  () => !!embeddingStatus.value?.activating,
);

const knowledgeTypes = computed<
  Array<{
    value: KnowledgeDocumentType;
    label: string;
    description: string;
  }>
>(() => [
  {
    value: "design",
    label: t("knowledge.type.design"),
    description: t("knowledge.type.designDesc"),
  },
  {
    value: "memory",
    label: t("knowledge.type.memory"),
    description: t("knowledge.type.memoryDesc"),
  },
  {
    value: "skill",
    label: t("knowledge.type.skill"),
    description: t("knowledge.type.skillDesc"),
  },
  {
    value: "reference",
    label: t("knowledge.type.reference"),
    description: t("knowledge.type.referenceDesc"),
  },
]);

let resizing: "sidebar" | null = null;
let resizeStartX = 0;
let resizeStartWidth = 0;

function onResizeStart(event: MouseEvent) {
  resizing = "sidebar";
  resizeStartX = event.clientX;
  resizeStartWidth = sidebarWidth.value;
  document.addEventListener("mousemove", onResizeMove);
  document.addEventListener("mouseup", onResizeEnd);
  document.body.style.cursor = "col-resize";
}

function onResizeMove(event: MouseEvent) {
  if (!resizing) return;
  const delta = event.clientX - resizeStartX;
  sidebarWidth.value = Math.min(420, Math.max(220, resizeStartWidth + delta));
}

function onResizeEnd() {
  resizing = null;
  document.removeEventListener("mousemove", onResizeMove);
  document.removeEventListener("mouseup", onResizeEnd);
  document.body.style.cursor = "";
}

function handleSelectType(type: KnowledgeDocumentType) {
  specialPage.value = null;
  overviewDismissed.value = false;
  clearSearch();
  void selectType(type);
}

function updateSearchQuery(value: string) {
  searchQuery.value = value;
}

function handleSaveSection(section: KnowledgeDocumentSection, value: string) {
  if (!selectedDocument.value) return;
  void updateSection(
    selectedDocument.value.id,
    selectedDocument.value.path,
    section,
    value,
  );
}

function handleSelectDocument(summary: Parameters<typeof selectDocument>[0]) {
  specialPage.value = null;
  overviewDismissed.value = false;
  void selectDocument(summary);
}

function handleSelectPackage(summary: Parameters<typeof selectPackage>[0]) {
  specialPage.value = null;
  overviewDismissed.value = false;
  void selectPackage(summary);
}

function handleSelectDirectory(path: string) {
  specialPage.value = null;
  overviewDismissed.value = false;
  void selectDirectory(path);
}

function handleSelectSearchResult(
  result: Parameters<typeof selectSearchResult>[0],
) {
  specialPage.value = null;
  overviewDismissed.value = false;
  void selectSearchResult(result);
}

function openRetrievalSettings() {
  overviewDismissed.value = false;
  clearSelection();
  clearSearch();
  specialPage.value = "retrieval";
  void refreshRetrievalState();
}

function openInjectionPreview() {
  overviewDismissed.value = false;
  clearSelection();
  clearSearch();
  specialPage.value = "injection";
}

function normalizeWorkspaceKey(path: string): string {
  return path.trim().replace(/\\/g, "/").replace(/\/+$/g, "").toLowerCase();
}

watch(
  () => props.workingDir,
  (workingDir, previousWorkingDir) => {
    if (specialPage.value !== "retrieval") return;
    const nextWorkspace = normalizeWorkspaceKey(workingDir);
    if (
      !nextWorkspace ||
      nextWorkspace === normalizeWorkspaceKey(previousWorkingDir ?? "")
    ) {
      return;
    }
    void refreshRetrievalState();
  },
);

function hasMoreActiveFolderDocuments(path: string): boolean {
  return hasMoreDirectoryDocuments(activeType.value, path);
}

function hasLoadedActiveFolderDocuments(path: string): boolean {
  return hasLoadedDirectoryDocuments(activeType.value, path);
}

function isActiveFolderDocumentsLoading(path: string): boolean {
  return isDirectoryDocumentsLoading(activeType.value, path);
}

function handleLoadMoreRoot() {
  void loadMoreRootDocuments(activeType.value);
}

function handleLoadMoreFolder(path: string) {
  void loadMoreDirectoryDocuments(activeType.value, path);
}

function handleClosePreview() {
  overviewDismissed.value = false;
  clearSelection();
}

function handleCloseOverview() {
  clearSelection();
  overviewDismissed.value = true;
}

function normalizeRelativePath(path: string): string {
  return path
    .trim()
    .replace(/\\/g, "/")
    .replace(/^\/+|\/+$/g, "");
}

function isUnityReferenceDocumentPath(path: string): boolean {
  const normalizedPath = normalizeRelativePath(path);
  return (
    normalizedPath === UNITY_REFERENCE_MANAGED_DIR ||
    normalizedPath.startsWith(`${UNITY_REFERENCE_MANAGED_DIR}/`)
  );
}

function referenceFolderExists(
  path: string,
  nodes: ExplorerNode[] = visibleExplorerTree.value,
): boolean {
  const normalizedPath = normalizeRelativePath(path);
  for (const node of nodes) {
    if (node.kind !== "folder") continue;
    if (normalizeRelativePath(node.relativePath) === normalizedPath)
      return true;
    if (referenceFolderExists(normalizedPath, node.children)) return true;
  }
  return false;
}

const hasUnityReferenceDocs = computed(
  () =>
    documents.value.some(
      (doc) =>
        doc.type === "reference" && isUnityReferenceDocumentPath(doc.path),
    ) || referenceFolderExists(UNITY_REFERENCE_MANAGED_DIR),
);

function openExternalImportWindow(
  parentDir = "",
  initialSource: ReferenceExternalImportSource | null = null,
) {
  if (activeType.value !== "reference") return;
  const normalizedParent = normalizeRelativePath(parentDir);
  const preferredSource =
    initialSource ??
    (!normalizedParent && !hasUnityReferenceDocs.value ? "unity" : null);
  void openReferenceExternalImportWindow({
    parentDir: normalizedParent,
    initialSource: preferredSource,
  });
}

async function ensureReferenceDirectory(path: string): Promise<boolean> {
  const normalizedPath = normalizeRelativePath(path);
  if (!normalizedPath) return false;
  if (referenceFolderExists(normalizedPath)) return true;
  const segments = normalizedPath.split("/").filter(Boolean);
  const name = segments.pop();
  if (!name) return false;
  await createFolder(segments.join("/"), name);
  return referenceFolderExists(normalizedPath);
}

async function focusReferenceDirectory(path: string) {
  const normalizedPath = normalizeRelativePath(path);
  if (!normalizedPath) return;
  specialPage.value = null;
  overviewDismissed.value = false;
  await selectDirectory(normalizedPath);
}

function handleToggleLexical(value: boolean) {
  void saveGeneralConfigPatch({ lexicalSearchEnabled: value });
}

function handleToggleSemantic(value: boolean) {
  void setSemanticSearchEnabled(value);
}

function handleUpdateMeta(patch: KnowledgeDocumentPatch) {
  if (!selectedDocument.value) return;
  void updateMeta(
    selectedDocument.value.id,
    selectedDocument.value.path,
    patch,
  );
}

function handleUpdatePackageConfig(patch: KnowledgeDocumentPatch) {
  if (!selectedPackageDocument.value) return;
  void updatePackageConfig(patch);
}

function handleImportSkillPackage() {
  void importSkillPackageArchive();
}

function handleExportPackage(packageId: string) {
  void exportSkillPackageArchive(packageId);
}

function handleExportPackageNode(node: Extract<ExplorerNode, { kind: "package" }>) {
  void exportSkillPackageArchive(node.packageId);
}

function handleSaveDirectoryConfig(
  path: string,
  config: Parameters<typeof saveDirectoryConfig>[1],
) {
  void saveDirectoryConfig(path, config);
}

function handleDelete() {
  if (!selectedDocument.value) return;
  void deleteDocument(selectedDocument.value.path, selectedDocument.value.type);
}

function deleteDialogMessage(nodes: ExplorerNode[]): string {
  if (nodes.length > 1) {
    return t("knowledge.explorer.deleteManyConfirm", nodes.length);
  }
  const [node] = nodes;
  if (!node) return "";
  if (node.kind === "folder") {
    return t("knowledge.explorer.deleteFolderConfirm", node.name);
  }
  if (node.kind === "package") {
    return t("knowledge.explorer.deletePackageConfirm", node.name);
  }
  return t("knowledge.explorer.deleteDocumentConfirm", node.name);
}

function requestDeleteNodes(nodes: ExplorerNode[]) {
  if (!nodes.length) return;
  deleteDialog.value = nodes;
}

function closeDeleteDialog() {
  if (deleteDialogBusy.value) return;
  deleteDialog.value = null;
}

async function confirmDeleteNode() {
  const nodes = deleteDialog.value;
  if (!nodes?.length || deleteDialogBusy.value) return;
  deleteDialogBusy.value = true;
  try {
    if (nodes.length === 1) {
      await deleteExplorerNode(nodes[0]);
    } else {
      await deleteExplorerNodes(nodes);
    }
    deleteDialog.value = null;
  } finally {
    deleteDialogBusy.value = false;
  }
}

onUnmounted(() => {
  document.removeEventListener("mousemove", onResizeMove);
  document.removeEventListener("mouseup", onResizeEnd);
});
</script>

<template>
  <div class="knowledge-view">
    <WorkspaceRequiredState
      v-if="!hasWorkspace"
      :description="t('workspace.required.knowledgeDescription')"
    />

    <template v-else>
      <div class="kx-type-sidebar">
        <div class="kx-type-sidebar-list">
          <button
            v-for="item in knowledgeTypes"
            :key="item.value"
            type="button"
            class="kx-type-tab"
            :class="{ active: !specialPage && activeType === item.value }"
            @click="handleSelectType(item.value)"
          >
            <div class="kx-type-tab-name">{{ item.label }}</div>
            <div class="kx-type-tab-meta">{{ item.description }}</div>
          </button>
          <div class="kx-type-tab-divider" aria-hidden="true"></div>
          <button
            type="button"
            class="kx-type-tab"
            :class="{ active: specialPage === 'retrieval' }"
            @click="openRetrievalSettings"
          >
            <div class="kx-type-tab-name">
              {{ t("knowledge.retrieval.entry") }}
            </div>
            <div class="kx-type-tab-meta">
              {{
                embeddingRuntimeLoading
                  ? t("knowledge.retrieval.runtimeStarting")
                  : t("knowledge.retrieval.entryHint")
              }}
            </div>
          </button>
          <button
            type="button"
            class="kx-type-tab"
            :class="{ active: specialPage === 'injection' }"
            @click="openInjectionPreview"
          >
            <div class="kx-type-tab-name">
              {{ t("knowledge.injectionPreview.entry") }}
            </div>
            <div class="kx-type-tab-meta">
              {{ t("knowledge.injectionPreview.entryHint") }}
            </div>
          </button>
        </div>
      </div>

      <div
        v-if="!specialPage"
        class="kx-side"
        :style="{ width: sidebarWidth + 'px' }"
      >
        <KnowledgeSearchBar
          :query="searchQuery"
          :searching="searching"
          @update:query="updateSearchQuery"
          @clear="clearSearch"
        />
        <KnowledgeExplorer
          :tree="visibleExplorerTree"
          :active-type="activeType"
          :root-directory-configs="rootDirectoryConfigs[activeType]"
          :external-directory-sources="
            activeType === 'reference' ? referenceExternalDirectorySources : {}
          "
          :folder-stats="
            activeType === 'reference' ? referenceManagedDirectoryStats : {}
          "
          :selected-path="selectedPath"
          :is-path-expanded="isPathExpanded"
          :has-more-root-documents="hasMoreRootDocuments(activeType)"
          :root-documents-loading="isRootDocumentsLoading(activeType)"
          :has-more-folder-documents="hasMoreActiveFolderDocuments"
          :folder-documents-loaded="hasLoadedActiveFolderDocuments"
          :folder-documents-loading="isActiveFolderDocumentsLoading"
          :loading="loading"
          :search-query="searchQuery"
          :search-results="searchResults"
          :searching="searching"
          @select-document="handleSelectDocument"
          @select-package="handleSelectPackage"
          @select-search-result="handleSelectSearchResult"
          @select-folder-config="handleSelectDirectory"
          @import-skill-package="handleImportSkillPackage"
          @export-package="handleExportPackageNode"
          @request-external-import-folder="
            (parentDir) => void openExternalImportWindow(parentDir)
          "
          @toggle="togglePath"
          @create-folder="createFolder"
          @create-document="createDocumentAt"
          @rename-folder="renameExplorerFolder"
          @rename-document="renameExplorerDocument"
          @copy-relative-path="copyExplorerRelativePath"
          @open-in-file-system="openExplorerInFileSystem"
          @request-delete-nodes="requestDeleteNodes"
          @move-node="moveExplorerNode"
          @load-more-root="handleLoadMoreRoot"
          @load-more-folder="handleLoadMoreFolder"
          @drag-state-change="
            (dragging: boolean) =>
              dragging ? beginExplorerDrag() : endExplorerDrag()
          "
        />
      </div>
      <div
        v-if="!specialPage"
        class="resize-handle"
        @mousedown="onResizeStart"
      ></div>

      <div class="kx-right">
        <div class="kx-content">
          <div
            v-if="embeddingRuntimeLoading && specialPage !== 'retrieval'"
            class="kx-runtime-loading"
          >
            <span class="kx-runtime-spinner" aria-hidden="true"></span>
            <span>{{ t("knowledge.retrieval.runtimeStarting") }}</span>
          </div>

          <KnowledgeSkillPackagePreview
            v-if="selectedPackageDocument"
            :package-document="selectedPackageDocument"
            :documents="documents"
            :save-loading="savingDocument"
            @select-document="handleSelectDocument"
            @update-config="handleUpdatePackageConfig"
            @export-package="handleExportPackage"
          />

          <KnowledgePreview
            v-else-if="selectedDocument"
            :document="selectedDocument"
            :search-context="selectedSearchContext"
            :loading="selectedDocumentLoading"
            :save-loading="savingDocument"
            @close="handleClosePreview"
            @delete="handleDelete"
            @save-section="handleSaveSection"
            @update-meta="handleUpdateMeta"
          />

          <KnowledgeDirectoryPreview
            v-else-if="selectedDirectoryConfig || selectedDirectoryLoading"
            :directory="selectedDirectoryConfig"
            :loading="selectedDirectoryLoading"
            :save-loading="savingDocument"
            :path-exists="referenceFolderExists"
            :ensure-directory="ensureReferenceDirectory"
            :select-directory="focusReferenceDirectory"
            :refresh-knowledge="refreshKnowledgeData"
            :delete-feishu-import="deleteFeishuReferenceDocs"
            :delete-unity-import="deleteUnityReferenceDocs"
            @close="handleClosePreview"
            @save="handleSaveDirectoryConfig"
          />

          <KnowledgeRetrievalPanel
            v-else-if="specialPage === 'retrieval'"
            :overview="retrievalOverview"
            :general-config="generalConfig"
            :embedding-config="embeddingConfig"
            :embedding-local-model-catalog="embeddingLocalModelCatalog"
            :embedding-status="embeddingStatus"
            :lexical-rebuild-status="lexicalRebuildStatus"
            :search-mode="searchMode"
            :search-latency-ms="searchLatencyMs"
            :recent-query-tokens="recentQueryTokens"
            :loading="overviewLoading"
            :pending="retrievalActionPending"
            @toggle-lexical="handleToggleLexical"
            @toggle-semantic="handleToggleSemantic"
            @set-device-policy="(value) => void setEmbeddingDevicePolicy(value)"
            @set-download-source="
              (value) => void setEmbeddingDownloadSource(value)
            "
            @select-local-model-option="
              (value) => void selectLocalEmbeddingModelOption(value)
            "
            @download-local-model="
              (value) => void downloadSelectedLocalEmbeddingModel(value)
            "
            @rebuild-lexical="() => void rebuildLexicalIndex()"
            @refresh="() => void refreshRetrievalState()"
            @rebuild-semantic="() => void activateSemanticRuntime()"
          />

          <KnowledgeInjectionPreviewPanel
            v-else-if="specialPage === 'injection'"
            :working-dir="props.workingDir"
          />

          <KnowledgeOverviewPanel
            v-else-if="!overviewDismissed"
            :stats="overview || catalogStats"
            :loading="overviewLoading || loading"
            :active-type="activeType"
            :documents="documents"
            :directory-count="activeDirectoryCount"
            :tree="visibleExplorerTree"
            @close="handleCloseOverview"
            @create-external-folder="
              (source) => void openExternalImportWindow('', source)
            "
          />

          <div v-else class="knowledge-empty-panel">
            <div class="knowledge-empty-title">
              {{ t("knowledge.empty.title") }}
            </div>
            <div class="knowledge-empty-hint">
              {{ t("knowledge.empty.hint") }}
            </div>
          </div>
        </div>
      </div>
    </template>

    <Teleport to="body">
      <div
        v-if="deleteDialog"
        class="commit-modal-overlay"
        @click.self="closeDeleteDialog"
      >
        <div class="commit-modal" style="max-width: 380px">
          <div class="commit-modal-header">
            <span class="commit-modal-title">{{
              t("common.confirmDelete")
            }}</span>
            <button class="commit-modal-close" @click="closeDeleteDialog">
              &times;
            </button>
          </div>
          <div class="commit-modal-body">
            <p class="commit-modal-message">
              {{ deleteDialogMessage(deleteDialog) }}
            </p>
          </div>
          <div class="commit-modal-footer">
            <div class="commit-modal-actions">
              <button
                class="commit-cancel-btn"
                :disabled="deleteDialogBusy"
                @click="closeDeleteDialog"
              >
                {{ t("common.cancel") }}
              </button>
              <button
                class="commit-confirm-btn"
                :disabled="deleteDialogBusy"
                style="background: var(--danger, #d73a49)"
                @click="confirmDeleteNode"
              >
                {{ t("common.confirm") }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.knowledge-view {
  flex: 1;
  display: flex;
  flex-direction: row;
  height: 100%;
  min-width: 0;
  background: var(--bg-color);
  overflow: hidden;
}

.kx-type-sidebar {
  width: 94px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--sidebar-bg) 90%, var(--bg-color) 10%);
  overflow-y: auto;
}

.kx-type-sidebar-list {
  display: flex;
  flex-direction: column;
  padding-top: 8px;
}

.kx-type-tab-divider {
  position: relative;
  height: 10px;
  margin: 10px 10px 8px;
}

.kx-type-tab-divider::before,
.kx-type-tab-divider::after {
  content: "";
  position: absolute;
  left: 0;
  right: 0;
}

.kx-type-tab-divider::before {
  top: 4px;
  height: 1px;
  background: color-mix(
    in srgb,
    var(--border-strong) 68%,
    var(--text-secondary) 32%
  );
}

.kx-type-tab-divider::after {
  top: 5px;
  height: 1px;
  background: color-mix(in srgb, var(--sidebar-bg) 62%, transparent);
  opacity: 0.72;
}

.kx-type-tab {
  appearance: none;
  width: 100%;
  box-sizing: border-box;
  padding: 10px 14px;
  border: none;
  border-left: 3px solid transparent;
  background: transparent;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 2px;
  text-align: left;
  cursor: pointer;
  transition:
    background 0.12s,
    border-color 0.12s;
}

.kx-type-tab:hover {
  background: var(--hover-bg);
}

.kx-type-tab.active {
  background: var(--active-bg, var(--hover-bg));
  border-left-color: var(--accent-color);
}

.kx-type-tab-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-color);
  line-height: 1.3;
}

.kx-type-tab.active .kx-type-tab-name {
  color: var(--accent-color);
}

.kx-type-tab-meta {
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.65;
  line-height: 1.3;
}

.kx-side {
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  border-right: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 84%, var(--bg-color) 16%);
  min-width: 220px;
  overflow: hidden;
}

.resize-handle {
  width: 0;
  cursor: col-resize;
  background: transparent;
  flex-shrink: 0;
  position: relative;
  z-index: 10;
}

.resize-handle::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -3px;
  width: 6px;
}

.resize-handle::after {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: -1px;
  width: 2px;
  background: transparent;
  transition: background 0.15s;
}

.resize-handle:hover::after {
  background: color-mix(in srgb, var(--accent-color) 40%, transparent);
}

.kx-right {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
}

.kx-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
}

.kx-runtime-loading {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--panel-bg) 88%, var(--bg-color) 12%);
  color: var(--text-secondary);
  font-size: 12px;
}

.kx-runtime-spinner {
  width: 12px;
  height: 12px;
  flex: 0 0 auto;
  border-radius: 999px;
  border: 2px solid color-mix(in srgb, currentColor 20%, transparent);
  border-top-color: currentColor;
  animation: kx-runtime-spin 0.8s linear infinite;
}

@keyframes kx-runtime-spin {
  to {
    transform: rotate(360deg);
  }
}

.knowledge-empty-panel {
  flex: 1;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 24px;
  background: color-mix(in srgb, var(--panel-bg) 94%, var(--bg-color) 6%);
  color: var(--text-secondary);
  text-align: center;
}

.knowledge-empty-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.knowledge-empty-hint {
  max-width: 420px;
  font-size: 12px;
  line-height: 1.6;
}

.knowledge-modal-overlay,
.commit-modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 90;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 20px;
  background: color-mix(in srgb, var(--bg-color) 56%, transparent);
  backdrop-filter: blur(6px);
}

.knowledge-modal,
.commit-modal {
  width: min(100%, 420px);
  border: 1px solid var(--border-color);
  border-radius: 12px;
  background: color-mix(in srgb, var(--panel-bg) 92%, var(--bg-color) 8%);
  box-shadow: 0 16px 36px rgba(0, 0, 0, 0.26);
  overflow: hidden;
}

.knowledge-modal-wide {
  width: min(100%, 1080px);
  max-height: min(92vh, 860px);
}

.knowledge-modal-header,
.commit-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border-color);
}

.knowledge-modal-title,
.commit-modal-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

.knowledge-modal-close,
.commit-modal-close {
  width: 28px;
  height: 28px;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
}

.knowledge-modal-close:hover,
.commit-modal-close:hover {
  background: var(--hover-bg);
  color: var(--text-color);
}

.knowledge-modal-body,
.commit-modal-body {
  padding: 16px;
}

.knowledge-modal-wide .knowledge-modal-body {
  max-height: min(92vh, 780px);
  overflow: auto;
}

.knowledge-form-stack {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.knowledge-field-row,
.knowledge-field-stack {
  display: flex;
  flex-direction: column;
  gap: 7px;
}

.knowledge-field-label {
  font-size: 11px;
  line-height: 1.4;
  color: var(--text-secondary);
}

.knowledge-field-value,
.knowledge-target-path {
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-color);
  font-family: var(--font-mono-identifier);
}

.knowledge-text-input {
  width: 100%;
  min-height: 34px;
  padding: 0 10px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 76%, var(--input-bg) 24%);
  color: var(--text-color);
  font: inherit;
  font-size: 13px;
  box-sizing: border-box;
}

.knowledge-text-input:focus {
  outline: none;
  border-color: var(--accent-color);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 24%, transparent);
}

.knowledge-field-hint,
.commit-modal-message {
  font-size: 12px;
  line-height: 1.6;
  color: var(--text-secondary);
}

.knowledge-target-card {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 10px 11px;
  border: 1px solid var(--border-color);
  border-radius: 8px;
  background: color-mix(in srgb, var(--panel-bg) 72%, var(--input-bg) 28%);
}

.knowledge-target-label {
  font-size: 11px;
  line-height: 1.4;
  color: var(--text-secondary);
}

.knowledge-field-error {
  font-size: 12px;
  line-height: 1.5;
  color: var(--status-danger-fg);
}

.knowledge-modal-footer,
.commit-modal-footer {
  display: flex;
  justify-content: flex-end;
  padding: 0 16px 16px;
}

.knowledge-modal-actions,
.commit-modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  width: 100%;
}

.commit-cancel-btn,
.commit-confirm-btn {
  min-height: 32px;
  padding: 0 14px;
  border-radius: 8px;
  border: 1px solid var(--border-color);
  font: inherit;
  font-size: 13px;
  cursor: pointer;
}

.commit-cancel-btn {
  background: transparent;
  color: var(--text-secondary);
}

.commit-cancel-btn:hover:not(:disabled) {
  background: var(--hover-bg);
  color: var(--text-color);
}

.commit-confirm-btn {
  border-color: transparent;
  color: var(--text-on-accent, #fff);
}

.commit-confirm-btn:hover:not(:disabled) {
  filter: brightness(1.06);
}

.commit-cancel-btn:disabled,
.commit-confirm-btn:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

@media (max-width: 980px) {
  .kx-type-sidebar {
    width: 82px;
  }

  .kx-type-tab {
    padding-inline: 12px;
  }
}

@media (max-width: 720px) {
  .knowledge-modal-overlay,
  .commit-modal-overlay {
    padding: 12px;
  }

  .knowledge-modal-actions,
  .commit-modal-actions {
    flex-direction: column-reverse;
  }
}
</style>
