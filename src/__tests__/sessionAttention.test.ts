// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";
import type { StreamEvent } from "../types";
import type { ProjectExplorerNode, ProjectExplorerOperation, ProjectExplorerSnapshot } from "../types/workbench";
import {
  acknowledgeSessionPromotions, applySessionAttentionEvent, emptySessionAttentionState,
  isSessionUnread, markSessionAttentionRead, pendingSessionPromotions, readSessionAttention, sessionAttention,
  sessionPromotionOperations, updateSessionAttention,
} from "../services/sessionAttention";

function done(id: string, runId = `${id}-run`): StreamEvent {
  return { type: "done", sessionId: id, runId, messageId: `${runId}-reply`, fullText: "Finished" };
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
  beforeEach(() => { localStorage.clear(); sessionAttention.value = emptySessionAttentionState(); });

  it("moves each completion once, preserves the other running sessions, and never moves on read", () => {
    const state = emptySessionAttentionState();
    const tree = snapshot();
    applySessionAttentionEvent(state, { type: "runStart", sessionId: "d", runId: "d-run" }, true);
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    applySessionAttentionEvent(state, done("b"), true);
    let pending = pendingSessionPromotions(state, tree);
    expect(applyMoves(tree, sessionPromotionOperations(tree, pending))).toEqual(["b", "a", "c", "d"]);
    acknowledgeSessionPromotions(state, tree, pending);
    applySessionAttentionEvent(state, done("d"), true);
    pending = pendingSessionPromotions(state, tree);
    expect(applyMoves(tree, sessionPromotionOperations(tree, pending))).toEqual(["d", "b", "a", "c"]);
    acknowledgeSessionPromotions(state, tree, pending);
    readSessionAttention(state, "d", state.sessions.d!.sequence);
    applySessionAttentionEvent(state, done("b"), true);
    applySessionAttentionEvent(state, { ...done("b"), messageId: "duplicate-terminal-reply" } as StreamEvent, true);
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

  it("does not let an old read or repeated/old terminal event consume a new result", () => {
    const state = emptySessionAttentionState();
    applySessionAttentionEvent(state, done("a"), true);
    const first = state.sessions.a!.sequence;
    applySessionAttentionEvent(state, { type: "runStart", sessionId: "a", runId: "second" }, true);
    applySessionAttentionEvent(state, done("a", "second"), true);
    readSessionAttention(state, "a", first);
    applySessionAttentionEvent(state, done("a"), true);
    expect(state.sessions.a!.runId).toBe("second");
    expect(isSessionUnread("a", state)).toBe(true);
  });

  it("ignores deltas and cancellation, and preserves unread attention when a new run starts", () => {
    const state = emptySessionAttentionState();
    applySessionAttentionEvent(state, done("a"), false);
    applySessionAttentionEvent(state, { type: "runStart", sessionId: "a", runId: "second" }, true);
    applySessionAttentionEvent(state, { type: "textDelta", sessionId: "a", runId: "second", text: "text" }, true);
    applySessionAttentionEvent(state, { type: "cancelled", sessionId: "a", runId: "second" }, true);
    expect(state.sequence).toBe(1);
    expect(state.sessions.a!.promoteCompletion).toBe(false);
    expect(isSessionUnread("a", state)).toBe(true);
  });

  it("does not promote questions or errors, and acknowledges manual placement until the next completion", () => {
    const state = emptySessionAttentionState();
    const tree = snapshot();
    applySessionAttentionEvent(state, { type: "askUser", sessionId: "a", runId: "first", questionId: "q", toolCallId: "tool", question: "Choose", options: [] }, true);
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    expect(isSessionUnread("a", state)).toBe(true);
    applySessionAttentionEvent(state, done("b"), true);
    acknowledgeSessionPromotions(state, tree, ["b"]);
    expect(pendingSessionPromotions(state, tree)).toEqual([]);
    applySessionAttentionEvent(state, done("b", "second"), true);
    expect(pendingSessionPromotions(state, tree)).toEqual(["b"]);
  });

  it("serializes concurrent events and reads and persists deduplication across reload", async () => {
    await Promise.all([
      updateSessionAttention(async (state) => { await Promise.resolve(); applySessionAttentionEvent(state, done("a"), true); }),
      updateSessionAttention((state) => applySessionAttentionEvent(state, done("b"), true)),
      updateSessionAttention((state) => applySessionAttentionEvent(state, done("a"), true)),
    ]);
    expect(sessionAttention.value.sessions.a!.completionSequence).toBe(1);
    expect(sessionAttention.value.sessions.b!.completionSequence).toBe(2);
    sessionAttention.value = emptySessionAttentionState();
    await updateSessionAttention((state) => {
      expect(state.sequence).toBe(2);
      applySessionAttentionEvent(state, done("a"), true);
      readSessionAttention(state, "a", 1);
    });
    expect(isSessionUnread("a")).toBe(false);
    expect(isSessionUnread("b")).toBe(true);
  });

  it("clears read indicators immediately while layout writes are still pending", async () => {
    await updateSessionAttention((state) => applySessionAttentionEvent(state, done("a"), true));
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const layoutWrite = updateSessionAttention(async (state) => {
      await blocked;
      state.promoted.layout = {};
    });
    const reading = markSessionAttentionRead("a", 1);
    expect(isSessionUnread("a")).toBe(false);
    const persisted = JSON.parse(localStorage.getItem("locus-session-attention-v1")!);
    expect(isSessionUnread("a", persisted)).toBe(true);
    window.dispatchEvent(new StorageEvent("storage", { key: "locus-session-attention-v1" }));
    expect(isSessionUnread("a")).toBe(false);
    release();
    await layoutWrite;
    expect(isSessionUnread("a")).toBe(false);
    await reading;
    expect(isSessionUnread("a", JSON.parse(localStorage.getItem("locus-session-attention-v1")!))).toBe(false);
  });

  it("preserves newer unread results when an earlier visible result is being saved", async () => {
    await updateSessionAttention((state) => applySessionAttentionEvent(state, done("a"), true));
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    const completion = updateSessionAttention(async (state) => {
      await blocked;
      applySessionAttentionEvent(state, done("a", "second"), true);
    });
    const reading = markSessionAttentionRead("a", 1);
    expect(isSessionUnread("a")).toBe(false);
    release();
    await completion;
    expect(isSessionUnread("a")).toBe(true);
    await reading;
    expect(sessionAttention.value.sessions.a!.readSequence).toBe(1);
    expect(isSessionUnread("a")).toBe(true);
  });
});
