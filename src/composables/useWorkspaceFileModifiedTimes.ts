import { onScopeDispose, reactive, watch } from "vue";
import { projectExplorerFileRevision, subscribeWorkspaceFileChanges } from "../services/workspaceExplorer";

export interface WorkspaceFileTimeTarget {
  projectId: string;
  path: string;
}

const normalizePath = (path: string) => path.replace(/\\/g, "/");
const targetKey = (target: WorkspaceFileTimeTarget) => JSON.stringify([target.projectId, normalizePath(target.path)]);

/** Load metadata only for the rendered file rows; reuse results across scrolling. */
export function useWorkspaceFileModifiedTimes(
  files: () => WorkspaceFileTimeTarget[],
  refreshToken: () => unknown,
  ownerWindow: Window | undefined = typeof window === "undefined" ? undefined : window,
) {
  const cache = reactive<Record<string, { modifiedAt?: number; checkedAt: number }>>({});
  const queued = new Map<string, WorkspaceFileTimeTarget>();
  const pending = new Set<string>();
  let disposed = false;

  function pump(): void {
    if (disposed) return;
    for (const [key, file] of queued) {
      if (pending.size >= 4) break;
      if (pending.has(key)) continue;
      queued.delete(key);
      pending.add(key);
      void projectExplorerFileRevision(file.projectId, file.path).then((revision) => {
        if (disposed) return;
        const seconds = Number(revision.modifiedAtNanos) / 1_000_000_000;
        cache[key] = {
          modifiedAt: revision.exists && Number.isFinite(seconds) && seconds > 0 ? seconds : undefined,
          checkedAt: Date.now(),
        };
      }).catch(() => {
        if (!disposed) cache[key] = { modifiedAt: undefined, checkedAt: Date.now() };
      }).finally(() => {
        pending.delete(key);
        pump();
      });
    }
  }

  function enqueue(targets: WorkspaceFileTimeTarget[], force = false): void {
    if (disposed) return;
    for (const target of targets) {
      const key = targetKey(target);
      if (!force && (pending.has(key) || (cache[key] && Date.now() - cache[key].checkedAt < 60_000))) continue;
      queued.set(key, target);
    }
    pump();
  }

  watch([files, refreshToken], ([targets, token], previous) => {
    const visibleKeys = new Set(targets.map(targetKey));
    for (const key of queued.keys()) if (!visibleKeys.has(key)) queued.delete(key);
    enqueue(targets, previous?.[1] !== undefined && token !== previous[1]);
  }, { immediate: true });

  let release: (() => void) | undefined;
  void subscribeWorkspaceFileChanges((event) => {
    const changedPath = normalizePath(event.payload.path);
    enqueue(files().filter((file) => file.projectId === event.projectId && (
      normalizePath(file.path) === changedPath || normalizePath(file.path).endsWith(`/${changedPath}`)
    )), true);
  }).then((unsubscribe) => {
    if (disposed) unsubscribe();
    else release = unsubscribe;
  }).catch(() => { /* The periodic metadata refresh remains available. */ });

  const refreshOnFocus = () => enqueue(files(), true);
  const refreshOnVisibility = () => {
    if (ownerWindow?.document.visibilityState === "visible") refreshOnFocus();
  };
  ownerWindow?.addEventListener("focus", refreshOnFocus);
  ownerWindow?.document.addEventListener("visibilitychange", refreshOnVisibility);

  onScopeDispose(() => {
    disposed = true;
    queued.clear();
    release?.();
    ownerWindow?.removeEventListener("focus", refreshOnFocus);
    ownerWindow?.document.removeEventListener("visibilitychange", refreshOnVisibility);
  });

  return {
    modifiedAt: (target: WorkspaceFileTimeTarget): number | undefined => cache[targetKey(target)]?.modifiedAt,
  };
}
