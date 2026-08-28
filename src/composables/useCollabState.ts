import { ref, computed, getCurrentInstance, onMounted, onUnmounted, watch, type Ref } from "vue";
import { getWarmup } from "./warmupCache";
import { useHideMeta, withMetaCompanionPaths } from "./useHideMeta";

// ── Perf logging ─────────────────────────────────────────────────
const PERF_TAG = "[collab-perf]";
function perfNow() {
  return performance.now();
}
function perfLog(label: string, startMs: number) {
  const elapsed = (performance.now() - startMs).toFixed(1);
  console.log(`${PERF_TAG} ${label}: ${elapsed}ms`);
}
import {
  gitBranches,
  gitCheckUserConfig,
  gitCommitBody,
  gitCommitFiles,
  gitHistorySnapshot,
  gitInitUnity,
  gitProbe,
  gitSetUserConfig,
  gitStage,
  gitStagePaths,
  gitStageAll,
  gitStatus,
  gitSubmodules,
  gitUnstage,
  gitUnstagePaths,
  gitUnstageAll,
} from "../services/git";
import type {
  GitBlockedPath, GitCommitInfo, GitFileChange,
  GitBranchInfo, GitRemoteBranch,
  GitProbeResult, GitStashEntry, GitSubmoduleInfo, ModelOption, GitGraphRef,
  GitStageAllResult,
  GitStatusResult, GitBranchesResult, GitHeadState, GitHistorySelection, GitHistorySnapshot,
  UnmergedFileEntry, MergeOperation,
} from "../types";
import { normalizeAppError } from "../services/errors";
import { t } from "../i18n";
import { acquireSelectionLock } from "./useSelectionLock";
import { useNotificationStore } from "../stores/notification";
import type { WorkspaceRef } from "../services/project";

interface CollabProps {
  workingDir: string;
  workspaceRef?: WorkspaceRef | null;
  isActive: boolean;
  selectedModelId: string;
  selectedAgentId: string;
  models: ModelOption[];
}

interface CollabStateOptions {
  onGitTerminalOutput?: (command: string, output: string, isError?: boolean) => void;
}

interface CollabWorkspaceSnapshot {
  workingDir: string;
  workspaceRef: WorkspaceRef;
  checkoutId: string;
  expectedGeneration: number | null;
}

