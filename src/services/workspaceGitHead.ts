import { shallowReactive } from "vue";
import type { GitHeadState, SessionExecutionTarget } from "../types";
import type { WorkspaceRef } from "./project";

interface ObservedHead {
  workspaceRef: WorkspaceRef;
  head: GitHeadState;
}

// Reuse HEADs returned by existing Git requests. Reading a session label never
// performs IPC, starts a watcher, or runs Git.
const heads = shallowReactive(new Map<string, ObservedHead>());
const observations = new Map<string, number>();
let nextObservation = 0;

export function beginWorkspaceGitHeadObservation(): number {
  return ++nextObservation;
}

export function rememberWorkspaceGitHead(
  workspaceRef: WorkspaceRef,
  head: GitHeadState,
  observation: number,
): void {
  const key = workspaceRef.checkoutId;
  if (observation < (observations.get(key) ?? 0)) return;
  observations.set(key, observation);
  const previous = heads.get(key);
  if (previous
    && previous.workspaceRef.expectedGeneration === workspaceRef.expectedGeneration
    && previous.workspaceRef.expectedMaterializationEpoch === workspaceRef.expectedMaterializationEpoch
    && previous.head.kind === head.kind
    && previous.head.refName === head.refName
    && previous.head.hash === head.hash) return;
  heads.set(key, { workspaceRef: { ...workspaceRef }, head: { ...head } });
}

export function workspaceGitHead(workspaceRef: WorkspaceRef): GitHeadState | undefined {
  const observed = heads.get(workspaceRef.checkoutId);
  if (!observed) return undefined;
  if (workspaceRef.expectedGeneration != null
    && workspaceRef.expectedGeneration !== observed.workspaceRef.expectedGeneration) return undefined;
  if (workspaceRef.expectedMaterializationEpoch != null
    && observed.workspaceRef.expectedMaterializationEpoch != null
    && workspaceRef.expectedMaterializationEpoch !== observed.workspaceRef.expectedMaterializationEpoch) return undefined;
  return observed.head;
}

function shortBranch(ref: string | null | undefined): string {
  return ref?.trim().replace(/^refs\/heads\//, "") ?? "";
}

export function differingSessionBranchLabel(
  target: SessionExecutionTarget | null | undefined,
  currentHead: GitHeadState | undefined,
): string {
  if (!target || !currentHead || (!currentHead.refName && !currentHead.hash)) return "";
  const branch = shortBranch(target.branchRef);
  const currentBranch = shortBranch(currentHead.refName);
  if (branch) return branch === currentBranch ? "" : branch;
  const oid = target.headOid?.trim();
  if (!oid || (!currentBranch && oid === currentHead.hash)) return "";
  return oid.slice(0, 8);
}
