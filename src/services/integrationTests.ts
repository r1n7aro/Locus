import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";

export type UnityIntegrationSuite =
  | "connect"
  | "sidecar"
  | "type-index"
  | "state-probe"
  | "native-bridge"
  | "hot-reload"
  | "execute";

export type TypeIndexSampleMode = "sample32" | "all";

export interface UnityIntegrationTestRunRequest {
  projectPath?: string | null;
  suites: UnityIntegrationSuite[];
  openUnity?: boolean;
  installPlugin?: boolean;
  forceEditMode?: boolean;
  typeIndexSampleMode?: TypeIndexSampleMode;
  connectTimeoutMs?: number;
  suiteTimeoutMs?: number;
  pollMs?: number;
  noProgressTimeoutMs?: number;
}

export interface UnityIntegrationTestRunStarted {
  runId: string;
}

export interface UnityIntegrationTestEvent {
  runId: string;
  event: string;
  payload: Record<string, unknown>;
}

export function runUnityIntegrationTests(
  request: UnityIntegrationTestRunRequest,
): Promise<UnityIntegrationTestRunStarted> {
  return ipcInvoke<UnityIntegrationTestRunStarted>(
    "unity_integration_test_run",
    { request },
    {
      operation: "unityIntegrationTestRun",
      notify: false,
      throwOnError: true,
    },
  );
}

export function cancelUnityIntegrationTests(): Promise<void> {
  return ipcInvoke<void>(
    "unity_integration_test_cancel",
    {},
    {
      operation: "unityIntegrationTestCancel",
      notify: false,
      throwOnError: true,
    },
  );
}

/**
 * Run the stand-alone recompile probe: the backend writes a throwaway harmless
 * `.cs` into the current project's `Assets`, drives a real recompile, then
 * deletes it and converges the deletion. Resolves to a line-oriented report
 * (`PASS`/`FAIL`/`WARN`-prefixed lines).
 */
export function runUnityRecompileProbe(): Promise<string> {
  return ipcInvoke<string>(
    "unity_recompile_probe_run",
    {},
    {
      operation: "unityRecompileProbe",
      notify: false,
      throwOnError: true,
    },
  );
}

export function subscribeUnityIntegrationTests(
  handler: (payload: UnityIntegrationTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<UnityIntegrationTestEvent>("unity-integration-test", handler);
}
