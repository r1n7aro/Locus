import { shallowRef } from "vue";
import type { ProjectExplorerOperation, ProjectExplorerSnapshot } from "../types/workbench";
import { ipcInvoke } from "./ipc";
import { getLocusRuntime } from "./locusRuntime";
import type { RoutedWorkspaceEvent } from "./project";
import { WORKSPACE_EVENT_NAME } from "./workspaceEventHub";

export type SessionAttentionKind = "done" | "error" | "askUser" | "toolConfirm" | "knowledgeProposal";

/** A session's latest result, kept by the desktop for every Locus window and
 * paired phone (`session::attention`). */
export interface SessionAttentionEntry {
  sequence: number;
  readSequence: number;
  kind: SessionAttentionKind;
  runId: string;
  targetId?: string | null;
  completionSequence: number;
  /** Store revision of the last change; newer values replace older ones. */
  revision: number;
  /** Unix milliseconds of the latest result. */
  updatedAt: number;
}

export interface SessionAttentionSnapshot {
  revision: number;
  sequence: number;
  sessions: Record<string, SessionAttentionEntry>;
}

export interface SessionAttentionChange {
  sessionId: string;
  entry: SessionAttentionEntry;
}

/** Shared results plus this machine's tree-ordering cursors (`promoted`,
 * per explorer layout), which stay local UI state. */
export interface SessionAttentionState {
  sequence: number;
  sessions: Record<string, SessionAttentionEntry>;
  promoted: Record<string, Record<string, number>>;
}

export const SESSION_ATTENTION_CHANGED_EVENT = "session-attention-changed";
/** Tree-ordering cursors. v1 also held the results themselves, numbered
 * locally; those cursors do not match the desktop's sequence and are dropped. */
const PROMOTION_STORAGE_KEY = "locus-session-promotion-v2";
const LEGACY_STORAGE_KEY = "locus-session-attention-v1";
const LOCK_NAME = "locus-session-attention";

export function emptySessionAttentionState(): SessionAttentionState {
  return { sequence: 0, sessions: {}, promoted: {} };
}

let server: SessionAttentionSnapshot = { revision: 0, sequence: 0, sessions: {} };
let promoted: SessionAttentionState["promoted"] = {};
/** Reads shown at once while their acknowledgement is on the way. */
const pendingReads = new Map<string, number>();

export const sessionAttention = shallowRef<SessionAttentionState>(emptySessionAttentionState());

function publish(): void {
  let sessions = server.sessions;
  if (pendingReads.size) {
    sessions = { ...sessions };
    for (const [id, sequence] of pendingReads) {
      const entry = sessions[id];
      if (entry && entry.readSequence < sequence) {
        sessions[id] = { ...entry, readSequence: Math.min(sequence, entry.sequence) };
      }
    }
  }
  sessionAttention.value = { sequence: server.sequence, sessions, promoted };
}

const resultListeners = new Set<(change: SessionAttentionChange) => void>();

/** Calls `listener` for each new unread result received live (not for
 * results loaded from a snapshot, e.g. after reconnecting). */
export function onSessionAttentionResult(listener: (change: SessionAttentionChange) => void): () => void {
  resultListeners.add(listener);
  return () => resultListeners.delete(listener);
}

/** Applies one change; an entry never goes back to an older revision. */
export function applySessionAttentionChange(change: SessionAttentionChange): void {
  const sessionId = change?.sessionId;
  const entry = change?.entry;
  if (!sessionId || !entry) return;
  const current = server.sessions[sessionId];
  if (current && current.revision >= entry.revision) return;
  server = {
    revision: Math.max(server.revision, entry.revision),
    sequence: Math.max(server.sequence, entry.sequence),
    sessions: { ...server.sessions, [sessionId]: entry },
  };
  publish();
  if (entry.sequence > (current?.sequence ?? 0) && entry.sequence > entry.readSequence) {
    for (const listener of [...resultListeners]) {
      try { listener({ sessionId, entry }); }
      catch (error) { console.warn("[sessionAttention] Result listener failed", error); }
    }
  }
}

/** Replaces the results with a snapshot, keeping changes that arrived after
 * the snapshot was cut. */
export function applySessionAttentionSnapshot(snapshot: SessionAttentionSnapshot): void {
  const sessions = { ...snapshot.sessions };
  for (const [id, entry] of Object.entries(server.sessions)) {
    if (entry.revision > snapshot.revision && (sessions[id]?.revision ?? -1) < entry.revision) {
      sessions[id] = entry;
    }
  }
  server = {
    revision: Math.max(server.revision, snapshot.revision),
    sequence: Math.max(server.sequence, snapshot.sequence),
    sessions,
  };
  publish();
}

