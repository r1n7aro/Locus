
<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted, watch } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ModelOption, GitFileChange, DiffSource, FileDiffPayload, UnmergedFileEntry, GitHistoryTarget, GitBranchTarget, GitStashEntry, GitGraphRef } from "../types";
import { gitCommitAction, gitBranchAction, gitStashAction, gitDiscardFile } from "../services/git";
import GitTerminal from "./GitTerminal.vue";
import GitInitOverlay from "./collab/GitInitOverlay.vue";
import GitSidebar from "./collab/GitSidebar.vue";
import GitConfigPopover from "./collab/GitConfigPopover.vue";
import GitGraph from "./collab/GitGraph.vue";
import StagingArea from "./collab/StagingArea.vue";
import CommitDetail from "./collab/CommitDetail.vue";
import MergeQueuePanel from "./collab/MergeQueuePanel.vue";
import MergeResolutionPanel from "./collab/MergeResolutionPanel.vue";
import WorkspaceRequiredState from "./WorkspaceRequiredState.vue";
import FileDiffViewer from "./diff/FileDiffViewer.vue";
import { useCollabState } from "../composables/useCollabState";
import { useProjectStore } from "../stores/project";
import { useNotificationStore } from "../stores/notification";
import { selectUnityAsset, openFileExternal, showInFolder } from "../services/unity";
import { diffSingleFile, refetchDiffByKey } from "../services/diff";
import { canOpenInEditor, useHideMeta, withMetaCompanionPaths } from "../composables/useHideMeta";
import { countLocusManagedFiles } from "../composables/locusManagedFiles";
import { useDiffProgress } from "../composables/useDiffProgress";
import { normalizeAppError } from "../services/errors";
import { t } from "../i18n";
import {
  resolveCollabRightPanelMode,
  resolveConflictActionHint,
  resolveMergeOperationBadge,
} from "./collab/collabViewMode";
import { resolveHistorySelectionKind } from "./collab/historySelection";
import { collectUnanchoredStashHashes } from "./collab/graph/normalize";
import { extractLocalBranchNamesForHash } from "./collab/graph/refs";
import { resolveBranchDblclickAction, resolveBranchTargetHash } from "./collab/branchInteraction";
import type { GitGraphPublicApi, GitGraphSelectionTarget, GitGraphSelectOptions } from "./collab/gitGraphSelection";
import { clampFloatingPosition } from "./ui/floatingPosition";
import {
  COLLAB_SEARCH_SELECT_EVENT,
  openCollabSearchWindow,
  type CollabSearchSelectionPayload,
} from "../services/collabSearchWindow";

const props = defineProps<{
  workingDir: string;
  isActive: boolean;
  selectedModelId: string;
  selectedAgentId: string;
  models: ModelOption[];
}>();

const emit = defineEmits<{
  (e: "selectModel", id: string): void;
}>();

const projectStore = useProjectStore();
const notificationStore = useNotificationStore();
const { hideMeta } = useHideMeta();
const hasWorkspace = computed(() => !!props.workingDir.trim());
const terminalRef = ref<InstanceType<typeof GitTerminal> | null>(null);
const gitGraphRef = ref<GitGraphPublicApi | null>(null);
const gitConfigPopoverOpen = ref(false);
let collabSearchSelectionUnlisten: UnlistenFn | null = null;

const {
  isRepo, commits, graphRefs, headState, loading, selectedHistory, selectedCommitHash, hasMoreCommits, loadingMore,
  initLoading, initError, gitProbeState, gitAvailable, gitHelpText,
  showGitConfigModal, gitConfigName, gitConfigEmail, gitConfigSaving, gitConfigError, currentGitAuthor,
  unstagedFiles, stagedFiles, blockedFiles, commitFiles, commitBody, filesLoading,
  pendingStagePaths, pendingUnstagePaths, stageOperationBusy,
  unmergedFiles, mergeOperation, isMerging, hasUnresolvedFiles,
  localBranches, remoteBranches, stashes, tags, submodules,
  sidebarCollapsed, expandLocal, expandRemotes, expandedRemoteNames, expandStashes, expandTags, expandSubmodules,
  containerRef, leftAreaRef, leftColRef, gitSidebarWidth, leftColWidth, terminalHeight, draggingClass,
  currentBranch, selectedCommit, totalChanges, workspaceChangeCount,
  initGitUnity, saveGitConfigAndInit, cancelGitConfig, toggleRemote,
  stageFile, unstageFile, stageFiles, unstageFiles, stageAll, unstageAll,
  loadMoreCommits, onCommitted, onTerminalDone, onTerminalTouched, onRefresh,
  onSidebarSplitterMouseDown, onVSplitterMouseDown, onHSplitterMouseDown,
} = useCollabState(props, {
  onGitTerminalOutput(command, output, isError) {
    terminalRef.value?.pushOutput(command, output, isError);
  },
});

const hasConflictState = computed(() => isMerging.value || hasUnresolvedFiles.value);
const selectedHistoryKind = computed(() =>
  resolveHistorySelectionKind(
    selectedHistory.value,
    commits.value,
    stashes.value,
    workspaceChangeCount.value > 0,
  ),
);
const mergeOperationBadge = computed(() =>
  resolveMergeOperationBadge(mergeOperation.value, hasUnresolvedFiles.value),
);
const rightPanelMode = computed(() =>
  resolveCollabRightPanelMode(selectedHistoryKind.value, hasConflictState.value),
);
const conflictActionHint = computed(() =>
  resolveConflictActionHint(mergeOperation.value),
);
const commitDetailKind = computed<"commit" | "stash">(() =>
  selectedHistoryKind.value === "stash" ? "stash" : "commit",
);
const commitDetailLabel = computed(() => {
  if (selectedHistory.value?.kind === "stash") return selectedHistory.value.refName.toUpperCase();
  if (selectedCommit.value?.shortHash) return `#${selectedCommit.value.shortHash}`;
  return "HEAD";
});
const unanchoredStashHashes = computed(() =>
  collectUnanchoredStashHashes({
    commits: commits.value,
    stashes: stashes.value,
  }),
);

const inlineDiff = ref<FileDiffPayload | null>(null);
const diffLoading = ref(false);
const activeFilePath = ref<string | null>(null);
const activeDiffSource = ref<DiffSource | null>(null);
const activeDiffCommitHash = ref<string | null>(null);
const pendingDiscardPaths = ref<Set<string>>(new Set());
const discardOperationBusy = computed(() => pendingDiscardPaths.value.size > 0);
const workspaceMutationBusy = computed(() => stageOperationBusy.value || discardOperationBusy.value);
const diffProgress = useDiffProgress();
const diffProgressWidth = computed(() => `${diffProgress.progress.value * 100}%`);
const mergeResolutionRef = ref<{ confirmDiscardChanges?: () => Promise<boolean> } | null>(null);
const fileDiffViewerRef = ref<InstanceType<typeof FileDiffViewer> | null>(null);

const activeWorkspaceFilePath = computed(() => {
  if (activeDiffSource.value === "gitStaged" || activeDiffSource.value === "gitUnstaged") {
    return activeFilePath.value;
  }
  return null;
});

const activeCommitFilePath = computed(() => {
  if (activeDiffSource.value !== "gitCommit") return null;
  if (activeDiffCommitHash.value !== selectedCommitHash.value) return null;
  return activeFilePath.value;
});

// Merge resolution: selected conflict file shown in the left overlay area
const selectedConflictFile = ref<UnmergedFileEntry | null>(null);

function conflictFileKey(file: UnmergedFileEntry): string {
  return `${file.path}:${file.conflictCode}:${file.baseOid}:${file.leftOid}:${file.rightOid}`;
}

function isSameConflictFile(a: UnmergedFileEntry | null, b: UnmergedFileEntry): boolean {
  return !!a && conflictFileKey(a) === conflictFileKey(b);
}

async function onSelectConflictFile(file: UnmergedFileEntry) {
  if (!isSameConflictFile(selectedConflictFile.value, file)) {
    const canLeave = await mergeResolutionRef.value?.confirmDiscardChanges?.() ?? true;
    if (!canLeave) return;
  }
  selectedConflictFile.value = file;
  // Clear any existing diff overlay
  inlineDiff.value = null;
}

function onConflictResolved() {
  selectedConflictFile.value = null;
  onRefresh();
}

function onConflictBack() {
  selectedConflictFile.value = null;
}

function onMergeActionDone() {
  selectedConflictFile.value = null;
  onRefresh();
}

function onConflictQuickResolved(file: UnmergedFileEntry) {
  if (isSameConflictFile(selectedConflictFile.value, file)) {
    selectedConflictFile.value = null;
  }
  onRefresh();
}

async function onSelectFile(file: GitFileChange, source: DiffSource, commitHash?: string) {
  // Toggle: clicking the same file again closes the diff
  const nextCommitHash = source === "gitCommit" ? (commitHash ?? null) : null;
  if (
    activeFilePath.value === file.path
    && inlineDiff.value
    && activeDiffSource.value === source
    && activeDiffCommitHash.value === nextCommitHash
  ) {
    closeDiff();
    return;
  }
  activeFilePath.value = file.path;
  activeDiffSource.value = source;
  activeDiffCommitHash.value = nextCommitHash;
  inlineDiff.value = null;
  diffLoading.value = true;
  diffProgress.reset();
  try {
    const payload = await diffSingleFile({
      source,
      filePath: file.path,
      oldPath: file.oldPath,
      commitHash,
      detail: "full",
    });
    inlineDiff.value = payload;
  } catch (e) {
    console.error("[CollabView] failed to fetch diff:", e);
  } finally {
    diffLoading.value = false;
  }
}

function closeDiff() {
  inlineDiff.value = null;
  activeFilePath.value = null;
  activeDiffSource.value = null;
  activeDiffCommitHash.value = null;
}

function openGitConfigPopover() {
  gitConfigPopoverOpen.value = true;
}

function closeGitConfigPopover() {
  gitConfigPopoverOpen.value = false;
}

function onGitConfigSaved() {
  void onRefresh();
}

watch(hasConflictState, (active, wasActive) => {
  if (!active || wasActive) return;
  selectedCommitHash.value = null;
  closeDiff();
});

watch(selectedCommitHash, (hash) => {
  if (activeDiffSource.value !== "gitCommit") return;
  if (activeDiffCommitHash.value === hash) return;
  closeDiff();
});

async function onLfsPulled() {
  if (!inlineDiff.value) return;
  try {
    const updated = await refetchDiffByKey(inlineDiff.value.key);
    if (updated) inlineDiff.value = updated;
  } catch (e) {
    console.error("[CollabView] refetch after LFS pull failed:", e);
  }
}

