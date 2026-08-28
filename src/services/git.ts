import { ipcInvoke } from "./ipc";
import type { WorkspaceRef } from "./project";
import type {
  GitActionResult,
  GitStageAllResult,
  GitBranchesResult,
  GitConfigEntry,
  GitConfigScope,
  GitConfigScopeSnapshot,
  GitConfigSnapshot,
  GitFileChange,
  GitHistorySearchRequest,
  GitHistorySearchResponse,
  GitHistorySnapshot,
  GitInstallHelp,
  GitLogResult,
  GitProbeResult,
  GitRuntimeState,
  GitStashEntry,
  GitStatusResult,
  GitSubmoduleInfo,
  MergeFileInfo,
  MergeApplyMode,
  MergeActionKind,
} from "../types";

let gitRuntimeStateCache: GitRuntimeState | null = null;
let gitRuntimeStateRequest: Promise<GitRuntimeState> | null = null;

export function gitLog(skip: number, limit: number, workspaceRef: WorkspaceRef): Promise<GitLogResult> {
  return ipcInvoke<GitLogResult>("git_log", { skip, limit, workspaceRef });
}

export function gitHistorySnapshot(skip: number, limit: number, workspaceRef: WorkspaceRef): Promise<GitHistorySnapshot> {
  return ipcInvoke<GitHistorySnapshot>("git_history_snapshot", { skip, limit, workspaceRef });
}

export function gitHistorySearch(request: GitHistorySearchRequest, workspaceRef: WorkspaceRef): Promise<GitHistorySearchResponse> {
  return ipcInvoke<GitHistorySearchResponse>("git_history_search", { request, workspaceRef });
}

export function gitCommitBody(hash: string, workspaceRef: WorkspaceRef): Promise<string> {
  return ipcInvoke<string>("git_commit_body", { hash, workspaceRef });
}

export function gitProbe(workspaceRef: WorkspaceRef): Promise<GitProbeResult> {
  return ipcInvoke<GitProbeResult>("git_probe", { workspaceRef });
}

export function gitRuntimeState(refresh = false): Promise<GitRuntimeState> {
  if (!refresh && gitRuntimeStateCache) {
    return Promise.resolve(gitRuntimeStateCache);
  }
  if (!refresh && gitRuntimeStateRequest) {
    return gitRuntimeStateRequest;
  }

  const request = ipcInvoke<GitRuntimeState>("git_runtime_state", { refresh })
    .then((state) => {
      gitRuntimeStateCache = state;
      return state;
    })
    .finally(() => {
      if (gitRuntimeStateRequest === request) {
        gitRuntimeStateRequest = null;
      }
    });

  gitRuntimeStateRequest = request;
  return request;
}

export function gitSaveRuntimeSelection(selectedId: string): Promise<GitRuntimeState> {
  return ipcInvoke<GitRuntimeState>("git_save_runtime_selection", { selectedId })
    .then((state) => {
      gitRuntimeStateCache = state;
      return state;
    });
}

export function gitHeadHash(workspaceRef: WorkspaceRef): Promise<string | null> {
  return ipcInvoke<string | null>("git_head_hash", { workspaceRef });
}

export function gitInstallHelp(): Promise<GitInstallHelp> {
  return ipcInvoke<GitInstallHelp>("git_install_help");
}

export function gitInstallVia(manager: string): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return ipcInvoke<{ stdout: string; stderr: string; exitCode: number }>("git_install_via", { manager });
}

export function gitSetOverride(path: string): Promise<string> {
  return ipcInvoke<string>("git_set_override", { path });
}

export function gitClearOverride(): Promise<void> {
  return ipcInvoke("git_clear_override");
}

export function gitStatus(workspaceRef: WorkspaceRef): Promise<GitStatusResult> {
  return ipcInvoke<GitStatusResult>("git_status", { workspaceRef });
}

export function gitBranches(workspaceRef: WorkspaceRef): Promise<GitBranchesResult> {
  return ipcInvoke<GitBranchesResult>("git_branches", { workspaceRef });
}

export function gitStashes(workspaceRef: WorkspaceRef): Promise<GitStashEntry[]> {
  return ipcInvoke<GitStashEntry[]>("git_stashes", { workspaceRef });
}

export function gitSubmodules(workspaceRef: WorkspaceRef): Promise<GitSubmoduleInfo[]> {
  return ipcInvoke<GitSubmoduleInfo[]>("git_submodules", { workspaceRef });
}

export function gitStage(path: string, workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_stage", { path, workspaceRef });
}

