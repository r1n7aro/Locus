import { getCurrentScope, onScopeDispose } from "vue";

/** Capture in setup, before any await. Late listener registration then observes
 * an already aborted signal instead of retaining an unmounted component.
 */
export function useWorkspaceEventScope(): AbortSignal {
  const controller = new AbortController();
  if (getCurrentScope()) onScopeDispose(() => controller.abort());
  return controller.signal;
}
