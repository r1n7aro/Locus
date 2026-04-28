<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from "vue";
import { ChevronRight } from "lucide";
import type { GitBlockedPath, GitFileChange, ModelOption } from "../../types";
import { gitCommit, gitGenerateCommitMessage } from "../../services/git";
import { t } from "../../i18n";
import { normalizeAppError } from "../../services/errors";
import { useHideMeta, isMetaFile, partitionMetaPaths } from "../../composables/useHideMeta";
import {
  getLocusManagedTagKind,
  getLocusManagedTagKindForPath,
  type LocusManagedFileLike,
} from "../../composables/locusManagedFiles";
import { acquireSelectionLock } from "../../composables/useSelectionLock";
import { resolveStagingFileSelection } from "./stagingSelection";
import {
  persistStagingLayout,
  persistStagingViewMode,
  readStoredStagingLayout,
  readStoredStagingViewMode,
  type StagingViewMode,
} from "./stagingLayout";
import {
  buildStagingTreeRows,
  buildStagingFolderFileMap,
  collectStagingFolderPaths,
  type StagingTreeRow,
} from "./stagingTree";
import LucideIcon from "../icons/LucideIcon.vue";
import {
  unityAssetIconClassForPath,
  unityFolderIconClass,
  unityAssetIconNodeForPath,
  unityFolderIconNode,
} from "../icons/unityAssetIcons";

const props = defineProps<{
  unstagedFiles: GitFileChange[];
  stagedFiles: GitFileChange[];
  blockedFiles: GitBlockedPath[];
  selectedModelId: string;
  models: ModelOption[];
  currentBranch: string;
  totalChanges: number;
  activeFilePath: string | null;
  pendingStagePaths: Set<string>;
  pendingUnstagePaths: Set<string>;
  pendingDiscardPaths: Set<string>;
  stageOperationBusy: boolean;
}>();

const emit = defineEmits<{
  (e: "stage", path: string): void;
  (e: "unstage", path: string): void;
  (e: "stageMany", paths: string[]): void;
  (e: "unstageMany", paths: string[]): void;
  (e: "stageAll"): void;
  (e: "unstageAll"): void;
  (e: "committed"): void;
  (e: "selectModel", id: string): void;
  (e: "selectFile", file: GitFileChange, source: "gitUnstaged" | "gitStaged"): void;
  (e: "fileContextmenu", event: MouseEvent, file: GitFileChange, source: "gitUnstaged" | "gitStaged", selectedPaths: Set<string>): void;
}>();

const { hideMeta } = useHideMeta();

const layoutHorizontal = ref(readStoredStagingLayout());
const fileViewMode = ref<StagingViewMode>(readStoredStagingViewMode());
const sectionsContainerRef = ref<HTMLElement | null>(null);
const splitRatioVertical = ref(readStoredSplit("locus.collab.stagingSplit.vertical", 50));
const splitRatioHorizontal = ref(readStoredSplit("locus.collab.stagingSplit.horizontal", 50));
const isDraggingSplit = ref(false);
const collapsedUnstagedFolders = ref(new Set<string>());
const collapsedStagedFolders = ref(new Set<string>());

const selectedUnstaged = ref(new Set<string>());
const selectedStaged = ref(new Set<string>());
const lastClickedUnstaged = ref<string | null>(null);
const lastClickedStaged = ref<string | null>(null);

const hasUnstagedSelection = computed(() => selectedUnstaged.value.size > 0);
const hasStagedSelection = computed(() => selectedStaged.value.size > 0);

function isStagePending(path: string) {
  return props.pendingStagePaths.has(path);
}

function isUnstagePending(path: string) {
  return props.pendingUnstagePaths.has(path);
}

function isDiscardPending(path: string) {
  return props.pendingDiscardPaths.has(path);
}

function isFilePending(path: string) {
  return isStagePending(path) || isUnstagePending(path) || isDiscardPending(path);
}

function fileActionLabel(path: string, idleKey: "collab.stage" | "collab.unstage") {
  if (isDiscardPending(path)) return t("collab.discarding");
  if (isStagePending(path)) return t("collab.staging");
  if (isUnstagePending(path)) return t("collab.unstaging");
  return t(idleKey);
}

function readStoredSplit(key: string, fallback: number) {
  try {
    const raw = Number(localStorage.getItem(key));
    if (Number.isFinite(raw) && raw >= 15 && raw <= 85) return raw;
  } catch { /* ignore */ }
  return fallback;
}

function clampSplitRatio(ratio: number, total: number) {
  if (!Number.isFinite(ratio)) return 50;
  const minPx = 120;
  if (total <= 0) return Math.max(20, Math.min(80, ratio));
  const minRatio = Math.min(45, Math.max(15, (minPx / total) * 100));
  return Math.max(minRatio, Math.min(100 - minRatio, ratio));
}

function getCurrentSplitRatio() {
  return layoutHorizontal.value ? splitRatioHorizontal.value : splitRatioVertical.value;
}

function setCurrentSplitRatio(ratio: number) {
  if (layoutHorizontal.value) {
    splitRatioHorizontal.value = ratio;
  } else {
    splitRatioVertical.value = ratio;
  }
}

function persistCurrentSplitRatio() {
  try {
    localStorage.setItem(
      layoutHorizontal.value ? "locus.collab.stagingSplit.horizontal" : "locus.collab.stagingSplit.vertical",
      String(Math.round(getCurrentSplitRatio())),
    );
  } catch { /* ignore */ }
}

