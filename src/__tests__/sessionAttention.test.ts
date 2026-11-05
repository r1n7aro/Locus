// @vitest-environment jsdom
import { afterAll, beforeAll, beforeEach, describe, expect, it } from "vitest";
import type { ProjectExplorerNode, ProjectExplorerOperation, ProjectExplorerSnapshot } from "../types/workbench";
import {
  acknowledgeSessionPromotions, applySessionAttentionChange, applySessionAttentionSnapshot, emptySessionAttentionState,
  isSessionUnread, markSessionAttentionRead, pendingSessionPromotions, resetSessionAttentionForTests, sessionAttention,
  sessionPromotionOperations, updateSessionAttention, type SessionAttentionEntry,
} from "../services/sessionAttention";
import { createSessionAttentionServer } from "./fixtures/sessionAttentionServer";

function entry(sequence: number, overrides: Partial<SessionAttentionEntry> = {}): SessionAttentionEntry {
  return { sequence, readSequence: 0, kind: "done", runId: `run-${sequence}`, completionSequence: sequence,
    revision: sequence, updatedAt: 0, ...overrides };
}
function node(id: string, position: number, kind = "session", parentNodeId?: string): ProjectExplorerNode {
  return { nodeId: id, resourceId: id, projectId: "project", resourceKind: kind,
    nodeKind: kind === "folder" ? "folder" : "resource", hidden: false, position, parentNodeId };
}
function snapshot(nodes = [node("a", 0), node("b", 1), node("c", 2), node("d", 3)]): ProjectExplorerSnapshot {
  return { projectId: "project", presetId: "default", presetName: "Default", manifestPath: "tree.json", revision: 1, nodes, presets: [] };
}
function applyMoves(tree: ProjectExplorerSnapshot, operations: ProjectExplorerOperation[]) {
  for (const operation of operations) {
    if (operation.kind !== "moveNode") throw new Error("Expected a move");
    const siblings = tree.nodes.filter((item) => (item.parentNodeId ?? null) === (operation.parentNodeId ?? null))
      .sort((a, b) => a.position - b.position);
    const index = siblings.findIndex((item) => item.nodeId === operation.nodeId);
    const [moving] = siblings.splice(index, 1);
    siblings.splice(operation.position, 0, moving!);
    siblings.forEach((item, position) => { item.position = position; });
  }
  return tree.nodes.filter((item) => !item.parentNodeId).sort((a, b) => a.position - b.position).map((item) => item.nodeId);
}

