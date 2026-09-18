
<script setup lang="ts">
import { ref, computed, nextTick, onMounted, onUnmounted, watch } from "vue";
import { X } from "lucide";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ModelOption, GitFileChange, DiffSource, FileDiffPayload, FileDiffRequest, UnmergedFileEntry, GitHistoryTarget, GitBranchTarget, GitStashEntry, GitGraphRef } from "../types";
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
import BaseContextMenu from "./ui/BaseContextMenu.vue";
import BaseButton from "./ui/BaseButton.vue";
import LucideIcon from "./icons/LucideIcon.vue";
import { useCollabState } from "../composables/useCollabState";
import { useProjectStore } from "../stores/project";
import { useNotificationStore } from "../stores/notification";
import { selectUnityAsset, openFileExternal, showInFolder } from "../services/unity";
import { computeRequestKey, diffSingleFile, refetchDiffByKey } from "../services/diff";
import { canOpenInEditor, useHideMeta, withMetaCompanionPaths } from "../composables/useHideMeta";
import { countLocusManagedFiles } from "../composables/locusManagedFiles";
import { useDiffProgress } from "../composables/useDiffProgress";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { normalizeAppError } from "../services/errors";
import { openFileDiffReviewWindow } from "../services/chatDiffReviewWindow";
import { t } from "../i18n";
import {
  resolveCollabRightPanelMode,
  resolveConflictActionHint,
  resolveMergeOperationBadge,
} from "./collab/collabViewMode";
import { resolveHistorySelectionKind } from "./collab/historySelection";
import { collectUnanchoredStashHashes } from "./collab/graph/normalize";
import { branchTargetName, resolveBranchDblclickAction, resolveBranchTargetHash } from "./collab/branchInteraction";
import { buildGitMenu, validateGitName, type GitContextTarget, type GitMenuItem } from "./collab/gitContextMenu";
import type { GitGraphPublicApi, GitGraphSelectionTarget, GitGraphSelectOptions } from "./collab/gitGraphSelection";
import {
  COLLAB_SEARCH_SELECT_EVENT,
  openCollabSearchWindow,
  type CollabSearchSelectionPayload,
} from "../services/collabSearchWindow";
import type { WorkspaceRef } from "../services/project";

const props = defineProps<{
  workingDir: string;
  workspaceRef?: WorkspaceRef | null;
  isActive: boolean;
  selectedModelId: string;
  selectedAgentId: string;
  models: ModelOption[];
  sidebarTarget?: HTMLElement | null;
  sidebarToolbarTarget?: HTMLElement | null;
  activateEditor?: () => Promise<void>;
  headFocusRequest?: {
    id: number;
    checkoutId: string;
  } | null;
}>();

interface CollabViewWorkspaceSnapshot {
  workingDir: string;
  workspaceRef: WorkspaceRef;
  checkoutId: string;
  expectedGeneration: number | null;
  expectedMaterializationEpoch: number | null;
}

function captureWorkspaceSnapshot(): CollabViewWorkspaceSnapshot {
  if (!props.workspaceRef) {
    throw new Error("Workspace checkout is required for Collab operations.");
  }
  const workspaceRef = {
    checkoutId: props.workspaceRef.checkoutId,
    expectedGeneration: props.workspaceRef.expectedGeneration ?? undefined,
    expectedMaterializationEpoch: props.workspaceRef.expectedMaterializationEpoch ?? undefined,
  };
  return {
    workingDir: props.workingDir,
    workspaceRef,
    checkoutId: workspaceRef.checkoutId,
    expectedGeneration: workspaceRef.expectedGeneration ?? null,
    expectedMaterializationEpoch: workspaceRef.expectedMaterializationEpoch ?? null,
  };
}

function isCurrentWorkspaceSnapshot(snapshot: CollabViewWorkspaceSnapshot) {
  return snapshot.workingDir === props.workingDir
    && snapshot.checkoutId === (props.workspaceRef?.checkoutId ?? null)
    && snapshot.expectedGeneration === (props.workspaceRef?.expectedGeneration ?? null)
    && snapshot.expectedMaterializationEpoch === (props.workspaceRef?.expectedMaterializationEpoch ?? null);
}

const emit = defineEmits<{
  (e: "selectModel", id: string): void;
}>();

const projectStore = useProjectStore();
const notificationStore = useNotificationStore();
const { state: displaySettings } = useDisplaySettings();
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
  expandLocal, expandRemotes, expandedRemoteNames, expandStashes, expandTags, expandSubmodules,
  containerRef, leftAreaRef, leftColRef, leftColWidth, terminalHeight, draggingClass,
  currentBranch, selectedCommit, totalChanges, workspaceChangeCount,
  initGitUnity, saveGitConfigAndInit, cancelGitConfig, toggleRemote,
  stageFile, unstageFile, stageFiles, unstageFiles, stageAll, unstageAll,
  loadMoreCommits, onCommitted, onTerminalDone, onTerminalTouched, onRefresh,
  onVSplitterMouseDown, onHSplitterMouseDown,
} = useCollabState(props, {
  isSidebarActive: () => !!props.sidebarTarget,
  onGitTerminalOutput(command, output, isError) {
    terminalRef.value?.pushOutput(command, output, isError);
  },
});

