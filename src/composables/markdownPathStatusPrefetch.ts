import { getActivePinia } from "pinia";
import { useProjectStore } from "../stores/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { WorkspaceRef } from "../services/project";
import { collectMarkdownPathCandidates } from "./markdownInject";
import { prefetchMarkdownPathStatuses } from "./markdownPathStatusCache";

interface MarkdownMessageSource {
  content?: string | null;
  thinkingContent?: string | null;
}

export interface MarkdownPathStatusScope {
  workingDir: string;
  workspaceRef: WorkspaceRef;
}

/**
 * Working directory that keys a rendered markdown's path statuses. Prefetches
 * must use the same key as MarkdownRenderer, or the renderer misses them.
 */
export function markdownPathStatusWorkingDir(
  workspaceRef: WorkspaceRef | null | undefined,
  checkoutsById: Record<string, { root?: string | null } | undefined> | null | undefined,
  projectWorkingDir: string,
): string {
  return (workspaceRef && checkoutsById?.[workspaceRef.checkoutId]?.root) || projectWorkingDir;
}

/** The status scope MarkdownRenderer uses for `workspaceRef` (or the focused workspace). */
export function resolveMarkdownPathStatusScope(
  workspaceRef?: WorkspaceRef | null,
): MarkdownPathStatusScope | null {
  if (!getActivePinia()) return null;
  const workspaceContext = useWorkspaceContextStore();
  const ref = workspaceRef ?? workspaceContext.focusedWorkspaceRef;
  if (!ref) return null;
  return {
    workingDir: markdownPathStatusWorkingDir(ref, workspaceContext.checkoutsById, useProjectStore().workingDir),
    workspaceRef: ref,
  };
}

/**
 * Starts resolving the statuses of the given candidates in `workspaceRef`'s
 * scope. Returns null when they are all known (or there is no scope), so
 * callers stay synchronous.
 */
export function prefetchMarkdownPathCandidates(
  candidates: Iterable<string>,
  workspaceRef?: WorkspaceRef | null,
): Promise<void> | null {
  const paths = [...new Set(candidates)];
  if (paths.length === 0) return null;
  const scope = resolveMarkdownPathStatusScope(workspaceRef);
  if (!scope) return null;
  return prefetchMarkdownPathStatuses(scope.workingDir, paths, scope.workspaceRef);
}

/**
 * Resolves the path statuses of messages before they are rendered, in the
 * scope MarkdownRenderer reads: inline code paths and the bare-text paths of
 * file references. A transcript then renders its file references once, in the
 * first frame, instead of rendering text and re-rendering every message when
 * the statuses arrive.
 *
 * Returns null when nothing has to be fetched, so callers stay synchronous.
 */
export function prefetchMessagePathStatuses(
  messages: readonly MarkdownMessageSource[],
  workspaceRef?: WorkspaceRef | null,
): Promise<void> | null {
  if (!getActivePinia()) return null;
  const candidates = new Set<string>();
  for (const message of messages) {
    for (const source of [message.content, message.thinkingContent]) {
      if (!source) continue;
      for (const candidate of collectMarkdownPathCandidates(source, { fileRefs: true })) candidates.add(candidate);
    }
  }
  return prefetchMarkdownPathCandidates(candidates, workspaceRef);
}
