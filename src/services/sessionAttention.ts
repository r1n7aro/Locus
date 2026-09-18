import { shallowRef } from "vue";
import type { StreamEvent } from "../types";
import type { ProjectExplorerOperation, ProjectExplorerSnapshot } from "../types/workbench";

export interface SessionAttentionEntry {
  sequence: number;
  readSequence: number;
  kind: "done" | "error" | "askUser" | "toolConfirm" | "knowledgeProposal";
  runId: string;
  targetId?: string;
  completionSequence: number;
  promoteCompletion: boolean;
}

/** UI state only; session messages, exports and the session database are unchanged. */
export interface SessionAttentionState {
  version: 1;
  revision: number;
  sequence: number;
  sessions: Record<string, SessionAttentionEntry>;
  seen: Record<string, true>;
  runs: Record<string, string>;
  promoted: Record<string, Record<string, number>>;
}

const STORAGE_KEY = "locus-session-attention-v1";
const LOCK_NAME = "locus-session-attention";

export function emptySessionAttentionState(): SessionAttentionState {
  return { version: 1, revision: 0, sequence: 0, sessions: {}, seen: {}, runs: {}, promoted: {} };
}

let memoryState = emptySessionAttentionState();
const pendingReads = new Map<string, number>();

/** Keep visible read acknowledgements independent of queued layout IPC writes. */
function withPendingReads(state: SessionAttentionState): SessionAttentionState {
  if (!pendingReads.size) return state;
  const sessions = { ...state.sessions };
  for (const [id, sequence] of pendingReads) {
    const entry = sessions[id];
    if (entry) sessions[id] = { ...entry, readSequence: Math.max(entry.readSequence, Math.min(sequence, entry.sequence)) };
  }
  return { ...state, sessions };
}

function load(): SessionAttentionState {
  if (typeof localStorage === "undefined") return structuredClone(memoryState);
  const raw = localStorage.getItem(STORAGE_KEY);
  if (!raw) return emptySessionAttentionState();
  const value = JSON.parse(raw) as SessionAttentionState;
  if (value.version !== 1 || !value.sessions || !value.seen || !value.runs || !value.promoted) {
    throw new Error("Unsupported session attention state");
  }
  return value;
}

export const sessionAttention = shallowRef<SessionAttentionState>(emptySessionAttentionState());
try { sessionAttention.value = load(); }
catch (error) { console.warn("[sessionAttention] Could not read saved state", error); }

if (typeof window !== "undefined") {
  window.addEventListener("storage", (event) => {
    if (event.key !== STORAGE_KEY && event.key !== null) return;
    try { sessionAttention.value = withPendingReads(load()); }
    catch (error) { console.warn("[sessionAttention] Could not synchronize state", error); }
  });
}

let queue: Promise<unknown> = Promise.resolve();

/** A shared-origin Web Lock serializes all windows, including layout IPC writes. */
export function updateSessionAttention<T>(update: (state: SessionAttentionState) => T | Promise<T>): Promise<T> {
  const run = async () => {
    const execute = async () => {
      let state: SessionAttentionState;
      try { state = load(); }
      catch (error) {
        console.warn("[sessionAttention] Using in-memory state", error);
        state = structuredClone(memoryState);
      }
      const before = JSON.stringify(state);
      const result = await update(state);
      if (JSON.stringify(state) !== before) {
        state.revision += 1;
        memoryState = state;
        try {
          if (typeof localStorage !== "undefined") localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
        } catch (error) { console.warn("[sessionAttention] Could not persist UI state", error); }
        sessionAttention.value = withPendingReads(state);
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

export function applySessionAttentionEvent(state: SessionAttentionState, event: StreamEvent, promote: boolean): void {
  if (event.type === "runStart") {
    const key = JSON.stringify([event.sessionId, event.runId, "runStart"]);
    if (state.seen[key]) return;
    state.seen[key] = true;
    state.runs[event.sessionId] = event.runId;
    return;
  }
  if (event.type !== "done" && event.type !== "error" && event.type !== "askUser"
    && event.type !== "toolConfirm" && event.type !== "knowledgeProposal") return;
  if (state.runs[event.sessionId] && state.runs[event.sessionId] !== event.runId) return;
  const targetId = event.type === "done" ? event.messageId
    : event.type === "askUser" || event.type === "toolConfirm" ? event.questionId
    : event.type === "knowledgeProposal" ? event.message.id : undefined;
  const key = JSON.stringify([event.sessionId, event.runId, event.type, event.type === "done" ? "" : targetId ?? ""]);
  if (state.seen[key]) return;
  state.seen[key] = true;
  const previous = state.sessions[event.sessionId];
  const sequence = ++state.sequence;
  state.sessions[event.sessionId] = {
    sequence,
    readSequence: previous?.readSequence ?? 0,
    kind: event.type,
    runId: event.runId,
    targetId,
    completionSequence: event.type === "done" ? sequence : previous?.completionSequence ?? 0,
    promoteCompletion: event.type === "done" ? promote : previous?.promoteCompletion ?? false,
  };
}

export function recordSessionAttention(event: StreamEvent, promote: boolean): void {
  if (!["runStart", "done", "error", "askUser", "toolConfirm", "knowledgeProposal"].includes(event.type)) return;
  void updateSessionAttention((state) => applySessionAttentionEvent(state, event, promote))
    .catch((error) => console.warn("[sessionAttention] Could not record event", error));
}

export function isSessionUnread(sessionId: string, state = sessionAttention.value): boolean {
  const entry = state.sessions[sessionId];
  return !!entry && entry.sequence > entry.readSequence;
}

export function readSessionAttention(state: SessionAttentionState, sessionId: string, sequence: number): void {
  const entry = state.sessions[sessionId];
  if (entry) entry.readSequence = Math.max(entry.readSequence, Math.min(sequence, entry.sequence));
}

export async function markSessionAttentionRead(sessionId: string, sequence: number): Promise<void> {
  const entry = sessionAttention.value.sessions[sessionId];
  if (!entry) return;
  const readSequence = Math.min(sequence, entry.sequence);
  if (readSequence <= entry.readSequence) return;
  pendingReads.set(sessionId, readSequence);
  sessionAttention.value = withPendingReads(sessionAttention.value);
  let saved = false;
  try {
    await updateSessionAttention((state) => readSessionAttention(state, sessionId, readSequence));
    saved = true;
  } finally {
    if (pendingReads.get(sessionId) === readSequence) pendingReads.delete(sessionId);
    if (!saved) {
      try { sessionAttention.value = withPendingReads(load()); }
      catch { sessionAttention.value = withPendingReads(memoryState); }
    }
  }
}

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
