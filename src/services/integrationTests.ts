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

export function subscribeUnityIntegrationTests(
  handler: (payload: UnityIntegrationTestEvent) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<UnityIntegrationTestEvent>("unity-integration-test", handler);
}