// ── Git operation progress ───────────────────────────────────────
async function runGitOp<T>(label: string, command: string, fn: () => Promise<T>): Promise<T> {
  const operation = `collabGitOp:${Date.now()}:${Math.random().toString(36).slice(2)}`;
  notificationStore.addNotice("info", label, {
    operation,
    sticky: true,
    spinner: true,
    replaceOperation: true,
  });
  try {
    const result = await fn();
    const msg = (result as any)?.message || label;
    const isError = (result as any)?.status === "conflict";
    terminalRef.value?.pushOutput(command, msg, isError);
    notificationStore.addNotice(isError ? "error" : "success", msg, {
      operation,
      ttl: 3000,
      replaceOperation: true,
    });
    return result;
  } catch (e) {
    const err = normalizeAppError(e);
    const errMsg = err.message || "Operation failed";
    terminalRef.value?.pushOutput(command, errMsg, true);
    notificationStore.addNotice("error", errMsg, {
      code: err.code,
      operation,
      ttl: 3000,
      replaceOperation: true,
    });
    throw err;
  }
}

// ── Context menu ─────────────────────────────────────────────────

type GitFileTarget = { kind: "file"; file: GitFileChange; source: "gitUnstaged" | "gitStaged"; selectedFiles: GitFileChange[] };
type CollabContextMenuTarget = GitHistoryTarget | GitBranchTarget | GitFileTarget;
type CollabContextMenuState = {
  x: number;
  y: number;
  target: CollabContextMenuTarget;
};

const CONTEXT_MENU_VIEWPORT_MARGIN = 8;
const ctxMenu = ref<CollabContextMenuState | null>(null);
const ctxMenuRef = ref<HTMLElement | null>(null);
const ctxMenuStyle = computed(() => ({
  left: `${ctxMenu.value?.x ?? 0}px`,
  top: `${ctxMenu.value?.y ?? 0}px`,
}));

const promptDialog = ref<{
  title: string;
  placeholder: string;
  value: string;
  action: (val: string) => void;
} | null>(null);

const confirmDialog = ref<{
  title: string;
  message: string;
  warning?: string | null;
  danger: boolean;
  action: () => void;
} | null>(null);

function closeCtxMenu() { ctxMenu.value = null; }

function viewportSize() {
  return {
    width: window.innerWidth || document.documentElement.clientWidth,
    height: window.innerHeight || document.documentElement.clientHeight,
  };
}

function updateCtxMenuPosition() {
  const menu = ctxMenu.value;
  const el = ctxMenuRef.value;
  if (!menu || !el) return;

  const rect = el.getBoundingClientRect();
  const next = clampFloatingPosition(
    { x: menu.x, y: menu.y },
    { width: rect.width, height: rect.height },
    viewportSize(),
    CONTEXT_MENU_VIEWPORT_MARGIN,
  );
  if (next.x === menu.x && next.y === menu.y) return;
  ctxMenu.value = { ...menu, x: next.x, y: next.y };
}

function openCtxMenu(event: MouseEvent, target: CollabContextMenuTarget) {
  ctxMenu.value = { x: event.clientX, y: event.clientY, target };
  void nextTick(updateCtxMenuPosition);
}

function addPendingDiscardPaths(paths: string[]) {
  if (paths.length === 0) return;
  const next = new Set(pendingDiscardPaths.value);
  for (const path of paths) next.add(path);
  pendingDiscardPaths.value = next;
}

function removePendingDiscardPaths(paths: string[]) {
  if (paths.length === 0) return;
  const next = new Set(pendingDiscardPaths.value);
  for (const path of paths) next.delete(path);
  pendingDiscardPaths.value = next;
}

function onHistoryContextMenu(e: MouseEvent, target: GitHistoryTarget) {
  e.preventDefault();
  e.stopPropagation();
  if (target.kind === "commit") selectedCommitHash.value = target.commit.hash;
  if (target.kind === "stash") selectedCommitHash.value = target.stash.hash;
  openCtxMenu(e, target);
}

function onBranchContextMenu(e: MouseEvent, target: GitBranchTarget) {
  e.preventDefault();
  e.stopPropagation();
  openCtxMenu(e, target);
}

function onStashContextMenu(e: MouseEvent, target: GitHistoryTarget) {
  e.preventDefault();
  e.stopPropagation();
  if (target.kind === "stash" && (target.selectedStashes?.length ?? 1) <= 1) {
    selectedCommitHash.value = target.stash.hash;
  }
  openCtxMenu(e, target);
}

function targetHistoryHash(target: GitGraphSelectionTarget): string | null {
  return target.kind === "workspace" ? null : target.hash;
}

function selectHistoryInGraph(target: GitGraphSelectionTarget, options: GitGraphSelectOptions = {}) {
  const graph = gitGraphRef.value;
  if (graph) {
    return graph.selectHistory(target, options);
  }

  const targetHash = targetHistoryHash(target);
  selectedCommitHash.value = options.toggle && selectedCommitHash.value === targetHash
    ? null
    : targetHash;
  return Promise.resolve(false);
}

function onSelectStash(stash: GitStashEntry) {
  void selectHistoryInGraph({ kind: "stash", hash: stash.hash }, { toggle: true });
}

function onSelectTag(tag: GitGraphRef) {
  void selectHistoryInGraph({ kind: "commit", hash: tag.targetHash });
}

function onSelectBranch(target: GitBranchTarget) {
  const hash = resolveBranchTargetHash(target, graphRefs.value);
  if (!hash) return;
  void selectHistoryInGraph({ kind: "commit", hash });
}

function isCollabSearchSelectionPayload(value: unknown): value is CollabSearchSelectionPayload {
  if (!value || typeof value !== "object") return false;
  const payload = value as Partial<CollabSearchSelectionPayload>;
  return (payload.kind === "commit" || payload.kind === "stash")
    && typeof payload.hash === "string"
    && payload.hash.trim().length > 0;
}

async function ensureSearchCommitLoaded(hash: string): Promise<boolean> {
  if (commits.value.some(commit => commit.hash === hash)) return true;

  let attempts = 0;
  while (hasMoreCommits.value && attempts < 200) {
    attempts += 1;
    const beforeCount = commits.value.length;
    await loadMoreCommits();
    await nextTick();
    if (commits.value.some(commit => commit.hash === hash)) return true;
    if (commits.value.length === beforeCount) break;
  }
  return false;
}

async function ensureSearchStashLoaded(hash: string): Promise<boolean> {
  if (stashes.value.some(stash => stash.hash === hash)) return true;
  await onRefresh();
  return stashes.value.some(stash => stash.hash === hash);
}

async function selectCollabSearchResult(payload: CollabSearchSelectionPayload) {
  const hash = payload.hash.trim();
  const loaded = payload.kind === "stash"
    ? await ensureSearchStashLoaded(hash)
    : await ensureSearchCommitLoaded(hash);

  if (!loaded) {
    notificationStore.addNotice("warning", t("collab.search.targetMissing"), {
      operation: "collabSearchSelect",
      ttl: 3000,
    });
    return;
  }

  await selectHistoryInGraph(
    { kind: payload.kind, hash },
    { scroll: true, behavior: "smooth" },
  );
}

async function openCollabSearch() {
  try {
    await openCollabSearchWindow();
  } catch (cause) {
    const err = normalizeAppError(cause);
    notificationStore.addNotice("error", err.message, {
      code: err.code,
      operation: "openCollabSearch",
    });
  }
}

onMounted(async () => {
  try {
    collabSearchSelectionUnlisten = await listen<CollabSearchSelectionPayload>(
      COLLAB_SEARCH_SELECT_EVENT,
      (event) => {
        if (!isCollabSearchSelectionPayload(event.payload)) return;
        void selectCollabSearchResult(event.payload);
      },
    );
  } catch {
    collabSearchSelectionUnlisten = null;
  }
});

onUnmounted(() => {
  collabSearchSelectionUnlisten?.();
  collabSearchSelectionUnlisten = null;
});

function onFileContextMenu(e: MouseEvent, file: GitFileChange, source: "gitUnstaged" | "gitStaged", selectedPaths: Set<string>) {
  const allFiles = source === "gitUnstaged" ? unstagedFiles.value : stagedFiles.value;
  // If the right-clicked file is in the selection, operate on all selected files; otherwise just the one
  const selectedFiles = selectedPaths.has(file.path)
    ? allFiles.filter((f) => selectedPaths.has(f.path))
    : [file];
  openCtxMenu(e, { kind: "file", file, source, selectedFiles });
}

function confirmDiscardFile() {
  if (workspaceMutationBusy.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "file") return;
  const { source, selectedFiles } = target;
  const sourceFiles = source === "gitUnstaged" ? unstagedFiles.value : stagedFiles.value;
  const expandedPaths = withMetaCompanionPaths(
    selectedFiles.map(file => file.path),
    sourceFiles.map(file => file.path),
    hideMeta.value,
  );
  const sourceFileMap = new Map(sourceFiles.map(file => [file.path, file] as const));
  const discardFiles = expandedPaths
    .map(path => sourceFileMap.get(path))
    .filter((file): file is GitFileChange => !!file);
  const locusManagedCount = countLocusManagedFiles(selectedFiles);
  closeCtxMenu();
  const count = selectedFiles.length;
  const isSingle = count === 1;
  const singleFile = selectedFiles[0];
  confirmDialog.value = {
    title: isSingle
      ? t("collab.discardTitle", singleFile.path.split("/").pop()!)
      : t("collab.discardTitleMulti", count),
    message: isSingle && singleFile.status === "?"
      ? t("collab.discardMsgUntracked")
      : isSingle
        ? t("collab.discardMsgSingle")
        : t("collab.discardMsgMulti", count),
    warning: locusManagedCount <= 0
      ? null
      : isSingle
        ? t("collab.discardLocusWarnSingle")
        : t("collab.discardLocusWarnMulti", locusManagedCount),
    danger: true,
    action: async () => {
      addPendingDiscardPaths(expandedPaths);
      try {
        if (source === "gitStaged") {
          await unstageFiles(expandedPaths);
        }
        for (const f of discardFiles) {
          await gitDiscardFile(f.path, f.status, f.oldPath);
        }
      } catch (e) {
        const err = normalizeAppError(e);
        notificationStore.addNotice("error", err.message || t("collab.discardFailed"), {
          code: err.code,
          operation: "collabDiscard",
        });
      } finally {
        try {
          await onRefresh();
        } finally {
          removePendingDiscardPaths(expandedPaths);
        }
      }
    },
  };
}

function doFileStage() {
  if (workspaceMutationBusy.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "file") return;
  closeCtxMenu();
  if (target.selectedFiles.length === 1) {
    stageFile(target.selectedFiles[0].path);
  } else {
    stageFiles(target.selectedFiles.map(f => f.path));
  }
}

function doFileUnstage() {
  if (workspaceMutationBusy.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "file") return;
  closeCtxMenu();
  if (target.selectedFiles.length === 1) {
    unstageFile(target.selectedFiles[0].path);
  } else {
    unstageFiles(target.selectedFiles.map(f => f.path));
  }
}

