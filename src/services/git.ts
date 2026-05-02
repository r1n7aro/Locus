import { ipcInvoke } from "./ipc";
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
  GitStashEntry,
  GitStatusResult,
  GitSubmoduleInfo,
  MergeFileInfo,
  MergeApplyMode,
  MergeActionKind,
} from "../types";

export function gitLog(skip: number, limit: number): Promise<GitLogResult> {
  return ipcInvoke<GitLogResult>("git_log", { skip, limit });
}

export function gitHistorySnapshot(skip: number, limit: number): Promise<GitHistorySnapshot> {
  return ipcInvoke<GitHistorySnapshot>("git_history_snapshot", { skip, limit });
}

export function gitHistorySearch(request: GitHistorySearchRequest): Promise<GitHistorySearchResponse> {
  return ipcInvoke<GitHistorySearchResponse>("git_history_search", { request });
}

export function gitCommitBody(hash: string): Promise<string> {
  return ipcInvoke<string>("git_commit_body", { hash });
}

export function gitProbe(): Promise<GitProbeResult> {
  return ipcInvoke<GitProbeResult>("git_probe");
}

export function gitHeadHash(): Promise<string | null> {
  return ipcInvoke<string | null>("git_head_hash");
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

export function gitStatus(): Promise<GitStatusResult> {
  return ipcInvoke<GitStatusResult>("git_status");
}

export function gitBranches(): Promise<GitBranchesResult> {
  return ipcInvoke<GitBranchesResult>("git_branches");
}

export function gitStashes(): Promise<GitStashEntry[]> {
  return ipcInvoke<GitStashEntry[]>("git_stashes");
}

export function gitSubmodules(): Promise<GitSubmoduleInfo[]> {
  return ipcInvoke<GitSubmoduleInfo[]>("git_submodules");
}

export function gitStage(path: string): Promise<void> {
  return ipcInvoke("git_stage", { path });
}

export function gitStagePaths(paths: string[]): Promise<void> {
  return ipcInvoke("git_stage_paths", { paths });
}

export function gitStageAll(): Promise<GitStageAllResult> {
  return ipcInvoke<GitStageAllResult>("git_stage_all");
}

export function gitUnstage(path: string): Promise<void> {
  return ipcInvoke("git_unstage", { path });
}

export function gitUnstagePaths(paths: string[]): Promise<void> {
  return ipcInvoke("git_unstage_paths", { paths });
}

export function gitUnstageAll(): Promise<void> {
  return ipcInvoke("git_unstage_all");
}

export function gitDiscardFile(path: string, status: string, oldPath?: string): Promise<void> {
  return ipcInvoke("git_discard_file", { path, status, oldPath });
}

export function gitCommit(message: string, description?: string | null): Promise<void> {
  return ipcInvoke("git_commit", { message, description });
}

export function gitCommitFiles(hash: string): Promise<GitFileChange[]> {
  return ipcInvoke<GitFileChange[]>("git_commit_files", { hash });
}

export function gitCompareFiles(fromHash: string, toHash: string): Promise<GitFileChange[]> {
  return ipcInvoke<GitFileChange[]>("git_compare_files", { fromHash, toHash });
}

export function gitGenerateCommitMessage(model: string | null): Promise<{ title: string; description: string }> {
  return ipcInvoke<{ title: string; description: string }>("git_generate_commit_message", { model });
}

export function gitCheckUserConfig(): Promise<{ name: string; email: string }> {
  return ipcInvoke<{ name: string; email: string }>("git_check_user_config");
}

export function gitSetUserConfig(name: string, email: string): Promise<void> {
  return ipcInvoke("git_set_user_config", { name, email });
}

export function gitConfigSnapshot(): Promise<GitConfigSnapshot> {
  return ipcInvoke<GitConfigSnapshot>("git_config_snapshot");
}

export function gitSaveConfig(scope: GitConfigScope, entries: GitConfigEntry[]): Promise<GitConfigScopeSnapshot> {
  return ipcInvoke<GitConfigScopeSnapshot>("git_save_config", { scope, entries });
}

export function gitInitUnity(): Promise<string> {
  return ipcInvoke<string>("git_init_unity");
}

export function gitExecute(command: string): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return ipcInvoke<{ stdout: string; stderr: string; exitCode: number }>("run_command", { command });
}

// ── Merge commands ──

export function gitMergeFile(
  path: string,
  conflictCode: string,
  baseOid: string,
  leftOid: string,
  rightOid: string,
  isLfs: boolean,
): Promise<MergeFileInfo> {
  return ipcInvoke<MergeFileInfo>("git_merge_file", {
    path, conflictCode, baseOid, leftOid, rightOid, isLfs,
  });
}

export function gitMergeApply(path: string, mode: MergeApplyMode): Promise<void> {
  return ipcInvoke("git_merge_apply", { path, mode });
}

export function gitMergeAction(action: MergeActionKind, operationKind: string): Promise<string> {
  return ipcInvoke<string>("git_merge_action", { action, operationKind });
}

// ── Context-menu actions ──

export function gitCommitAction(
  rev: string,
  action: string,
  mode?: string,
  branchName?: string,
): Promise<GitActionResult> {
  return ipcInvoke<GitActionResult>("git_commit_action", { rev, action, mode, branchName });
}

export function gitBranchAction(
  target: string,
  targetKind: string,
  action: string,
  newName?: string,
): Promise<GitActionResult> {
  return ipcInvoke<GitActionResult>("git_branch_action", { target, targetKind, action, newName });
}

export function gitStashAction(
  refName: string,
  action: string,
): Promise<GitActionResult> {
  return ipcInvoke<GitActionResult>("git_stash_action", { refName, action });
}