function updateSplitFromPointer(event: MouseEvent) {
  if (!sectionsContainerRef.value) return;
  const rect = sectionsContainerRef.value.getBoundingClientRect();
  const total = layoutHorizontal.value ? rect.width : rect.height;
  if (total <= 0) return;
  const offset = layoutHorizontal.value ? event.clientX - rect.left : event.clientY - rect.top;
  const ratio = clampSplitRatio((offset / total) * 100, total);
  setCurrentSplitRatio(ratio);
}

let splitMoveHandler: ((event: MouseEvent) => void) | null = null;
let splitUpHandler: (() => void) | null = null;
let releaseSelectionLock: (() => void) | null = null;

function stopSplitDrag() {
  isDraggingSplit.value = false;
  if (splitMoveHandler) {
    document.removeEventListener("mousemove", splitMoveHandler);
    splitMoveHandler = null;
  }
  if (splitUpHandler) {
    document.removeEventListener("mouseup", splitUpHandler);
    splitUpHandler = null;
  }
  document.body.style.cursor = "";
  releaseSelectionLock?.();
  releaseSelectionLock = null;
}

function onSplitDividerMouseDown(event: MouseEvent) {
  event.preventDefault();
  event.stopPropagation();
  stopSplitDrag();
  isDraggingSplit.value = true;
  updateSplitFromPointer(event);

  splitMoveHandler = (nextEvent: MouseEvent) => {
    if (!isDraggingSplit.value) return;
    updateSplitFromPointer(nextEvent);
  };

  splitUpHandler = () => {
    persistCurrentSplitRatio();
    stopSplitDrag();
  };

  document.addEventListener("mousemove", splitMoveHandler);
  document.addEventListener("mouseup", splitUpHandler);
  document.body.style.cursor = layoutHorizontal.value ? "col-resize" : "row-resize";
  releaseSelectionLock?.();
  releaseSelectionLock = acquireSelectionLock();
}

function onFileClick(e: MouseEvent, f: GitFileChange, source: "gitUnstaged" | "gitStaged") {
  const list = source === "gitUnstaged" ? visibleUnstagedPaths.value : visibleStagedPaths.value;
  const sel = source === "gitUnstaged" ? selectedUnstaged : selectedStaged;
  const lastClicked = source === "gitUnstaged" ? lastClickedUnstaged : lastClickedStaged;

  if (e.shiftKey || e.ctrlKey || e.metaKey) {
    e.preventDefault();
  }

  const result = resolveStagingFileSelection({
    visiblePaths: list,
    selectedPaths: sel.value,
    lastClickedPath: lastClicked.value,
    clickedPath: f.path,
    shiftKey: e.shiftKey,
    ctrlKey: e.ctrlKey,
    metaKey: e.metaKey,
  });

  sel.value = result.nextSelectedPaths;
  lastClicked.value = result.nextLastClickedPath;

  if (result.shouldActivateFile) {
    emit("selectFile", f, source);
  }
}

function onFileContextMenu(e: MouseEvent, f: GitFileChange, source: "gitUnstaged" | "gitStaged") {
  const sel = source === "gitUnstaged" ? selectedUnstaged : selectedStaged;
  // If right-clicked file is not in the current selection, select only that file
  if (!sel.value.has(f.path)) {
    sel.value = new Set([f.path]);
    const lastClicked = source === "gitUnstaged" ? lastClickedUnstaged : lastClickedStaged;
    lastClicked.value = f.path;
  }
  emit("fileContextmenu", e, f, source, sel.value);
}

function stageSelected() {
  const paths = [...selectedUnstaged.value];
  selectedUnstaged.value = new Set();
  lastClickedUnstaged.value = null;
  if (paths.length === 1) {
    emit("stage", paths[0]);
  } else if (paths.length > 1) {
    emit("stageMany", paths);
  }
}

function unstageSelected() {
  const paths = [...selectedStaged.value];
  selectedStaged.value = new Set();
  lastClickedStaged.value = null;
  if (paths.length === 1) {
    emit("unstage", paths[0]);
  } else if (paths.length > 1) {
    emit("unstageMany", paths);
  }
}