// vue-tsc with TypeScript 6 does not count string template refs returned by a
// composable as script reads, even though Vue assigns these refs at runtime.
void containerRef;
void leftAreaRef;
void leftColRef;

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
let diffRequestGeneration = 0;
const activeFilePath = ref<string | null>(null);
const activeDiffSource = ref<DiffSource | null>(null);
const activeDiffCommitHash = ref<string | null>(null);
const pendingDiscardPaths = ref<Set<string>>(new Set());
const discardOperationBusy = computed(() => pendingDiscardPaths.value.size > 0);
const gitOperationBusy = ref(false);
const workspaceMutationBusy = computed(() => stageOperationBusy.value || discardOperationBusy.value || gitOperationBusy.value);
const diffProgress = useDiffProgress(() => props.workspaceRef ?? null);
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
  const workspace = captureWorkspaceSnapshot();
  const requestGeneration = ++diffRequestGeneration;
  // Toggle: clicking the same file again closes the diff
  const nextCommitHash = source === "gitCommit" ? (commitHash ?? null) : null;
  if (
    displaySettings.gitDiffReviewTarget !== "window"
    && activeFilePath.value === file.path
    && inlineDiff.value
    && activeDiffSource.value === source
    && activeDiffCommitHash.value === nextCommitHash
  ) {
    closeDiff();
    return;
  }

  const request: FileDiffRequest = {
    source,
    filePath: file.path,
    oldPath: file.oldPath,
    commitHash,
    detail: "full",
  };

  if (displaySettings.gitDiffReviewTarget === "window") {
    activeFilePath.value = file.path;
    activeDiffSource.value = source;
    activeDiffCommitHash.value = nextCommitHash;
    inlineDiff.value = null;
    diffLoading.value = false;
    diffProgress.reset(computeRequestKey(request, workspace.workspaceRef));
    try {
      const opened = await openFileDiffReviewWindow({
        request,
        workspaceRef: workspace.workspaceRef,
      });
      if (opened) return;
    } catch (e) {
      console.error("[CollabView] failed to open diff review window:", e);
    }
  }

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
  diffProgress.reset(computeRequestKey(request, workspace.workspaceRef));
  try {
    const payload = await diffSingleFile(request, workspace.workspaceRef);
    if (
      requestGeneration !== diffRequestGeneration
      || !isCurrentWorkspaceSnapshot(workspace)
    ) return;
    inlineDiff.value = payload;
  } catch (e) {
    if (
      requestGeneration !== diffRequestGeneration
      || !isCurrentWorkspaceSnapshot(workspace)
    ) return;
    console.error("[CollabView] failed to fetch diff:", e);
  } finally {
    if (
      requestGeneration === diffRequestGeneration
      && isCurrentWorkspaceSnapshot(workspace)
    ) {
      diffLoading.value = false;
    }
  }
}

function closeDiff() {
  diffRequestGeneration += 1;
  inlineDiff.value = null;
  diffLoading.value = false;
  activeFilePath.value = null;
  activeDiffSource.value = null;
  activeDiffCommitHash.value = null;
}

function selectInlineDiffInUnity(path: string) {
  if (!props.workspaceRef) return;
  void selectUnityAsset(props.workspaceRef, path);
}

