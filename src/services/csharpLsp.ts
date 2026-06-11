import type { CsharpLspStatus } from "../types";
import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";

export function csharpLspGetStatus(): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>("csharp_lsp_get_status", undefined, {
    operation: "csharpLspGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function csharpLspSetEnabled(value: boolean): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>(
    "csharp_lsp_set_enabled",
    { value },
    { operation: "csharpLspSetEnabled", notify: false, throwOnError: true },
  );
}

export function csharpLspRestart(): Promise<CsharpLspStatus> {
  return ipcInvoke<CsharpLspStatus>("csharp_lsp_restart", undefined, {
    operation: "csharpLspRestart",
    notify: false,
    throwOnError: true,
  });
}

export function subscribeCsharpLspStatus(
  handler: (payload: CsharpLspStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<CsharpLspStatus>("csharp-lsp-status", handler);
}