async function doShowInFolder() {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "file") return;
  const filePath = target.file.path;
  closeCtxMenu();
  try {
    await showInFolder(filePath);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message || "Failed to show file in folder", {
      code: err.code,
      operation: "collabShowInFolder",
    });
  }
}

function commitLocalBranches(): string[] {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "commit") return [];
  return extractLocalBranchNamesForHash(target.commit.hash, graphRefs.value);
}

async function doCheckoutBranch(branchName: string) {
  if (hasConflictState.value) return;
  closeCtxMenu();
  try {
    await runGitOp(`switch ${branchName}`, `git switch ${branchName}`, () => gitBranchAction(branchName, "local", "switch"));
  } catch {}
  onRefresh();
}

type BranchActionName = "switch" | "checkoutTracking" | "mergeIntoCurrent" | "rebaseCurrentOnto";

async function performBranchAction(
  branchName: string,
  targetKind: "local" | "remote",
  action: BranchActionName,
) {
  if (hasConflictState.value) return;
  const cmdMap: Record<BranchActionName, string> = {
    switch: `git switch ${branchName}`,
    checkoutTracking: `git checkout --track ${branchName}`,
    mergeIntoCurrent: `git merge ${branchName}`,
    rebaseCurrentOnto: `git rebase ${branchName}`,
  };
  const cmd = cmdMap[action];
  try {
    await runGitOp(`${action} ${branchName}`, cmd, () => gitBranchAction(branchName, targetKind, action));
  } catch {}
  onRefresh();
}

async function runBranchAction(target: GitBranchTarget, action: BranchActionName) {
  if (action === "switch" && target.kind === "localBranch" && target.branch.isCurrent) return;
  const branchName = target.kind === "localBranch"
    ? target.branch.name
    : `${target.remoteName}/${target.branch.name}`;
  const targetKind = target.kind === "localBranch" ? "local" : "remote";
  await performBranchAction(branchName, targetKind, action);
}

async function doCommitAction(action: string, mode?: string) {
  if (hasConflictState.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "commit") return;
  const rev = target.commit.hash;
  const shortHash = target.commit.shortHash;
  closeCtxMenu();
  const label = mode ? `${action} (${mode}) → ${shortHash}` : `${action} → ${shortHash}`;
  const cmd = mode ? `git ${action} --${mode} ${shortHash}` : `git ${action} ${shortHash}`;
  try {
    await runGitOp(label, cmd, () => gitCommitAction(rev, action, mode));
  } catch {}
  onRefresh();
}

async function doStashAction(action: string) {
  if (hasConflictState.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "stash") return;
  const selected = target.selectedStashes?.length ? target.selectedStashes : [target.stash];
  if (selected.length !== 1) return;
  const refName = selected[0].refName;
  closeCtxMenu();
  try {
    await runGitOp(`stash ${action} ${refName}`, `git stash ${action} ${refName}`, () => gitStashAction(refName, action));
  } catch {}
  onRefresh();
}

async function doBranchAction(action: BranchActionName) {
  const target = ctxMenu.value?.target;
  if (!target || (target.kind !== "localBranch" && target.kind !== "remoteBranch")) return;
  closeCtxMenu();
  await runBranchAction(target, action);
}

async function onBranchDblclick(target: GitBranchTarget) {
  const action = resolveBranchDblclickAction(target, localBranches.value);
  if (!action) return;
  await performBranchAction(action.branchName, action.targetKind, action.action);
}

function confirmResetHard() {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "commit") return;
  const hash = target.commit.shortHash;
  const rev = target.commit.hash;
  closeCtxMenu();
  confirmDialog.value = {
    title: t("collab.resetHardTitle", hash),
    message: t("collab.resetHardMsg"),
    danger: true,
    action: async () => {
      try {
        await runGitOp(`reset --hard → ${hash}`, `git reset --hard ${hash}`, () => gitCommitAction(rev, "reset", "hard"));
      } catch {}
      onRefresh();
    },
  };
}

function confirmDropStash() {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "stash") return;
  const selected = target.selectedStashes?.length ? target.selectedStashes : [target.stash];
  const count = selected.length;
  closeCtxMenu();
  confirmDialog.value = {
    title: count === 1
      ? t("collab.dropStashTitle", selected[0].refName)
      : `Drop ${count} stashes?`,
    message: count === 1
      ? t("collab.dropStashMsg")
      : "These stashes will be permanently removed.",
    danger: true,
    action: async () => {
      try {
        await runGitOp(
          count === 1 ? `stash drop ${selected[0].refName}` : `stash drop ${count} stashes`,
          count === 1 ? `git stash drop ${selected[0].refName}` : "git stash drop <selected>",
          async () => {
            const dropOrder = [...selected].sort((left, right) => right.index - left.index);
            for (const stash of dropOrder) {
              await gitStashAction(stash.refName, "drop");
            }
            return {
              status: "success" as const,
              message: count === 1 ? `Dropped ${selected[0].refName}` : `Dropped ${count} stashes`,
              stdout: "",
              stderr: "",
            };
          },
        );
      } catch {}
      onRefresh();
    },
  };
}

function confirmDeleteBranch() {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "localBranch") return;
  const name = target.branch.name;
  closeCtxMenu();
  confirmDialog.value = {
    title: t("collab.deleteBranchTitle", name),
    message: t("collab.deleteBranchMsg"),
    danger: true,
    action: async () => {
      try {
        await runGitOp(`delete branch ${name}`, `git branch -d ${name}`, () => gitBranchAction(name, "local", "delete"));
      } catch {}
      onRefresh();
    },
  };
}

function promptNewBranch() {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "commit") return;
  const hash = target.commit.shortHash;
  const rev = target.commit.hash;
  closeCtxMenu();
  promptDialog.value = {
    title: t("collab.createBranchTitle", hash),
    placeholder: t("collab.createBranchPlaceholder", hash),
    value: "",
    action: async (val: string) => {
      try {
        await runGitOp(`create branch ${val.trim()}`, `git checkout -b ${val.trim()} ${hash}`, () => gitCommitAction(rev, "createBranchAndCheckout", undefined, val.trim()));
      } catch {}
      onRefresh();
    },
  };
}

function promptRenameBranch() {
  const target = ctxMenu.value?.target;
  if (target?.kind !== "localBranch") return;
  const oldName = target.branch.name;
  closeCtxMenu();
  promptDialog.value = {
    title: t("collab.renameBranch"),
    placeholder: oldName,
    value: oldName,
    action: async (val: string) => {
      try {
        await runGitOp(`rename ${oldName} → ${val.trim()}`, `git branch -m ${oldName} ${val.trim()}`, () => gitBranchAction(oldName, "local", "rename", val.trim()));
      } catch {}
      onRefresh();
    },
  };
}

function submitPrompt() {
  if (!promptDialog.value || !promptDialog.value.value.trim()) return;
  promptDialog.value.action(promptDialog.value.value);
  promptDialog.value = null;
}

function doConfirm() {
  if (!confirmDialog.value) return;
  confirmDialog.value.action();
  confirmDialog.value = null;
}

function copyBranchName() {
  const target = ctxMenu.value?.target;
  if (!target) return;
  let name = "";
  if (target.kind === "localBranch") name = target.branch.name;
  else if (target.kind === "remoteBranch") name = `${target.remoteName}/${target.branch.name}`;
  navigator.clipboard.writeText(name);
  closeCtxMenu();
  notificationStore.addNotice("success", t("collab.branchCopied"), {
    operation: "collabCopyBranch",
  });
}
</script>