function openInlineDiffExternal(path: string) {
  if (!props.workspaceRef) return;
  void openFileExternal(path, props.workspaceRef);
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

watch(
  () => [
    props.workingDir,
    props.workspaceRef?.checkoutId ?? null,
    props.workspaceRef?.expectedGeneration ?? null,
    props.workspaceRef?.expectedMaterializationEpoch ?? null,
  ] as const,
  () => {
    closeDiff();
    selectedConflictFile.value = null;
    pendingDiscardPaths.value = new Set();
    gitConfigPopoverOpen.value = false;
    ctxMenu.value = null;
    promptDialog.value = null;
    confirmDialog.value = null;
  },
);

async function onLfsPulled() {
  if (!inlineDiff.value) return;
  const workspace = captureWorkspaceSnapshot();
  const key = inlineDiff.value.key;
  try {
    const updated = await refetchDiffByKey(key);
    if (
      updated
      && isCurrentWorkspaceSnapshot(workspace)
      && inlineDiff.value?.key === key
    ) inlineDiff.value = updated;
  } catch (e) {
    if (!isCurrentWorkspaceSnapshot(workspace)) return;
    console.error("[CollabView] refetch after LFS pull failed:", e);
  }
}

// ── Git operation progress ───────────────────────────────────────
async function runGitOp<T>(
  label: string,
  command: string,
  fn: (workspaceRef: WorkspaceRef) => Promise<T>,
): Promise<T> {
  if (workspaceMutationBusy.value || hasConflictState.value) throw new Error(t("collab.menu.busy"));
  const workspace = captureWorkspaceSnapshot();
  gitOperationBusy.value = true;
  const operation = `collabGitOp:${Date.now()}:${Math.random().toString(36).slice(2)}`;
  notificationStore.addNotice("info", label, {
    operation,
    sticky: true,
    spinner: true,
    replaceOperation: true,
  });
  try {
    const result = await fn(workspace.workspaceRef);
    const msg = (result as any)?.message || label;
    const isError = (result as any)?.status === "conflict";
    if (isCurrentWorkspaceSnapshot(workspace)) {
      terminalRef.value?.pushOutput(command, msg, isError);
    }
    notificationStore.addNotice(isError ? "error" : "success", isError ? t("collab.menu.operationConflict") : label, {
      operation,
      ttl: 3000,
      replaceOperation: true,
    });
    return result;
  } catch (e) {
    const err = normalizeAppError(e);
    const localizedErrors: Record<string, string> = {
      "git.no_upstream": "noUpstream", "git.branch_changed": "branchChanged",
      "git.stash_changed": "stashChanged", "git.dirty_stash_branch": "dirtyStashBranch",
    };
    const errMsg = localizedErrors[err.code] ? t(`collab.menu.${localizedErrors[err.code]}`) : err.message || t("collab.menu.operationFailed");
    if (isCurrentWorkspaceSnapshot(workspace)) {
      terminalRef.value?.pushOutput(command, errMsg, true);
    }
    notificationStore.addNotice("error", errMsg, {
      code: err.code,
      operation,
      ttl: 3000,
      replaceOperation: true,
    });
    throw err;
  } finally {
    try {
      if (isCurrentWorkspaceSnapshot(workspace)) await onRefresh();
    } finally {
      gitOperationBusy.value = false;
    }
  }
}

// ── Context menu ─────────────────────────────────────────────────

type CollabContextMenuTarget = GitContextTarget;
type CollabContextMenuState = {
  x: number;
  y: number;
  target: CollabContextMenuTarget;
};

const ctxMenu = ref<CollabContextMenuState | null>(null);

const promptDialog = ref<{
  title: string;
  placeholder: string;
  value: string;
  validate?: (value: string) => string | null;
  hint?: string;
  action: (val: string) => void;
} | null>(null);
const promptError = computed(() => promptDialog.value?.value.trim()
  ? promptDialog.value.validate?.(promptDialog.value.value.trim()) ?? null : null);
const gitMenuGroups = computed(() => ctxMenu.value ? buildGitMenu(ctxMenu.value.target, {
  busy: workspaceMutationBusy.value,
  conflict: hasConflictState.value,
  conflictHint: conflictActionHint.value,
  currentBranch: headState.value?.kind === "attached" ? currentBranch.value : "",
  localBranches: localBranches.value,
  graphRefs: graphRefs.value,
  unityConnected: projectStore.unityConnected,
}) : []);

const confirmDialog = ref<{
  title: string;
  message: string;
  warning?: string | null;
  danger: boolean;
  action: () => void;
} | null>(null);

function closeCtxMenu() { ctxMenu.value = null; }

function openCtxMenu(event: MouseEvent, target: CollabContextMenuTarget) {
  ctxMenu.value = { x: event.clientX, y: event.clientY, target };
  void nextTick(() => document.querySelector<HTMLButtonElement>(".collab-git-menu button:not(:disabled)")?.focus());
}

function navigateGitMenu(event: KeyboardEvent) {
  if (event.key === "Tab") { closeCtxMenu(); return; }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  event.preventDefault();
  const buttons = [...(event.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
  const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
  const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
    : (index + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length;
  buttons[next]?.focus();
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

let appliedHeadFocusRequestId = 0;

watch(
  () => [
    props.headFocusRequest?.id ?? 0,
    props.headFocusRequest?.checkoutId ?? "",
    props.workspaceRef?.checkoutId ?? "",
    headState.value.hash ?? "",
    loading.value,
    commits.value,
  ] as const,
  async ([requestId, requestedCheckoutId, currentCheckoutId, headHash, isLoading, loadedCommits]) => {
    if (!requestId || requestId === appliedHeadFocusRequestId) return;
    if (requestedCheckoutId !== currentCheckoutId || isLoading || !headHash) return;
    if (!loadedCommits.some((commit) => commit.hash === headHash)) return;

    await nextTick();
    if (props.headFocusRequest?.id !== requestId) return;
    if (props.workspaceRef?.checkoutId !== requestedCheckoutId) return;
    appliedHeadFocusRequestId = requestId;
    await selectHistoryInGraph(
      { kind: "commit", hash: headHash },
      { scroll: true, behavior: "smooth" },
    );
  },
  { flush: "post" },
);

function onSelectStash(stash: GitStashEntry) {
  void revealSidebarHistory({ kind: "stash", hash: stash.hash });
}

function onSelectTag(tag: GitGraphRef) {
  void revealSidebarHistory({ kind: "commit", hash: tag.targetHash });
}

function onSelectBranch(target: GitBranchTarget) {
  const hash = resolveBranchTargetHash(target, graphRefs.value);
  if (!hash) return;
  void selectHistoryInGraph({ kind: "commit", hash });
}

let sidebarNavigationRequestId = 0;

watch(() => props.sidebarTarget, (target, previous) => {
  if (previous && target !== previous) {
    sidebarNavigationRequestId += 1;
    closeCtxMenu();
  }
});

async function revealSidebarHistory(target: GitGraphSelectionTarget) {
  const requestId = ++sidebarNavigationRequestId;
  const workspace = captureWorkspaceSnapshot();
  try {
    await props.activateEditor?.();
    if (requestId !== sidebarNavigationRequestId || !isCurrentWorkspaceSnapshot(workspace)) return;
    const loaded = target.kind === "commit"
      ? await ensureSearchCommitLoaded(target.hash)
      : target.kind === "stash" ? await ensureSearchStashLoaded(target.hash) : true;
    if (requestId !== sidebarNavigationRequestId || !isCurrentWorkspaceSnapshot(workspace)) return;
    if (!loaded) {
      notificationStore.addNotice("warning", t("collab.search.targetMissing"));
      return;
    }
    await nextTick();
    if (requestId !== sidebarNavigationRequestId || !isCurrentWorkspaceSnapshot(workspace)) return;
    await selectHistoryInGraph(target, { scroll: true, behavior: "smooth" });
  } catch (error) {
    if (requestId === sidebarNavigationRequestId && isCurrentWorkspaceSnapshot(workspace)) {
      notificationStore.addNotice("error", normalizeAppError(error).message);
    }
  }
}

function onSidebarBranchDblclick(target: GitBranchTarget) {
  const hash = resolveBranchTargetHash(target, graphRefs.value);
  if (hash) void revealSidebarHistory({ kind: "commit", hash });
}


async function onSidebarGitConfig() {
  const workspace = captureWorkspaceSnapshot();
  await props.activateEditor?.();
  if (!isCurrentWorkspaceSnapshot(workspace)) return;
  openGitConfigPopover();
}

function isCollabSearchSelectionPayload(value: unknown): value is CollabSearchSelectionPayload {
  if (!value || typeof value !== "object") return false;
  const payload = value as Partial<CollabSearchSelectionPayload>;
  return (payload.kind === "commit" || payload.kind === "stash")
    && typeof payload.hash === "string"
    && payload.hash.trim().length > 0
    && typeof payload.workspaceRef?.checkoutId === "string"
    && typeof payload.workspaceRef?.expectedGeneration === "number";
}

async function ensureSearchCommitLoaded(hash: string): Promise<boolean> {
  const workspace = captureWorkspaceSnapshot();
  if (commits.value.some(commit => commit.hash === hash)) return true;

  let attempts = 0;
  while (hasMoreCommits.value && attempts < 200) {
    attempts += 1;
    const beforeCount = commits.value.length;
    await loadMoreCommits();
    await nextTick();
    if (!isCurrentWorkspaceSnapshot(workspace)) return false;
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
    await openCollabSearchWindow(captureWorkspaceSnapshot().workspaceRef);
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
        if (!isCurrentWorkspaceSnapshot({
          workingDir: props.workingDir,
          workspaceRef: event.payload.workspaceRef,
          checkoutId: event.payload.workspaceRef.checkoutId,
          expectedGeneration: event.payload.workspaceRef.expectedGeneration ?? null,
          expectedMaterializationEpoch: event.payload.workspaceRef.expectedMaterializationEpoch ?? null,
        })) return;
        void selectCollabSearchResult(event.payload);
      },
    );
  } catch {
    collabSearchSelectionUnlisten = null;
  }
});

onUnmounted(() => {
  sidebarNavigationRequestId += 1;
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

function onCommitFileContextMenu(event: MouseEvent, file: GitFileChange) {
  event.preventDefault();
  event.stopPropagation();
  if (!selectedCommitHash.value) return;
  openCtxMenu(event, { kind: "file", file, source: "gitCommit", commitHash: selectedCommitHash.value, selectedFiles: [file] });
}

function confirmDiscardFile() {
  if (workspaceMutationBusy.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "file" || target.source === "gitCommit" || hasConflictState.value) return;
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
      const workspace = captureWorkspaceSnapshot();
      addPendingDiscardPaths(expandedPaths);
      try {
        if (source === "gitStaged") {
          await unstageFiles(expandedPaths);
        }
        for (const f of discardFiles) {
          await gitDiscardFile(f.path, f.status, f.oldPath, workspace.workspaceRef);
        }
      } catch (e) {
        if (!isCurrentWorkspaceSnapshot(workspace)) return;
        const err = normalizeAppError(e);
        notificationStore.addNotice("error", err.message || t("collab.discardFailed"), {
          code: err.code,
          operation: "collabDiscard",
        });
      } finally {
        try {
          if (isCurrentWorkspaceSnapshot(workspace)) await onRefresh();
        } finally {
          if (isCurrentWorkspaceSnapshot(workspace)) {
            removePendingDiscardPaths(expandedPaths);
          }
        }
      }
    },
  };
}

function doFileStage() {
  if (workspaceMutationBusy.value) return;
  const target = ctxMenu.value?.target;
  if (target?.kind !== "file" || target.source !== "gitUnstaged") return;
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
  if (target?.kind !== "file" || target.source !== "gitStaged") return;
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
    await showInFolder(filePath, captureWorkspaceSnapshot().workspaceRef);
  } catch (e) {
    const err = normalizeAppError(e);
    notificationStore.addNotice("error", err.message || t("collab.menu.showInFolderFailed"), {
      code: err.code,
      operation: "collabShowInFolder",
    });
  }
}


type BranchActionName = "switch" | "checkoutTracking" | "mergeIntoCurrent" | "rebaseCurrentOnto" | "pull" | "push" | "fetch";

async function performBranchAction(branchName: string, targetKind: "local" | "remote", action: BranchActionName, remoteName?: string) {
  if (hasConflictState.value || workspaceMutationBusy.value) return;
  const commands: Record<BranchActionName, string> = {
    switch: 'git switch', checkoutTracking: 'git checkout --track', mergeIntoCurrent: 'git merge',
    rebaseCurrentOnto: 'git rebase', pull: 'git pull --ff-only', push: 'git push', fetch: 'git fetch --prune',
  };
  const key = action === 'switch' || action === 'checkoutTracking' ? 'checkoutBranch' : action;
  try {
    await runGitOp(t('collab.menu.' + key, branchName), commands[action] + ' ' + branchName,
      workspace => gitBranchAction(branchName, targetKind, action, undefined, workspace, remoteName));
  } catch { /* runGitOp reports errors and refreshes conflict state. */ }
}

async function onBranchDblclick(target: GitBranchTarget) {
  const action = resolveBranchDblclickAction(target, localBranches.value);
  if (!action) return;
  await performBranchAction(action.branchName, action.targetKind, action.action);
}

function promptGitName(title: string, action: (value: string) => void, value = '') {
  promptDialog.value = {
    title, placeholder: t('collab.menu.namePlaceholder'), value,
    validate: name => validateGitName(name) ? null : t('collab.menu.invalidName'), action,
  };
  void nextTick(() => document.querySelector<HTMLInputElement>('.collab-git-prompt input')?.focus());
}

async function copyGitText(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    notificationStore.addNotice('success', t('common.copied'), { operation: 'collabCopy' });
  } catch {
    notificationStore.addNotice('error', t('collab.menu.copyFailed'), { operation: 'collabCopy' });
  }
}