const unstagedMetaPartition = computed(() =>
  partitionMetaPaths(props.unstagedFiles),
);
const stagedMetaPartition = computed(() =>
  partitionMetaPaths(props.stagedFiles),
);
const blockedMetaPartition = computed(() =>
  partitionMetaPaths(props.blockedFiles),
);
const unstagedOrphanMetaPaths = computed(() => unstagedMetaPartition.value.orphanMetaPaths);
const stagedOrphanMetaPaths = computed(() => stagedMetaPartition.value.orphanMetaPaths);
const blockedOrphanMetaPaths = computed(() => blockedMetaPartition.value.orphanMetaPaths);
const unstagedOrphanMetaCount = computed(() => unstagedOrphanMetaPaths.value.size);
const stagedOrphanMetaCount = computed(() => stagedOrphanMetaPaths.value.size);
const filteredBlockedFiles = computed(() =>
  hideMeta.value
    ? props.blockedFiles.filter((f) => !blockedMetaPartition.value.hideableMetaPaths.has(f.path))
    : props.blockedFiles,
);
const filteredUnstagedFiles = computed(() =>
  hideMeta.value
    ? props.unstagedFiles.filter((f) => !unstagedMetaPartition.value.hideableMetaPaths.has(f.path))
    : props.unstagedFiles,
);
const filteredStagedFiles = computed(() =>
  hideMeta.value
    ? props.stagedFiles.filter((f) => !stagedMetaPartition.value.hideableMetaPaths.has(f.path))
    : props.stagedFiles,
);
const unstagedTreeRows = computed(() =>
  buildStagingTreeRows(filteredUnstagedFiles.value, collapsedUnstagedFolders.value),
);
const stagedTreeRows = computed(() =>
  buildStagingTreeRows(filteredStagedFiles.value, collapsedStagedFolders.value),
);
const unstagedFolderFileMap = computed(() =>
  buildStagingFolderFileMap(filteredUnstagedFiles.value),
);
const stagedFolderFileMap = computed(() =>
  buildStagingFolderFileMap(filteredStagedFiles.value),
);
const hiddenMetaCount = computed(() => {
  if (!hideMeta.value) return 0;
  return (
    unstagedMetaPartition.value.hideableMetaPaths.size
    + stagedMetaPartition.value.hideableMetaPaths.size
    + blockedMetaPartition.value.hideableMetaPaths.size
  );
});
const unstagedSectionStyle = computed(() => ({
  flex: `0 0 ${getCurrentSplitRatio()}%`,
}));
const visibleUnstagedPaths = computed(() =>
  fileViewMode.value === "tree"
    ? unstagedTreeRows.value.filter(isFileTreeRow).map((row) => row.file.path)
    : filteredUnstagedFiles.value.map((file) => file.path),
);
const visibleStagedPaths = computed(() =>
  fileViewMode.value === "tree"
    ? stagedTreeRows.value.filter(isFileTreeRow).map((row) => row.file.path)
    : filteredStagedFiles.value.map((file) => file.path),
);
const stageAllTitle = computed(() =>
  props.blockedFiles.length > 0
    ? t("collab.stageAllCompatibleTitle")
    : t("collab.stageAll"),
);

// Set of file paths that have a hidden .meta companion in the same list
const unstagedWithMeta = computed(() => {
  if (!hideMeta.value) return new Set<string>();
  const metaPaths = new Set(props.unstagedFiles.filter(f => isMetaFile(f.path)).map(f => f.path));
  return new Set(filteredUnstagedFiles.value.filter(f => metaPaths.has(f.path + ".meta")).map(f => f.path));
});
const stagedWithMeta = computed(() => {
  if (!hideMeta.value) return new Set<string>();
  const metaPaths = new Set(props.stagedFiles.filter(f => isMetaFile(f.path)).map(f => f.path));
  return new Set(filteredStagedFiles.value.filter(f => metaPaths.has(f.path + ".meta")).map(f => f.path));
});

// Prune stale selections when file lists change (e.g. after context-menu stage/unstage)
watch(filteredUnstagedFiles, (list) => {
  const paths = new Set(list.map((f) => f.path));
  pruneCollapsedFolders(collapsedUnstagedFolders, collectStagingFolderPaths(list));
  if (selectedUnstaged.value.size > 0) {
    const pruned = new Set([...selectedUnstaged.value].filter((p) => paths.has(p)));
    if (pruned.size !== selectedUnstaged.value.size) {
      selectedUnstaged.value = pruned;
      if (pruned.size === 0) lastClickedUnstaged.value = null;
    }
  }
  if (lastClickedUnstaged.value && !paths.has(lastClickedUnstaged.value)) {
    lastClickedUnstaged.value = null;
  }
});
watch(filteredStagedFiles, (list) => {
  const paths = new Set(list.map((f) => f.path));
  pruneCollapsedFolders(collapsedStagedFolders, collectStagingFolderPaths(list));
  if (selectedStaged.value.size > 0) {
    const pruned = new Set([...selectedStaged.value].filter((p) => paths.has(p)));
    if (pruned.size !== selectedStaged.value.size) {
      selectedStaged.value = pruned;
      if (pruned.size === 0) lastClickedStaged.value = null;
    }
  }
  if (lastClickedStaged.value && !paths.has(lastClickedStaged.value)) {
    lastClickedStaged.value = null;
  }
});

watch(layoutHorizontal, (horizontal) => {
  persistStagingLayout(horizontal);
  if (isDraggingSplit.value) {
    document.body.style.cursor = horizontal ? "col-resize" : "row-resize";
  }
});
watch(fileViewMode, (mode) => {
  persistStagingViewMode(mode);
});

onBeforeUnmount(() => {
  stopSplitDrag();
});

const showCommitModal = ref(false);
const commitMessage = ref("");
const commitDescription = ref("");
const commitLoading = ref(false);
const commitError = ref<string | null>(null);
const aiGenerating = ref(false);

async function aiGenerateCommitMessage() {
  aiGenerating.value = true;
  commitError.value = null;
  try {
    const result = await gitGenerateCommitMessage(props.selectedModelId || null);
    commitMessage.value = result.title;
    commitDescription.value = result.description;
  } catch (e) {
    commitError.value = normalizeAppError(e).message;
  } finally {
    aiGenerating.value = false;
  }
}

async function doCommit() {
  if (!commitMessage.value.trim()) return;
  commitLoading.value = true;
  commitError.value = null;
  try {
    await gitCommit(commitMessage.value, commitDescription.value || null);
    showCommitModal.value = false;
    commitMessage.value = "";
    commitDescription.value = "";
    emit("committed");
  } catch (e) {
    commitError.value = normalizeAppError(e).message;
  } finally {
    commitLoading.value = false;
  }
}

function openCommitModal() {
  commitError.value = null;
  showCommitModal.value = true;
}

function closeCommitModal() {
  showCommitModal.value = false;
}

