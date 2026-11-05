import { computed, onScopeDispose, shallowRef, watch } from "vue";
import { subscribeUnitySemanticState, unitySemanticStateGet } from "../services/csharpLsp";
import { normalizeAppError } from "../services/errors";
import type { WorkspaceRef } from "../services/project";
import type { UnitySemanticState } from "../types";

type Listener = (state: UnitySemanticState | null, error: string) => void;

interface SemanticStateEntry {
  workspaceRef: WorkspaceRef;
  state: UnitySemanticState | null;
  error: string;
  listeners: Set<Listener>;
  unsubscribe: (() => void) | null;
  closed: boolean;
  fetchSeq: number;
}

// One subscription and one snapshot per workspace scope, shared by every
// status bar in this renderer. The backend observer pushes each change, so
// nothing here polls: a snapshot is fetched once when the scope opens, when
// the window becomes visible again, and on explicit refreshes after actions.
const entries = new Map<string, SemanticStateEntry>();
let visibilityListening = false;

function scopeKey(workspaceRef: WorkspaceRef): string {
  return JSON.stringify([
    workspaceRef.checkoutId,
    workspaceRef.expectedGeneration ?? null,
    workspaceRef.expectedMaterializationEpoch ?? null,
  ]);
}

function publish(entry: SemanticStateEntry) {
  for (const listener of entry.listeners) listener(entry.state, entry.error);
}

function accept(entry: SemanticStateEntry, next: UnitySemanticState) {
  // A snapshot request can resolve after a newer pushed sample.
  if (entry.state && next.checkedAtMs < entry.state.checkedAtMs) return;
  entry.state = next;
  entry.error = "";
  publish(entry);
}

async function fetchEntry(entry: SemanticStateEntry): Promise<void> {
  const seq = ++entry.fetchSeq;
  try {
    const state = await unitySemanticStateGet(entry.workspaceRef);
    if (entry.closed || seq !== entry.fetchSeq) return;
    accept(entry, state);
  } catch (error) {
    if (entry.closed || seq !== entry.fetchSeq) return;
    entry.error = normalizeAppError(error).message;
    publish(entry);
  }
}

function refreshVisibleEntries() {
  if (typeof document !== "undefined" && document.visibilityState === "hidden") return;
  for (const entry of entries.values()) void fetchEntry(entry);
}

function syncVisibilityListener() {
  if (typeof document === "undefined") return;
  const wanted = entries.size > 0;
  if (wanted === visibilityListening) return;
  visibilityListening = wanted;
  if (wanted) document.addEventListener("visibilitychange", refreshVisibleEntries);
  else document.removeEventListener("visibilitychange", refreshVisibleEntries);
}

function openEntry(key: string, workspaceRef: WorkspaceRef): SemanticStateEntry {
  const entry: SemanticStateEntry = {
    workspaceRef: { ...workspaceRef },
    state: null,
    error: "",
    listeners: new Set(),
    unsubscribe: null,
    closed: false,
    fetchSeq: 0,
  };
  entries.set(key, entry);
  syncVisibilityListener();
  // Subscribe before the snapshot so no change falls between the two.
  void subscribeUnitySemanticState(entry.workspaceRef, (state) => {
    if (!entry.closed) accept(entry, state);
  })
    .then((unsubscribe) => {
      if (entry.closed) unsubscribe();
      else entry.unsubscribe = unsubscribe;
    })
    .catch(() => {
      // The snapshot below still shows the state; refreshes keep it current.
    })
    .finally(() => {
      if (!entry.closed) void fetchEntry(entry);
    });
  return entry;
}

function closeEntry(key: string, entry: SemanticStateEntry) {
  entry.closed = true;
  entry.unsubscribe?.();
  entry.unsubscribe = null;
  if (entries.get(key) === entry) entries.delete(key);
  syncVisibilityListener();
}

export interface UnitySemanticStateOptions {
  workspaceRef: () => WorkspaceRef | null | undefined;
  /** The workspace offers the Unity editor connection. */
  enabled: () => boolean;
  /** The consuming surface is shown. Hidden consumers keep their last snapshot
   * without receiving updates, so they do not re-render in the background. */
  active?: () => boolean;
}

export function useUnitySemanticState(options: UnitySemanticStateOptions) {
  const state = shallowRef<UnitySemanticState | null>(null);
  const error = shallowRef("");
  let lease: { key: string; entry: SemanticStateEntry; listener: Listener } | null = null;

  function release() {
    if (!lease) return;
    const { key, entry, listener } = lease;
    lease = null;
    entry.listeners.delete(listener);
    if (entry.listeners.size === 0) closeEntry(key, entry);
  }

  function acquire(key: string, workspaceRef: WorkspaceRef) {
    const entry = entries.get(key) ?? openEntry(key, workspaceRef);
    const listener: Listener = (next, nextError) => {
      state.value = next;
      error.value = nextError;
    };
    entry.listeners.add(listener);
    lease = { key, entry, listener };
    // A scope that just opened has no data yet; keep the previous snapshot
    // until its first sample instead of flashing an empty state.
    if (entry.state || entry.error) listener(entry.state, entry.error);
  }

  watch(
    () => {
      const workspaceRef = options.workspaceRef();
      const key = workspaceRef && options.enabled() ? scopeKey(workspaceRef) : "";
      return [key, options.active?.() ?? true] as const;
    },
    ([key, active], previous) => {
      if (!previous || key !== previous[0]) {
        release();
        state.value = null;
        error.value = "";
      }
      const workspaceRef = options.workspaceRef();
      if (!key || !active || !workspaceRef) {
        release();
        return;
      }
      if (!lease) acquire(key, workspaceRef);
    },
    { immediate: true },
  );

  onScopeDispose(release);

  return {
    // Read-only views; the shared entry owns the values.
    state: computed(() => state.value),
    error: computed(() => error.value),
    /** Fetch a snapshot now, e.g. right after an action changed the editor. */
    refresh(): Promise<void> {
      return lease ? fetchEntry(lease.entry) : Promise.resolve();
    },
  };
}