/** Reloads every result, e.g. after a phone reconnects. */
export async function reloadSessionAttention(): Promise<void> {
  applySessionAttentionSnapshot(await ipcInvoke<SessionAttentionSnapshot>("get_session_attention"));
}

let syncStarted = false;
let resolveInitialLoad!: () => void;
/** Settles once the first snapshot was applied (or could not be read). Tree
 * cursors wait for it: a layout first seen before the results arrived would
 * otherwise treat every earlier completion as new. */
const initialLoad = new Promise<void>((resolve) => { resolveInitialLoad = resolve; });

/** Starts following the desktop's results once per frontend: the snapshot,
 * then changes, either scoped to an open checkout (the only kind a phone
 * receives) or global. */
export function ensureSessionAttentionSync(): void {
  if (syncStarted) return;
  const runtime = getLocusRuntime();
  if (runtime.kind !== "tauri" && runtime.kind !== "remote") {
    resolveInitialLoad();
    return;
  }
  syncStarted = true;
  if (runtime.kind === "tauri") {
    try { localStorage.removeItem(LEGACY_STORAGE_KEY); } catch { /* storage unavailable */ }
  }
  void runtime.subscribe<RoutedWorkspaceEvent<SessionAttentionChange>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName === SESSION_ATTENTION_CHANGED_EVENT) applySessionAttentionChange(event.payload);
    },
    { owner: "sessionAttention" },
  ).catch((error) => console.warn("[sessionAttention] Could not follow workspace changes", error));
  void runtime.subscribe<SessionAttentionChange>(SESSION_ATTENTION_CHANGED_EVENT, applySessionAttentionChange)
    .catch((error) => console.warn("[sessionAttention] Could not follow changes", error));
  void reloadSessionAttention()
    .catch((error) => console.warn("[sessionAttention] Could not load results", error))
    .finally(resolveInitialLoad);
}

// ----- tree-ordering cursors (local)

function loadPromoted(): SessionAttentionState["promoted"] {
  if (typeof localStorage === "undefined") return structuredClone(promoted);
  const raw = localStorage.getItem(PROMOTION_STORAGE_KEY);
  if (!raw) return {};
  const value = JSON.parse(raw) as { version?: number; promoted?: SessionAttentionState["promoted"] };
  if (value.version !== 2 || !value.promoted) throw new Error("Unsupported session promotion state");
  return value.promoted;
}

try { promoted = loadPromoted(); }
catch (error) { console.warn("[sessionAttention] Could not read saved state", error); }
publish();

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key !== PROMOTION_STORAGE_KEY && event.key !== null) return;
    try {
      promoted = loadPromoted();
      publish();
    } catch (error) { console.warn("[sessionAttention] Could not synchronize state", error); }
  });
}

let queue: Promise<unknown> = Promise.resolve();

/** Runs `update` on the current state; only changes to `promoted` are kept.
 * A shared-origin Web Lock serializes all windows of this machine. */
export function updateSessionAttention<T>(update: (state: SessionAttentionState) => T | Promise<T>): Promise<T> {
  ensureSessionAttentionSync();
  const run = async () => {
    await initialLoad;
    const execute = async () => {
      let saved: SessionAttentionState["promoted"];
      try { saved = loadPromoted(); }
      catch (error) {
        console.warn("[sessionAttention] Using in-memory state", error);
        saved = structuredClone(promoted);
      }
      const view = sessionAttention.value;
      const state: SessionAttentionState = { sequence: view.sequence, sessions: view.sessions, promoted: saved };
      const before = JSON.stringify(saved);
      const result = await update(state);
      if (JSON.stringify(state.promoted) !== before) {
        promoted = state.promoted;
        try {
          if (typeof localStorage !== "undefined") {
            localStorage.setItem(PROMOTION_STORAGE_KEY, JSON.stringify({ version: 2, promoted }));
          }
        } catch (error) { console.warn("[sessionAttention] Could not persist UI state", error); }
        publish();
      }
      return result;
    };
    return typeof navigator !== "undefined" && navigator.locks
      ? navigator.locks.request(LOCK_NAME, execute)
      : execute();
  };
  const result = queue.then(run, run);
  queue = result.catch(() => {});
  return result;
}

// ----- reading