<template>
  <div class="collab-view" ref="containerRef" :class="draggingClass">
    <WorkspaceRequiredState
      v-if="!hasWorkspace"
      :description="t('workspace.required.collabDescription')"
    />

    <template v-else>
      <GitInitOverlay
        :is-repo="isRepo"
        :loading="loading"
        :init-loading="initLoading"
        :init-error="initError"
        :git-probe-state="gitProbeState"
        :git-available="gitAvailable"
        :git-help-text="gitHelpText"
        :show-git-config-modal="showGitConfigModal"
        :git-config-name="gitConfigName"
        :git-config-email="gitConfigEmail"
        :git-config-saving="gitConfigSaving"
        :git-config-error="gitConfigError"
        @init="initGitUnity"
        @config-save="saveGitConfigAndInit"
        @config-cancel="cancelGitConfig"
        @refresh-git-probe="onRefresh"
        @update:git-config-name="gitConfigName = $event"
        @update:git-config-email="gitConfigEmail = $event"
      />

      <template v-if="isRepo">

    <div class="left-area" ref="leftAreaRef" :style="{ flexBasis: leftColWidth + '%' }">
      <div class="git-sidebar-shell" :style="!sidebarCollapsed ? { width: gitSidebarWidth + 'px' } : undefined">
        <GitSidebar
          :local-branches="localBranches"
          :remote-branches="remoteBranches"
          :stashes="stashes"
          :unanchored-stash-hashes="unanchoredStashHashes"
          :tags="tags"
          :submodules="submodules"
          :selected-history-hash="selectedCommitHash"
          :sidebar-collapsed="sidebarCollapsed"
          :expand-local="expandLocal"
          :expand-remotes="expandRemotes"
          :expanded-remote-names="expandedRemoteNames"
          :expand-stashes="expandStashes"
          :expand-tags="expandTags"
          :expand-submodules="expandSubmodules"
          @toggle-sidebar="sidebarCollapsed = !sidebarCollapsed"
          @toggle-local="expandLocal = !expandLocal"
          @toggle-remotes="expandRemotes = !expandRemotes"
          @toggle-remote-name="toggleRemote"
          @toggle-stashes="expandStashes = !expandStashes"
          @toggle-tags="expandTags = !expandTags"
          @toggle-submodules="expandSubmodules = !expandSubmodules"
          @select-stash="onSelectStash"
          @select-tag="onSelectTag"
          @select-branch="onSelectBranch"
          @branch-contextmenu="onBranchContextMenu"
          @branch-dblclick="onBranchDblclick"
          @stash-contextmenu="onStashContextMenu"
          @open-git-config="openGitConfigPopover"
          @open-search="openCollabSearch"
        />
      </div>
      <div
        v-if="!sidebarCollapsed"
        class="git-sidebar-divider"
        @mousedown="onSidebarSplitterMouseDown"
      ></div>

      <div class="left-column" ref="leftColRef">
        <GitGraph
          ref="gitGraphRef"
          :commits="commits"
          :graph-refs="graphRefs"
          :head-state="headState"
          :selected-history="selectedHistory"
          :loading="loading"
          :loading-more="loadingMore"
          :has-more-commits="hasMoreCommits"
          :current-branch="currentBranch"
          :current-author="currentGitAuthor"
          :stashes="stashes"
          :workspace-change-count="workspaceChangeCount"
          :is-merging="hasConflictState"
          :operation-badge="mergeOperationBadge"
          @select-commit="selectedCommitHash = $event"
          @load-more="loadMoreCommits"
          @history-contextmenu="onHistoryContextMenu"
        />

        <div class="panel-divider-h" @mousedown="onHSplitterMouseDown"></div>

        <div class="terminal-panel" :style="{ height: terminalHeight + 'px' }">
          <GitTerminal
            ref="terminalRef"
            :working-dir="props.workingDir"
            :selected-model-id="props.selectedModelId"
            :selected-agent-id="props.selectedAgentId"
            :current-branch="currentBranch"
            :models="props.models"
            @command-done="onTerminalDone"
            @workspace-touched="onTerminalTouched"
            @select-model="(id: string) => emit('selectModel', id)"
          />
        </div>
      </div>

      <GitConfigPopover
        :open="gitConfigPopoverOpen"
        @close="closeGitConfigPopover"
        @saved="onGitConfigSaved"
      />

      <!-- Merge resolution overlay: covers sidebar + left column -->
      <div v-if="selectedConflictFile" class="inline-diff-panel">
        <MergeResolutionPanel
          ref="mergeResolutionRef"
          :key="conflictFileKey(selectedConflictFile)"
          :file="selectedConflictFile"
          @resolved="onConflictResolved"
          @back="onConflictBack"
        />
      </div>

      <!-- Inline diff overlay: covers sidebar + left column -->
      <div v-else-if="inlineDiff || diffLoading" class="inline-diff-panel">
        <template v-if="inlineDiff">
          <div class="inline-diff-header">
            <button class="inline-diff-back" @click="closeDiff" title="Back">
              <svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor"><path d="M7.78 12.53a.75.75 0 0 1-1.06 0L2.47 8.28a.75.75 0 0 1 0-1.06l4.25-4.25a.75.75 0 0 1 1.06 1.06L4.56 7.25h7.69a.75.75 0 0 1 0 1.5H4.56l3.22 3.22a.75.75 0 0 1 0 1.06z"/></svg>
            </button>
            <span class="inline-diff-status" :class="'status-' + (inlineDiff.status ?? '').toLowerCase()">{{ inlineDiff.status }}</span>
            <span v-if="inlineDiff.oldPath" class="inline-diff-path">{{ inlineDiff.oldPath }} → {{ inlineDiff.filePath }}</span>
            <span v-else class="inline-diff-path">{{ inlineDiff.filePath }}</span>
            <span v-if="inlineDiff.stats" class="inline-diff-stats">
              <span class="diff-additions">+{{ inlineDiff.stats.additions }}</span>
              <span class="diff-deletions">-{{ inlineDiff.stats.deletions }}</span>
            </span>
            <span class="inline-diff-actions">
              <span v-if="fileDiffViewerRef?.hasSemanticAndText" class="inline-diff-tab-group">
                <button
                  class="inline-diff-tab-btn"
                  :class="{ active: fileDiffViewerRef.activeTab === 'semantic' }"
                  @click="fileDiffViewerRef.activeTab = 'semantic'"
                >{{ t('diff.tabs.semantic') }}</button>
                <button
                  class="inline-diff-tab-btn"
                  :class="{ active: fileDiffViewerRef.activeTab === 'text' }"
                  @click="fileDiffViewerRef.activeTab = 'text'"
                >{{ t('diff.tabs.text') }}</button>
              </span>
              <button
                v-if="projectStore.unityConnected"
                class="inline-diff-action-btn"
                :title="t('common.selectInUnity')"
                @click="selectUnityAsset(inlineDiff.filePath)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
                {{ t('common.selectInUnity') }}
              </button>
              <button
                v-if="!inlineDiff.isBinary && canOpenInEditor(inlineDiff.filePath)"
                class="inline-diff-action-btn"
                :title="t('common.openInEditor')"
                @click="openFileExternal(inlineDiff.filePath)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
                {{ t('common.openInEditor') }}
              </button>
            </span>
          </div>
          <div class="inline-diff-body">
            <FileDiffViewer ref="fileDiffViewerRef" :payload="inlineDiff" :hide-builtin-tabs="true" @lfs-pulled="onLfsPulled" />
          </div>
        </template>
        <div v-else class="inline-diff-loading">
          <div class="diff-progress-info">
            <span class="diff-progress-text">{{ diffProgress.phaseLabel }}</span>
            <div class="diff-progress-bar">
              <div class="diff-progress-fill" :style="{ width: diffProgressWidth }"></div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="panel-divider-v" @mousedown="onVSplitterMouseDown"></div>

    <MergeQueuePanel
      v-if="rightPanelMode === 'merge'"
      :unmerged-files="unmergedFiles"
      :staged-files="stagedFiles"
      :operation="mergeOperation"
      :current-branch="currentBranch"
      :has-unresolved-files="hasUnresolvedFiles"
      :selected-conflict-path="selectedConflictFile?.path ?? null"
      @select-conflict-file="onSelectConflictFile"
      @file-resolved="onConflictQuickResolved"
      @action-done="onMergeActionDone"
    />

    <CommitDetail
      v-else-if="rightPanelMode === 'commit'"
      :commit="selectedCommit"
      :commit-body="commitBody"
      :commit-files="commitFiles"
      :files-loading="filesLoading"
      :detail-kind="commitDetailKind"
      :detail-label="commitDetailLabel"
      :active-file-path="activeCommitFilePath"
      @select-file="(f: GitFileChange) => onSelectFile(f, 'gitCommit', selectedCommitHash ?? undefined)"
    />

    <StagingArea
      v-else
      :unstaged-files="unstagedFiles"
      :staged-files="stagedFiles"
      :blocked-files="blockedFiles"
      :selected-model-id="props.selectedModelId"
      :models="props.models"
      :current-branch="currentBranch"
      :total-changes="totalChanges"
      :active-file-path="activeWorkspaceFilePath"
      :pending-stage-paths="pendingStagePaths"
      :pending-unstage-paths="pendingUnstagePaths"
      :pending-discard-paths="pendingDiscardPaths"
      :stage-operation-busy="workspaceMutationBusy"
      @stage="stageFile"
      @unstage="unstageFile"
      @stage-many="stageFiles"
      @unstage-many="unstageFiles"
      @stage-all="stageAll"
      @unstage-all="unstageAll"
      @committed="onCommitted"
      @select-model="(id: string) => emit('selectModel', id)"
      @select-file="(f: GitFileChange, source: 'gitUnstaged' | 'gitStaged') => onSelectFile(f, source)"
      @file-contextmenu="onFileContextMenu"
    />

      </template><!-- end v-if isRepo -->
    </template>

    <!-- Context menu -->
    <Teleport to="body">
      <div v-if="ctxMenu" class="ctx-backdrop" @click="closeCtxMenu" @contextmenu.prevent="closeCtxMenu">
        <div ref="ctxMenuRef" class="ctx-menu" :style="ctxMenuStyle" @click.stop>
          <!-- disabled reason hint -->
          <div v-if="hasConflictState" class="ctx-item ctx-hint">{{ conflictActionHint }}</div>
          <div v-if="hasConflictState" class="ctx-sep" />
          <!-- commit -->
          <template v-if="ctxMenu.target.kind === 'commit'">
            <template v-for="br in commitLocalBranches()" :key="br">
              <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doCheckoutBranch(br)">Checkout Branch「{{ br }}」</div>
            </template>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doCommitAction('checkoutDetached')">Checkout Detached HEAD</div>
            <div class="ctx-sep" />
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doCommitAction('reset', 'soft')">Soft Reset</div>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doCommitAction('reset', 'mixed')">Mixed Reset</div>
            <div class="ctx-item ctx-danger" :class="{ disabled: hasConflictState }" @click="confirmResetHard()">Hard Reset</div>
            <div class="ctx-sep" />
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doCommitAction('revert')">Revert Commit</div>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="promptNewBranch()">Create Branch…</div>
          </template>
          <!-- stash -->
          <template v-else-if="ctxMenu.target.kind === 'stash'">
            <template v-if="(ctxMenu.target.selectedStashes?.length ?? 1) <= 1">
              <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doStashAction('apply')">Apply Stash</div>
              <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doStashAction('pop')">Pop Stash</div>
              <div class="ctx-item ctx-danger" :class="{ disabled: hasConflictState }" @click="confirmDropStash()">Drop Stash</div>
            </template>
            <template v-else>
              <div class="ctx-item ctx-danger" :class="{ disabled: hasConflictState }" @click="confirmDropStash()">Drop {{ ctxMenu.target.selectedStashes?.length }} Stashes</div>
            </template>
          </template>
          <!-- local branch -->
          <template v-else-if="ctxMenu.target.kind === 'localBranch'">
            <div class="ctx-item" :class="{ disabled: hasConflictState || ctxMenu.target.branch.isCurrent }" @click="doBranchAction('switch')">Switch to Branch</div>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doBranchAction('mergeIntoCurrent')">Merge into Current</div>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doBranchAction('rebaseCurrentOnto')">Rebase Current onto This</div>
            <div class="ctx-sep" />
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="promptRenameBranch()">Rename Branch…</div>
            <div class="ctx-item ctx-danger" :class="{ disabled: hasConflictState || ctxMenu.target.branch.isCurrent }" @click="confirmDeleteBranch()">Delete Branch</div>
            <div class="ctx-sep" />
            <div class="ctx-item" @click="copyBranchName()">Copy Branch Name</div>
          </template>
          <!-- remote branch -->
          <template v-else-if="ctxMenu.target.kind === 'remoteBranch'">
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doBranchAction('checkoutTracking')">Checkout as Local Tracking</div>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doBranchAction('mergeIntoCurrent')">Merge into Current</div>
            <div class="ctx-item" :class="{ disabled: hasConflictState }" @click="doBranchAction('rebaseCurrentOnto')">Rebase Current onto This</div>
            <div class="ctx-sep" />
            <div class="ctx-item" @click="copyBranchName()">Copy Remote Branch Name</div>
          </template>
          <!-- file -->
          <template v-else-if="ctxMenu.target.kind === 'file'">
            <div class="ctx-item" @click="doShowInFolder()">Show In Folder</div>
            <div class="ctx-sep" />
            <div
              v-if="ctxMenu.target.source === 'gitUnstaged'"
              class="ctx-item"
              :class="{ disabled: workspaceMutationBusy }"
              @click="doFileStage()"
            >{{ ctxMenu.target.selectedFiles.length > 1 ? `Stage ${ctxMenu.target.selectedFiles.length} Files` : 'Stage' }}</div>
            <div
              v-else
              class="ctx-item"
              :class="{ disabled: workspaceMutationBusy }"
              @click="doFileUnstage()"
            >{{ ctxMenu.target.selectedFiles.length > 1 ? `Unstage ${ctxMenu.target.selectedFiles.length} Files` : 'Unstage' }}</div>
            <div class="ctx-sep" />
            <div
              class="ctx-item ctx-danger"
              :class="{ disabled: workspaceMutationBusy }"
              @click="confirmDiscardFile()"
            >{{ ctxMenu.target.selectedFiles.length > 1 ? `Discard ${ctxMenu.target.selectedFiles.length} Files` : 'Discard Changes' }}</div>
          </template>
        </div>
      </div>

      <!-- Prompt dialog -->
      <div v-if="promptDialog" class="commit-modal-overlay" @click.self="promptDialog = null">
        <div class="commit-modal" style="max-width: 400px">
          <div class="commit-modal-header">
            <span class="commit-modal-title">{{ promptDialog.title }}</span>
            <button class="commit-modal-close" @click="promptDialog = null">&times;</button>
          </div>
          <div class="commit-modal-body">
            <input v-model="promptDialog.value" class="commit-input" :placeholder="promptDialog.placeholder" @keyup.enter="submitPrompt" />
          </div>
          <div class="commit-modal-footer">
            <div class="commit-modal-actions">
              <button class="commit-cancel-btn" @click="promptDialog = null">{{ t('common.cancel') }}</button>
              <button class="commit-confirm-btn" :disabled="!promptDialog.value.trim()" @click="submitPrompt">{{ t('common.confirm') }}</button>
            </div>
          </div>
        </div>
      </div>

      <!-- Confirm dialog -->
      <div v-if="confirmDialog" class="commit-modal-overlay" @click.self="confirmDialog = null">
        <div class="commit-modal" style="max-width: 380px">
          <div class="commit-modal-header">
            <span class="commit-modal-title">{{ confirmDialog.title }}</span>
            <button class="commit-modal-close" @click="confirmDialog = null">&times;</button>
          </div>
          <div class="commit-modal-body">
            <p class="commit-modal-message">{{ confirmDialog.message }}</p>
            <p v-if="confirmDialog.warning" class="commit-modal-warning">{{ confirmDialog.warning }}</p>
          </div>
          <div class="commit-modal-footer">
            <div class="commit-modal-actions">
              <button class="commit-cancel-btn" @click="confirmDialog = null">{{ t('common.cancel') }}</button>
              <button class="commit-confirm-btn" :style="confirmDialog.danger ? 'background: var(--danger, #d73a49)' : ''" @click="doConfirm">{{ t('common.confirm') }}</button>
            </div>
          </div>
        </div>
      </div>
    </Teleport>

  </div>
