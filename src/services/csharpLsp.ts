import type { CodeAnalysisToolsConfig, CsharpCompileStatus, CsharpLspStatus } from "../types";
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

export function codeAnalysisToolsGetConfig(): Promise<CodeAnalysisToolsConfig> {
  return ipcInvoke<CodeAnalysisToolsConfig>("code_analysis_tools_get_config", undefined, {
    operation: "codeAnalysisToolsGetConfig",
    notify: false,
    throwOnError: true,
  });
}

export function codeAnalysisToolsSetConfig(
  value: CodeAnalysisToolsConfig,
): Promise<CodeAnalysisToolsConfig> {
  return ipcInvoke<CodeAnalysisToolsConfig>(
    "code_analysis_tools_set_config",
    { value },
    { operation: "codeAnalysisToolsSetConfig", notify: false, throwOnError: true },
  );
}

export function unitySidecarCompilerGetStatus(): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>("unity_sidecar_compiler_get_status", undefined, {
    operation: "unitySidecarCompilerGetStatus",
    notify: false,
    throwOnError: true,
  });
}

export function unitySidecarCompilerSetEnabled(value: boolean): Promise<CsharpCompileStatus> {
  return ipcInvoke<CsharpCompileStatus>(
    "unity_sidecar_compiler_set_enabled",
    { value },
    { operation: "unitySidecarCompilerSetEnabled", notify: false, throwOnError: true },
  );
}

export function subscribeCsharpLspStatus(
  handler: (payload: CsharpLspStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<CsharpLspStatus>("csharp-lsp-status", handler);
}

export function subscribeUnitySidecarCompilerStatus(
  handler: (payload: CsharpCompileStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<CsharpCompileStatus>("csharp-compile-status", handler);
}
