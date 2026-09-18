import { readFileSync } from "node:fs";
import ts from "typescript";
import { ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { workspaceTreeMoveOperations } from "../components/explorer/workspaceTreeDropPreview";
import { workspaceTreeItemState } from "../components/explorer/workspaceTreeItemState";

// Exercise the production drop transaction with only persistence mocked.
const source = readFileSync("src/components/workbench/DevelopmentWorkbench.vue", "utf8")
  .split('<script setup lang="ts">')[1]!.split("</script>")[0]!;
const parsed = ts.createSourceFile("workbench.ts", source, ts.ScriptTarget.Latest, true);
const names = ["moveExplorerNodeToIntent", "canMoveExplorerNodeToIntent", "restoreArchivedDropItems", "ensureArchivedDropPlacements"];
const handlers = parsed.statements.filter((statement) => ts.isFunctionDeclaration(statement)
  && statement.name && names.includes(statement.name.text)).map((statement) => statement.getText(parsed)).join("\n");
const code = ts.transpileModule(handlers, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

function fixture() {
  const item = (id: string, archived = true) => ({
    meta: { kind: "session", projectId: "p", archived, session: { id },
      explorerNode: { nodeId: id, projectId: "p", nodeKind: "resource", resourceKind: "session", resourceId: id, position: 0, parentNodeId: null, hidden: true } },
  });
  const first = item("first"), second = item("second");
  const applyOperations = vi.fn().mockResolvedValue(undefined);
  const restoreArchivedSession = vi.fn().mockResolvedValue(undefined);
  const addNotice = vi.fn();
  const expanded = ref(new Set<string>());
  const deps = {
    explorerStore: { snapshots: { p: { nodes: [first.meta.explorerNode, second.meta.explorerNode,
      { nodeId: "folder", nodeKind: "folder" }], itemStates: [] } }, applyOperations },
    workspaceTreeMoveOperations, workspaceTreeItemState, restoreArchivedSession,
    resetSessionMultiSelection: vi.fn(),
    notificationStore: { addNotice }, normalizeAppError: (error: Error) => error, expanded,
  };
  const move = new Function(...Object.keys(deps), `${code}\nreturn moveExplorerNodeToIntent;`)(...Object.values(deps));
  return { first, second, move, applyOperations, restoreArchivedSession, expanded, addNotice, snapshot: deps.explorerStore.snapshots.p };
}

describe("restore archived sessions by dropping into the workspace tree", () => {
  it.each([null, "folder"])("restores after placing every selected session under %s", async (parentNodeId) => {
    const f = fixture();
    let finish!: () => void;
    f.applyOperations.mockReturnValue(new Promise<void>((resolve) => { finish = resolve; }));
    const move = f.move(f.first, { projectId: "p", parentNodeId, position: 2 }, [f.first, f.second]);
    expect(f.restoreArchivedSession).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(f.applyOperations).toHaveBeenCalled());
    const operations = f.applyOperations.mock.calls[0]![1];
    expect(operations.filter((op: { kind: string }) => op.kind === "moveNode")).toEqual([
      expect.objectContaining({ nodeId: "first", parentNodeId }),
      expect.objectContaining({ nodeId: "second", parentNodeId }),
    ]);
    expect(operations).toContainEqual({ kind: "setNodeHidden", nodeId: "first", hidden: false });
    finish();
    await move;
    expect(f.restoreArchivedSession.mock.calls).toEqual([["first", "p"], ["second", "p"]]);
    if (parentNodeId) expect(f.expanded.value.has("folder:p:folder")).toBe(true);
  });

  it("keeps sessions archived when placement fails or the target is invalid", async () => {
    const f = fixture();
    f.applyOperations.mockRejectedValue(new Error("write failed"));
    await f.move(f.first, { projectId: "p", parentNodeId: "folder", position: 0 });
    expect(f.addNotice).toHaveBeenCalled();
    expect(f.restoreArchivedSession).not.toHaveBeenCalled();
    f.applyOperations.mockClear();
    await f.move(f.first, { projectId: "other", parentNodeId: null, position: 0 });
    await f.move(f.first, { projectId: "p", parentNodeId: "first", position: 0 });
    expect(f.applyOperations).not.toHaveBeenCalled();
  });

  it("recreates a placement when the current preset has never contained the archive", async () => {
    const f = fixture();
    f.snapshot.nodes = f.snapshot.nodes.filter((node) => node.nodeId !== "first");
    f.applyOperations.mockImplementation(async (_projectId, operations) => {
      if (operations.some((operation: { kind: string }) => operation.kind === "placeResource")) {
        f.snapshot.nodes.push(f.first.meta.explorerNode);
      }
    });
    await f.move(f.first, { projectId: "p", parentNodeId: "folder", position: 0 });
    expect(f.applyOperations.mock.calls[0]![1]).toEqual([
      expect.objectContaining({ kind: "placeResource", nodeId: "first", resourceId: "first" }),
    ]);
    expect(f.applyOperations.mock.calls[1]![1]).toContainEqual(
      expect.objectContaining({ kind: "moveNode", nodeId: "first", parentNodeId: "folder" }),
    );
    expect(f.restoreArchivedSession).toHaveBeenCalledWith("first", "p");
  });
});
