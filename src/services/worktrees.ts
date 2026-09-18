import { ipcInvoke } from "./ipc";
import { beginWorkspaceGitHeadObservation, rememberWorkspaceGitHead } from "./workspaceGitHead";
import { Channel } from "@tauri-apps/api/core";
import type { WorkspaceRef } from "./project";

export interface WorktreeBranchOption {
  branch: string | null;
  remote?: boolean;
  root: string | null;
  current: boolean;
  dirty: boolean;
  headOid: string;
  unavailable: boolean;
}

export interface SelectWorktreeRequest {
  workspaceRef: WorkspaceRef;
  branch: string;
  createBranch: boolean;
  includeDirty: boolean;
  allowNewProject?: boolean;
  expectedStartOid?: string;
  startRef?: string;
}

export interface WorktreeDiskBudget {
  checkoutBytes: number;
  referenceCacheBytes: number;
  cacheKnown: boolean;
  estimatedBytes: number;
  freeBytes: number | null;
  directory: string;
}
export interface WorktreeCreationPlan {
  startOid: string;
  requiresNewProject: boolean;
  atCapacity: boolean;
  budget: WorktreeDiskBudget | null;
}
export interface WorktreePoolUsage {
  items: Array<{ worktree: ManagedWorktree; sizeBytes: number }>;
  totalBytes: number;
  availableProjects: number;
}
export interface WorktreeProjectUsage {
  projectId: string;
  sourceRoot: string;
  usage: WorktreePoolUsage | null;
  error: string | null;
}
export const getAllWorktreePoolUsage = () => ipcInvoke<WorktreeProjectUsage[]>("get_all_worktree_pool_usage");
export interface PlanWorktreeCreationRequest {
  sourceRoot: string;
  startRef: string;
  includeDirty: boolean;
  poolMode: boolean;
  directory: string;
  maxSlots: number;
}
export interface WorktreePlanProgress {
  phase: "checking" | "checkout" | "untracked" | "cache" | "space" | "creating";
  files: number;
  totalFiles: number | null;
  bytes: number;
}
export type WorktreeProgressHandler = (progress: WorktreePlanProgress) => void;
function progressArgs(onProgress?: WorktreeProgressHandler) {
  return onProgress ? { onProgress: new Channel<WorktreePlanProgress>(onProgress) } : {};
}
export const planWorktreeSelection = (request: SelectWorktreeRequest, onProgress?: WorktreeProgressHandler) => ipcInvoke<WorktreeCreationPlan>("plan_worktree_selection", { request, ...progressArgs(onProgress) });
export const planWorktreeCreation = (request: PlanWorktreeCreationRequest, onProgress?: WorktreeProgressHandler) => ipcInvoke<WorktreeCreationPlan>("plan_worktree_creation", { request, ...progressArgs(onProgress) });
export const getWorktreePoolUsage = (sourceRoot: string) => ipcInvoke<WorktreePoolUsage>("get_worktree_pool_usage", { sourceRoot });

export async function listWorktreeBranches(workspaceRef: WorkspaceRef): Promise<WorktreeBranchOption[]> {
  const observation = beginWorkspaceGitHeadObservation();
  const branches = await ipcInvoke<WorktreeBranchOption[]>("list_worktree_branches", { workspaceRef });
  const current = branches.find((branch) => branch.current);
  if (current) rememberWorkspaceGitHead(workspaceRef, {
    kind: current.branch ? "attached" : "detached",
    refName: current.branch,
    hash: current.headOid || null,
  }, observation);
  return branches;
}
export const selectWorktreeBranch = (request: SelectWorktreeRequest, onProgress?: WorktreeProgressHandler) =>
  ipcInvoke<ManagedWorktree>("select_worktree_branch", { request, ...progressArgs(onProgress) });

export interface ManagedWorktree {
  checkoutId: string; projectId: string; root: string; repoRoot: string;
  projectRelativePath: string; branch: string | null; headOid: string;
  materializationEpoch: number; managed: boolean; lifecycle: string; dirty: boolean;
  poolSlot: boolean; assignmentId: string | null; editorVersion: string | null;
  lastError: string | null;
}

export interface CreateWorktreeRequest {
  sourceRoot: string; destination: string; branch: string;
  startRef?: string | null; includeDirty: boolean;
}

export const createWorktree = (request: CreateWorktreeRequest) => ipcInvoke<ManagedWorktree>("create_worktree", { request });
export const listManagedWorktrees = (sourceRoot: string) => ipcInvoke<ManagedWorktree[]>("list_managed_worktrees", { sourceRoot });
export const discoverWorktrees = (sourceRoot: string) => ipcInvoke<string[]>("discover_worktrees", { sourceRoot });
export const importWorktree = (sourceRoot: string, targetRoot: string) => ipcInvoke<ManagedWorktree>("import_worktree", { sourceRoot, targetRoot });
export const removeManagedWorktree = (sourceRoot: string, item: ManagedWorktree) => ipcInvoke<void>("remove_managed_worktree", {
  sourceRoot, checkoutId: item.checkoutId, expectedEpoch: item.materializationEpoch,
});

export interface WorkspaceResourceLimits {
  maxRunningSessions: number; maxUnityEditors: number;
  maxRunningWorkspaceServices: number; maxWatchedWorkspaces: number; maxLspProcesses: number;
  maxConcurrentServiceStarts: number; maxConcurrentCompileJobs: number; maxCompileQueueDepth: number;
  workspaceIdleTimeoutSecs: number; serviceIdleTimeoutSecs: number; lspIdleTimeoutSecs: number;
}
export interface ResourcePolicySnapshot { revision: number; limits: WorkspaceResourceLimits }
export const getWorkspaceResourceLimits = () => ipcInvoke<ResourcePolicySnapshot>("get_workspace_service_resource_limits");
export const setWorkspaceResourceLimits = (limits: WorkspaceResourceLimits) => ipcInvoke<ResourcePolicySnapshot>("set_workspace_service_resource_limits", { limits });

export interface UnityEditorResource {
  projectPath: string;
  processId: number;
  mode: "interactive" | "headless";
  managed: boolean;
  workingSetBytes: number | null;
  importWorkerCount: number;
  lastError: string | null;
}
export const getUnityEditorResources = () => ipcInvoke<UnityEditorResource[]>("get_unity_editor_resources");

export interface AcquirePoolRequest {
  sourceRoot: string; poolRoot: string; commit: string; branch?: string | null;
  maxSlots: number; assignmentId?: string | null;
}
export const acquireUnityProjectSlot = (request: AcquirePoolRequest, allowNewProject = false) => ipcInvoke<{
  worktree: ManagedWorktree; reused: boolean; preservedLibrary: boolean;
}>("acquire_unity_project_slot", { request, allowNewProject });
export const releaseUnityProjectSlot = (sourceRoot: string, item: ManagedWorktree) => ipcInvoke<ManagedWorktree>("release_unity_project_slot", {
  sourceRoot, checkoutId: item.checkoutId, expectedEpoch: item.materializationEpoch, assignmentId: item.assignmentId,
});