function fileStatusClass(status: string): string {
  switch (status) {
    case "M": return "status-modified";
    case "A": case "?": return "status-added";
    case "D": return "status-deleted";
    case "R": return "status-renamed";
    default: return "status-modified";
  }
}

function fileStatusLabel(status: string): string {
  switch (status) {
    case "M": return "M";
    case "A": return "A";
    case "?": return "A";
    case "D": return "D";
    case "R": return "R";
    default: return status;
  }
}

function isFileTreeRow(row: StagingTreeRow): row is Extract<StagingTreeRow, { kind: "file" }> {
  return row.kind === "file";
}

function treeIndentPx(depth: number) {
  if (depth <= 0) return 12;
  return 12 + depth * 20;
}

function toggleFileViewMode(mode: StagingViewMode) {
  fileViewMode.value = mode;
}

function toggleTreeFolder(section: "unstaged" | "staged", chainPaths: readonly string[], expanded: boolean) {
  const target = section === "unstaged" ? collapsedUnstagedFolders : collapsedStagedFolders;
  const next = new Set(target.value);
  if (expanded) {
    const collapsedPath = chainPaths[chainPaths.length - 1];
    if (collapsedPath) next.add(collapsedPath);
  } else {
    for (const path of chainPaths) {
      next.delete(path);
    }
  }
  target.value = next;
}

function folderPathsFor(section: "unstaged" | "staged", path: string): string[] {
  const map = section === "unstaged" ? unstagedFolderFileMap.value : stagedFolderFileMap.value;
  return map.get(path) ?? [];
}

function isFolderPending(section: "unstaged" | "staged", path: string) {
  const paths = folderPathsFor(section, path);
  if (section === "unstaged") {
    return paths.some((filePath) => isStagePending(filePath));
  }
  return paths.some((filePath) => isUnstagePending(filePath));
}

function stageFolder(path: string) {
  const paths = folderPathsFor("unstaged", path);
  if (paths.length === 1) {
    emit("stage", paths[0]);
  } else if (paths.length > 1) {
    emit("stageMany", paths);
  }
}

function unstageFolder(path: string) {
  const paths = folderPathsFor("staged", path);
  if (paths.length === 1) {
    emit("unstage", paths[0]);
  } else if (paths.length > 1) {
    emit("unstageMany", paths);
  }
}

function pruneCollapsedFolders(target: typeof collapsedUnstagedFolders, validPaths: Set<string>) {
  if (target.value.size === 0) return;
  const next = new Set([...target.value].filter((path) => validPaths.has(path)));
  if (next.size !== target.value.size) {
    target.value = next;
  }
}

function fileName(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1];
}

function fileDir(path: string): string {
  const parts = path.split("/");
  if (parts.length <= 1) return "";
  return parts.slice(0, -1).join("/") + "/";
}

function locusBadgeLabel(file: LocusManagedFileLike): string | null {
  const kind = getLocusManagedTagKind(file);
  return kind ? t(`collab.locusTag.${kind}`) : null;
}

function folderLocusBadgeLabel(path: string): string | null {
  const kind = getLocusManagedTagKindForPath(path);
  return kind ? t(`collab.locusTag.${kind}`) : null;
}

function fileTreeIconClass(path: string) {
  return unityAssetIconClassForPath(path, { isFolder: false });
}

function formatBlockedReason(file: GitBlockedPath): string {
  switch (file.reason) {
    case "windowsReservedName":
      return t("collab.blockedReason.windowsReservedName", file.segment);
    case "windowsTrailingDot":
      return t("collab.blockedReason.windowsTrailingDot", file.segment);
    case "windowsTrailingSpace":
      return t("collab.blockedReason.windowsTrailingSpace", file.segment);
    default:
      return file.segment;
  }
}
</script>

