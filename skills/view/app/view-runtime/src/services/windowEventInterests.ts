import { invoke } from "@tauri-apps/api/core";
import { watch, type WatchStopHandle } from "vue";
import type { WorkspaceRef } from "./project";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import { ensureWorkspaceEventTransport } from "./workspaceEventHub";
import { remoteClientTransport } from "./remoteClient";

export interface WindowEventInterests {
  checkouts: WorkspaceRef[];
  sessions: string[];
  streamCheckouts: WorkspaceRef[];
}
export type WindowEventInterest =
  | { sessionId: string }
  | { workspaceRef: WorkspaceRef }
  | { streamWorkspaceRef: WorkspaceRef };

export interface WindowEventInterestOptions {
  /** Delays between the immediate attempts of one flush (attempts = length + 1). */
  retryDelaysMs?: readonly number[];
  /** Escalating delays of the background retries after a flush gives up. */
  backgroundRetryMs?: readonly number[];
  warn?: (message: string, error: unknown) => void;
}

const DEFAULT_RETRY_DELAYS_MS = [50, 200];
const DEFAULT_BACKGROUND_RETRY_MS = [250, 1_000, 3_000, 10_000, 30_000];

function scopeKey(scope: WorkspaceRef): string {
  return JSON.stringify([scope.checkoutId, scope.expectedGeneration ?? null, scope.expectedMaterializationEpoch ?? null]);
}

/** One registry per renderer, including all of its teleported panes. Updates
 * are serialized and acknowledged before snapshots or launches can race them.
 *
 * Registration only narrows high-frequency delivery, so it must never fail the
 * snapshot or launch that awaits it: `flush` retries briefly, then resolves and
 * keeps retrying in the background. Until it succeeds the window still receives
 * every lifecycle event, and a terminal event carries the final transcript, so
 * the window misses live deltas but never results. Identical snapshots are not
 * resent. */
export function createWindowEventInterests(
  send: (interests: WindowEventInterests) => Promise<unknown>,
  options: WindowEventInterestOptions = {},
) {
  const retryDelays = options.retryDelaysMs ?? DEFAULT_RETRY_DELAYS_MS;
  const backgroundDelays = options.backgroundRetryMs ?? DEFAULT_BACKGROUND_RETRY_MS;
  const warn = options.warn ?? ((message: string, error: unknown) => console.warn(`[workspace-events] ${message}`, error));
  const interests = new Map<object, WindowEventInterest>();
  let version = 0;
  // A reload reuses its native label but must replace the previous realm's
  // interests, including when the new page currently has no consumers.
  let sentVersion = -1;
  let sentKey: string | null = null;
  let sending: Promise<void> | null = null;
  let scheduled = false;
  let retryTimer: ReturnType<typeof setTimeout> | null = null;
  let retryAttempt = 0;
  let failing = false;

  function snapshot(): WindowEventInterests {
    const sessions = new Set<string>();
    const checkouts = new Map<string, WorkspaceRef>();
    const streamCheckouts = new Map<string, WorkspaceRef>();
    for (const interest of interests.values()) {
      if ("sessionId" in interest) {
        if (interest.sessionId.trim()) sessions.add(interest.sessionId.trim());
      } else {
        const scope = "workspaceRef" in interest ? interest.workspaceRef : interest.streamWorkspaceRef;
        const target = "workspaceRef" in interest ? checkouts : streamCheckouts;
        target.set(scopeKey(scope), { ...scope });
      }
    }
    // Canonical order, so an unchanged set compares equal however it was built.
    const sorted = (scopes: Map<string, WorkspaceRef>) => [...scopes.entries()].sort(([a], [b]) => a.localeCompare(b)).map(([, scope]) => scope);
    return { checkouts: sorted(checkouts), sessions: [...sessions].sort(), streamCheckouts: sorted(streamCheckouts) };
  }

  /** One serialized attempt; it covers every change made before it settles.
   * A plain promise chain: launches await it, so it adds no hops beyond the IPC. */
  function attempt(): Promise<void> {
    if (sending) return sending.then(attempt, attempt);
    if (sentVersion === version) return Promise.resolve();
    const currentVersion = version;
    const next = snapshot();
    const key = JSON.stringify(next);
    if (key === sentKey) {
      sentVersion = currentVersion;
      return Promise.resolve();
    }
    const request: Promise<void> = Promise.resolve(send(next)).then(() => {
      sentKey = key;
      sentVersion = currentVersion;
      if (sending === request) sending = null;
      return sentVersion === version ? undefined : attempt();
    }, (error: unknown) => {
      if (sending === request) sending = null;
      throw error;
    });
    sending = request;
    return request;
  }

  function recovered(): void {
    if (retryTimer !== null) clearTimeout(retryTimer);
    retryTimer = null;
    retryAttempt = 0;
    failing = false;
  }

  function scheduleRetry(): void {
    if (retryTimer !== null) return;
    const delay = backgroundDelays[Math.min(retryAttempt, backgroundDelays.length - 1)] ?? 30_000;
    retryAttempt += 1;
    retryTimer = setTimeout(() => {
      retryTimer = null;
      attempt().then(recovered, () => scheduleRetry());
    }, delay);
  }

  // The common path adds no hops beyond the IPC itself: launches await it.
  function flush(): Promise<void> {
    if (!sending && sentVersion === version) return Promise.resolve();
    return attempt().then(recovered, retryFlush);
  }

  async function retryFlush(firstError: unknown): Promise<void> {
    let lastError = firstError;
    for (const delayMs of retryDelays) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
      try {
        await attempt();
        recovered();
        return;
      } catch (error) {
        lastError = error;
      }
    }
    if (!failing) {
      failing = true;
      warn("interest update failed; this window keeps lifecycle events and retries targeted delivery in the background", lastError);
    }
    scheduleRetry();
  }

  function changed(): void {
    version += 1;
    if (scheduled) return;
    scheduled = true;
    queueMicrotask(() => {
      scheduled = false;
      void flush();
    });
  }

  function retain(interest: WindowEventInterest): () => void {
    const owner = {};
    interests.set(owner, interest);
    changed();
    return () => { if (interests.delete(owner)) changed(); };
  }
  return { retain, flush, snapshot };
}

