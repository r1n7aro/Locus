import { ipcInvoke } from "./ipc";
import { getLocusRuntime } from "./locusRuntime";
import type { WorkspaceRef } from "./project";

export type BlenderConnection = "connecting" | "connected" | "disconnected" | "host_error";
export type BlenderSupportStatus = "supported" | "unverified" | "unsupported" | "unknown";
export type BlenderAddonStatus = "missing" | "outdated" | "up_to_date" | "unknown";
export type BlenderInstallationSource =
  | "manual"
  | "running"
  | "program_files"
  | "steam"
  | "path"
  | "file_association";

export interface BlenderProbe {
  version: number[];
  versionString: string;
  versionCycle?: string | null;
  buildHash: string;
  buildBranch: string;
  python: string;
  userScripts: string;
  portable: boolean;
  binary: string;
}

export interface BlenderAddonState {
  status: BlenderAddonStatus;
  path: string;
  installedHash?: string;
}

export interface BlenderInstallation {
  id: string;
  path: string;
  sources: BlenderInstallationSource[];
  hidden: boolean;
  isDefault: boolean;
  probeState: "ready" | "pending" | "failed";
  probe?: BlenderProbe;
  probeError?: string;
  support: BlenderSupportStatus;
  displayVersion?: string;
  addon?: BlenderAddonState;
}

export interface BlenderInstance {
  key: string;
  pid: number;
  mode: "interactive" | "background" | string;
  version: string;
  versionString: string;
  buildBranch: string;
  binary: string;
  python: string;
  file: string | null;
  dirty: boolean;
  savedVersion: string | null;
  hostHash: string;
  connection: BlenderConnection;
  error?: string;
  launchedByCheckout: string | null;
  busy: boolean;
  mainThreadStallMs?: number;
  pendingRequests?: number;
  capabilities: string[];
  inWorkspace: boolean;
  isTarget: boolean;
  /** Another session targets this instance. */
  otherSessionTarget?: boolean;
  ownedBackground: boolean;
  sharedFile: boolean;
}

export interface BlenderBackgroundInstance {
  pid: number;
  checkout: string;
  installation: string;
  version: string;
  file: string | null;
  log: string | null;
  uptimeSecs: number;
}

export interface BlenderWorkspaceVersion {
  pinned: string | null;
  inferred: string | null;
  effective: string | null;
  distribution: {
    versions: [string, number][];
    files: number;
    unreadable: number;
    truncated: boolean;
  };
}

export interface BlenderWorkspaceStatus {
  enabled: boolean;
  running: boolean;
  instances?: BlenderInstance[];
  target?: string | null;
  background?: BlenderBackgroundInstance[];
  limits?: { maxBackground: number; idleTimeoutSecs: number };
  workspaceVersion?: BlenderWorkspaceVersion;
  withoutHost?: { pid: number; path: string | null }[];
  addon?: {
    installation: BlenderInstallation | null;
    status: BlenderAddonStatus | null;
    outdatedInstances: number[];
  };
}

export interface BlenderInstallReport {
  installation: string;
  version: string | null;
  ok: boolean;
  error?: string;
  reloaded: number[];
  restartNeeded: number[];
}

export function listBlenderInstallations(): Promise<BlenderInstallation[]> {
  return ipcInvoke("blender_list_installations");
}

export function refreshBlenderInstallations(): Promise<BlenderInstallation[]> {
  return ipcInvoke("blender_refresh_installations");
}

export function addBlenderInstallation(path: string): Promise<BlenderInstallation> {
  return ipcInvoke("blender_add_installation", { path });
}

export function removeBlenderInstallation(id: string): Promise<void> {
  return ipcInvoke("blender_remove_installation", { id });
}

export function unhideBlenderInstallation(id: string): Promise<void> {
  return ipcInvoke("blender_unhide_installation", { id });
}

export function setDefaultBlenderInstallation(id: string | null): Promise<void> {
  return ipcInvoke("blender_set_default_installation", { id });
}

export function installBlenderAddon(installation: string | null, all = false): Promise<BlenderInstallReport[]> {
  return ipcInvoke("blender_install_addon", { installation, all });
}

export function uninstallBlenderAddon(installation: string): Promise<void> {
  return ipcInvoke("blender_uninstall_addon", { installation });
}

export function reloadBlenderHosts(): Promise<{ reloaded: number[]; restartNeeded: number[] }> {
  return ipcInvoke("blender_reload_hosts");
}

export function getBlenderWorkspaceStatus(
  workspaceRef: WorkspaceRef,
  sessionId: string | null,
  detail = false,
): Promise<BlenderWorkspaceStatus> {
  return ipcInvoke("blender_workspace_status", { workspaceRef, sessionId, detail });
}

export function setBlenderSessionTarget(
  workspaceRef: WorkspaceRef,
  sessionId: string,
  targetSpec: string | null,
): Promise<string | null> {
  return ipcInvoke("blender_set_session_target", { workspaceRef, sessionId, targetSpec });
}

export function launchBlender(
  workspaceRef: WorkspaceRef,
  sessionId: string | null,
  file?: string | null,
  installation?: string | null,
): Promise<BlenderInstance> {
  return ipcInvoke("blender_launch", { workspaceRef, sessionId, file: file ?? null, installation: installation ?? null });
}

export function closeBlenderBackground(pid: number): Promise<void> {
  return ipcInvoke("blender_close_background", { pid });
}

export function setBlenderWorkspaceVersion(workspaceRef: WorkspaceRef, version: string | null): Promise<void> {
  return ipcInvoke("blender_set_workspace_version", { workspaceRef, version });
}

/** Instances appeared, disappeared, connected or changed file/target state. */
export function subscribeBlenderInstancesChanged(handler: () => void) {
  return getLocusRuntime().subscribe<unknown>("blender-instances-changed", () => handler());
}

/** Probe results, add-on installs and table edits. */
export function subscribeBlenderInstallationsChanged(handler: () => void) {
  return getLocusRuntime().subscribe<unknown>("blender-installations-changed", () => handler());
}

export function listBlenderBackgroundInstances(): Promise<BlenderBackgroundInstance[]> {
  return ipcInvoke("blender_background_instances");
}