describe("session attention and stable completion ordering", () => {
  let server: ReturnType<typeof createSessionAttentionServer>;
  beforeAll(() => { server = createSessionAttentionServer(); });
  afterAll(() => server.uninstall());
  beforeEach(() => { localStorage.clear(); resetSessionAttentionForTests(); server.markRead.mockClear(); });

  it("moves each completion once, preserves the other running sessions, and never moves on read", () => {
    const state = emptySessionAttentionState();
    const tree = snapshot();
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    state.sessions.b = entry(1);
    let pending = pendingSessionPromotions(state, tree);
    expect(applyMoves(tree, sessionPromotionOperations(tree, pending))).toEqual(["b", "a", "c", "d"]);
    acknowledgeSessionPromotions(state, tree, pending);
    state.sessions.d = entry(2);
    pending = pendingSessionPromotions(state, tree);
    expect(applyMoves(tree, sessionPromotionOperations(tree, pending))).toEqual(["d", "b", "a", "c"]);
    acknowledgeSessionPromotions(state, tree, pending);
    state.sessions.d = { ...state.sessions.d!, readSequence: 2 };
    // A question after the completion is unread but not a new completion.
    state.sessions.b = entry(3, { kind: "askUser", completionSequence: 1 });
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    expect(isSessionUnread("d", state)).toBe(false);
    expect(isSessionUnread("b", state)).toBe(true);
  });

  it("keeps documents, hidden boundaries, pinned items and pinned subtrees fixed", () => {
    const tree = snapshot([node("a", 0), node("b", 1), { ...node("doc", 2, "knowledge"), hidden: true },
      node("c", 3), node("d", 4), node("pin", 5), node("e", 6), node("f", 7),
      node("folder", 8, "folder"), node("child-a", 0, "session", "folder"), node("child-b", 1, "session", "folder")]);
    tree.itemStates = [{ nodeId: "pin", pinned: true, highlighted: false }, { nodeId: "folder", pinned: true, highlighted: false }];
    const moves = sessionPromotionOperations(tree, ["b", "d", "pin", "f", "child-b"]);
    expect(moves.map((move) => move.kind === "moveNode" && move.nodeId)).toEqual(["b", "d", "f"]);
    expect(applyMoves(tree, moves)).toEqual(["b", "a", "doc", "d", "c", "pin", "f", "e", "folder"]);
  });

  it("moves child sessions only inside their own parent and keeps their descendants attached", () => {
    const tree = snapshot([node("a", 0), node("b", 1), node("child-a", 0, "session", "b"),
      node("child-b", 1, "session", "b"), node("grandchild", 0, "session", "child-b")]);
    expect(sessionPromotionOperations(tree, ["child-b"])).toEqual([
      { kind: "moveNode", nodeId: "child-b", parentNodeId: "b", position: 0 },
    ]);
    applyMoves(tree, sessionPromotionOperations(tree, ["b", "child-b"]));
    expect(tree.nodes.find((item) => item.nodeId === "grandchild")?.parentNodeId).toBe("child-b");
  });

  it("acknowledges manual placement until the next completion", () => {
    const state = emptySessionAttentionState();
    const tree = snapshot();
    state.sessions.a = entry(1, { kind: "askUser", completionSequence: 0 });
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    state.sessions.b = entry(2);
    acknowledgeSessionPromotions(state, tree, ["b"]);
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    state.sessions.b = entry(3);
    expect(pendingSessionPromotions(state, tree)).toEqual(["b"]);
  });

  it("merges changes and snapshots by revision in any order", () => {
    applySessionAttentionChange({ sessionId: "a", entry: entry(1, { revision: 4 }) });
    applySessionAttentionChange({ sessionId: "a", entry: entry(1, { revision: 3, readSequence: 1 }) });
    expect(isSessionUnread("a")).toBe(true);
    // A snapshot cut before the change keeps it; one cut after it decides.
    applySessionAttentionSnapshot({ revision: 2, sequence: 2, sessions: { b: entry(2, { revision: 2 }) } });
    expect(Object.keys(sessionAttention.value.sessions).sort()).toEqual(["a", "b"]);
    applySessionAttentionSnapshot({ revision: 6, sequence: 2, sessions: { a: entry(1, { revision: 5, readSequence: 1 }) } });
    expect(isSessionUnread("a")).toBe(false);
    expect(sessionAttention.value.sessions.b).toBeUndefined();
    expect(sessionAttention.value.sequence).toBe(2);
  });

  it("clears a read result at once and keeps a newer result unread", async () => {
    const first = server.record("a");
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const original = server.markRead.getMockImplementation()!;
    server.markRead.mockImplementationOnce(async (sessionId, read) => { await blocked; return original(sessionId, read); });
    const reading = markSessionAttentionRead("a", first.sequence);
    expect(isSessionUnread("a")).toBe(false);
    server.record("a");
    expect(isSessionUnread("a")).toBe(true);
    release();
    await reading;
    expect(server.markRead).toHaveBeenCalledWith("a", first.sequence);
    expect(sessionAttention.value.sessions.a!.readSequence).toBe(first.sequence);
    expect(isSessionUnread("a")).toBe(true);
  });

  it("leaves a result unread when the desktop refuses the read", async () => {
    const result = server.record("a");
    server.markRead.mockRejectedValueOnce(new Error("offline"));
    await expect(markSessionAttentionRead("a", result.sequence)).rejects.toBeTruthy();
    expect(isSessionUnread("a")).toBe(true);
  });

  it("keeps only tree cursors on this machine and shares them between windows", async () => {
    server.record("a");
    await updateSessionAttention((state) => { state.promoted.layout = { a: state.sessions.a!.completionSequence }; });
    const saved = JSON.parse(localStorage.getItem("locus-session-promotion-v2")!);
    expect(saved).toEqual({ version: 2, promoted: { layout: { a: sessionAttention.value.sessions.a!.completionSequence } } });
    localStorage.setItem("locus-session-promotion-v2", JSON.stringify({ version: 2, promoted: { other: { b: 3 } } }));
    window.dispatchEvent(new StorageEvent("storage", { key: "locus-session-promotion-v2" }));
    expect(sessionAttention.value.promoted).toEqual({ other: { b: 3 } });
    expect(isSessionUnread("a")).toBe(true);
  });
});