</template>

<style scoped>
.collab-view {
  position: relative;
  flex: 1;
  display: flex;
  flex-direction: row;
  height: 100%;
  min-width: 0;
  background: var(--bg-color);
}

:deep(.vcs-init-overlay) {
  position: absolute;
  inset: 0;
  z-index: 10;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background: var(--bg-color);
  color: var(--text-secondary);
}

:deep(.collab-view.dragging-v) {
  cursor: col-resize;
}

:deep(.collab-view.dragging-sidebar) {
  cursor: col-resize;
}

:deep(.collab-view.dragging-h) {
  cursor: row-resize;
}

:deep(.left-area) {
  position: relative;
  display: flex;
  flex-direction: row;
  flex-shrink: 0;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

:deep(.left-column) {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

.git-sidebar-shell {
  display: flex;
  flex-shrink: 0;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
}

:deep(.graph-panel) {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
}

:deep(.graph-empty) {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  color: var(--text-secondary);
}

:deep(.empty-icon-sm) {
  font-size: 32px;
  opacity: 0.4;
}

:deep(.empty-text) {
  font-size: 14px;
  font-weight: 500;
}

:deep(.empty-hint) {
  font-size: 12px;
  opacity: 0.6;
}

:deep(.vcs-init-options) {
  display: flex;
  gap: 12px;
  margin-top: 12px;
}

:deep(.vcs-init-btn) {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 16px 24px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-secondary);
  color: var(--text-primary);
  cursor: pointer;
  transition: all 0.15s ease;
  min-width: 180px;
}

:deep(.vcs-init-btn:hover:not(:disabled)) {
  border-color: var(--accent);
  background: var(--bg-hover);
}

:deep(.vcs-init-btn:disabled) {
  opacity: 0.4;
  cursor: not-allowed;
}

:deep(.vcs-init-icon) {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 8px;
  background: var(--bg-tertiary);
}

:deep(.git-init-btn .vcs-init-icon) {
  color: #f05032;
}

:deep(.p4-init-btn .vcs-init-icon) {
  color: #6c757d;
}

:deep(.vcs-init-label) {
  font-size: 13px;
  font-weight: 600;
}

:deep(.vcs-init-desc) {
  font-size: 11px;
  opacity: 0.6;
  text-align: center;
}

:deep(.init-error) {
  margin-top: 8px;
  color: #e15759;
  font-size: 12px;
}

:deep(.graph-scroll) {
  flex: 1;
  overflow: auto;
}

:deep(.graph-table-shell) {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

:deep(.graph-header-row) {
  display: grid;
  grid-template-columns:
    var(--graph-refs-width)
    var(--graph-graph-width)
    var(--graph-message-width)
    var(--graph-meta-width);
  align-items: center;
  gap: 0;
  width: 100%;
  min-width: var(--graph-body-width);
  position: sticky;
  top: 0;
  z-index: 2;
  border-bottom: 1px solid var(--border-color);
  background: color-mix(in srgb, var(--bg-color) 92%, var(--sidebar-bg));
}

:deep(.graph-header-cell) {
  position: relative;
  display: flex;
  align-items: center;
  min-width: 0;
  padding: 6px 12px;
  font-size: 10px;
  line-height: 1.2;
  color: var(--text-secondary);
  letter-spacing: 0.08em;
  text-transform: uppercase;
  white-space: nowrap;
}

:deep(.graph-header-cell-label) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
}

:deep(.graph-header-cell-resizable) {
  padding-right: 16px;
}

:deep(.graph-column-handle) {
  position: absolute;
  top: 0;
  right: -2px;
  width: 6px;
  height: 100%;
  cursor: col-resize;
  user-select: none;
}

:deep(.graph-column-handle::before) {
  content: "";
  position: absolute;
  top: 6px;
  bottom: 6px;
  left: 2px;
  width: 1px;
  background: transparent;
  transition: background 0.12s ease;
}

:deep(.graph-header-cell-resizable:hover .graph-column-handle::before),
:deep(.graph-column-handle.dragging::before) {
  background: color-mix(in srgb, var(--border-color) 80%, var(--accent-color) 20%);
}

:deep(.graph-header-cell:nth-child(2)) {
  padding-left: 0;
}

:deep(.graph-table) {
  position: relative;
  width: 100%;
  min-width: var(--graph-body-width);
}

:deep(.load-more-indicator) {
  text-align: center;
  padding: 8px;
  color: var(--text-secondary);
  font-size: 12px;
}

:deep(.graph-svg) {
  position: absolute;
  top: 0;
  z-index: 2;
  pointer-events: none;
}

:deep(.graph-rows) {
  position: relative;
  z-index: 1;
  width: 100%;
  min-width: var(--graph-body-width);
  padding: 8px 0 14px;
}

:deep(.graph-virtual-spacer) {
  width: 100%;
  min-width: var(--graph-body-width);
  pointer-events: none;
}

:deep(.graph-row) {
  display: grid;
  grid-template-columns:
    var(--graph-refs-width)
    var(--graph-graph-width)
    var(--graph-message-width)
    var(--graph-meta-width);
  align-items: center;
  min-width: var(--graph-body-width);
  width: 100%;
  min-height: var(--graph-row-height);
  border: 0;
  border-radius: 0;
  background: transparent;
  color: inherit;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: background 0.12s ease, box-shadow 0.12s ease;
}

:deep(.graph-row:hover) {
  background: color-mix(in srgb, var(--hover-bg) 85%, transparent);
}

:deep(.graph-row.current-branch-row) {
  background: color-mix(in srgb, var(--active-bg) 22%, transparent);
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 42%, transparent);
}

:deep(.graph-row.current-branch-row:hover) {
  background: color-mix(in srgb, var(--hover-bg) 76%, var(--active-bg) 18%);
}

:deep(.graph-row.selected) {
  background: color-mix(in srgb, var(--active-bg) 82%, transparent);
  box-shadow: inset 2px 0 0 color-mix(in srgb, var(--accent-color) 76%, white 8%);
}

:deep(.graph-row-workspace) {
  background: color-mix(in srgb, var(--active-bg) 14%, transparent);
}

:deep(.graph-row-workspace.selected) {
  background: color-mix(in srgb, var(--active-bg) 82%, transparent);
}

:deep(.graph-row.unanchored) {
  opacity: 0.86;
}

:deep(.graph-row-refs) {
  display: flex;
  align-items: center;
  min-width: 0;
  padding: 0 0 0 12px;
  overflow: visible;
}

:deep(.graph-row-ref-badges) {
  position: relative;
  display: inline-flex;
  align-items: center;
  min-width: 0;
  flex-shrink: 0;
}

:deep(.graph-row-ref-badges-summary) {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}

:deep(.graph-row-ref-badges-expanded) {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: 3;
  display: none;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  max-width: var(--graph-ref-popup-width);
  padding: 6px;
  border: 1px solid color-mix(in srgb, var(--border-color) 82%, var(--accent-color) 18%);
  border-radius: 6px;
  background: color-mix(in srgb, var(--sidebar-bg) 92%, var(--bg-color) 8%);
  box-shadow: 0 10px 20px rgba(0, 0, 0, 0.16);
}

:deep(.graph-row-ref-badges.is-collapsed:hover .graph-row-ref-badges-expanded),
:deep(.graph-row:focus-visible .graph-row-ref-badges.is-collapsed .graph-row-ref-badges-expanded) {
  display: inline-flex;
}

:deep(.graph-row-ref-connector) {
  position: relative;
  flex: 1;
  min-width: 0;
  height: var(--graph-row-height);
}

:deep(.graph-row-track) {
  min-height: var(--graph-row-height);
  position: relative;
}

:deep(.graph-row-ref-connector::before) {
  content: "";
  position: absolute;
  left: 0;
  right: calc(-1 * var(--graph-connector-runout, 0px));
  top: 50%;
  height: 0;
  border-top: 1px solid var(--graph-connector-color);
  transform: translateY(-0.5px);
}

:deep(.graph-row-message) {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 8px;
  min-width: 0;
  padding-right: 12px;
}

:deep(.graph-row-title) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  color: var(--text-color);
  line-height: 1.35;
  font-size: 13px;
}

:deep(.graph-row.head .graph-row-title) {
  font-weight: 700;
}

:deep(.graph-row.current-branch-row .graph-row-title) {
  font-weight: 600;
}

:deep(.graph-row-stash .graph-row-title) {
  font-style: italic;
}

:deep(.graph-row-meta) {
  display: flex;
  align-items: center;
  min-width: 0;
  padding-left: 12px;
  padding-right: 12px;
}

:deep(.graph-header-cell:last-child) {
  padding-right: 12px;
}

