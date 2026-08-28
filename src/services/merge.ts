import { ipcInvoke } from "./ipc";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
  type WorkspaceRef,
} from "./project";
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

function cloneWorkspaceRef(workspaceRef: WorkspaceRef): WorkspaceRef {
  return {
    checkoutId: workspaceRef.checkoutId,
    expectedGeneration: workspaceRef.expectedGeneration ?? undefined,
  };
}

export async function listenMergeProgress(
  cb: (evt: MergeProgressEvent) => void,
  workspaceRef: WorkspaceRef,
): Promise<UnlistenFn> {
  const scopedRef = cloneWorkspaceRef(workspaceRef);
  return listen<RoutedWorkspaceEvent<MergeProgressEvent>>(
    WORKSPACE_EVENT_NAME,
    ({ payload }) => {
      if (payload.eventName !== "merge-progress") return;
      if (payload.checkoutId !== scopedRef.checkoutId) return;
      if (
        scopedRef.expectedGeneration != null
        && payload.workspaceGeneration !== scopedRef.expectedGeneration
      ) return;
      cb(payload.payload);
    },
  );
}

/**
 * Build or retrieve a cached merge semantic session.
 * Returns the session summary + tree + targets (no inspector data).
 */
export function mergeSemanticSession(
  request: MergeSessionRequest,
  workspaceRef: WorkspaceRef,
): Promise<MergeSessionPayload> {
  const scopedRef = cloneWorkspaceRef(workspaceRef);
  return ipcInvoke<MergeSessionPayload>("git_merge_semantic_session", {
    request,
    workspaceRef: scopedRef,
  });
}

/**
 * Lazily load a single target's merge inspector.
 */
export function mergeSemanticTarget(
  request: MergeTargetRequest,
  workspaceRef: WorkspaceRef,
): Promise<MergeTargetInspector> {
  return ipcInvoke<MergeTargetInspector>("git_merge_semantic_target", {
    request,
    workspaceRef: cloneWorkspaceRef(workspaceRef),
  });
}

/**
 * Apply semantic merge resolution: writes the resolved file and stages it.
 */
export function mergeSemanticApply(
  request: MergeApplyRequest,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke("git_merge_semantic_apply", {
    request,
    workspaceRef: cloneWorkspaceRef(workspaceRef),
  });
}

/**
 * Validate semantic merge resolution without writing to disk.
 */
export function mergeSemanticValidate(
  request: MergeApplyRequest,
  workspaceRef: WorkspaceRef,
): Promise<void> {
  return ipcInvoke("git_merge_semantic_validate", {
    request,
    workspaceRef: cloneWorkspaceRef(workspaceRef),
  });
}
