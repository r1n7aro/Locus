import { ipcInvoke } from "./ipc";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  MergeSessionRequest,
  MergeSessionPayload,
  MergeTargetInspector,
  MergeTargetRequest,
  MergeApplyRequest,
} from "../types";

// ── Merge progress events ──

export interface MergeProgressEvent {
  requestKey: string;
  phase: "fetchContent" | "textDiff" | "parseYaml" | "buildSemantic" | "done" | "error";
  current: number;
  total: number;
  elapsedMs: number;
  phaseDurations?: Record<string, number>;
}

export function listenMergeProgress(
  cb: (evt: MergeProgressEvent) => void,
): Promise<UnlistenFn> {
  return listen<MergeProgressEvent>("merge-progress", (e) => cb(e.payload));
}

/**
 * Build or retrieve a cached merge semantic session.
 * Returns the session summary + tree + targets (no inspector data).
 */
export function mergeSemanticSession(
  request: MergeSessionRequest,
): Promise<MergeSessionPayload> {
  return ipcInvoke<MergeSessionPayload>("git_merge_semantic_session", { request });
}

/**
 * Lazily load a single target's merge inspector.
 */
export function mergeSemanticTarget(
  request: MergeTargetRequest,
): Promise<MergeTargetInspector> {
  return ipcInvoke<MergeTargetInspector>("git_merge_semantic_target", {
    request,
  });
}

/**
 * Apply semantic merge resolution: writes the resolved file and stages it.
 */
export function mergeSemanticApply(
  request: MergeApplyRequest,
): Promise<void> {
  return ipcInvoke("git_merge_semantic_apply", { request });
}

/**
 * Validate semantic merge resolution without writing to disk.
 */
export function mergeSemanticValidate(
  request: MergeApplyRequest,
): Promise<void> {
  return ipcInvoke("git_merge_semantic_validate", { request });
}