:deep(.graph-row-meta-text) {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  color: var(--text-secondary);
  text-align: left;
}

:deep(.workspace-meta) {
  display: inline-flex;
  align-items: center;
  gap: 0;
  padding: 0 8px;
  height: 20px;
  border-radius: var(--radius-badge);
  border: 1px solid color-mix(in srgb, #2ea043 36%, transparent);
  background: color-mix(in srgb, #2ea043 14%, transparent);
  color: #2ea043;
  font-size: 11px;
  font-weight: 600;
  line-height: 1.4;
}

:deep(.workspace-inline-badge) {
  display: inline-flex;
  align-items: center;
  padding: 0 8px;
  height: 20px;
  border-radius: var(--radius-badge);
  border: 1px solid color-mix(in srgb, #2ea043 36%, transparent);
  background: color-mix(in srgb, #2ea043 14%, transparent);
  color: #2ea043;
  font-size: 11px;
  font-weight: 700;
  line-height: 1;
  flex-shrink: 0;
}

:deep(.graph-row-current-branch-indicator) {
  width: 6px;
  height: 6px;
  border-radius: 999px;
  flex-shrink: 0;
  background: color-mix(in srgb, var(--accent-color) 72%, white 12%);
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent-color) 22%, transparent);
}

:deep(.ref-badge) {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  flex-shrink: 0;
  min-height: 18px;
  padding: 1px 8px;
  border-radius: 4px;
  font-size: 11px;
  font-weight: 600;
  line-height: 1.3;
  border: 1px solid transparent;
}

:deep(.ref-badge-markers) {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  flex-shrink: 0;
}

:deep(.ref-badge-marker) {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 11px;
  height: 11px;
  opacity: 0.86;
}

:deep(.ref-badge-marker-remote) {
  width: 11px;
  height: 11px;
  opacity: 0.9;
}

:deep(.ref-badge-marker svg) {
  width: 11px;
  height: 11px;
  display: block;
}

:deep(.ref-badge-text) {
  min-width: 0;
  white-space: nowrap;
}

:deep(.ref-badge-plain) {
  min-height: auto;
  padding: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  font-weight: 600;
}

:deep(.ref-badge-overflow) {
  color: var(--text-secondary);
  border-color: color-mix(in srgb, var(--border-color) 78%, transparent);
  background: color-mix(in srgb, var(--hover-bg) 76%, transparent);
  font-weight: 700;
}

:deep(.ref-head) {
  color: #2ea043;
}

:deep(.ref-branch),
:deep(.ref-remote),
:deep(.ref-tag) {
  background: var(--graph-ref-bg, transparent);
  color: var(--graph-ref-color, var(--text-color));
  border-color: var(--graph-ref-border, var(--border-color));
}

:deep(.ref-current-branch) {
  position: relative;
  font-weight: 700;
  background: color-mix(in srgb, var(--graph-ref-bg, transparent) 45%, var(--active-bg) 55%);
  color: color-mix(in srgb, var(--graph-ref-color, var(--text-color)) 92%, white 8%);
  border-color: color-mix(in srgb, var(--graph-ref-color, var(--accent-color)) 55%, var(--border-color) 45%);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--graph-ref-color, var(--accent-color)) 16%, transparent);
}

:deep(.ref-current-branch::before) {
  content: "";
  width: 5px;
  height: 5px;
  margin-right: 6px;
  border-radius: 999px;
  background: currentColor;
  opacity: 0.88;
}

:deep(.ref-remote) {
  border-style: dashed;
}

:deep(.ref-tag) {
  font-weight: 700;
}

:deep(.ref-stash) {
  color: #af7aa1;
}

:deep(.ref-workspace) {
  background: transparent;
  color: var(--text-secondary);
  border-color: var(--border-color);
  border-style: dashed;
}

:deep(.load-more-indicator) {
  text-align: center;
  padding: 8px;
  color: var(--text-secondary);
  font-size: 12px;
}

:deep(.panel-divider-v) {
  width: 3px;
  background: var(--border-color);
  flex-shrink: 0;
  cursor: col-resize;
  transition: background 0.15s;
}

.panel-divider-v:hover,
:deep(.collab-view.dragging-v .panel-divider-v) {
  background: var(--text-secondary);
}

:deep(.panel-divider-h) {
  height: 3px;
  background: var(--border-color);
  flex-shrink: 0;
  cursor: row-resize;
  transition: background 0.15s;
}

.panel-divider-h:hover,
:deep(.collab-view.dragging-h .panel-divider-h) {
  background: var(--text-secondary);
}

:deep(.terminal-panel) {
  flex-shrink: 0;
  overflow: hidden;
}

:deep(.inline-diff-header) {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
  background: var(--bg-color);
}

:deep(.inline-diff-back) {
  width: 26px;
  height: 26px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: all 0.15s;
}

:deep(.inline-diff-back:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--text-secondary);
}

:deep(.inline-diff-status) {
  font-size: 11px;
  font-weight: 700;
  padding: 1px 6px;
  border-radius: 3px;
  flex-shrink: 0;
}

:deep(.inline-diff-status.status-m) {
  background: #d29b0022;
  color: #d29b00;
}

:deep(.inline-diff-status.status-a) {
  background: #2ea04322;
  color: #2ea043;
}

:deep(.inline-diff-status.status-d) {
  background: #e1575922;
  color: #e15759;
}

:deep(.inline-diff-status.status-r) {
  background: #388bfd22;
  color: #388bfd;
}

:deep(.inline-diff-path) {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  color: var(--text-color);
  font-weight: 500;
}

:deep(.inline-diff-stats) {
  display: flex;
  gap: 6px;
  font-size: 12px;
  font-weight: 600;
  flex-shrink: 0;
}

:deep(.diff-additions) {
  color: #2ea043;
}

:deep(.diff-deletions) {
  color: #e15759;
}

:deep(.inline-diff-actions) {
  display: flex;
  gap: 6px;
  margin-left: auto;
  flex-shrink: 0;
}

:deep(.inline-diff-tab-group) {
  display: flex;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  overflow: hidden;
}

:deep(.inline-diff-tab-btn) {
  padding: 2px 10px;
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
  white-space: nowrap;
  transition: all 0.15s;
}

:deep(.inline-diff-tab-btn + .inline-diff-tab-btn) {
  border-left: 1px solid var(--border-color);
}

:deep(.inline-diff-tab-btn.active) {
  background: var(--accent-color);
  color: #fff;
}

:deep(.inline-diff-tab-btn:not(.active):hover) {
  background: var(--hover-bg);
}

:deep(.inline-diff-action-btn) {
  display: flex;
  align-items: center;
  gap: 4px;
  background: none;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  padding: 3px 8px;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: 11px;
  white-space: nowrap;
}

:deep(.inline-diff-action-btn:hover) {
  color: var(--text-color);
  border-color: var(--text-secondary);
}

:deep(.inline-diff-body) {
  flex: 1;
  min-height: 0;
  overflow: auto;
}

:deep(.inline-diff-panel) {
  position: absolute;
  top: 0;
  bottom: 0;
  left: 0;
  right: 0;
  z-index: 5;
  display: flex;
  flex-direction: column;
  background: var(--bg-color);
  overflow: hidden;
}

:deep(.inline-diff-loading) {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-secondary);
  font-size: 13px;
}

:deep(.diff-progress-info) {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  min-width: 200px;
}

:deep(.diff-progress-text) {
  font-size: 13px;
  color: var(--text-secondary);
}

:deep(.diff-progress-bar) {
  width: 100%;
  height: 3px;
  background: var(--border-color, rgba(255, 255, 255, 0.08));
  border-radius: 2px;
  overflow: hidden;
}

:deep(.diff-progress-fill) {
  height: 100%;
  background: var(--accent-color, #58a6ff);
  border-radius: 2px;
  transition: width 0.3s ease;
}

:deep(.git-sidebar) {
  width: 100%;
  min-width: 0;
  max-width: none;
  display: flex;
  flex-direction: column;
  border-right: none;
  background: var(--bg-color);
  flex-shrink: 0;
  overflow: hidden;
}

.git-sidebar-divider {
  position: relative;
  width: 6px;
  flex-shrink: 0;
  cursor: col-resize;
}

.git-sidebar-divider::before {
  content: "";
  position: absolute;
  top: 0;
  bottom: 0;
  left: 50%;
  width: 1px;
  transform: translateX(-50%);
  background: var(--border-color);
  transition: background 0.15s;
}

.git-sidebar-divider:hover::before,
.collab-view.dragging-sidebar .git-sidebar-divider::before {
  background: var(--text-secondary);
}

:deep(.sidebar-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
}

:deep(.sidebar-title) {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--text-secondary);
}

:deep(.sidebar-header-actions) {
  display: flex;
  align-items: center;
  gap: 4px;
}

:deep(.sidebar-collapse-btn),
:deep(.sidebar-search-btn) {
  width: 22px;
  height: 22px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

:deep(.sidebar-collapse-btn:hover),
:deep(.sidebar-search-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
}

:deep(.sidebar-scroll) {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
}

:deep(.sidebar-footer) {
  flex-shrink: 0;
  padding: 6px 8px;
  border-top: 1px solid var(--border-color);
}

:deep(.sidebar-config-btn) {
  width: 100%;
  min-height: 28px;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: 7px;
  padding: 0 8px;
  border: 1px solid transparent;
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 12px;
  font-weight: 500;
  cursor: pointer;
  transition: background 0.15s ease, color 0.15s ease, border-color 0.15s ease;
}

:deep(.sidebar-config-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--border-color);
}

:deep(.sidebar-config-icon) {
  flex-shrink: 0;
  opacity: 0.7;
}

:deep(.sidebar-collapsed) {
  width: 28px;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding-top: 10px;
  border-right: 1px solid var(--border-color);
  flex-shrink: 0;
  cursor: pointer;
  color: var(--text-secondary);
  transition: all 0.15s;
}

:deep(.sidebar-collapsed:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
}

/* ── Section ── */
:deep(.sidebar-section) {
  border-bottom: 1px solid var(--border-color);
}

:deep(.sidebar-section:last-child) {
  border-bottom: none;
}

:deep(.sidebar-section-header) {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 6px 8px;
  cursor: pointer;
  transition: background 0.1s;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  color: var(--text-secondary);
}

:deep(.sidebar-section-header:hover) {
  background: var(--hover-bg);
}

:deep(.chevron) {
  font-size: 9px;
  transition: transform 0.15s;
  flex-shrink: 0;
  width: 12px;
  text-align: center;
  color: var(--text-secondary);
}

:deep(.chevron.expanded) {
  transform: rotate(90deg);
}

:deep(.chevron.small) {
  font-size: 9px;
  width: 10px;
}

:deep(.section-icon) {
  flex-shrink: 0;
  opacity: 0.6;
}

:deep(.section-label) {
  flex: 1;
  min-width: 0;
}

