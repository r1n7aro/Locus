import { ipcInvoke } from "./ipc";
import { getLocusRuntime } from "./locusRuntime";

/** Workspace services with an app-level switch (Settings > Services). */
export type ServiceKindId = "unity" | "blender";

/** App-level switches. Off: the service is never detected or started. */
export interface ServiceSwitches {
  unity: boolean;
  blender: boolean;
}

export function getServiceSwitches(): Promise<ServiceSwitches> {
  return ipcInvoke<ServiceSwitches>("get_service_switches");
}

export function setServiceEnabled(service: ServiceKindId, enabled: boolean): Promise<ServiceSwitches> {
  return ipcInvoke<ServiceSwitches>("set_service_enabled", { service, enabled });
}

/** Broadcast after a switch changes; every window refreshes its runtimes. */
export function subscribeWorkspaceServicesChanged(handler: (switches: ServiceSwitches) => void) {
  return getLocusRuntime().subscribe<ServiceSwitches>("workspace-services-changed", handler);
}

export function startWorkspaceService(
  workspaceRef: import("./project").WorkspaceRef,
  service: ServiceKindId,
): Promise<import("./project").ServiceBindingSnapshot> {
  return ipcInvoke("start_workspace_service", { workspaceRef, service });
}