function targetRevision(target: GitContextTarget): string | null {
  if (target.kind === 'commit') return target.commit.hash;
  if (target.kind === 'stash') return target.stash.hash;
  if (target.kind === 'localBranch') return 'refs/heads/' + target.branch.name;
  if (target.kind === 'remoteBranch') return 'refs/remotes/' + branchTargetName(target);
  return null;
}

async function executeCommitAction(rev: string, action: string, label: string, mode?: string, name?: string) {
  const commands: Record<string, string> = {
    checkoutDetached: 'checkout --detach', cherryPick: 'cherry-pick', revert: 'revert --no-edit',
    createBranch: 'branch', createBranchAndCheckout: 'checkout -b', createTag: 'tag', reset: 'reset',
  };
  const option = mode ? (action === 'reset' ? ' --' + mode : ' --mainline ' + mode) : '';
  try {
    await runGitOp(label, 'git ' + commands[action] + option + (name ? ' ' + name : '') + ' ' + rev,
      workspace => gitCommitAction(rev, action, mode, name, workspace));
  } catch { /* reported by runGitOp */ }
}

function confirmGitAction(title: string, message: string, action: () => void) {
  confirmDialog.value = { title, message, danger: true, action };
}

async function onGitMenuAction(item: GitMenuItem) {
  // Recheck the current model so a pending operation cannot be queued twice.
  const current = gitMenuGroups.value.flat().find(entry => entry.action === item.action
    && (!item.branch || (entry.branch && branchTargetName(entry.branch) === branchTargetName(item.branch))));
  if (!current || current.disabled || !ctxMenu.value) return;
  const target = ctxMenu.value.target;
  const action = item.action;
  if (action === 'stage') { doFileStage(); return; }
  if (action === 'unstage') { doFileUnstage(); return; }
  if (action === 'discard') { confirmDiscardFile(); return; }
  if (action === 'showInFolder') { await doShowInFolder(); return; }
  closeCtxMenu();

  if (action === 'checkoutBranch' && item.branch) { await onBranchDblclick(item.branch); return; }
  if (action === 'copyHash') {
    const hash = target.kind === 'commit' ? target.commit.hash : target.kind === 'stash' ? target.stash.hash
      : target.kind === 'localBranch' || target.kind === 'remoteBranch' ? resolveBranchTargetHash(target, graphRefs.value) : null;
    if (hash) await copyGitText(hash);
    return;
  }
  if (action === 'copyMessage') {
    if (target.kind === 'commit') await copyGitText(target.commit.message);
    if (target.kind === 'stash') await copyGitText(target.stash.message);
    return;
  }
  const rev = targetRevision(target);
  if (rev && ['createBranch', 'createBranchAndCheckout', 'createTag'].includes(action)) {
    promptGitName(item.label, name => {
      if (action !== 'createTag' && localBranches.value.some(branch => branch.name === name)) {
        notificationStore.addNotice('error', t('collab.menu.branchExists', name));
        return;
      }
      void executeCommitAction(rev, action, item.label, undefined, name);
    });
    return;
  }
  if (target.kind === 'commit') {
    const commit = target.commit;
    if (action === 'resetHard') {
      confirmGitAction(t('collab.resetHardTitle', commit.shortHash), t('collab.resetHardMsg'),
        () => { void executeCommitAction(commit.hash, 'reset', item.label, 'hard'); });
    } else if (action === 'resetSoft' || action === 'resetMixed') {
      await executeCommitAction(commit.hash, 'reset', item.label, action === 'resetSoft' ? 'soft' : 'mixed');
    } else if (action === 'checkoutDetached' || action === 'cherryPick' || action === 'revert') {
      if (commit.parents.length > 1 && action !== 'checkoutDetached') {
        promptDialog.value = {
          title: item.label, placeholder: '1', value: '1', hint: t('collab.menu.mainlineHint', commit.parents.length),
          validate: value => /^\d+$/.test(value) && Number(value) >= 1 && Number(value) <= commit.parents.length
            ? null : t('collab.menu.invalidMainline', commit.parents.length),
          action: value => { void executeCommitAction(commit.hash, action, item.label, value); },
        };
        void nextTick(() => document.querySelector<HTMLInputElement>('.collab-git-prompt input')?.focus());
      } else await executeCommitAction(commit.hash, action, item.label);
    }
    return;
  }
  if (target.kind === 'localBranch' || target.kind === 'remoteBranch') {
    const name = branchTargetName(target);
    const kind = target.kind === 'localBranch' ? 'local' : 'remote';
    const remote = target.kind === 'remoteBranch' ? target.remoteName : undefined;
    if (action === 'copyBranch') { await copyGitText(name); return; }
    if (action === 'renameBranch') {
      promptGitName(t('collab.renameBranch'), async newName => {
        if (newName === name) return;
        try { await runGitOp(item.label, 'git branch -m ' + name + ' ' + newName,
          workspace => gitBranchAction(name, kind, 'rename', newName, workspace)); } catch {}
      }, name);
    } else if (action === 'deleteBranch' || action === 'deleteRemoteBranch') {
      confirmGitAction(t(action === 'deleteBranch' ? 'collab.deleteBranchTitle' : 'collab.menu.deleteRemoteTitle', name),
        t(action === 'deleteBranch' ? 'collab.deleteBranchMsg' : 'collab.menu.deleteRemoteMessage'), async () => {
          try { await runGitOp(item.label, action === 'deleteBranch' ? 'git branch -d ' + name : 'git push --delete ' + remote + ' ' + target.branch.name,
            workspace => gitBranchAction(name, kind, action === 'deleteBranch' ? 'delete' : 'deleteRemote', undefined, workspace, remote)); } catch {}
        });
    } else if (action === 'rebaseCurrentOnto') {
      confirmGitAction(t('collab.menu.rebaseTitle', currentBranch.value, name), t('collab.menu.rebaseMessage'),
        () => { void performBranchAction(name, kind, 'rebaseCurrentOnto', remote); });
    } else if (['mergeIntoCurrent', 'pull', 'push', 'fetch'].includes(action)) {
      await performBranchAction(name, kind, action as BranchActionName, remote);
    }
    return;
  }
  if (target.kind === 'stash') {
    const selected = target.selectedStashes?.length ? target.selectedStashes : [target.stash];
    const perform = async (stashAction: string, name?: string) => {
      try {
        await runGitOp(item.label, 'git stash ' + stashAction + ' ' + (name ? name + ' ' : '') + target.stash.refName,
          workspace => gitStashAction(target.stash.refName, stashAction, workspace, { branchName: name, expectedHash: target.stash.hash }));
      } catch {}
    };
    if (action === 'stashDrop') {
      confirmGitAction(selected.length === 1 ? t('collab.dropStashTitle', target.stash.refName) : t('collab.menu.dropStashTitleMulti', selected.length),
        t(selected.length === 1 ? 'collab.dropStashMsg' : 'collab.menu.dropStashMessageMulti'), async () => {
          try {
            await runGitOp(item.label, 'git stash drop', async workspace => {
              for (const stash of [...selected].sort((a, b) => b.index - a.index)) {
                await gitStashAction(stash.refName, 'drop', workspace, { expectedHash: stash.hash });
              }
            });
          } catch {}
        });
    } else if (action === 'stashBranch') {
      promptGitName(item.label, name => { void perform('branch', name); });
    } else if (action === 'stashApply' || action === 'stashApplyIndex' || action === 'stashPop') {
      await perform(action === 'stashApply' ? 'apply' : action === 'stashApplyIndex' ? 'applyIndex' : 'pop');
    }
    return;
  }
  if (target.kind === 'file') {
    if (action === 'copyRelativePath' || action === 'copyAbsolutePath') {
      const root = props.workingDir.replace(/\\/g, '/').replace(/\/$/, '');
      await copyGitText(target.selectedFiles.map(file => action === 'copyRelativePath' ? file.path : root + '/' + file.path).join('\n'));
    } else if (action === 'viewDiff') {
      // Menu commands open the diff; unlike row clicks they do not toggle it closed.
      if (activeFilePath.value !== target.file.path || activeDiffSource.value !== target.source
        || activeDiffCommitHash.value !== (target.commitHash ?? null) || !inlineDiff.value) {
        await onSelectFile(target.file, target.source, target.commitHash);
      }
    } else {
      try {
        const workspace = captureWorkspaceSnapshot().workspaceRef;
        if (action === 'openEditor') await openFileExternal(target.file.path, workspace);
        if (action === 'selectUnity') await selectUnityAsset(workspace, target.file.path);
      } catch (error) {
        notificationStore.addNotice('error', normalizeAppError(error).message || t('collab.menu.operationFailed'));
      }
    }
  }
}