:deep(.section-count) {
  font-size: 10px;
  font-weight: 500;
  color: var(--text-secondary);
  background: var(--active-bg);
  padding: 0 5px;
  border-radius: 8px;
  min-width: 16px;
  text-align: center;
  line-height: 1.5;
}

:deep(.sidebar-section-body) {
  padding-bottom: 2px;
}

/* ── Item ── */
:deep(.sidebar-item) {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 3px 8px 3px 24px;
  font-size: 12px;
  cursor: default;
  transition: background 0.1s;
  color: var(--text-color);
  min-height: 24px;
}

:deep(.sidebar-item:hover) {
  background: var(--hover-bg);
}

:deep(.sidebar-item.active) {
  background: var(--active-bg);
}

:deep(.sidebar-item.stash-item),
:deep(.sidebar-item.tag-item),
:deep(.sidebar-item.branch-item) {
  cursor: pointer;
}

:deep(.sidebar-item.remote-group) {
  padding-left: 16px;
  cursor: pointer;
  font-weight: 500;
  color: var(--text-secondary);
}

:deep(.sidebar-item.nested) {
  padding-left: 36px;
}

:deep(.item-icon) {
  flex-shrink: 0;
  opacity: 0.5;
}

:deep(.item-icon.branch-icon) {
  opacity: 0.6;
}

:deep(.item-icon.stash-icon) {
  opacity: 0.4;
}

:deep(.item-label) {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:deep(.item-label.stash-label) {
  font-size: 11px;
  color: var(--text-secondary);
}

:deep(.stash-state-tag) {
  flex-shrink: 0;
  font-size: 9px;
  font-weight: 600;
  line-height: 1.5;
  padding: 0 4px;
  border-radius: 4px;
  color: var(--text-secondary);
  background: color-mix(in srgb, var(--sidebar-bg) 78%, var(--hover-bg) 22%);
  border: 1px solid color-mix(in srgb, var(--border-color) 88%, var(--text-secondary) 12%);
}

:deep(.current-badge) {
  font-size: 9px;
  font-weight: 600;
  padding: 0 4px;
  border-radius: 3px;
  background: #2ea04333;
  color: #2ea043;
  border: 1px solid #2ea04355;
  flex-shrink: 0;
  line-height: 1.5;
}

:deep(.submodule-status) {
  flex-shrink: 0;
  display: flex;
  align-items: center;
}

:deep(.sub-ok) {
  color: #2ea043;
}

:deep(.sub-modified) {
  color: #d29b00;
}

:deep(.sub-uninitialized) {
  color: var(--text-secondary);
  opacity: 0.5;
}

:deep(.sidebar-empty) {
  padding: 6px 24px;
  font-size: 11px;
  color: var(--text-secondary);
  opacity: 0.6;
}

/* ── Merge Queue Panel ── */
/* ── Merge Resolution Panel ── */
:deep(.merge-resolution-panel) {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow: hidden;
}

:deep(.merge-resolution-header) {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto auto;
  align-items: center;
  gap: 8px 12px;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border-color);
  flex-shrink: 0;
  background: var(--bg-color);
}

:deep(.merge-filter-conflicts-btn) {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  cursor: pointer;
  white-space: nowrap;
  transition: all 0.15s;
}

:deep(.merge-filter-conflicts-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--text-secondary);
}

:deep(.merge-filter-conflicts-btn.active) {
  background: rgba(210, 155, 0, 0.15);
  color: #d29b00;
  border-color: rgba(210, 155, 0, 0.4);
}

:deep(.merge-header-main) {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

:deep(.merge-back-btn) {
  width: 26px;
  height: 26px;
  border: 1px solid var(--border-color);
  border-radius: 5px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: all 0.15s;
}

:deep(.merge-back-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--text-secondary);
}

:deep(.merge-resolution-title) {
  display: flex;
  align-items: center;
  flex: 1;
  gap: 8px;
  font-size: 13px;
  font-weight: 500;
  color: var(--text-color);
  min-width: 0;
  overflow: hidden;
}

:deep(.merge-header-tabs) {
  justify-self: center;
}

:deep(.merge-loading) {
  padding: 16px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
}

