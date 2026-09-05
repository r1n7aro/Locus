import { ipcInvoke } from "./ipc";

export interface WorkspaceRef {
  checkoutId: string;
  expectedGeneration?: number | null;
}

export interface WorkspaceRuntimeDescriptor {
  projectId: string;
  checkoutId: string;
  root: string;
  workspaceGeneration: number;
  leaseCount: number;
  detectedServices: string[];
}

export interface ServiceBindingSnapshot {
  serviceKind: "unity";
  serviceInstanceId: string;
  runtimeGeneration: number;
}

export type WorkspaceServiceStatus =
  | "detected"
  | "dormant"
  | "starting"
  | "running"
  | "suspending"
  | "failed"
  | "stopping"
  | "stopped";

export type WorkspaceServiceReadinessPhase =
  | "starting"
  | "connected"
  | "ready"
  | "reloading"
  | "degraded"
  | "stopped";

export interface WorkspaceServiceStateSnapshot {
  serviceKind: "unity";
  activationPolicy: "disabled" | "manual" | "lazy" | "auto";
  identity?: {
    projectId: string;
    checkoutId: string;
    serviceInstanceId: string;
    runtimeGeneration: number;
  } | null;
  status: WorkspaceServiceStatus;
  readiness?: {
    phase: WorkspaceServiceReadinessPhase;
    revision: number;
    detail?: string | null;
  } | null;
  leaseCount: number;
}

export interface WorkspaceCheckoutDescriptor {
  checkoutId: string;
  projectId: string;
  root: string;
  normalizedRoot: string;
  lastOpenedAt: number;
  runtime?: WorkspaceRuntimeDescriptor | null;
}

export interface ProjectContextDescriptor {
  projectId: string;
  detectedServices: string[];
  checkouts: WorkspaceCheckoutDescriptor[];
}

export interface WindowPaneWorkspaceContext {
  windowId: string;
  paneId: string;
  focusedCheckoutId: string;
  workspaceGeneration: number;
  activeSessionId?: string | null;
  intentEpoch: number;
  revision: number;
}

export interface WindowIntentEpochSnapshot {
  windowId: string;
  paneId?: string | null;
  intentEpoch: number;
}

export const WORKSPACE_EVENT_NAME = "locus://workspace-event";

export interface RoutedWorkspaceEvent<T = unknown> {
  eventName: string;
  streamRevision: number;
  projectId: string;
  checkoutId: string;
  workspaceGeneration: number;
  serviceInstanceId?: string | null;
  serviceGeneration?: number | null;
  payload: T;
}

export interface DirEntry {
  relPath: string;
  name: string;
  isDir: boolean;
}

export interface DirEntriesPage {
  entries: DirEntry[];
  totalCount: number;
  nextOffset: number;
  hasMore: boolean;
}

export interface WorkspaceSearchEntry {
  relPath: string;
  name: string;
  parentPath: string;
  isDir: boolean;
  matchScore: number;
}

export type WorkspaceEntryKind = "file" | "folder" | "other" | "missing";

export interface WorkspaceEntryStat {
  path: string;
  exists: boolean;
  entryKind: WorkspaceEntryKind;
}

export function listProjectContexts(): Promise<ProjectContextDescriptor[]> {
  return ipcInvoke<ProjectContextDescriptor[]>("list_project_contexts");
}

export function listWorkspaceRuntimes(): Promise<WorkspaceRuntimeDescriptor[]> {
  return ipcInvoke<WorkspaceRuntimeDescriptor[]>("list_workspace_runtimes");
}

export function openWorkspace(path: string): Promise<WorkspaceRuntimeDescriptor> {
  return ipcInvoke<WorkspaceRuntimeDescriptor>("open_workspace", { path });
}

export function removeWorkspace(projectId: string): Promise<boolean> {
  return ipcInvoke<boolean>("remove_workspace", { projectId });
}