function quoteGitPath(path: string) {
  if (!/[\s"]/.test(path)) return path;
  return `"${path.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

function summarizeGitCommand(base: string, paths: string[]) {
  if (paths.length === 0) return base;
  if (paths.length <= 3) return `${base} ${paths.map(quoteGitPath).join(" ")}`;
  const preview = paths.slice(0, 2).map(quoteGitPath).join(" ");
  return `${base} ${preview} ... (${paths.length} paths)`;
}

function collectGitMutationPathspecs(paths: string[], files: GitFileChange[]) {
  if (paths.length === 0) return [];
  const fileMap = new Map(files.map(file => [file.path, file] as const));
  const seen = new Set<string>();
  const pathspecs: string[] = [];

  for (const path of paths) {
    if (!seen.has(path)) {
      seen.add(path);
      pathspecs.push(path);
    }

    const oldPath = fileMap.get(path)?.oldPath;
    if (oldPath && !seen.has(oldPath)) {
      seen.add(oldPath);
      pathspecs.push(oldPath);
    }
  }

  return pathspecs;
}

const STAGE_ALL_COMMAND = "git add --pathspec-from-file=- --pathspec-file-nul --ignore-errors";

function formatBlockedPathReason(file: GitBlockedPath) {
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

function buildStageAllOutput(result: GitStageAllResult) {
  const lines: string[] = [];

  if (result.skippedCount > 0) {
    lines.push(
      result.stagedCount > 0
        ? t("collab.stageAllPartial", result.stagedCount, result.skippedCount)
        : t("collab.stageAllBlockedOnly", result.skippedCount),
    );

    for (const file of result.blocked) {
      lines.push(`- ${file.path}: ${formatBlockedPathReason(file)}`);
    }
  }

  const stdout = result.stdout.trim();
  const stderr = result.stderr.trim();

  if (stdout) {
    if (lines.length > 0) lines.push("");
    lines.push(stdout);
  }

  if (stderr) {
    if (lines.length > 0) lines.push("");
    lines.push(stderr);
  }

  return lines.join("\n");
}

export function useCollabState(props: CollabProps, options: CollabStateOptions = {}) {
  const notificationStore = useNotificationStore();

  // ── Core git state ──────────────────────────────────────────────
  const isRepo = ref(false);
  const commits = ref<GitCommitInfo[]>([]);
  const graphRefs = ref<GitGraphRef[]>([]);
  const headHash = ref<string | null>(null);
  const headState = ref<GitHeadState>({
    hash: null,
    kind: "detached",
    refName: null,
  });
  const loading = ref(true);
  const selectedHistory = ref<GitHistorySelection | null>(null);
  const hasMoreCommits = ref(false);
  const loadingMore = ref(false);

  // ── Init state ──────────────────────────────────────────────────
  const initLoading = ref(false);
  const initError = ref<string | null>(null);
  const gitProbeState = ref<GitProbeResult | null>(null);
  const gitAvailable = computed(() => !!gitProbeState.value?.available);
  const gitHelpText = computed(() => {
    const probe = gitProbeState.value;
    if (!probe) return "";
    if (!probe.available) {
      return probe.envOverride
        ? t("git.detect.invalidOverride", probe.envOverride)
        : t("git.detect.missing");
    }
    if (!probe.inPath && probe.path) {
      return t("git.detect.foundOutsidePath", probe.path);
    }
    return "";
  });

  // ── Git config modal ────────────────────────────────────────────
  const showGitConfigModal = ref(false);
  const gitConfigName = ref("");
  const gitConfigEmail = ref("");
  const gitConfigSaving = ref(false);
  const gitConfigError = ref("");
  const currentGitAuthor = ref("");

  // ── File change lists ───────────────────────────────────────────
  const unstagedFiles = ref<GitFileChange[]>([]);
  const stagedFiles = ref<GitFileChange[]>([]);
  const blockedFiles = ref<GitBlockedPath[]>([]);
  const commitFiles = ref<GitFileChange[]>([]);
  const commitBody = ref("");
  const filesLoading = ref(false);
  const pendingStagePaths = ref<Set<string>>(new Set());
  const pendingUnstagePaths = ref<Set<string>>(new Set());
  const stageOperationBusy = computed(() =>
    pendingStagePaths.value.size > 0 || pendingUnstagePaths.value.size > 0,
  );
  let statusLoadedOnce = false;
  let logLoadedOnce = false;
  const snapshotWorkspace = ref(0);

  // ── Merge / conflict state ──────────────────────────────────────
  const unmergedFiles = ref<UnmergedFileEntry[]>([]);
  const mergeOperation = ref<MergeOperation | null>(null);
  // Use operation presence (not unmerged count) to determine merge flow.
  // After all conflicts are resolved, MERGE_HEAD still exists until continue.
  const isMerging = computed(() => !!mergeOperation.value);
  const hasUnresolvedFiles = computed(() => unmergedFiles.value.length > 0);

  // ── Sidebar data ────────────────────────────────────────────────
  const localBranches = ref<GitBranchInfo[]>([]);
  const remoteBranches = ref<[string, GitRemoteBranch[]][]>([]);
  const stashes = ref<GitStashEntry[]>([]);
  const submodules = ref<GitSubmoduleInfo[]>([]);
  const tags = computed(() =>
    graphRefs.value
      .filter(ref => ref.kind === "tag")
      .sort((left, right) => left.shortName.localeCompare(right.shortName)),
  );

  // ── Sidebar UI toggles ─────────────────────────────────────────
  const sidebarCollapsed = ref(false);
  const expandLocal = ref(true);
  const expandRemotes = ref(true);
  const expandedRemoteNames = ref<Set<string>>(new Set());
  const expandStashes = ref(true);
  const expandTags = ref(true);
  const expandSubmodules = ref(true);

  // ── Resize / layout ─────────────────────────────────────────────
  const STORAGE_KEY_SIDEBAR_W = "locus:collabSidebarWidth";
  const STORAGE_KEY_LEFT_COL = "locus:collabLeftColWidth";
  const STORAGE_KEY_TERMINAL_H = "locus:collabTerminalHeight";

  function readStoredNumber(key: string, min: number, max: number, fallback: number): number {
    try {
      const v = localStorage.getItem(key);
      if (v) return Math.max(min, Math.min(max, Number(v)));
    } catch {}
    return fallback;
  }

  const containerRef = ref<HTMLElement | null>(null);
  const leftAreaRef = ref<HTMLElement | null>(null);
  const leftColRef = ref<HTMLElement | null>(null);
  const gitSidebarWidth = ref(readStoredNumber(STORAGE_KEY_SIDEBAR_W, 180, 360, 220));
  const isDraggingSidebar = ref(false);
  const leftColWidth = ref(readStoredNumber(STORAGE_KEY_LEFT_COL, 20, 85, 70));
  const isDraggingV = ref(false);
  const terminalHeight = ref(readStoredNumber(STORAGE_KEY_TERMINAL_H, 80, 600, 240));
  const isDraggingH = ref(false);
  let sidebarResizeMoveHandler: ((event: MouseEvent) => void) | null = null;
  let sidebarResizeUpHandler: (() => void) | null = null;
  let releaseSelectionLock: (() => void) | null = null;

  // ── Data loading ────────────────────────────────────────────────
  const PAGE_SIZE = 30;
  let gitRefreshToken = 0;
  let scheduledRefreshTimer: ReturnType<typeof setTimeout> | null = null;

  function dedupeCommitsByHash(items: GitCommitInfo[]): GitCommitInfo[] {
    const seen = new Set<string>();
    const deduped: GitCommitInfo[] = [];
    for (const item of items) {
      if (seen.has(item.hash)) continue;
      seen.add(item.hash);
      deduped.push(item);
    }
    return deduped;
  }

  function mergeCommitsByHash(existing: GitCommitInfo[], incoming: GitCommitInfo[]): GitCommitInfo[] {
    if (incoming.length === 0) return existing;
    const seen = new Set(existing.map(commit => commit.hash));
    const merged = [...existing];
    for (const commit of incoming) {
      if (seen.has(commit.hash)) continue;
      seen.add(commit.hash);
      merged.push(commit);
    }
    return merged;
  }

  function nextGitRefreshToken() {
    gitRefreshToken += 1;
    return gitRefreshToken;
  }

  function captureWorkspaceSnapshot(): CollabWorkspaceSnapshot {
    if (!props.workspaceRef) {
      throw new Error("Workspace checkout is required for Collab operations.");
    }
    const workspaceRef = {
      checkoutId: props.workspaceRef.checkoutId,
      expectedGeneration: props.workspaceRef.expectedGeneration ?? undefined,
    };
    return {
      workingDir: props.workingDir,
      workspaceRef,
      checkoutId: workspaceRef.checkoutId,
      expectedGeneration: workspaceRef.expectedGeneration ?? null,
    };
  }

  function isCurrentWorkspaceSnapshot(snapshot: CollabWorkspaceSnapshot) {
    return snapshot.workingDir === props.workingDir
      && snapshot.checkoutId === (props.workspaceRef?.checkoutId ?? null)
      && snapshot.expectedGeneration === (props.workspaceRef?.expectedGeneration ?? null);
  }

  function isCurrentGitRefresh(token: number, snapshot: CollabWorkspaceSnapshot) {
    return token === gitRefreshToken && isCurrentWorkspaceSnapshot(snapshot);
  }

  function clearScheduledGitRefresh() {
    if (scheduledRefreshTimer !== null) {
      clearTimeout(scheduledRefreshTimer);
      scheduledRefreshTimer = null;
    }
  }

  function scheduleGitRefresh(delay = 120) {
    clearScheduledGitRefresh();
    scheduledRefreshTimer = setTimeout(() => {
      scheduledRefreshTimer = null;
      void refreshGitData();
    }, delay);
  }

  function refreshWhenVisible(delay = 120) {
    if (!props.isActive) return;
    if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
    scheduleGitRefresh(delay);
  }

  function handleWindowFocus() {
    refreshWhenVisible(80);
  }

  function handleVisibilityChange() {
    if (typeof document === "undefined" || document.visibilityState === "visible") {
      refreshWhenVisible(80);
    }
  }

  async function loadHistorySnapshot(token: number, workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir) return;
    if (gitProbeState.value && !gitProbeState.value.available) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      isRepo.value = false;
      commits.value = [];
      graphRefs.value = [];
      headHash.value = null;
      headState.value = {
        hash: null,
        kind: "detached",
        refName: null,
      };
      stashes.value = [];
      hasMoreCommits.value = false;
      return;
    }
    if (isCurrentGitRefresh(token, workspace) && !logLoadedOnce) {
      loading.value = true;
    }
    const t0 = perfNow();
    try {
      const result = await gitHistorySnapshot(0, PAGE_SIZE, workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      isRepo.value = result.isRepo;
      commits.value = dedupeCommitsByHash(result.commits);
      graphRefs.value = result.refs;
      headHash.value = result.head.hash;
      headState.value = result.head;
      stashes.value = result.stashes;
      snapshotWorkspace.value = result.workspace.changeCount;
      hasMoreCommits.value = result.hasMore;
    } catch (e) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      console.error("git_history_snapshot failed:", e);
      isRepo.value = false;
      commits.value = [];
      graphRefs.value = [];
      stashes.value = [];
      hasMoreCommits.value = false;
    } finally {
      if (isCurrentGitRefresh(token, workspace)) {
        logLoadedOnce = true;
        loading.value = false;
        perfLog("gitHistorySnapshot", t0);
      }
    }
  }

  async function loadMoreCommits() {
    if (loadingMore.value || !hasMoreCommits.value) return;
    const token = gitRefreshToken;
    const workspace = captureWorkspaceSnapshot();
    if (!workspace.workingDir) return;
    loadingMore.value = true;
    try {
      const result = await gitHistorySnapshot(
        commits.value.length,
        PAGE_SIZE,
        workspace.workspaceRef,
      );
      if (!isCurrentGitRefresh(token, workspace)) return;
      commits.value = mergeCommitsByHash(commits.value, result.commits);
      hasMoreCommits.value = result.hasMore;
    } catch (e) {
      if (isCurrentGitRefresh(token, workspace)) {
        console.error("git_history_snapshot loadMore failed:", e);
      }
    } finally {
      if (isCurrentWorkspaceSnapshot(workspace)) loadingMore.value = false;
    }
  }

  async function loadGitStatus(token: number, workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir) return;
    if (gitProbeState.value && !gitProbeState.value.available) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      applyGitStatusResult({
        unstaged: [],
        staged: [],
        blocked: [],
        unmerged: [],
        operation: null,
      });
      return;
    }
    // Only show loading indicator on first load; skip for background refreshes
    // to avoid flickering the file list with "Loading..." text.
    if (isCurrentGitRefresh(token, workspace) && !statusLoadedOnce) {
      filesLoading.value = true;
    }
    const t0 = perfNow();
    try {
      const result = await gitStatus(workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      applyGitStatusResult(result);
    } catch (e) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      console.error("git_status failed:", e);
      applyGitStatusResult({
        unstaged: [],
        staged: [],
        blocked: [],
        unmerged: [],
        operation: null,
      });
    } finally {
      if (isCurrentGitRefresh(token, workspace)) {
        statusLoadedOnce = true;
        filesLoading.value = false;
        perfLog("gitStatus", t0);
      }
    }
  }

  let commitFilesRequestToken = 0;

  async function loadCommitFiles(hash: string) {
    const workspace = captureWorkspaceSnapshot();
    const requestToken = ++commitFilesRequestToken;
    filesLoading.value = true;
    commitBody.value = "";
    const t0 = perfNow();
    try {
      const [files, body] = await Promise.all([
        gitCommitFiles(hash, workspace.workspaceRef),
        gitCommitBody(hash, workspace.workspaceRef),
      ]);
      if (
        requestToken !== commitFilesRequestToken
        || !isCurrentWorkspaceSnapshot(workspace)
        || selectedCommitHash.value !== hash
      ) return;
      commitFiles.value = files;
      commitBody.value = body;
    } catch (e) {
      if (
        requestToken !== commitFilesRequestToken
        || !isCurrentWorkspaceSnapshot(workspace)
      ) return;
      console.error("git_commit_files failed:", e);
      commitFiles.value = [];
      commitBody.value = "";
    } finally {
      if (
        requestToken === commitFilesRequestToken
        && isCurrentWorkspaceSnapshot(workspace)
      ) {
        filesLoading.value = false;
        perfLog("gitCommitFiles", t0);
      }
    }
  }

  async function loadBranches(token: number, workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir) return;
    if (gitProbeState.value && !gitProbeState.value.available) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      localBranches.value = [];
      remoteBranches.value = [];
      return;
    }
    const t0 = perfNow();
    try {
      const result = await gitBranches(workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      localBranches.value = result.local;
      remoteBranches.value = result.remotes;
      for (const [name] of result.remotes) {
        expandedRemoteNames.value.add(name);
      }
    } catch (e) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      console.error("git_branches failed:", e);
    } finally {
      if (isCurrentGitRefresh(token, workspace)) {
        perfLog("gitBranches", t0);
      }
    }
  }

  async function loadSubmodules(token: number, workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir) return;
    if (gitProbeState.value && !gitProbeState.value.available) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      submodules.value = [];
      return;
    }
    const t0 = perfNow();
    try {
      const result = await gitSubmodules(workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      submodules.value = result;
    } catch (e) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      console.error("git_submodules failed:", e);
    } finally {
      if (isCurrentGitRefresh(token, workspace)) {
        perfLog("gitSubmodules", t0);
      }
    }
  }

  async function loadSidebarData(token: number, workspace: CollabWorkspaceSnapshot) {
    const t0 = perfNow();
    await Promise.all([
      loadBranches(token, workspace),
      loadSubmodules(token, workspace),
    ]);
    if (isCurrentGitRefresh(token, workspace)) {
      perfLog("sidebar [branches + submodules]", t0);
    }
  }

  async function loadGitUserConfig(token: number, workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir) {
      if (isCurrentGitRefresh(token, workspace)) {
        currentGitAuthor.value = "";
      }
      return;
    }
    if (gitProbeState.value && !gitProbeState.value.available) {
      if (isCurrentGitRefresh(token, workspace)) {
        currentGitAuthor.value = "";
      }
      return;
    }

    const t0 = perfNow();
    try {
      const cfg = await gitCheckUserConfig(workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      const name = cfg.name.trim();
      const email = cfg.email.trim();
      currentGitAuthor.value = name || email;
    } catch {
      if (!isCurrentGitRefresh(token, workspace)) return;
      currentGitAuthor.value = "";
    } finally {
      if (isCurrentGitRefresh(token, workspace)) {
        perfLog("gitCheckUserConfig", t0);
      }
    }
  }

  function resetGitData() {
    logLoadedOnce = false;
    statusLoadedOnce = false;
    commits.value = [];
    graphRefs.value = [];
    isRepo.value = false;
    headHash.value = null;
    headState.value = {
      hash: null,
      kind: "detached",
      refName: null,
    };
    hasMoreCommits.value = false;
    loadingMore.value = false;
    unstagedFiles.value = [];
    stagedFiles.value = [];
    blockedFiles.value = [];
    commitFiles.value = [];
    commitBody.value = "";
    selectedHistory.value = null;
    localBranches.value = [];
    remoteBranches.value = [];
    stashes.value = [];
    submodules.value = [];
    snapshotWorkspace.value = 0;
    unmergedFiles.value = [];
    mergeOperation.value = null;
    pendingStagePaths.value = new Set();
    pendingUnstagePaths.value = new Set();
    currentGitAuthor.value = "";
  }

  async function loadGitAvailability(token: number, workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir) {
      if (isCurrentGitRefresh(token, workspace)) {
        gitProbeState.value = null;
      }
      return;
    }
    const t0 = perfNow();
    try {
      const probe = await gitProbe(workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      gitProbeState.value = probe;
    } catch {
      if (!isCurrentGitRefresh(token, workspace)) return;
      gitProbeState.value = {
        available: false,
        inPath: false,
        isRepo: false,
      };
    }
    if (isCurrentGitRefresh(token, workspace)) {
      perfLog("gitProbe", t0);
    }
  }

  async function refreshGitData() {
    const workspace = captureWorkspaceSnapshot();
    const token = nextGitRefreshToken();
    const t0 = perfNow();
    // Don't set loading=true on background refreshes; loadGitLog handles its own first-load state.
    await loadGitAvailability(token, workspace);
    if (!isCurrentGitRefresh(token, workspace)) return;
    if (!gitProbeState.value?.available) {
      resetGitData();
      loading.value = false;
      filesLoading.value = false;
      perfLog("refreshGitData (no git)", t0);
      return;
    }
    const t1 = perfNow();
    await Promise.all([
      loadHistorySnapshot(token, workspace),
      loadGitStatus(token, workspace),
      loadSidebarData(token, workspace),
      loadGitUserConfig(token, workspace),
    ]);
    if (!isCurrentGitRefresh(token, workspace)) return;
    perfLog("parallel [gitHistorySnapshot + gitStatus + sidebar + userConfig]", t1);
    perfLog("refreshGitData (total)", t0);
  }

  // ── Git init flow ───────────────────────────────────────────────
  let initRequestToken = 0;

  async function initGitUnity() {
    const workspace = captureWorkspaceSnapshot();
    const requestToken = ++initRequestToken;
    initError.value = null;
    if (!gitAvailable.value) {
      initError.value = gitHelpText.value || t("collab.gitInitFailed");
      return;
    }
    try {
      const cfg = await gitCheckUserConfig(workspace.workspaceRef);
      if (requestToken !== initRequestToken || !isCurrentWorkspaceSnapshot(workspace)) return;
      currentGitAuthor.value = cfg.name.trim() || cfg.email.trim();
      if (!cfg.name || !cfg.email) {
        gitConfigName.value = cfg.name;
        gitConfigEmail.value = cfg.email;
        showGitConfigModal.value = true;
        return;
      }
      await doInitGitUnity(workspace, requestToken);
    } catch (e) {
      if (requestToken !== initRequestToken || !isCurrentWorkspaceSnapshot(workspace)) return;
      initError.value = normalizeAppError(e).message || t("collab.gitInitFailed");
    }
  }

  async function saveGitConfigAndInit() {
    const workspace = captureWorkspaceSnapshot();
    const requestToken = ++initRequestToken;
    gitConfigSaving.value = true;
    gitConfigError.value = "";
    try {
      await gitSetUserConfig(gitConfigName.value, gitConfigEmail.value);
      if (requestToken !== initRequestToken || !isCurrentWorkspaceSnapshot(workspace)) return;
      currentGitAuthor.value = gitConfigName.value.trim() || gitConfigEmail.value.trim();
      showGitConfigModal.value = false;
      await doInitGitUnity(workspace, requestToken);
    } catch (e) {
      if (requestToken !== initRequestToken || !isCurrentWorkspaceSnapshot(workspace)) return;
      gitConfigError.value = normalizeAppError(e).message;
    } finally {
      if (requestToken === initRequestToken && isCurrentWorkspaceSnapshot(workspace)) {
        gitConfigSaving.value = false;
      }
    }
  }

  function cancelGitConfig() {
    showGitConfigModal.value = false;
  }

  async function doInitGitUnity(
    workspace: CollabWorkspaceSnapshot,
    requestToken: number,
  ) {
    initLoading.value = true;
    initError.value = null;
    try {
      await gitInitUnity(workspace.workspaceRef);
      if (requestToken !== initRequestToken || !isCurrentWorkspaceSnapshot(workspace)) return;
      clearScheduledGitRefresh();
      await refreshGitData();
    } catch (e) {
      if (requestToken !== initRequestToken || !isCurrentWorkspaceSnapshot(workspace)) return;
      initError.value = normalizeAppError(e).message || t("collab.gitInitFailed");
    } finally {
      if (requestToken === initRequestToken && isCurrentWorkspaceSnapshot(workspace)) {
        initLoading.value = false;
      }
    }
  }

  // ── Sidebar toggles ────────────────────────────────────────────
  function toggleRemote(name: string) {
    if (expandedRemoteNames.value.has(name)) {
      expandedRemoteNames.value.delete(name);
    } else {
      expandedRemoteNames.value.add(name);
    }
    expandedRemoteNames.value = new Set(expandedRemoteNames.value);
  }

  // ── Staging actions ─────────────────────────────────────────────
  const { hideMeta } = useHideMeta();

  function extractGitIndexLockPath(message: string) {
    const quotedMatch = message.match(/Unable to create '([^']*index\.lock)'/i);
    if (quotedMatch?.[1]) return quotedMatch[1];

    const prefix = "Git index is locked:";
    const prefixStart = message.indexOf(prefix);
    if (prefixStart === -1) return "";
    const rest = message.slice(prefixStart + prefix.length).trim();
    const marker = "index.lock";
    const markerEnd = rest.toLowerCase().indexOf(marker);
    if (markerEnd === -1) return "";
    return rest.slice(0, markerEnd + marker.length).trim();
  }

  function formatGitStatusWarningMessage(warning: ReturnType<typeof normalizeAppError>) {
    if (warning.code !== "git.index_lock") return warning.message;
    const lockPath = extractGitIndexLockPath(warning.detail ?? warning.message);
    return lockPath
      ? t("collab.gitIndexLocked", lockPath)
      : t("collab.gitIndexLockedNoPath");
  }

  function notifyGitStatusWarnings(result: GitStatusResult) {
    for (const warning of result.warnings ?? []) {
      const normalized = normalizeAppError(warning);
      notificationStore.addNotice(normalized.severity, formatGitStatusWarningMessage(normalized), {
        code: normalized.code,
        operation: normalized.operation ?? "git_status",
        replaceOperation: true,
        ttl: 10_000,
      });
    }
  }

  function applyGitStatusResult(result: GitStatusResult) {
    unstagedFiles.value = result.unstaged;
    stagedFiles.value = result.staged;
    blockedFiles.value = result.blocked ?? [];
    unmergedFiles.value = result.unmerged ?? [];
    mergeOperation.value = result.operation ?? null;
    notifyGitStatusWarnings(result);
  }

  function addPendingPaths(target: Ref<Set<string>>, paths: string[]) {
    if (paths.length === 0) return;
    const next = new Set(target.value);
    for (const path of paths) next.add(path);
    target.value = next;
  }

  function removePendingPaths(target: Ref<Set<string>>, paths: string[]) {
    if (paths.length === 0) return;
    const next = new Set(target.value);
    for (const path of paths) next.delete(path);
    target.value = next;
  }

  function pushGitTerminalOutput(command: string, output: string, isError = false) {
    if (!output) return;
    options.onGitTerminalOutput?.(command, output, isError);
  }

  async function reconcileGitStatusAfterMutation(workspace: CollabWorkspaceSnapshot) {
    if (!workspace.workingDir || !isCurrentWorkspaceSnapshot(workspace)) return;
    const token = nextGitRefreshToken();
    const t0 = perfNow();
    try {
      const result = await gitStatus(workspace.workspaceRef);
      if (!isCurrentGitRefresh(token, workspace)) return;
      applyGitStatusResult(result);
      perfLog("gitStatus (mutation reconcile)", t0);
    } catch (e) {
      if (!isCurrentGitRefresh(token, workspace)) return;
      pushGitTerminalOutput(
        "git status --porcelain=v2 -z -uall",
        normalizeAppError(e).message,
        true,
      );
      scheduleGitRefresh(300);
    }
  }

  async function runStageMutation(
    paths: string[],
    target: Ref<Set<string>>,
    action: (workspaceRef: WorkspaceRef) => Promise<void>,
    errorLabel: string,
    commandLabel: string,
  ) {
    const workspace = captureWorkspaceSnapshot();
    const uniquePaths = [...new Set(paths)];
    if (uniquePaths.length === 0 || stageOperationBusy.value) return;
    addPendingPaths(target, uniquePaths);
    clearScheduledGitRefresh();
    try {
      await action(workspace.workspaceRef);
    } catch (e) {
      if (!isCurrentWorkspaceSnapshot(workspace)) return;
      const err = normalizeAppError(e);
      pushGitTerminalOutput(commandLabel, err.message || errorLabel, true);
    } finally {
      try {
        await reconcileGitStatusAfterMutation(workspace);
      } finally {
        if (isCurrentWorkspaceSnapshot(workspace)) {
          removePendingPaths(target, uniquePaths);
        }
      }
    }
  }

  async function stageFile(path: string) {
    await stageFiles([path]);
  }

  async function unstageFile(path: string) {
    await unstageFiles([path]);
  }

  async function stageFiles(paths: string[]) {
    const expanded = withMetaCompanionPaths(
      paths,
      unstagedFiles.value.map(file => file.path),
      hideMeta.value,
    );
    const pathspecs = collectGitMutationPathspecs(expanded, unstagedFiles.value);
    await runStageMutation(
      expanded,
      pendingStagePaths,
      (workspaceRef) => pathspecs.length === 1
        ? gitStage(pathspecs[0], workspaceRef)
        : gitStagePaths(pathspecs, workspaceRef),
      expanded.length === 1 ? "stage failed:" : "stage files failed:",
      summarizeGitCommand("git add --", pathspecs),
    );
  }

  async function unstageFiles(paths: string[]) {
    const expanded = withMetaCompanionPaths(
      paths,
      stagedFiles.value.map(file => file.path),
      hideMeta.value,
    );
    const pathspecs = collectGitMutationPathspecs(expanded, stagedFiles.value);
    await runStageMutation(
      expanded,
      pendingUnstagePaths,
      (workspaceRef) => pathspecs.length === 1
        ? gitUnstage(pathspecs[0], workspaceRef)
        : gitUnstagePaths(pathspecs, workspaceRef),
      expanded.length === 1 ? "unstage failed:" : "unstage files failed:",
      summarizeGitCommand("git restore --staged --", pathspecs),
    );
  }

  async function stageAll() {
    const workspace = captureWorkspaceSnapshot();
    const paths = unstagedFiles.value.map(file => file.path);
    if ((paths.length === 0 && blockedFiles.value.length === 0) || stageOperationBusy.value) return;
    addPendingPaths(pendingStagePaths, paths);
    clearScheduledGitRefresh();
    try {
      const result = await gitStageAll(workspace.workspaceRef);
      if (!isCurrentWorkspaceSnapshot(workspace)) return;
      const output = buildStageAllOutput(result);
      if (output) {
        pushGitTerminalOutput(STAGE_ALL_COMMAND, output, false);
      }
      if (result.skippedCount > 0) {
        notificationStore.addNotice(
          "warning",
          result.stagedCount > 0
            ? t("collab.stageAllPartial", result.stagedCount, result.skippedCount)
            : t("collab.stageAllBlockedOnly", result.skippedCount),
          {
            operation: "collabStageAll",
            ttl: 4000,
          },
        );
      }
    } catch (e) {
      if (!isCurrentWorkspaceSnapshot(workspace)) return;
      const err = normalizeAppError(e);
      pushGitTerminalOutput(STAGE_ALL_COMMAND, err.message || "stage all failed:", true);
    } finally {
      try {
        await reconcileGitStatusAfterMutation(workspace);
      } finally {
        if (isCurrentWorkspaceSnapshot(workspace)) {
          removePendingPaths(pendingStagePaths, paths);
        }
      }
    }
  }

  async function unstageAll() {
    await runStageMutation(
      stagedFiles.value.map(file => file.path),
      pendingUnstagePaths,
      (workspaceRef) => gitUnstageAll(workspaceRef),
      "unstage all failed:",
      "git reset HEAD",
    );
  }

  async function onCommitted() {
    clearScheduledGitRefresh();
    await refreshGitData();
  }

  function onTerminalDone() {
    scheduleGitRefresh(180);
  }

  function onTerminalTouched() {
    scheduleGitRefresh(120);
  }

  function onRefresh() {
    clearScheduledGitRefresh();
    return refreshGitData();
  }

  // ── Computed ────────────────────────────────────────────────────
  const currentBranch = computed(() => {
    if (headState.value.kind === "attached") {
      return headState.value.refName ?? "";
    }
    return headState.value.hash ? "HEAD (detached)" : "";
  });

  const hasWorkspaceChanges = computed(() => {
    const uniquePaths = new Set<string>();
    for (const file of unstagedFiles.value) uniquePaths.add(file.path);
    for (const file of stagedFiles.value) uniquePaths.add(file.path);
    for (const file of blockedFiles.value) uniquePaths.add(file.path);
    for (const file of unmergedFiles.value) uniquePaths.add(file.path);
    return uniquePaths.size > 0 || snapshotWorkspace.value > 0;
  });

  const selectedCommitHash = computed<string | null>({
    get() {
      if (!selectedHistory.value) return null;
      return selectedHistory.value.kind === "commit" || selectedHistory.value.kind === "stash"
        ? selectedHistory.value.hash
        : null;
    },
    set(hash) {
      if (!hash) {
        selectedHistory.value = hasWorkspaceChanges.value ? { kind: "workspace" } : null;
        return;
      }

      const stash = stashes.value.find(entry => entry.hash === hash);
      if (stash) {
        selectedHistory.value = { kind: "stash", hash, refName: stash.refName };
        return;
      }

      selectedHistory.value = { kind: "commit", hash };
    },
  });

  const selectedCommit = computed(() => {
    if (!selectedCommitHash.value) return null;
    const stash = stashes.value.find(entry => entry.hash === selectedCommitHash.value);
    if (stash) {
      return {
        hash: stash.hash,
        shortHash: stash.shortHash,
        parents: [],
        author: stash.author,
        date: stash.date,
        message: stash.message,
        refs: ["refs/stash"],
        isStash: true,
      };
    }

    return commits.value.find(c => c.hash === selectedCommitHash.value) ?? null;
  });

  const totalChanges = computed(() =>
    unstagedFiles.value.length + stagedFiles.value.length + blockedFiles.value.length,
  );
  const workspaceChangeCount = computed(() => {
    const uniquePaths = new Set<string>();
    for (const file of unstagedFiles.value) uniquePaths.add(file.path);
    for (const file of stagedFiles.value) uniquePaths.add(file.path);
    for (const file of blockedFiles.value) uniquePaths.add(file.path);
    for (const file of unmergedFiles.value) uniquePaths.add(file.path);
    return uniquePaths.size || snapshotWorkspace.value;
  });

  const draggingClass = computed(() => {
    if (isDraggingSidebar.value) return "dragging-sidebar";
    if (isDraggingV.value) return "dragging-v";
    if (isDraggingH.value) return "dragging-h";
    return "";
  });

  // ── Resize: vertical splitter ───────────────────────────────────
  function clampSidebarWidth(width: number) {
    const leftAreaWidth = leftAreaRef.value?.getBoundingClientRect().width ?? 0;
    const maxWidth = leftAreaWidth > 0
      ? Math.max(180, Math.min(360, leftAreaWidth - 220))
      : 360;
    return Math.max(180, Math.min(maxWidth, width));
  }

  function stopSidebarResize() {
    isDraggingSidebar.value = false;
    if (sidebarResizeMoveHandler) {
      document.removeEventListener("mousemove", sidebarResizeMoveHandler);
      sidebarResizeMoveHandler = null;
    }
    if (sidebarResizeUpHandler) {
      document.removeEventListener("mouseup", sidebarResizeUpHandler);
      sidebarResizeUpHandler = null;
    }
    document.body.style.cursor = "";
    releaseSelectionLock?.();
    releaseSelectionLock = null;
  }

  function onSidebarSplitterMouseDown(e: MouseEvent) {
    e.preventDefault();
    e.stopPropagation();
    stopSidebarResize();
    isDraggingSidebar.value = true;
    const startX = e.clientX;
    const startWidth = gitSidebarWidth.value;

    sidebarResizeMoveHandler = (event: MouseEvent) => {
      if (!isDraggingSidebar.value) return;
      const delta = event.clientX - startX;
      gitSidebarWidth.value = clampSidebarWidth(startWidth + delta);
    };

    sidebarResizeUpHandler = () => {
      try { localStorage.setItem(STORAGE_KEY_SIDEBAR_W, String(Math.round(gitSidebarWidth.value))); } catch {}
      stopSidebarResize();
    };

    document.addEventListener("mousemove", sidebarResizeMoveHandler);
    document.addEventListener("mouseup", sidebarResizeUpHandler);
    document.body.style.cursor = "col-resize";
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
  }

  function onVSplitterMouseDown(e: MouseEvent) {
    e.preventDefault();
    isDraggingV.value = true;
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
    document.addEventListener("mousemove", onVSplitterMouseMove);
    document.addEventListener("mouseup", onVSplitterMouseUp);
  }

  function onVSplitterMouseMove(e: MouseEvent) {
    if (!isDraggingV.value || !containerRef.value) return;
    const rect = containerRef.value.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const pct = (x / rect.width) * 100;
    leftColWidth.value = Math.max(20, Math.min(85, pct));
  }

  function onVSplitterMouseUp() {
    isDraggingV.value = false;
    document.removeEventListener("mousemove", onVSplitterMouseMove);
    document.removeEventListener("mouseup", onVSplitterMouseUp);
    releaseSelectionLock?.();
    releaseSelectionLock = null;
    try { localStorage.setItem(STORAGE_KEY_LEFT_COL, String(leftColWidth.value)); } catch {}
  }

  // ── Resize: horizontal splitter ─────────────────────────────────
  function onHSplitterMouseDown(e: MouseEvent) {
    e.preventDefault();
    isDraggingH.value = true;
    releaseSelectionLock?.();
    releaseSelectionLock = acquireSelectionLock();
    document.addEventListener("mousemove", onHSplitterMouseMove);
    document.addEventListener("mouseup", onHSplitterMouseUp);
  }

  function onHSplitterMouseMove(e: MouseEvent) {
    if (!isDraggingH.value || !leftColRef.value) return;
    const rect = leftColRef.value.getBoundingClientRect();
    const bottomY = rect.bottom;
    const h = bottomY - e.clientY;
    terminalHeight.value = Math.max(80, Math.min(rect.height - 80, h));
  }

  function onHSplitterMouseUp() {
    isDraggingH.value = false;
    document.removeEventListener("mousemove", onHSplitterMouseMove);
    document.removeEventListener("mouseup", onHSplitterMouseUp);
    releaseSelectionLock?.();
    releaseSelectionLock = null;
    try { localStorage.setItem(STORAGE_KEY_TERMINAL_H, String(terminalHeight.value)); } catch {}
  }

  // ── Watchers ────────────────────────────────────────────────────
  watch(selectedCommitHash, (hash) => {
    if (hash) {
      loadCommitFiles(hash);
    } else {
      commitFiles.value = [];
      commitBody.value = "";
    }
  });

  watch([selectedCommitHash, commits, stashes], ([hash]) => {
    if (!hash) return;
    const existsInCommits = commits.value.some(commit => commit.hash === hash);
    const existsInStashes = stashes.value.some(stash => stash.hash === hash);
    if (!existsInCommits && !existsInStashes) {
      selectedHistory.value = hasWorkspaceChanges.value ? { kind: "workspace" } : null;
    }
  });

  watch(hasWorkspaceChanges, (hasChanges) => {
    if (hasChanges) return;
    if (selectedHistory.value?.kind === "workspace") {
      selectedHistory.value = null;
    }
  });

  watch(
    () => [
      props.workingDir,
      props.workspaceRef?.checkoutId ?? null,
      props.workspaceRef?.expectedGeneration ?? null,
    ] as const,
    () => {
      clearScheduledGitRefresh();
      nextGitRefreshToken();
      commitFilesRequestToken += 1;
      initRequestToken += 1;
      initLoading.value = false;
      gitConfigSaving.value = false;
      resetGitData();
      void refreshGitData();
    },
  );

  watch(
    () => props.isActive,
    (active) => {
      if (active) {
        refreshWhenVisible(80);
      }
    },
  );

  // ── Lifecycle ───────────────────────────────────────────────────
  if (getCurrentInstance()) {
    onMounted(async () => {
      if (typeof window !== "undefined") {
        window.addEventListener("focus", handleWindowFocus);
      }
      if (typeof document !== "undefined") {
        document.addEventListener("visibilitychange", handleVisibilityChange);
      }

      // Startup warmup entries carry the checkout identity in their key, so a
      // pane can only consume data fetched for the same runtime generation.
      const warmupScopeKey = props.workspaceRef
        ? `${props.workspaceRef.checkoutId}@${props.workspaceRef.expectedGeneration ?? "current"}`
        : "";
      const cachedProbe = warmupScopeKey
        ? getWarmup<GitProbeResult>(`collab:probe:${warmupScopeKey}`)
        : undefined;
      if (cachedProbe !== undefined) {
        gitProbeState.value = cachedProbe;
        if (cachedProbe.available) {
          const snapshot = getWarmup<GitHistorySnapshot>(`collab:snapshot:${warmupScopeKey}`);
          // If probe is cached but log data is not yet available (warmup still
          // in progress), fall through to a full refresh so we don't display
          // stale "not a repo" state due to isRepo's false default.
          if (!snapshot) {
            await refreshGitData();
            return;
          }
          const status = getWarmup<GitStatusResult>(`collab:status:${warmupScopeKey}`);
          const br = getWarmup<GitBranchesResult>(`collab:branches:${warmupScopeKey}`);
          const sm = getWarmup<GitSubmoduleInfo[]>(`collab:submodules:${warmupScopeKey}`);
          isRepo.value = snapshot.isRepo;
          commits.value = dedupeCommitsByHash(snapshot.commits);
          graphRefs.value = snapshot.refs;
          headHash.value = snapshot.head.hash;
          headState.value = snapshot.head;
          stashes.value = snapshot.stashes;
          snapshotWorkspace.value = snapshot.workspace.changeCount;
          hasMoreCommits.value = snapshot.hasMore;
          if (status) {
            applyGitStatusResult(status);
          }
          if (br) {
            localBranches.value = br.local;
            remoteBranches.value = br.remotes;
            for (const [name] of br.remotes) {
              expandedRemoteNames.value.add(name);
            }
          }
          if (sm) submodules.value = sm;
          loading.value = false;
          filesLoading.value = false;
        } else {
          resetGitData();
          loading.value = false;
          filesLoading.value = false;
        }
        refreshWhenVisible(180);
        return;
      }
      await refreshGitData();
    });

    onUnmounted(() => {
      clearScheduledGitRefresh();
      if (typeof window !== "undefined") {
        window.removeEventListener("focus", handleWindowFocus);
      }
      if (typeof document !== "undefined") {
        document.removeEventListener("visibilitychange", handleVisibilityChange);
      }
      document.removeEventListener("mousemove", onVSplitterMouseMove);
      document.removeEventListener("mouseup", onVSplitterMouseUp);
      document.removeEventListener("mousemove", onHSplitterMouseMove);
      document.removeEventListener("mouseup", onHSplitterMouseUp);
      stopSidebarResize();
      releaseSelectionLock?.();
      releaseSelectionLock = null;
    });
  }

  // ── Public API ──────────────────────────────────────────────────
  return {
    // core state
    isRepo,
    commits,
    graphRefs,
    headHash,
    headState,
    loading,
    selectedHistory,
    selectedCommitHash,
    hasMoreCommits,
    loadingMore,

    // init
    initLoading,
    initError,
    gitProbeState,
    gitAvailable,
    gitHelpText,

    // git config modal
    showGitConfigModal,
    gitConfigName,
    gitConfigEmail,
    gitConfigSaving,
    gitConfigError,
    currentGitAuthor,

    // file changes
    unstagedFiles,
    stagedFiles,
    blockedFiles,
    commitFiles,
    commitBody,
    filesLoading,
    pendingStagePaths,
    pendingUnstagePaths,
    stageOperationBusy,

    // merge / conflict
    unmergedFiles,
    mergeOperation,
    isMerging,
    hasUnresolvedFiles,

    // sidebar data
    localBranches,
    remoteBranches,
    stashes,
    tags,
    submodules,

    // sidebar toggles
    sidebarCollapsed,
    expandLocal,
    expandRemotes,
    expandedRemoteNames,
    expandStashes,
    expandTags,
    expandSubmodules,

    // layout / resize
    containerRef,
    leftAreaRef,
    leftColRef,
    gitSidebarWidth,
    leftColWidth,
    terminalHeight,
    draggingClass,

    // computed
    currentBranch,
    selectedCommit,
    totalChanges,
    workspaceChangeCount,
    hasWorkspaceChanges,

    // actions
    initGitUnity,
    saveGitConfigAndInit,
    cancelGitConfig,
    toggleRemote,
    stageFile,
    unstageFile,
    stageFiles,
    unstageFiles,
    stageAll,
    unstageAll,
    onCommitted,
    onTerminalDone,
    onTerminalTouched,
    loadMoreCommits,
    onRefresh,
    onSidebarSplitterMouseDown,
    onVSplitterMouseDown,
    onHSplitterMouseDown,
  };
}
