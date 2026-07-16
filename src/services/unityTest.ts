import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ipcInvoke } from "./ipc";

import type {
  UnityTestDiscovery,
  UnityTestFilter,
  UnityTestProgress,
  UnityTestProgressEvent,
  UnityTestRunRequest,
  UnityTestSnapshot,
  UnityTestSnapshotChangedEvent,
  UnityTestSourceNavigationResult,
} from "../types";

export const UNITY_TEST_PROGRESS_EVENT = "unity-test-progress";
export const UNITY_TEST_SNAPSHOT_CHANGED_EVENT = "unity-test-snapshot-changed";

export async function discoverUnityTests(filter: UnityTestFilter): Promise<UnityTestDiscovery> {
  return ipcInvoke<UnityTestDiscovery>("unity_test_discover", { filter });
}

export async function runUnityTestsFromDashboard(
  request: UnityTestRunRequest,
): Promise<UnityTestSnapshot> {
  return ipcInvoke<UnityTestSnapshot>("unity_test_run_dashboard", { request });
}

export async function cancelUnityTestsFromDashboard(): Promise<void> {
  return ipcInvoke<void>("unity_test_cancel_dashboard");
}

export async function getUnityTestActiveProgress(): Promise<UnityTestProgress | null> {
  return ipcInvoke<UnityTestProgress | null>("unity_test_active_progress");
}

export async function getUnityTestLatestSnapshot(): Promise<UnityTestSnapshot | null> {
  return ipcInvoke<UnityTestSnapshot | null>("unity_test_latest_snapshot");
}

export async function openUnityTestSource(
  path: string,
  line?: number,
): Promise<UnityTestSourceNavigationResult> {
  return ipcInvoke<UnityTestSourceNavigationResult>("unity_test_open_source", {
    path,
    line: line ?? null,
  });
}

export function listenUnityTestProgress(
  callback: (event: UnityTestProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<UnityTestProgressEvent>(UNITY_TEST_PROGRESS_EVENT, (event) => callback(event.payload));
}

export function listenUnityTestSnapshotChanged(
  callback: (event: UnityTestSnapshotChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<UnityTestSnapshotChangedEvent>(
    UNITY_TEST_SNAPSHOT_CHANGED_EVENT,
    (event) => callback(event.payload),
  );
}