<template>
  <div class="files-panel">
    <div class="files-top-header">
      <div class="files-change-count">
        <span class="change-number">{{ totalChanges }}</span>
        <span class="change-label">file changes</span>
        <span v-if="hiddenMetaCount > 0" class="files-change-hidden">({{ t("collab.hiddenMetaInline", hiddenMetaCount) }})</span>
        <span class="change-label">on</span>
        <span class="change-branch">{{ currentBranch || 'HEAD' }}</span>
      </div>
      <div class="header-actions">
        <button
          class="view-toggle-btn"
          :class="{ active: fileViewMode === 'tree' }"
          :aria-pressed="fileViewMode === 'tree'"
          :title="fileViewMode === 'tree' ? t('collab.view.list') : t('collab.view.tree')"
          @click="toggleFileViewMode(fileViewMode === 'tree' ? 'list' : 'tree')"
        >
          <svg v-if="fileViewMode === 'tree'" viewBox="0 0 16 16" width="14" height="14" fill="currentColor" aria-hidden="true">
            <path d="M2.75 3a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75zm0 4.25a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75zm0 4.25a.75.75 0 0 0 0 1.5h10.5a.75.75 0 0 0 0-1.5H2.75z"/>
          </svg>
          <svg v-else viewBox="0 0 16 16" width="14" height="14" fill="none" aria-hidden="true">
            <path d="M3 3.5a1 1 0 1 1 2 0 1 1 0 0 1-2 0zm8.25 0a.75.75 0 0 0 0 1.5h2a.75.75 0 0 0 0-1.5h-2zM5 4.25h2.5v3H11a1.75 1.75 0 0 1 1.75 1.75v1.75h.5a.75.75 0 0 1 0 1.5h-2a.75.75 0 0 1 0-1.5h.5V9A.25.25 0 0 0 11 8.75H7.5v2A1.75 1.75 0 0 1 5.75 12.5h-.5a1 1 0 1 1 0-1.5h.5a.25.25 0 0 0 .25-.25v-6.5H5z" fill="currentColor"/>
          </svg>
        </button>
        <button
          class="hide-meta-btn"
          :class="{ active: hideMeta }"
          @click="hideMeta = !hideMeta"
          :title="t('common.hideMeta')"
        >.meta</button>
        <button
          class="layout-toggle-btn"
          :title="layoutHorizontal ? t('collab.layout.vertical') : t('collab.layout.horizontal')"
          @click="layoutHorizontal = !layoutHorizontal"
        >
          <svg v-if="!layoutHorizontal" viewBox="0 0 16 16" width="14" height="14" fill="currentColor"><path d="M1.5 2A1.5 1.5 0 0 0 0 3.5v9A1.5 1.5 0 0 0 1.5 14h13a1.5 1.5 0 0 0 1.5-1.5v-9A1.5 1.5 0 0 0 14.5 2h-13zM1.5 3h13a.5.5 0 0 1 .5.5V7H1V3.5a.5.5 0 0 1 .5-.5zM1 8h14v4.5a.5.5 0 0 1-.5.5h-13a.5.5 0 0 1-.5-.5V8z"/></svg>
          <svg v-else viewBox="0 0 16 16" width="14" height="14" fill="currentColor"><path d="M1.5 2A1.5 1.5 0 0 0 0 3.5v9A1.5 1.5 0 0 0 1.5 14h13a1.5 1.5 0 0 0 1.5-1.5v-9A1.5 1.5 0 0 0 14.5 2h-13zM1.5 3H7v10H1.5a.5.5 0 0 1-.5-.5v-9a.5.5 0 0 1 .5-.5zM8 3h6.5a.5.5 0 0 1 .5.5v9a.5.5 0 0 1-.5.5H8V3z"/></svg>
        </button>
      </div>
    </div>

    <div ref="sectionsContainerRef" class="sections-container" :class="{ horizontal: layoutHorizontal, dragging: isDraggingSplit }">
      <div class="fixed-section section-unstaged" :style="unstagedSectionStyle">
        <div class="files-section-header unstaged-header">
          <span class="files-section-title">{{ t("collab.unstaged") }}</span>
          <span class="files-section-count">({{ filteredUnstagedFiles.length }})</span>
          <button v-if="hasUnstagedSelection" class="section-action-btn stage-all-btn" :disabled="props.stageOperationBusy" @click.stop="stageSelected" :title="t('collab.stageSelected', selectedUnstaged.size)">{{ t("collab.stageSelected", selectedUnstaged.size) }}</button>
          <button v-else class="section-action-btn stage-all-btn" :disabled="props.stageOperationBusy" @click.stop="emit('stageAll')" :title="stageAllTitle">{{ t("collab.stageAll") }}</button>
        </div>
        <div v-if="unstagedOrphanMetaCount > 0" class="files-section-warning">
          {{ t("collab.orphanMetaWarning", unstagedOrphanMetaCount) }}
        </div>
        <div v-if="filteredBlockedFiles.length > 0" class="files-section-warning blocked-files-warning">
          {{ t("collab.blockedHint", filteredBlockedFiles.length) }}
        </div>
        <div class="fixed-section-scroll" :class="{ 'empty-state': filteredUnstagedFiles.length === 0 && filteredBlockedFiles.length === 0 }">
          <div v-if="filteredBlockedFiles.length > 0" class="file-list blocked-file-list">
            <div
              v-for="f in filteredBlockedFiles"
              :key="'b_' + f.path"
              class="file-row blocked-file-row"
            >
              <div
                class="file-item blocked-file-main"
                :title="`${f.path}\n${formatBlockedReason(f)}`"
              >
                <span class="file-status" :class="fileStatusClass(f.status)">{{ fileStatusLabel(f.status) }}</span>
                <span class="blocked-file-copy">
                  <span class="blocked-file-title">
                    <span class="file-name">{{ fileName(f.path) }}</span>
                    <span v-if="locusBadgeLabel(f)" class="locus-badge">{{ locusBadgeLabel(f) }}</span>
                    <span v-if="blockedOrphanMetaPaths.has(f.path)" class="orphan-meta-badge" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
                  </span>
                  <span class="blocked-file-meta">
                    <span v-if="fileDir(f.path)" class="file-dir">{{ fileDir(f.path) }}</span>
                    <span class="blocked-file-reason">{{ formatBlockedReason(f) }}</span>
                  </span>
                </span>
              </div>
            </div>
          </div>
          <div v-if="filteredUnstagedFiles.length > 0" class="file-list" :class="{ 'staging-tree-list': fileViewMode === 'tree' }">
            <template v-if="fileViewMode === 'tree'">
              <div
                v-for="row in unstagedTreeRows"
                :key="'u_tree_' + row.key"
              >
                <div
                  v-if="row.kind === 'folder'"
                  class="staging-tree-row staging-tree-folder-row"
                  :class="{ pending: isFolderPending('unstaged', row.path) }"
                >
                  <button
                    type="button"
                    class="staging-tree-folder-btn staging-tree-folder-main"
                    :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
                    :title="row.path"
                    :aria-label="row.expanded ? t('merge.tree.toggleCollapse', row.name) : t('merge.tree.toggleExpand', row.name)"
                    @click="toggleTreeFolder('unstaged', row.chainPaths, row.expanded)"
                  >
                    <span class="staging-tree-branch" :class="{ open: row.expanded }" aria-hidden="true">
                      <LucideIcon class="staging-tree-chevron" :icon="ChevronRight" :size="10" />
                    </span>
                    <span
                      class="staging-tree-folder-icon"
                      :class="[{ open: row.expanded }, unityFolderIconClass(row.expanded)]"
                      aria-hidden="true"
                    >
                      <LucideIcon :icon="unityFolderIconNode(row.expanded)" :size="13" />
                    </span>
                    <span class="staging-tree-folder-name">{{ row.name }}</span>
                    <span v-if="folderLocusBadgeLabel(row.path)" class="locus-badge">{{ folderLocusBadgeLabel(row.path) }}</span>
                  </button>
                  <div class="file-row-actions staging-row-actions">
                    <button
                      class="file-stage-btn stage"
                      :class="{ 'is-pending': isFolderPending('unstaged', row.path) }"
                      :disabled="props.stageOperationBusy"
                      @click.stop="stageFolder(row.path)"
                      :title="t('collab.stageSelected', folderPathsFor('unstaged', row.path).length)"
                    >{{ isFolderPending('unstaged', row.path) ? t("collab.staging") : t("collab.stage") }}</button>
                  </div>
                </div>

                <div
                  v-else
                  class="file-row staging-file-row staging-tree-file-row"
                  :class="{ selected: selectedUnstaged.has(row.file.path) || props.activeFilePath === row.file.path, pending: isFilePending(row.file.path) }"
                >
                  <button
                    type="button"
                    class="file-item staging-file-main staging-tree-file-main"
                    :class="{ selected: selectedUnstaged.has(row.file.path) || props.activeFilePath === row.file.path, pending: isFilePending(row.file.path) }"
                    :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
                    :title="row.file.path"
                    @click="onFileClick($event, row.file, 'gitUnstaged')"
                    @contextmenu.prevent="onFileContextMenu($event, row.file, 'gitUnstaged')"
                  >
                    <span class="file-status" :class="fileStatusClass(row.file.status)">{{ fileStatusLabel(row.file.status) }}</span>
                    <LucideIcon
                      class="staging-tree-file-icon"
                      :class="fileTreeIconClass(row.file.path)"
                      :icon="unityAssetIconNodeForPath(row.file.path, { isFolder: false })"
                      :size="14"
                    />
                    <span class="staging-file-copy">
                      <span class="file-name">{{ fileName(row.file.path) }}</span>
                      <span v-if="locusBadgeLabel(row.file)" class="locus-badge">{{ locusBadgeLabel(row.file) }}</span>
                      <span v-if="row.file.lfs" class="lfs-badge">LFS</span>
                      <span v-if="unstagedWithMeta.has(row.file.path)" class="meta-badge" title=".meta file included">.meta</span>
                      <span v-if="unstagedOrphanMetaPaths.has(row.file.path)" class="orphan-meta-badge" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
                    </span>
                  </button>
                  <div class="file-row-actions staging-row-actions">
                    <button
                      class="file-stage-btn stage"
                      :class="{ 'is-pending': isFilePending(row.file.path) }"
                      :disabled="props.stageOperationBusy"
                      @click.stop="emit('stage', row.file.path)"
                      :title="fileActionLabel(row.file.path, 'collab.stage')"
                    >{{ fileActionLabel(row.file.path, "collab.stage") }}</button>
                  </div>
                </div>
              </div>
            </template>

            <template v-else>
              <div
                v-for="f in filteredUnstagedFiles"
                :key="'u_' + f.path"
                class="file-row staging-file-row"
                :class="{ selected: selectedUnstaged.has(f.path) || props.activeFilePath === f.path, pending: isFilePending(f.path) }"
              >
                <button
                  type="button"
                  class="file-item staging-file-main"
                  :class="{ selected: selectedUnstaged.has(f.path) || props.activeFilePath === f.path, pending: isFilePending(f.path) }"
                  :title="f.path"
                  @click="onFileClick($event, f, 'gitUnstaged')"
                  @contextmenu.prevent="onFileContextMenu($event, f, 'gitUnstaged')"
                >
                  <span class="file-status" :class="fileStatusClass(f.status)">{{ fileStatusLabel(f.status) }}</span>
                  <span class="staging-file-copy">
                    <span class="file-name">{{ fileName(f.path) }}</span>
                    <span v-if="locusBadgeLabel(f)" class="locus-badge">{{ locusBadgeLabel(f) }}</span>
                    <span v-if="f.lfs" class="lfs-badge">LFS</span>
                    <span v-if="unstagedWithMeta.has(f.path)" class="meta-badge" title=".meta file included">.meta</span>
                    <span v-if="unstagedOrphanMetaPaths.has(f.path)" class="orphan-meta-badge" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
                    <span class="file-dir">{{ fileDir(f.path) }}</span>
                  </span>
                </button>
                <div class="file-row-actions staging-row-actions">
                  <button
                    class="file-stage-btn stage"
                    :class="{ 'is-pending': isFilePending(f.path) }"
                    :disabled="props.stageOperationBusy"
                    @click.stop="emit('stage', f.path)"
                    :title="fileActionLabel(f.path, 'collab.stage')"
                  >{{ fileActionLabel(f.path, "collab.stage") }}</button>
                </div>
              </div>
            </template>
          </div>
          <div v-else-if="filteredBlockedFiles.length === 0" class="files-empty-section">{{ t("collab.noUnstaged") }}</div>
        </div>
      </div>

      <div class="staging-divider" :class="{ horizontal: layoutHorizontal }" @mousedown="onSplitDividerMouseDown"></div>

      <div class="fixed-section section-staged">
        <div class="files-section-header staged-header">
          <span class="files-section-title">{{ t("collab.staged") }}</span>
          <span class="files-section-count">({{ filteredStagedFiles.length }})</span>
          <button v-if="hasStagedSelection" class="section-action-btn unstage-all-btn" :disabled="props.stageOperationBusy" @click.stop="unstageSelected" :title="t('collab.unstageSelected', selectedStaged.size)">{{ t("collab.unstageSelected", selectedStaged.size) }}</button>
          <button v-else class="section-action-btn unstage-all-btn" :disabled="props.stageOperationBusy" @click.stop="emit('unstageAll')" :title="t('collab.unstageAll')">{{ t("collab.unstageAll") }}</button>
        </div>
        <div v-if="stagedOrphanMetaCount > 0" class="files-section-warning">
          {{ t("collab.orphanMetaWarning", stagedOrphanMetaCount) }}
        </div>
        <div class="fixed-section-scroll" :class="{ 'empty-state': filteredStagedFiles.length === 0 }">
          <div v-if="filteredStagedFiles.length > 0" class="file-list" :class="{ 'staging-tree-list': fileViewMode === 'tree' }">
            <template v-if="fileViewMode === 'tree'">
              <div
                v-for="row in stagedTreeRows"
                :key="'s_tree_' + row.key"
              >
                <div
                  v-if="row.kind === 'folder'"
                  class="staging-tree-row staging-tree-folder-row"
                  :class="{ pending: isFolderPending('staged', row.path) }"
                >
                  <button
                    type="button"
                    class="staging-tree-folder-btn staging-tree-folder-main"
                    :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
                    :title="row.path"
                    :aria-label="row.expanded ? t('merge.tree.toggleCollapse', row.name) : t('merge.tree.toggleExpand', row.name)"
                    @click="toggleTreeFolder('staged', row.chainPaths, row.expanded)"
                  >
                    <span class="staging-tree-branch" :class="{ open: row.expanded }" aria-hidden="true">
                      <LucideIcon class="staging-tree-chevron" :icon="ChevronRight" :size="10" />
                    </span>
                    <span
                      class="staging-tree-folder-icon"
                      :class="[{ open: row.expanded }, unityFolderIconClass(row.expanded)]"
                      aria-hidden="true"
                    >
                      <LucideIcon :icon="unityFolderIconNode(row.expanded)" :size="13" />
                    </span>
                    <span class="staging-tree-folder-name">{{ row.name }}</span>
                    <span v-if="folderLocusBadgeLabel(row.path)" class="locus-badge">{{ folderLocusBadgeLabel(row.path) }}</span>
                  </button>
                  <div class="file-row-actions staging-row-actions">
                    <button
                      class="file-stage-btn unstage"
                      :class="{ 'is-pending': isFolderPending('staged', row.path) }"
                      :disabled="props.stageOperationBusy"
                      @click.stop="unstageFolder(row.path)"
                      :title="t('collab.unstageSelected', folderPathsFor('staged', row.path).length)"
                    >{{ isFolderPending('staged', row.path) ? t("collab.unstaging") : t("collab.unstage") }}</button>
                  </div>
                </div>

                <div
                  v-else
                  class="file-row staging-file-row staging-tree-file-row"
                  :class="{ selected: selectedStaged.has(row.file.path) || props.activeFilePath === row.file.path, pending: isFilePending(row.file.path) }"
                >
                  <button
                    type="button"
                    class="file-item staging-file-main staging-tree-file-main"
                    :class="{ selected: selectedStaged.has(row.file.path) || props.activeFilePath === row.file.path, pending: isFilePending(row.file.path) }"
                    :style="{ paddingLeft: `${treeIndentPx(row.depth)}px` }"
                    :title="row.file.path"
                    @click="onFileClick($event, row.file, 'gitStaged')"
                    @contextmenu.prevent="onFileContextMenu($event, row.file, 'gitStaged')"
                  >
                    <span class="file-status" :class="fileStatusClass(row.file.status)">{{ fileStatusLabel(row.file.status) }}</span>
                    <LucideIcon
                      class="staging-tree-file-icon"
                      :class="fileTreeIconClass(row.file.path)"
                      :icon="unityAssetIconNodeForPath(row.file.path, { isFolder: false })"
                      :size="14"
                    />
                    <span class="staging-file-copy">
                      <span class="file-name">{{ fileName(row.file.path) }}</span>
                      <span v-if="locusBadgeLabel(row.file)" class="locus-badge">{{ locusBadgeLabel(row.file) }}</span>
                      <span v-if="row.file.lfs" class="lfs-badge">LFS</span>
                      <span v-if="stagedWithMeta.has(row.file.path)" class="meta-badge" title=".meta file included">.meta</span>
                      <span v-if="stagedOrphanMetaPaths.has(row.file.path)" class="orphan-meta-badge" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
                    </span>
                  </button>
                  <div class="file-row-actions staging-row-actions">
                    <button
                      class="file-stage-btn unstage"
                      :class="{ 'is-pending': isFilePending(row.file.path) }"
                      :disabled="props.stageOperationBusy"
                      @click.stop="emit('unstage', row.file.path)"
                      :title="fileActionLabel(row.file.path, 'collab.unstage')"
                    >{{ fileActionLabel(row.file.path, "collab.unstage") }}</button>
                  </div>
                </div>
              </div>
            </template>

            <template v-else>
              <div
                v-for="f in filteredStagedFiles"
                :key="'s_' + f.path"
                class="file-row staging-file-row"
                :class="{ selected: selectedStaged.has(f.path) || props.activeFilePath === f.path, pending: isFilePending(f.path) }"
              >
                <button
                  type="button"
                  class="file-item staging-file-main"
                  :class="{ selected: selectedStaged.has(f.path) || props.activeFilePath === f.path, pending: isFilePending(f.path) }"
                  :title="f.path"
                  @click="onFileClick($event, f, 'gitStaged')"
                  @contextmenu.prevent="onFileContextMenu($event, f, 'gitStaged')"
                >
                  <span class="file-status" :class="fileStatusClass(f.status)">{{ fileStatusLabel(f.status) }}</span>
                  <span class="staging-file-copy">
                    <span class="file-name">{{ fileName(f.path) }}</span>
                    <span v-if="locusBadgeLabel(f)" class="locus-badge">{{ locusBadgeLabel(f) }}</span>
                    <span v-if="f.lfs" class="lfs-badge">LFS</span>
                    <span v-if="stagedWithMeta.has(f.path)" class="meta-badge" title=".meta file included">.meta</span>
                    <span v-if="stagedOrphanMetaPaths.has(f.path)" class="orphan-meta-badge" :title="t('collab.orphanMetaHint')">{{ t("collab.orphanMetaTag") }}</span>
                    <span class="file-dir">{{ fileDir(f.path) }}</span>
                  </span>
                </button>
                <div class="file-row-actions staging-row-actions">
                  <button
                    class="file-stage-btn unstage"
                    :class="{ 'is-pending': isFilePending(f.path) }"
                    :disabled="props.stageOperationBusy"
                    @click.stop="emit('unstage', f.path)"
                    :title="fileActionLabel(f.path, 'collab.unstage')"
                  >{{ fileActionLabel(f.path, "collab.unstage") }}</button>
                </div>
              </div>
            </template>
          </div>
          <div v-else class="files-empty-section">{{ t("collab.noStaged") }}</div>
        </div>
      </div>
    </div>

    <div v-if="stagedFiles.length > 0" class="commit-btn-container">
      <button class="commit-btn" @click="openCommitModal">
        <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
          <path d="M11.75 7.5a3.75 3.75 0 1 0-7.5 0 3.75 3.75 0 0 0 7.5 0zm-2.5 0a1.25 1.25 0 1 1-2.5 0 1.25 1.25 0 0 1 2.5 0zM8 12.5a.75.75 0 0 1 .75.75v2a.75.75 0 0 1-1.5 0v-2A.75.75 0 0 1 8 12.5zm0-12a.75.75 0 0 1 .75.75v2a.75.75 0 0 1-1.5 0v-2A.75.75 0 0 1 8 .5z"/>
        </svg>
        Commit
      </button>
    </div>

    <Teleport to="body">
      <div v-if="showCommitModal" class="commit-modal-overlay" @click.self="closeCommitModal">
        <div class="commit-modal">
          <div class="commit-modal-header">
            <span class="commit-modal-title">Commit to <strong>{{ currentBranch || 'HEAD' }}</strong></span>
            <button class="commit-modal-close" @click="closeCommitModal">&times;</button>
          </div>
          <div class="commit-modal-body">
            <div class="commit-input-row">
              <input
                v-model="commitMessage"
                class="commit-input"
                placeholder="Commit message"
                @keydown.enter.exact="doCommit"
                autofocus
              />
              <button
                class="ai-generate-btn"
                :disabled="aiGenerating"
                @click="aiGenerateCommitMessage"
                :title="aiGenerating ? 'Generating...' : 'AI Generate'"
              >
                <svg v-if="!aiGenerating" viewBox="0 0 16 16" width="14" height="14" fill="currentColor">
                  <path d="M8 1a.75.75 0 0 1 .75.75v1.5h1.5a.75.75 0 0 1 0 1.5h-1.5v1.5a.75.75 0 0 1-1.5 0v-1.5h-1.5a.75.75 0 0 1 0-1.5h1.5v-1.5A.75.75 0 0 1 8 1zm4.5 5a.75.75 0 0 1 .75.75v.5h.5a.75.75 0 0 1 0 1.5h-.5v.5a.75.75 0 0 1-1.5 0v-.5h-.5a.75.75 0 0 1 0-1.5h.5v-.5A.75.75 0 0 1 12.5 6zM6 9.5a.75.75 0 0 1 .75.75v1h1a.75.75 0 0 1 0 1.5h-1v1a.75.75 0 0 1-1.5 0v-1h-1a.75.75 0 0 1 0-1.5h1v-1A.75.75 0 0 1 6 9.5z"/>
                </svg>
                <span v-else class="ai-spinner"></span>
              </button>
            </div>
            <textarea
              v-model="commitDescription"
              class="commit-textarea"
              placeholder="Description (optional)"
              rows="4"
            ></textarea>
            <div v-if="commitError" class="commit-error">{{ commitError }}</div>
          </div>
          <div class="commit-modal-footer">
            <span class="commit-staged-count">{{ t("collab.stagedCount", stagedFiles.length) }}</span>
            <div class="commit-modal-actions">
              <button class="commit-cancel-btn" @click="closeCommitModal">Cancel</button>
              <button
                class="commit-confirm-btn"
                :disabled="!commitMessage.trim() || commitLoading"
                @click="doCommit"
              >
                {{ commitLoading ? 'Committing...' : 'Commit' }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>