export function startWorkspaceUnityService(
  workspaceRef: WorkspaceRef,
): Promise<ServiceBindingSnapshot> {
  return ipcInvoke<ServiceBindingSnapshot>("start_workspace_unity_service", { workspaceRef });
}

export function getWorkspaceServiceStates(
  workspaceRef: WorkspaceRef,
): Promise<WorkspaceServiceStateSnapshot[]> {
  return ipcInvoke<WorkspaceServiceStateSnapshot[]>("get_workspace_service_states", {
    workspaceRef,
  });
}

export function focusWorkspace(
  windowId: string,
  paneId: string,
  workspaceRef: WorkspaceRef,
  intentEpoch: number,
): Promise<WindowPaneWorkspaceContext> {
  return ipcInvoke<WindowPaneWorkspaceContext>("focus_workspace", {
    windowId,
    paneId,
    workspaceRef,
    intentEpoch,
  });
}

export function setActiveWorkspaceSession(
  windowId: string,
  paneId: string,
  activeSessionId: string | null,
  intentEpoch: number,
): Promise<WindowPaneWorkspaceContext> {
  return ipcInvoke<WindowPaneWorkspaceContext>("set_active_session", {
    windowId,
    paneId,
    activeSessionId,
    intentEpoch,
  });
}

export function detachWorkspacePane(
  windowId: string,
  paneId: string,
  intentEpoch: number,
): Promise<boolean> {
  return ipcInvoke<boolean>("detach_workspace_pane", { windowId, paneId, intentEpoch });
}

export function detachWorkspaceWindow(windowId: string, intentEpoch: number): Promise<number> {
  return ipcInvoke<number>("detach_workspace_window", { windowId, intentEpoch });
}

export function listWindowWorkspaceContexts(): Promise<WindowPaneWorkspaceContext[]> {
  return ipcInvoke<WindowPaneWorkspaceContext[]>("list_window_workspace_contexts");
}

export function listWindowWorkspaceIntentEpochs(): Promise<WindowIntentEpochSnapshot[]> {
  return ipcInvoke<WindowIntentEpochSnapshot[]>("list_window_workspace_intent_epochs");
}

export function listRecentDirs(): Promise<string[]> {
  return ipcInvoke<string[]>("list_recent_dirs");
}

export function removeRecentDir(path: string): Promise<string[]> {
  return ipcInvoke<string[]>("remove_recent_dir", { path });
}

export function openDirInFileExplorer(path: string): Promise<void> {
  return ipcInvoke<void>("open_dir_in_file_explorer", { path });
}

export function listDirEntries(
  subPath: string,
  workspaceRef: WorkspaceRef,
): Promise<DirEntry[]> {
  return ipcInvoke<DirEntry[]>("list_dir_entries", { subPath, workspaceRef });
}

export function listDirEntriesPage(
  subPath: string,
  workspaceRef: WorkspaceRef,
  offset = 0,
  limit = 200,
  excludeMeta = false,
  hiddenDirs?: string[],
): Promise<DirEntriesPage> {
  return ipcInvoke<DirEntriesPage>("list_dir_entries_page", {
    subPath,
    offset,
    limit,
    excludeMeta,
    hiddenDirs,
    workspaceRef,
  });
}

export function searchWorkspaceEntries(
  query: string,
  workspaceRef: WorkspaceRef,
  limit = 200,
  hiddenDirs?: string[],
): Promise<WorkspaceSearchEntry[]> {
  return ipcInvoke<WorkspaceSearchEntry[]>("search_workspace_entries", {
    query,
    limit,
    hiddenDirs,
    workspaceRef,
  });
}

export function statWorkspaceEntries(
  paths: string[],
  workspaceRef: WorkspaceRef,
): Promise<WorkspaceEntryStat[]> {
  return ipcInvoke<WorkspaceEntryStat[]>("stat_workspace_entries", { paths, workspaceRef });
}

export function resetAllConfig(): Promise<void> {
  return ipcInvoke("reset_all_config");
}