:deep(.merge-resolution-content) {
  flex: 1;
  overflow-y: auto;
  min-height: 0;
  padding: 10px 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

:deep(.merge-manual-edit-banner) {
  padding: 6px 10px;
  background: rgba(210, 155, 0, 0.1);
  border: 1px solid rgba(210, 155, 0, 0.25);
  border-radius: 4px;
  color: #d29b00;
  font-size: 12px;
}

:deep(.merge-binary-actions) {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.merge-binary-label) {
  font-size: 13px;
  color: var(--text-secondary);
  margin: 0;
}

:deep(.merge-blocks) {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

:deep(.merge-blocks-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
}

:deep(.merge-blocks-resolved) {
  color: #3fb950;
  font-weight: 500;
}

:deep(.merge-block-item) {
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
  transition: border-color 0.15s;
}

:deep(.merge-block-item.resolved) {
  border-color: rgba(63, 185, 80, 0.3);
}

:deep(.merge-block-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  background: var(--hover-bg);
  border-bottom: 1px solid var(--border-color);
}

:deep(.merge-block-label) {
  font-size: 12px;
  font-weight: 500;
  color: var(--text-secondary);
}

:deep(.merge-block-choices) {
  display: flex;
  gap: 4px;
}

:deep(.merge-block-btn) {
  padding: 3px 10px;
  border: 1px solid var(--border-color);
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  transition: all 0.15s;
}

:deep(.merge-block-btn:hover) {
  background: var(--hover-bg);
  color: var(--text-color);
  border-color: var(--text-secondary);
}

:deep(.merge-block-btn.active) {
  background: #4e79a7;
  color: #fff;
  border-color: #4e79a7;
}

:deep(.merge-block-preview) {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 0;
}

:deep(.merge-block-side) {
  border-right: 1px solid var(--border-color);
  overflow: hidden;
}

:deep(.merge-block-side:last-child) {
  border-right: none;
}

:deep(.merge-block-side-label) {
  padding: 4px 8px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.3px;
  color: var(--text-secondary);
  background: var(--hover-bg);
  border-bottom: 1px solid var(--border-color);
}

:deep(.merge-block-code) {
  margin: 0;
  padding: 8px;
  font-size: 12px;
  font-family: var(--font-mono-block);
  line-height: 1.5;
  color: var(--text-color);
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 200px;
  overflow-y: auto;
}

:deep(.merge-editor-section) {
  display: flex;
  flex-direction: column;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  overflow: hidden;
}

:deep(.merge-editor-header) {
  padding: 6px 10px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  background: var(--hover-bg);
  border-bottom: 1px solid var(--border-color);
}

:deep(.merge-editor-textarea) {
  width: 100%;
  min-height: 200px;
  padding: 8px 10px;
  border: none;
  background: var(--bg-color);
  color: var(--text-color);
  font-size: 12px;
  font-family: var(--font-mono-editor);
  line-height: 1.5;
  resize: vertical;
  outline: none;
  box-sizing: border-box;
}

:deep(.merge-apply-row) {
  display: flex;
  justify-content: flex-end;
}

:deep(.merge-diff-summaries) {
  display: flex;
  gap: 12px;
  padding-top: 6px;
  border-top: 1px solid var(--border-color);
}

:deep(.merge-diff-summary) {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

:deep(.merge-diff-summary-label) {
  color: var(--text-secondary);
  font-weight: 500;
}

:deep(.merge-diff-summary-loading) {
  color: var(--text-secondary);
  opacity: 0.6;
  font-size: 11px;
}

:deep(.merge-diff-summary-stats) {
  font-weight: 600;
  color: var(--text-color);
}

/* ── Merge Tab Bar ── */

:deep(.merge-tab-bar) {
  display: flex;
  gap: 2px;
  background: var(--input-bg, #2a2a2a);
  border-radius: 6px;
  padding: 2px;
}

:deep(.merge-tab-btn) {
  padding: 3px 12px;
  font-size: 12px;
  font-weight: 500;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--text-secondary);
  cursor: pointer;
  transition: background 0.15s, color 0.15s;
}

:deep(.merge-tab-btn:hover) {
  color: var(--text-color);
  background: var(--hover-bg, #333);
}

:deep(.merge-tab-btn.active) {
  background: var(--bg-color);
  color: var(--text-color);
  font-weight: 600;
}

@media (max-width: 760px) {
  :deep(.merge-resolution-header) {
    grid-template-columns: minmax(0, 1fr);
  }

  :deep(.merge-header-tabs) {
    justify-self: end;
  }
}

/* ── Merge Semantic View ── */

:deep(.merge-semantic-view) {
  display: flex;
  gap: 0;
  min-height: 300px;
  border: 1px solid var(--border-color, #333);
  border-radius: 6px;
  overflow: hidden;
}

:deep(.merge-semantic-sidebar) {
  width: 240px;
  min-width: 200px;
  border-right: 1px solid var(--border-color, #333);
  background: var(--sidebar-bg, #1a1a1a);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

:deep(.merge-semantic-sidebar-header) {
  padding: 8px 12px;
  font-size: 11px;
  font-weight: 600;
  color: var(--text-secondary);
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--border-color, #333);
  flex-shrink: 0;
}

:deep(.merge-semantic-target-list) {
  flex: 1;
  overflow-y: auto;
  padding: 4px 0;
}

:deep(.merge-semantic-target-item) {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 12px;
  font-size: 12px;
  color: var(--text-color);
  cursor: pointer;
  border-left: 2px solid transparent;
  transition: background 0.1s;
}

:deep(.merge-semantic-target-item:hover) {
  background: var(--hover-bg);
}

:deep(.merge-semantic-target-item.selected) {
  background: var(--active-bg);
  border-left-color: var(--accent-color);
}

:deep(.merge-semantic-target-item.conflict) {
  color: var(--warning-color, #e8a838);
}

:deep(.merge-target-label) {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:deep(.merge-target-badge) {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 3px;
  text-transform: uppercase;
  letter-spacing: 0.3px;
}

:deep(.merge-target-badge.badge-auto) {
  background: rgba(78, 167, 78, 0.2);
  color: #6cc070;
}

:deep(.merge-target-badge.badge-conflict) {
  background: rgba(232, 168, 56, 0.2);
  color: #e8a838;
}

:deep(.merge-target-badge.badge-ours) {
  background: rgba(74, 158, 255, 0.2);
  color: #4a9eff;
}

:deep(.merge-target-badge.badge-theirs) {
  background: rgba(187, 128, 255, 0.2);
  color: #bb80ff;
}

:deep(.merge-target-badge.badge-removed) {
  background: rgba(255, 85, 85, 0.15);
  color: #ff5555;
}

/* ── Merge Semantic Main ── */

:deep(.merge-semantic-main) {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 12px;
  background: var(--bg-color);
}

:deep(.merge-semantic-empty),
:deep(.merge-semantic-loading),
:deep(.merge-semantic-error) {
  padding: 24px;
  text-align: center;
  color: var(--text-secondary);
  font-size: 13px;
}

:deep(.merge-semantic-error) {
  color: var(--error-color, #f44);
}

:deep(.merge-inspector-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 12px;
}

:deep(.merge-inspector-header h3) {
  margin: 0;
  font-size: 14px;
  font-weight: 600;
  color: var(--text-color);
}

:deep(.merge-inspector-actions) {
  display: flex;
  gap: 6px;
}

/* ── Merge Panel Cards ── */

:deep(.merge-panel-card) {
  margin-bottom: 8px;
  border: 1px solid var(--border-color, #333);
  border-radius: 6px;
  overflow: hidden;
}

:deep(.merge-panel-header) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 10px;
  background: var(--hover-bg);
  border-bottom: 1px solid var(--border-color);
}

:deep(.merge-panel-title) {
  font-size: 12px;
  font-weight: 600;
  color: var(--text-color);
}

:deep(.merge-panel-status) {
  font-size: 10px;
  font-weight: 600;
  padding: 1px 5px;
  border-radius: 3px;
  text-transform: uppercase;
}

:deep(.merge-panel-status.status-autoResolved) {
  background: rgba(78, 167, 78, 0.2);
  color: #6cc070;
}

:deep(.merge-panel-status.status-hasConflicts) {
  background: rgba(232, 168, 56, 0.2);
  color: #e8a838;
}

:deep(.merge-panel-fields) {
  padding: 4px 0;
}

:deep(.merge-field-row) {
  display: grid;
  grid-template-columns: 1fr 80px 80px 80px 80px;
  gap: 4px;
  padding: 3px 10px;
  font-size: 12px;
  align-items: center;
  border-bottom: 1px solid var(--border-color-subtle, #2a2a2a);
}

:deep(.merge-field-row:last-child) {
  border-bottom: none;
}

:deep(.merge-field-label) {
  color: var(--text-color);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:deep(.merge-field-base),
:deep(.merge-field-ours),
:deep(.merge-field-theirs),
:deep(.merge-field-result) {
  color: var(--text-secondary);
  font-family: var(--font-mono-identifier);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:deep(.merge-field-result.state-conflict) {
  color: var(--warning-color, #e8a838);
  font-weight: 600;
}

:deep(.merge-field-result.state-auto) {
  color: #6cc070;
}

/* ── Merge Semantic Footer ── */

:deep(.merge-semantic-footer-bar) {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border-top: 1px solid var(--border-color);
  background: var(--bg-color);
}

:deep(.merge-semantic-footer) {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 8px 12px;
  margin-top: 8px;
  font-size: 12px;
}

:deep(.merge-unresolved-count) {
  color: var(--warning-color, #e8a838);
  font-weight: 600;
}

:deep(.merge-all-resolved) {
  color: #6cc070;
  font-weight: 600;
}

</style>

<style src="./collab/collabPreview.css"></style>

<style>
.commit-modal-overlay {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  backdrop-filter: blur(2px);
}

.commit-modal {
  width: 440px;
  max-width: 90vw;
  border-radius: 10px;
  background: var(--bg-color, #1e1e1e);
  border: 1px solid var(--border-color, #333);
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.commit-modal-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  border-bottom: 1px solid var(--border-color, #333);
}

.commit-modal-title {
  font-size: 13px;
  color: var(--text-secondary, #aaa);
}

.commit-modal-title strong {
  color: var(--text-color, #ddd);
}

.commit-modal-close {
  background: none;
  border: none;
  color: var(--text-secondary, #aaa);
  font-size: 18px;
  cursor: pointer;
  padding: 0 4px;
  line-height: 1;
}

.commit-modal-close:hover {
  color: var(--text-color, #ddd);
}

.commit-modal-body {
  padding: 14px 16px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.commit-modal-message {
  margin: 0;
  color: var(--text-secondary, #aaa);
  font-size: 12px;
  line-height: 1.6;
}

.commit-modal-warning {
  margin: 0;
  padding: 10px 12px;
  border: 1px solid color-mix(in srgb, var(--status-warn-fg, #d29b00) 28%, transparent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--status-warn-bg, rgba(210, 155, 0, 0.12)) 78%, transparent);
  color: var(--status-warn-fg, #d29b00);
  font-size: 12px;
  line-height: 1.6;
}

.commit-input-row {
  display: flex;
  gap: 6px;
  align-items: stretch;
}

.commit-input {
  flex: 1;
  min-width: 0;
  padding: 8px 10px;
  border: 1px solid var(--border-color, #333);
  border-radius: 6px;
  background: var(--input-bg, #2a2a2a);
  color: var(--text-color, #ddd);
  font-size: 13px;
  outline: none;
  box-sizing: border-box;
}

.commit-input:focus {
  border-color: #4e79a7;
}

.ai-generate-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 34px;
  flex-shrink: 0;
  border: 1px solid var(--border-color, #333);
  border-radius: 6px;
  background: linear-gradient(135deg, #6366f1, #8b5cf6);
  color: #fff;
  cursor: pointer;
  transition: all 0.15s;
}

.ai-generate-btn:hover:not(:disabled) {
  background: linear-gradient(135deg, #5558e6, #7c4de8);
  border-color: #6366f1;
}

.ai-generate-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.ai-spinner {
  display: inline-block;
  width: 14px;
  height: 14px;
  border: 2px solid rgba(255, 255, 255, 0.3);
  border-top-color: #fff;
  border-radius: 50%;
  animation: ai-spin 0.6s linear infinite;
}

@keyframes ai-spin {
  to { transform: rotate(360deg); }
}

.commit-textarea {
  width: 100%;
  padding: 8px 10px;
  border: 1px solid var(--border-color, #333);
  border-radius: 6px;
  background: var(--input-bg, #2a2a2a);
  color: var(--text-color, #ddd);
  font-size: 12px;
  resize: vertical;
  outline: none;
  font-family: inherit;
  box-sizing: border-box;
}

.commit-textarea:focus {
  border-color: #4e79a7;
}

.commit-error {
  padding: 6px 10px;
  background: rgba(225, 87, 89, 0.15);
  border: 1px solid rgba(225, 87, 89, 0.3);
  border-radius: 4px;
  color: #e15759;
  font-size: 12px;
}

.commit-modal-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border-top: 1px solid var(--border-color, #333);
}

.commit-staged-count {
  font-size: 12px;
  color: var(--text-secondary, #aaa);
}

.commit-modal-actions {
  display: flex;
  gap: 8px;
  margin-left: auto;
}

.commit-cancel-btn {
  padding: 6px 14px;
  border: 1px solid var(--border-color, #333);
  border-radius: 6px;
  background: transparent;
  color: var(--text-secondary, #aaa);
  font-size: 12px;
  cursor: pointer;
}

.commit-cancel-btn:hover {
  background: var(--hover-bg, #333);
  color: var(--text-color, #ddd);
}

.commit-confirm-btn {
  padding: 6px 16px;
  border: none;
  border-radius: 6px;
  background: #4e79a7;
  color: #fff;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
  transition: background 0.15s;
}

.commit-confirm-btn:hover:not(:disabled) {
  background: #3d6590;
}

.commit-confirm-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* ── Git Config Modal ── */
.git-config-overlay {
  position: fixed;
  inset: 0;
  z-index: 2000;
  background: rgba(0, 0, 0, 0.4);
  display: flex;
  align-items: center;
  justify-content: center;
}
.git-config-modal {
  background: var(--bg-color);
  border-radius: 10px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.25);
  max-width: 420px;
  width: 90%;
  display: flex;
  flex-direction: column;
}
.git-config-header {
  padding: 16px 20px;
  border-bottom: 1px solid var(--border-color);
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-weight: 600;
  font-size: 14px;
}
.git-config-close {
  background: none;
  border: none;
  font-size: 22px;
  cursor: pointer;
  color: var(--text-secondary);
  line-height: 1;
}
.git-config-body {
  padding: 20px;
}
.git-config-desc {
  font-size: 13px;
  color: var(--text-secondary);
  margin-bottom: 16px;
  line-height: 1.5;
}
.git-config-label {
  display: block;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-secondary);
  margin-bottom: 4px;
  margin-top: 12px;
}
.git-config-label:first-of-type {
  margin-top: 0;
}
.git-config-input {
  width: 100%;
  padding: 8px 12px;
  border: 1px solid var(--border-color);
  border-radius: 6px;
  background: var(--input-bg, var(--bg-color));
  color: var(--text-primary);
  font-size: 14px;
  box-sizing: border-box;
  outline: none;
  transition: border-color 0.15s;
}
.git-config-input:focus {
  border-color: var(--accent-color);
}
.git-config-footer {
  padding: 12px 20px;
  border-top: 1px solid var(--border-color);
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}
.collab-btn {
  padding: 8px 16px;
  border-radius: 6px;
  font-size: 13px;
  cursor: pointer;
  border: none;
  transition: background 0.15s;
}
.collab-btn.primary {
  background: var(--accent-color);
  color: var(--bg-color);
}
.collab-btn.primary:hover:not(:disabled) {
  filter: brightness(1.1);
}
.collab-btn.primary:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
.collab-btn.secondary {
  background: transparent;
  color: var(--text-secondary);
  border: 1px solid var(--border-color);
}
.collab-btn.secondary:hover {
  background: var(--hover-bg, rgba(255,255,255,0.05));
}

/* ── Context menu ── */
.ctx-backdrop {
  position: fixed;
  inset: 0;
  z-index: 9999;
}
.ctx-menu {
  position: fixed;
  box-sizing: border-box;
  min-width: min(180px, calc(100vw - 16px));
  max-width: calc(100vw - 16px);
  max-height: calc(100vh - 16px);
  overflow: auto;
  background: var(--sidebar-bg, #1e1e2e);
  border: 1px solid var(--border-color, #333);
  border-radius: 8px;
  padding: 4px 0;
  box-shadow: 0 4px 16px rgba(0,0,0,.25);
  z-index: 10000;
}
.ctx-item {
  padding: 6px 16px;
  font-size: 13px;
  cursor: pointer;
  color: var(--text-color, #ccc);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  border-radius: 4px;
  margin: 1px 4px;
  transition: background 0.15s ease, color 0.15s ease;
}
.ctx-item:hover {
  background: color-mix(in srgb, currentColor 10%, transparent);
}
.ctx-danger:hover {
  background: rgba(238, 85, 85, 0.12);
}
.ctx-danger {
  color: #e55;
}
.ctx-sep {
  height: 1px;
  background: var(--border-color, #333);
  margin: 4px 0;
}
.ctx-hint {
  color: #e5a545;
  cursor: default;
  pointer-events: none;
}
.ctx-item.disabled {
  opacity: 0.4;
  pointer-events: none;
}

</style>