type Registry = ReturnType<typeof createWindowEventInterests>;
let registry: Registry | undefined = import.meta.hot?.data?.windowEventInterests;
function currentRegistry(): Registry {
  registry ??= createWindowEventInterests(async interests => {
    // The mobile app also has a Tauri runtime (its own shell), so check it first.
    const remote = remoteClientTransport();
    if (remote) {
      await remote.setInterests(interests);
      return;
    }
    // Native Unity Views use the unfiltered internal router stream, not Tauri.
    if (!hasTauriWindowRuntime()) return;
    await ensureWorkspaceEventTransport();
    await invoke("set_workspace_event_interests", { interests });
  });
  return registry;
}

export function retainWindowEventInterest(interest: WindowEventInterest): () => void {
  return currentRegistry().retain(interest);
}
/** Resolves once the current interests are registered, or after bounded
 * retries failed (registration then continues in the background). Never rejects. */
export function flushWindowEventInterests(): Promise<void> {
  return currentRegistry().flush();
}

/** Explicitly stopped by the subscription owner, even when created outside a
 * component scope (for example a lazily loaded file-change subscription). */
export function watchWindowEventInterests(resolve: () => WindowEventInterest[]): () => void {
  let releases: (() => void)[] = [];
  let currentKey: string | null = null;
  const stop: WatchStopHandle = watch(resolve, values => {
    // Deep watchers also fire for unrelated reactive changes; keep the retains
    // while the resolved interest set is unchanged.
    const key = JSON.stringify(values);
    if (key === currentKey) return;
    currentKey = key;
    const previous = releases;
    releases = values.map(retainWindowEventInterest);
    previous.forEach(release => release());
  }, { immediate: true, deep: true, flush: "sync" });
  void flushWindowEventInterests();
  return () => { stop(); releases.forEach(release => release()); releases = []; currentKey = null; };
}

if (typeof import.meta.hot?.dispose === "function") {
  import.meta.hot.dispose(data => { data.windowEventInterests = registry; });
}