function submitPrompt() {
  if (!promptDialog.value || !promptDialog.value.value.trim() || promptError.value || workspaceMutationBusy.value || hasConflictState.value) return;
  const dialog = promptDialog.value;
  promptDialog.value = null;
  dialog.action(dialog.value.trim());
}

function doConfirm() {
  if (!confirmDialog.value || workspaceMutationBusy.value || hasConflictState.value) return;
  const dialog = confirmDialog.value;
  confirmDialog.value = null;
  dialog.action();
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

    <Teleport v-if="props.sidebarTarget" :to="props.sidebarTarget">
        <GitSidebar
          :key="`${props.workspaceRef?.checkoutId}:${props.workspaceRef?.expectedGeneration}:${props.workspaceRef?.expectedMaterializationEpoch}`"
          :local-branches="localBranches"
          :remote-branches="remoteBranches"
          :stashes="stashes"
          :unanchored-stash-hashes="unanchoredStashHashes"
          :tags="tags"
          :submodules="submodules"
          :selected-history-hash="selectedCommitHash"
          :toolbar-target="props.sidebarToolbarTarget"
          :expand-local="expandLocal"
          :expand-remotes="expandRemotes"
          :expanded-remote-names="expandedRemoteNames"
          :expand-stashes="expandStashes"
          :expand-tags="expandTags"
          :expand-submodules="expandSubmodules"
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
          @branch-dblclick="onSidebarBranchDblclick"
          @stash-contextmenu="onStashContextMenu"
          @open-git-config="onSidebarGitConfig"
          @open-search="openCollabSearch"
        />
    </Teleport>

    <div class="left-area" ref="leftAreaRef" :style="{ flexBasis: leftColWidth + '%' }">
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
          @branch-contextmenu="onBranchContextMenu"
        />

        <div class="panel-divider-h" @mousedown="onHSplitterMouseDown"></div>

        <div class="terminal-panel" :style="{ height: terminalHeight + 'px' }">
          <GitTerminal
            ref="terminalRef"
            :working-dir="props.workingDir"
            :workspace-ref="props.workspaceRef"
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
        :workspace-ref="props.workspaceRef"
        @close="closeGitConfigPopover"
        @saved="onGitConfigSaved"
      />

      <!-- Merge resolution overlay: covers the left column -->
      <div v-if="selectedConflictFile" class="inline-diff-panel">
        <MergeResolutionPanel
          ref="mergeResolutionRef"
          :key="conflictFileKey(selectedConflictFile)"
          :file="selectedConflictFile"
          :workspace-ref="props.workspaceRef"
          @resolved="onConflictResolved"
          @back="onConflictBack"
        />
      </div>

      <!-- Inline diff overlay: covers the left column -->
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
                v-if="projectStore.unityConnected && props.workspaceRef"
                class="inline-diff-action-btn"
                :title="t('common.selectInUnity')"
                @click="selectInlineDiffInUnity(inlineDiff.filePath)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M6.4 1L1 8l5.4 7h3.2L6.2 9.5H15v-3H6.2L9.6 1H6.4z"/></svg>
                {{ t('common.selectInUnity') }}
              </button>
              <button
                v-if="!inlineDiff.isBinary && canOpenInEditor(inlineDiff.filePath)"
                class="inline-diff-action-btn"
                :title="t('common.openInEditor')"
                @click="openInlineDiffExternal(inlineDiff.filePath)"
              >
                <svg viewBox="0 0 16 16" width="12" height="12" fill="currentColor"><path d="M8 1C4.1 1 1 4.1 1 8s3.1 7 7 7 7-3.1 7-7-3.1-7-7-7zm0 12.5c-3 0-5.5-2.5-5.5-5.5S5 2.5 8 2.5s5.5 2.5 5.5 5.5-2.5 5.5-5.5 5.5zM6 5l6 3-6 3V5z"/></svg>
                {{ t('common.openInEditor') }}
              </button>
            </span>
          </div>
          <div class="inline-diff-body">
            <FileDiffViewer
              ref="fileDiffViewerRef"
              :payload="inlineDiff"
              :workspace-ref="props.workspaceRef"
              :hide-builtin-tabs="true"
              @lfs-pulled="onLfsPulled"
            />
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
      :workspace-ref="props.workspaceRef"
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
      :workspace-ref="props.workspaceRef"
      @select-file="(f: GitFileChange) => onSelectFile(f, 'gitCommit', selectedCommitHash ?? undefined)"
      @file-contextmenu="onCommitFileContextMenu"
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
      :workspace-ref="props.workspaceRef"
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

    <!-- Context menu: shared neutral desktop controls, grouped by operation. -->
    <BaseContextMenu
      v-if="ctxMenu"
      class="collab-git-menu"
      :x="ctxMenu.x"
      :y="ctxMenu.y"
      :min-width="220"
      max-width="min(420px, calc(100vw - 16px))"
      :aria-label="t('collab.menu.ariaLabel')"
      @keydown="navigateGitMenu"
      @close="closeCtxMenu"
    >
      <template v-for="(group, index) in gitMenuGroups" :key="index">
        <div v-if="index" class="ctx-sep" role="separator" />
        <button
          v-for="item in group"
          :key="item.action + (item.branch ? branchTargetName(item.branch) : '')"
          type="button"
          role="menuitem"
          :data-action="item.action"
          :class="{ 'ctx-danger': item.danger }"
          :disabled="item.disabled"
          :title="item.title || item.label"
          @click="onGitMenuAction(item)"
        >
          <LucideIcon :icon="item.icon" :size="13" />
          <span class="git-menu-label">{{ item.label }}</span>
        </button>
      </template>
    </BaseContextMenu>

    <Teleport to="body">
      <!-- Prompt dialog -->
      <div v-if="promptDialog" class="commit-modal-overlay collab-git-prompt" @click.self="promptDialog = null" @keydown.esc.stop="promptDialog = null">
        <div class="commit-modal" role="dialog" aria-modal="true" :aria-label="promptDialog.title" style="max-width: 400px">
          <div class="commit-modal-header">
            <span class="commit-modal-title">{{ promptDialog.title }}</span>
            <BaseButton class="git-dialog-close" :aria-label="t('common.close')" @click="promptDialog = null"><LucideIcon :icon="X" :size="14" /></BaseButton>
          </div>
          <div class="commit-modal-body">
            <input v-model="promptDialog.value" class="commit-input" :placeholder="promptDialog.placeholder" :aria-label="promptDialog.title" :aria-invalid="!!promptError" @keyup.enter="submitPrompt" />
            <p v-if="promptDialog.hint" class="commit-modal-message">{{ promptDialog.hint }}</p>
            <p v-if="promptError" class="commit-modal-warning" role="alert">{{ promptError }}</p>
          </div>
          <div class="commit-modal-footer">
            <div class="commit-modal-actions">
              <BaseButton @click="promptDialog = null">{{ t('common.cancel') }}</BaseButton>
              <BaseButton variant="primary" :disabled="!promptDialog.value.trim() || !!promptError || workspaceMutationBusy || hasConflictState" @click="submitPrompt">{{ t('common.confirm') }}</BaseButton>
            </div>
          </div>
        </div>
      </div>

      <!-- Confirm dialog -->
      <div v-if="confirmDialog" class="commit-modal-overlay" @click.self="confirmDialog = null" @keydown.esc.stop="confirmDialog = null">
        <div class="commit-modal" role="dialog" aria-modal="true" :aria-label="confirmDialog.title" style="max-width: 380px">
          <div class="commit-modal-header">
            <span class="commit-modal-title">{{ confirmDialog.title }}</span>
            <BaseButton class="git-dialog-close" :aria-label="t('common.close')" @click="confirmDialog = null"><LucideIcon :icon="X" :size="14" /></BaseButton>
          </div>
          <div class="commit-modal-body">
            <p class="commit-modal-message">{{ confirmDialog.message }}</p>
            <p v-if="confirmDialog.warning" class="commit-modal-warning">{{ confirmDialog.warning }}</p>
          </div>
          <div class="commit-modal-footer">
            <div class="commit-modal-actions">
              <BaseButton @click="confirmDialog = null">{{ t('common.cancel') }}</BaseButton>
              <BaseButton :variant="confirmDialog.danger ? 'danger' : 'primary'" :disabled="workspaceMutationBusy || hasConflictState" @click="doConfirm">{{ t('common.confirm') }}</BaseButton>
            </div>
          </div>
        </div>
      </div>
    </Teleport>

  </div>
</template>

<style scoped>
.git-menu-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.git-dialog-close {
  border-color: transparent;
  width: 28px;
  padding: 0;
}
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

:deep(.graph-row:focus-visible) {
  outline: 1px solid color-mix(in srgb, var(--accent-color) 64%, var(--border-color));
  outline-offset: -2px;
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
  font-size: calc(12px * var(--text-viewer-font-scale, 1));
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
  font-size: calc(12px * var(--text-viewer-font-scale, 1));
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

</style>