export function gitStagePaths(paths: string[], workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_stage_paths", { paths, workspaceRef });
}

export function gitStageAll(workspaceRef: WorkspaceRef): Promise<GitStageAllResult> {
  return ipcInvoke<GitStageAllResult>("git_stage_all", { workspaceRef });
}

export function gitUnstage(path: string, workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_unstage", { path, workspaceRef });
}

export function gitUnstagePaths(paths: string[], workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_unstage_paths", { paths, workspaceRef });
}

export function gitUnstageAll(workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_unstage_all", { workspaceRef });
}

export function gitDiscardFile(path: string, status: string, oldPath: string | undefined, workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_discard_file", { path, status, oldPath, workspaceRef });
}

export function gitCommit(message: string, description: string | null | undefined, workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_commit", { message, description, workspaceRef });
}

export function gitCommitFiles(hash: string, workspaceRef: WorkspaceRef): Promise<GitFileChange[]> {
  return ipcInvoke<GitFileChange[]>("git_commit_files", { hash, workspaceRef });
}

export function gitCompareFiles(fromHash: string, toHash: string, workspaceRef: WorkspaceRef): Promise<GitFileChange[]> {
  return ipcInvoke<GitFileChange[]>("git_compare_files", { fromHash, toHash, workspaceRef });
}

export function gitGenerateCommitMessage(model: string | null, workspaceRef: WorkspaceRef): Promise<{ title: string; description: string }> {
  return ipcInvoke<{ title: string; description: string }>("git_generate_commit_message", { model, workspaceRef });
}

export function gitCheckUserConfig(workspaceRef: WorkspaceRef): Promise<{ name: string; email: string }> {
  return ipcInvoke<{ name: string; email: string }>("git_check_user_config", { workspaceRef });
}

export function gitSetUserConfig(name: string, email: string): Promise<void> {
  return ipcInvoke("git_set_user_config", { name, email });
}

export function gitConfigSnapshot(workspaceRef: WorkspaceRef): Promise<GitConfigSnapshot> {
  return ipcInvoke<GitConfigSnapshot>("git_config_snapshot", { workspaceRef });
}

export function gitSaveConfig(scope: GitConfigScope, entries: GitConfigEntry[], workspaceRef: WorkspaceRef): Promise<GitConfigScopeSnapshot> {
  return ipcInvoke<GitConfigScopeSnapshot>("git_save_config", { scope, entries, workspaceRef });
}

export function gitInitUnity(workspaceRef: WorkspaceRef): Promise<string> {
  return ipcInvoke<string>("git_init_unity", { workspaceRef });
}

export function gitExecute(command: string, workspaceRef: WorkspaceRef): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return ipcInvoke<{ stdout: string; stderr: string; exitCode: number }>("run_command", { command, workspaceRef });
}

// ── Merge commands ──

export function gitMergeFile(
  path: string,
  conflictCode: string,
  baseOid: string,
  leftOid: string,
  rightOid: string,
  isLfs: boolean,
  workspaceRef: WorkspaceRef,
): Promise<MergeFileInfo> {
  return ipcInvoke<MergeFileInfo>("git_merge_file", {
    path, conflictCode, baseOid, leftOid, rightOid, isLfs, workspaceRef,
  });
}

export function gitMergeApply(path: string, mode: MergeApplyMode, workspaceRef: WorkspaceRef): Promise<void> {
  return ipcInvoke("git_merge_apply", { path, mode, workspaceRef });
}

export function gitMergeAction(action: MergeActionKind, operationKind: string, workspaceRef: WorkspaceRef): Promise<string> {
  return ipcInvoke<string>("git_merge_action", { action, operationKind, workspaceRef });
}

// ── Context-menu actions ──

export function gitCommitAction(
  rev: string,
  action: string,
  mode: string | undefined,
  branchName: string | undefined,
  workspaceRef: WorkspaceRef,
): Promise<GitActionResult> {
  return ipcInvoke<GitActionResult>("git_commit_action", { rev, action, mode, branchName, workspaceRef });
}

export function gitBranchAction(
  target: string,
  targetKind: string,
  action: string,
  newName: string | undefined,
  workspaceRef: WorkspaceRef,
): Promise<GitActionResult> {
  return ipcInvoke<GitActionResult>("git_branch_action", { target, targetKind, action, newName, workspaceRef });
}

export function gitStashAction(
    refName: string,
    action: string,
    workspaceRef: WorkspaceRef,
): Promise<GitActionResult> {
  return ipcInvoke<GitActionResult>("git_stash_action", { refName, action, workspaceRef });
}
