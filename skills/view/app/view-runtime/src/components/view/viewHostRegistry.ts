import type { WorkspaceRef } from "../../services/project";

export interface NativeViewHost {
  instanceId: string;
  windowLabel?: string;
  viewId: string;
  workspaceRef: WorkspaceRef;
  active(): boolean;
  root(): HTMLElement | null;
  ready(): Promise<void>;
  reload(): Promise<void>;
  activate(): Promise<void>;
  execute(kind: string, payload: Record<string, unknown>): Promise<unknown>;
}
const hosts = new Map<string, NativeViewHost>();
export function registerNativeViewHost(host: NativeViewHost): () => void {
  const key = JSON.stringify([host.windowLabel ?? "main", host.instanceId]);
  hosts.set(key, host);
  return () => { if (hosts.get(key) === host) hosts.delete(key); };
}
export function nativeViewHosts(): NativeViewHost[] { return [...hosts.values()]; }
