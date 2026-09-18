// @vitest-environment jsdom
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceExplorerStore } from "../stores/workspaceExplorer";
import { useDisplaySettings } from "../composables/useDisplaySettings";
import { applySessionAttentionEvent, emptySessionAttentionState, sessionAttention, updateSessionAttention } from "../services/sessionAttention";
import type { ProjectExplorerSnapshot } from "../types/workbench";

const mocks = vi.hoisted(() => ({ read: vi.fn(), apply: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock("../services/workspaceExplorer", async (original) => ({
  ...await original<typeof import("../services/workspaceExplorer")>(),
  projectExplorerSnapshot: mocks.read, projectExplorerApplyOperations: mocks.apply,
}));

function snapshot(revision = 1): ProjectExplorerSnapshot {
  return { projectId: "p", presetId: "default", presetName: "Default", revision, manifestPath: "tree.json", presets: [],
    nodes: ["a", "b", "c"].map((id, position) => ({ nodeId: id, projectId: "p", nodeKind: "resource", resourceKind: "session", resourceId: id, hidden: false, position })) };
}
async function complete(id: string) {
  await updateSessionAttention((state) => applySessionAttentionEvent(state, { type: "done", sessionId: id, runId: `${id}-run`, messageId: `${id}-reply`, fullText: "Done" }, true));
}

describe("workspace completion promotion persistence", () => {
  beforeEach(() => {
    setActivePinia(createPinia()); localStorage.clear(); sessionAttention.value = emptySessionAttentionState(); vi.clearAllMocks();
    useDisplaySettings().set("autoPromoteCompletedSessions", true);
    mocks.read.mockResolvedValue(snapshot());
    mocks.apply.mockImplementation(async (_project, _revision, operations, operationId) => {
      const tree = snapshot(2);
      const moved = tree.nodes.find((node) => node.nodeId === operations[0]?.nodeId);
      if (moved) { tree.nodes = [moved, ...tree.nodes.filter((node) => node !== moved)]; tree.nodes.forEach((node, i) => { node.position = i; }); }
      return { operationId, snapshot: tree };
    });
  });

  it("defers during interaction, writes once afterwards and keeps the saved layout when disabled", async () => {
    const store = useWorkspaceExplorerStore(); store.snapshots.p = snapshot();
    await store.promoteCompletedSessions("p", () => true);
    await complete("c");
    await store.promoteCompletedSessions("p", () => false);
    expect(mocks.apply).not.toHaveBeenCalled();
    await Promise.all([store.promoteCompletedSessions("p", () => true), store.promoteCompletedSessions("p", () => true)]);
    expect(mocks.apply).toHaveBeenCalledTimes(1);
    expect(store.snapshots.p!.nodes.map((node) => node.nodeId)).toEqual(["c", "a", "b"]);
    useDisplaySettings().set("autoPromoteCompletedSessions", false);
    await complete("b");
    await store.promoteCompletedSessions("p", () => true);
    useDisplaySettings().set("autoPromoteCompletedSessions", true);
    await store.promoteCompletedSessions("p", () => true);
    expect(mocks.apply).toHaveBeenCalledTimes(1);
  });

  it("recalculates the block on a revision conflict instead of replaying stale positions", async () => {
    const store = useWorkspaceExplorerStore(); store.snapshots.p = snapshot();
    await store.promoteCompletedSessions("p", () => true);
    await complete("c");
    mocks.apply.mockRejectedValueOnce({ code: "workspace.explorer_revision_conflict", message: "changed" });
    const fresh = snapshot(3);
    fresh.nodes.splice(1, 0, { nodeId: "doc", projectId: "p", nodeKind: "resource", resourceKind: "knowledge", resourceId: "doc", position: 1, hidden: false });
    fresh.nodes.forEach((node, position) => { node.position = position; });
    mocks.read.mockResolvedValueOnce(snapshot()).mockResolvedValueOnce(fresh);
    await store.promoteCompletedSessions("p", () => true);
    expect(mocks.apply.mock.calls[1]?.[2]).toEqual([{ kind: "moveNode", nodeId: "c", parentNodeId: undefined, position: 2 }]);
  });

  it("preserves an explicit move over a pending completion", async () => {
    const store = useWorkspaceExplorerStore(); store.snapshots.p = snapshot();
    await store.promoteCompletedSessions("p", () => true);
    await complete("c");
    await store.applyOperations("p", [{ kind: "moveNode", nodeId: "c", position: 1 }]);
    await store.promoteCompletedSessions("p", () => true);
    expect(mocks.apply).toHaveBeenCalledTimes(1);
  });
});