export function isSessionUnread(sessionId: string, state = sessionAttention.value): boolean {
  ensureSessionAttentionSync();
  const entry = state.sessions[sessionId];
  return !!entry && entry.sequence > entry.readSequence;
}

/** Marks a session's results up to `sequence` read on every client. Shown at
 * once; a failed acknowledgement leaves the result unread. */
export async function markSessionAttentionRead(sessionId: string, sequence: number): Promise<void> {
  ensureSessionAttentionSync();
  const entry = sessionAttention.value.sessions[sessionId];
  if (!entry) return;
  const readSequence = Math.min(sequence, entry.sequence);
  if (readSequence <= entry.readSequence) return;
  pendingReads.set(sessionId, readSequence);
  publish();
  try {
    const updated = await ipcInvoke<SessionAttentionEntry | null>("mark_session_attention_read", {
      sessionId, sequence: readSequence,
    });
    if (updated) applySessionAttentionChange({ sessionId, entry: updated });
  } finally {
    if (pendingReads.get(sessionId) === readSequence) pendingReads.delete(sessionId);
    publish();
  }
}

// ----- tree ordering of completed sessions (desktop explorer)

export function attentionLayoutKey(snapshot: ProjectExplorerSnapshot): string {
  return JSON.stringify([snapshot.projectId, snapshot.presetId]);
}

export function pendingSessionPromotions(state: SessionAttentionState, snapshot: ProjectExplorerSnapshot): string[] {
  const cursors = state.promoted[attentionLayoutKey(snapshot)] ?? {};
  return snapshot.nodes.flatMap((node) => {
    const id = node.resourceKind === "session" ? node.resourceId : null;
    const entry = id ? state.sessions[id] : undefined;
    return id && entry && entry.completionSequence > (cursors[id] ?? 0) ? [id] : [];
  }).sort((a, b) => state.sessions[a]!.completionSequence - state.sessions[b]!.completionSequence || a.localeCompare(b));
}

/** Move only the completed sibling within its original contiguous session block. */
export function sessionPromotionOperations(snapshot: ProjectExplorerSnapshot, sessionIds: readonly string[]): ProjectExplorerOperation[] {
  const pinned = new Set(snapshot.itemStates?.filter((item) => item.pinned && !item.relativePath).map((item) => item.nodeId));
  const children = new Map<string | null, typeof snapshot.nodes>();
  for (const node of snapshot.nodes) {
    const parent = node.parentNodeId ?? null;
    const siblings = children.get(parent) ?? [];
    siblings.push(node);
    children.set(parent, siblings);
  }
  for (const siblings of children.values()) siblings.sort((a, b) => a.position - b.position || a.nodeId.localeCompare(b.nodeId));
  const operations: ProjectExplorerOperation[] = [];
  const isSession = (node: typeof snapshot.nodes[number]) => node.nodeKind === "resource"
    && node.resourceKind === "session" && !pinned.has(node.nodeId);
  for (const id of sessionIds) {
    const node = snapshot.nodes.find((item) => item.resourceKind === "session" && item.resourceId === id);
    if (!node || !isSession(node)) continue;
    // A pinned folder owns a manually arranged subtree too.
    let ancestor = node.parentNodeId;
    const visited = new Set<string>();
    while (ancestor && !pinned.has(ancestor) && !visited.has(ancestor)) {
      visited.add(ancestor);
      ancestor = snapshot.nodes.find((item) => item.nodeId === ancestor)?.parentNodeId;
    }
    if (ancestor) continue;
    const siblings = children.get(node.parentNodeId ?? null)!;
    const index = siblings.indexOf(node);
    let start = index;
    while (start > 0 && isSession(siblings[start - 1]!)) start -= 1;
    if (start === index) continue;
    siblings.splice(index, 1);
    siblings.splice(start, 0, node);
    operations.push({ kind: "moveNode", nodeId: node.nodeId, parentNodeId: node.parentNodeId, position: start });
  }
  return operations;
}

export function acknowledgeSessionPromotions(state: SessionAttentionState, snapshot: ProjectExplorerSnapshot, sessionIds: readonly string[]): void {
  const cursors = state.promoted[attentionLayoutKey(snapshot)] ??= {};
  for (const id of sessionIds) cursors[id] = state.sessions[id]?.completionSequence ?? 0;
}

/** Test hook: forget every result and pending read. */
export function resetSessionAttentionForTests(): void {
  server = { revision: 0, sequence: 0, sessions: {} };
  promoted = {};
  pendingReads.clear();
  publish();
}
