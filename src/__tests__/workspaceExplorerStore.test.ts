import { resetWorkspaceEventHubForTests } from "../services/workspaceEventHub";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useWorkspaceExplorerStore } from "../stores/workspaceExplorer";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { ProjectContextDescriptor, WorkspaceCheckoutDescriptor } from "../services/project";
import type {
  ProjectExplorerMutationResult,
  ProjectExplorerOperation,
  ProjectExplorerSnapshot,
} from "../types/workbench";

const eventMocks = vi.hoisted(() => ({
  listen: vi.fn(),
}));

const explorerMocks = vi.hoisted(() => ({
  projectExplorerSnapshot: vi.fn(),
  projectExplorerApplyOperations: vi.fn(),
  projectKnowledgeList: vi.fn(),
  projectCollaborationSnapshot: vi.fn(),
}));

const sessionMocks = vi.hoisted(() => ({
  listProjectSessions: vi.fn(),
}));
const viewMocks = vi.hoisted(() => ({ viewList: vi.fn() }));
vi.mock("../services/view", async (original) => ({
  ...await original<typeof import("../services/view")>(),
  viewList: viewMocks.viewList,
}));

const eventListeners = new Map<string, (event: { payload: unknown }) => void>();

vi.mock("@tauri-apps/api/event", () => eventMocks);
vi.mock("../services/workspaceExplorer", () => ({
  ...explorerMocks,
  PROJECT_EXPLORER_CHANGED_EVENT: "project-explorer-changed",
}));
vi.mock("../services/session", () => sessionMocks);

function snapshot(revision = 0): ProjectExplorerSnapshot {
  return {
    projectId: "project-a",
    presetId: "default",
    presetName: "Default",
    manifestPath: "F:/Project/Locus/workspace-trees/default.json",
    revision,
    nodes: [],
    presets: [{
      presetId: "default",
      name: "Default",
      revision,
      active: true,
      filePath: "F:/Project/Locus/workspace-trees/default.json",
    }],
  };
}

describe("workspace explorer store", () => {
  beforeEach(() => {
    resetWorkspaceEventHubForTests();
    setActivePinia(createPinia());
    vi.clearAllMocks();
    viewMocks.viewList.mockResolvedValue([]);
    eventListeners.clear();
    eventMocks.listen.mockImplementation((
      eventName: string,
      handler: (event: { payload: unknown }) => void,
    ) => {
      eventListeners.set(eventName, handler);
      return Promise.resolve(vi.fn());
    });
    explorerMocks.projectExplorerSnapshot.mockResolvedValue(snapshot());
    explorerMocks.projectCollaborationSnapshot.mockResolvedValue({
      projectId: "project-a",
      checkouts: [],
    });
    sessionMocks.listProjectSessions.mockResolvedValue([{
      id: "session-a",
      title: "Session A",
      sessionType: "chat",
      updatedAt: 1,
      projectId: "project-a",
      defaultCheckoutId: "checkout-a",
    }]);
    explorerMocks.projectKnowledgeList.mockResolvedValue([{
      id: "memory-a",
      type: "memory",
      path: "systems/combat/overview.md",
      title: "Combat Overview",
      modifiedAt: 2,
      sourceCheckoutId: "checkout-a",
      sourceWorkspaceGeneration: 1,
      sourceRoot: "F:/Project",
      availableCheckoutIds: ["checkout-a"],
    }]);
    explorerMocks.projectExplorerApplyOperations.mockImplementation((
      projectId: string,
      _revision: number,
      _operations: ProjectExplorerOperation[],
      operationId: string,
    ): Promise<ProjectExplorerMutationResult> => Promise.resolve({
      operationId,
      snapshot: { ...snapshot(1), projectId },
    }));
  });

  it("restores placed view names and icons from their checkout catalog", async () => {
    const checkout = {
      checkoutId: "checkout-a", projectId: "project-a", root: "F:/Project",
      runtime: { workspaceGeneration: 3, materializationEpoch: 7 },
    } as WorkspaceCheckoutDescriptor;
    useWorkspaceContextStore().projectsById["project-a"] = {
      projectId: "project-a", checkouts: [checkout],
    } as ProjectContextDescriptor;
    const stored = snapshot();
    stored.nodes.push({ nodeId: "view-node", projectId: "project-a", nodeKind: "resource", resourceKind: "view", resourceId: "combat", hidden: false, position: 5 });
    explorerMocks.projectExplorerSnapshot.mockResolvedValue(stored);
    viewMocks.viewList.mockResolvedValue([{ id: "combat", name: "Combat View", icon: "eye" }]);
    const store = useWorkspaceExplorerStore();
    await store.loadProject("project-a");
    expect(viewMocks.viewList).toHaveBeenCalledWith({ checkoutId: "checkout-a", expectedGeneration: 3, expectedMaterializationEpoch: 7 });
    expect(store.viewResources["project-a"]).toEqual([{
      projectId: "project-a",
      workspaceRef: { checkoutId: "checkout-a", expectedGeneration: 3, expectedMaterializationEpoch: 7 },
      view: { id: "combat", name: "Combat View", icon: "eye" },
    }]);
  });

  it("places newly discovered sessions after all default special nodes in an empty layout", async () => {
    const store = useWorkspaceExplorerStore();
    await store.loadProject("project-a");

    expect(explorerMocks.projectExplorerApplyOperations).toHaveBeenCalledTimes(1);
    const operations = (
      explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]
    ) as ProjectExplorerOperation[];
    expect(operations.some((operation) => operation.kind === "createFolder")).toBe(false);
    expect(operations).toContainEqual(expect.objectContaining({
      kind: "placeResource",
      resourceKind: "system",
      resourceId: "newSession",
      position: 0,
    }));
    expect(operations).toContainEqual(expect.objectContaining({
      kind: "placeResource",
      resourceKind: "system",
      resourceId: "knowledge",
      position: 1,
    }));
    const sessionPlacement = operations.find(
      (operation): operation is Extract<ProjectExplorerOperation, { kind: "placeResource" }> => (
        operation.kind === "placeResource" && operation.resourceKind === "session"
      ),
    );
    expect(sessionPlacement?.parentNodeId).toBeUndefined();
    expect(sessionPlacement?.position).toBe(7);
    expect(operations).toContainEqual(expect.objectContaining({
      kind: "placeResource",
      resourceKind: "system",
      resourceId: "collaboration",
      position: 2,
    }));
    expect(operations.slice(0, operations.indexOf(sessionPlacement!)).map((operation) => (
      operation.kind === "placeResource" ? operation.resourceId : operation.kind
    ))).toEqual(["newSession", "knowledge", "collaboration", "assets", "views", "agents", "archived"]);
  });

  it("keeps session placement available when the knowledge catalog fails to load", async () => {
    explorerMocks.projectKnowledgeList.mockRejectedValueOnce(new Error("knowledge unavailable"));
    const store = useWorkspaceExplorerStore();

    await store.loadProject("project-a");

    const operations = (
      explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]
    ) as ProjectExplorerOperation[];
    expect(operations).toContainEqual(expect.objectContaining({
      kind: "placeResource",
      resourceKind: "session",
      resourceId: "session-a",
    }));
    expect(store.errors["project-a"]).toBe("knowledge unavailable");
  });

  it("places subagent sessions beneath their parent session", async () => {
    sessionMocks.listProjectSessions.mockResolvedValueOnce([{
      id: "subagent-a",
      title: "Inspect code",
      sessionType: "chat",
      parentSessionId: "session-a",
      updatedAt: 2,
      projectId: "project-a",
      defaultCheckoutId: "checkout-a",
    }, {
      id: "session-a",
      title: "Session A",
      sessionType: "chat",
      parentSessionId: null,
      updatedAt: 1,
      projectId: "project-a",
      defaultCheckoutId: "checkout-a",
    }]);
    const store = useWorkspaceExplorerStore();

    await store.loadProject("project-a");

    const operations = (
      explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]
    ) as ProjectExplorerOperation[];
    const sessionPlacements = operations.filter(
      (operation): operation is Extract<ProjectExplorerOperation, { kind: "placeResource" }> => (
        operation.kind === "placeResource" && operation.resourceKind === "session"
      ),
    );
    expect(sessionPlacements.map((operation) => operation.resourceId))
      .toEqual(["session-a", "subagent-a"]);
    expect(sessionPlacements[0]?.nodeId).toEqual(expect.any(String));
    expect(sessionPlacements[1]?.parentNodeId).toBe(sessionPlacements[0]?.nodeId);
    expect(sessionPlacements[1]?.position).toBe(0);
  });

  it("places a newly created session before the first following session", async () => {
    const store = useWorkspaceExplorerStore();
    await store.loadProject("project-a");
    store.snapshots["project-a"] = {
      ...snapshot(1),
      nodes: [{
        nodeId: "system:new-session",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "newSession",
        hidden: false,
        position: 0,
      }, {
        nodeId: "system:knowledge",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "knowledge",
        hidden: false,
        position: 1,
      }, {
        nodeId: "session:session-a",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "session",
        resourceId: "session-a",
        hidden: false,
        position: 2,
      }, {
        nodeId: "system:collaboration",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "collaboration",
        hidden: false,
        position: 3,
      }],
    };
    explorerMocks.projectExplorerApplyOperations.mockClear();
    sessionMocks.listProjectSessions.mockResolvedValueOnce([
      {
        id: "session-a",
        title: "Session A",
        sessionType: "chat",
        updatedAt: 1,
        projectId: "project-a",
        defaultCheckoutId: "checkout-a",
      },
      {
        id: "session-b",
        title: "Session B",
        sessionType: "chat",
        updatedAt: 2,
        projectId: "project-a",
        defaultCheckoutId: "checkout-a",
      },
    ]);

    await store.refreshProjectSessions("project-a");

    expect(store.resources["project-a"].sessions.map((session) => session.id))
      .toEqual(["session-a", "session-b"]);
    expect(explorerMocks.projectExplorerApplyOperations).toHaveBeenCalledTimes(1);
    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2].filter(
      (operation: { resourceKind?: string }) => operation.resourceKind === "session",
    )).toEqual([
      expect.objectContaining({
        kind: "placeResource",
        resourceKind: "session",
        resourceId: "session-b",
        position: 2,
      }),
    ]);
  });

  it("places a newly created session after consecutive special nodes when none follows", async () => {
    const store = useWorkspaceExplorerStore();
    await store.loadProject("project-a");
    store.snapshots["project-a"] = {
      ...snapshot(1),
      nodes: [{
        nodeId: "system:new-session",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "newSession",
        hidden: false,
        position: 0,
      }, {
        nodeId: "system:collaboration",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "collaboration",
        hidden: false,
        position: 1,
      }, {
        nodeId: "system:knowledge",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "knowledge",
        hidden: false,
        position: 2,
      }],
    };
    explorerMocks.projectExplorerApplyOperations.mockClear();
    sessionMocks.listProjectSessions.mockResolvedValueOnce([{
      id: "session-b",
      title: "Session B",
      sessionType: "chat",
      updatedAt: 2,
      projectId: "project-a",
      defaultCheckoutId: "checkout-a",
    }]);

    await store.refreshProjectSessions("project-a");

    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2].filter(
      (operation: { resourceKind?: string }) => operation.resourceKind === "session",
    )).toEqual([{
      kind: "placeResource",
      nodeId: expect.any(String),
      resourceKind: "session",
      resourceId: "session-b",
      parentNodeId: undefined,
      position: 7,
    }]);
  });

  it.each([
    { boundary: "end", existingSession: false, parentNodeId: undefined },
    { boundary: "folder", existingSession: false, parentNodeId: undefined },
    { boundary: "file", existingSession: false, parentNodeId: undefined },
    { boundary: "view", existingSession: false, parentNodeId: undefined },
    { boundary: "folder", existingSession: true, parentNodeId: undefined },
    { boundary: "end", existingSession: false, parentNodeId: "folder:chat" },
  ])("groups new sessions after knowledge documents: $boundary, existing=$existingSession, parent=$parentNodeId", async ({
    boundary, existingSession, parentNodeId,
  }) => {
    const stored = snapshot();
    const resources = [
      ["system", "newSession"],
      ["system", "archived"],
      ["system", "knowledge"],
      ["knowledge", "plan-a"],
      ["knowledge", "design-a"],
      ...(boundary === "end" ? [] : [[boundary, "boundary"]]),
      ...(existingSession ? [["session", "existing"]] : []),
      ["system", "collaboration"],
      ["system", "assets"],
      ["system", "views"],
      ["system", "agents"],
    ];
    stored.nodes = resources.map<ProjectExplorerSnapshot["nodes"][number]>(([resourceKind, resourceId], index) => ({
      nodeId: `${resourceKind}:${resourceId}`,
      projectId: "project-a",
      nodeKind: resourceKind === "folder" ? "folder" : "resource",
      resourceKind: resourceKind === "folder" ? undefined : resourceKind,
      resourceId: resourceKind === "folder" ? undefined : resourceId,
      folderName: resourceKind === "folder" ? "Custom folder" : undefined,
      sourcePath: resourceKind === "file" ? "F:/Project/notes.md" : undefined,
      parentNodeId,
      position: index * 10,
      hidden: false,
    })).reverse();
    if (parentNodeId) {
      stored.nodes.push({
        nodeId: parentNodeId, projectId: "project-a", nodeKind: "folder",
        folderName: "Chat", position: 0, hidden: false,
      });
    }
    explorerMocks.projectExplorerSnapshot.mockResolvedValueOnce(stored);
    sessionMocks.listProjectSessions.mockResolvedValueOnce([
      ...(existingSession ? ["existing"] : []), "new-a", "new-b",
    ].map((id) => ({
      id, title: id, sessionType: "chat", updatedAt: 1,
      projectId: "project-a", defaultCheckoutId: "checkout-a",
    })));
    const store = useWorkspaceExplorerStore();

    await store.loadProject("project-a");

    const position = boundary === "end" ? resources.length : existingSession ? 6 : 5;
    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]).toEqual(
      ["new-a", "new-b"].map((resourceId, index) => ({
        kind: "placeResource", nodeId: expect.any(String), resourceKind: "session",
        resourceId, parentNodeId, position: position + index,
      })),
    );
  });

  it("keeps a newly created session beside a nested New Session node", async () => {
    const store = useWorkspaceExplorerStore();
    await store.loadProject("project-a");
    store.snapshots["project-a"] = {
      ...snapshot(1),
      nodes: [{
        nodeId: "folder:chat",
        projectId: "project-a",
        nodeKind: "folder",
        folderName: "Chat",
        hidden: false,
        position: 0,
      }, {
        nodeId: "system:new-session",
        projectId: "project-a",
        nodeKind: "resource",
        parentNodeId: "folder:chat",
        resourceKind: "system",
        resourceId: "newSession",
        hidden: false,
        position: 0,
      }, {
        nodeId: "system:knowledge",
        projectId: "project-a",
        nodeKind: "resource",
        parentNodeId: "folder:chat",
        resourceKind: "system",
        resourceId: "knowledge",
        hidden: false,
        position: 1,
      }, {
        nodeId: "session:session-a",
        projectId: "project-a",
        nodeKind: "resource",
        parentNodeId: "folder:chat",
        resourceKind: "session",
        resourceId: "session-a",
        hidden: false,
        position: 2,
      }, {
        nodeId: "system:collaboration",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "collaboration",
        hidden: false,
        position: 1,
      }],
    };
    explorerMocks.projectExplorerApplyOperations.mockClear();
    sessionMocks.listProjectSessions.mockResolvedValueOnce([{
      id: "session-b",
      title: "Session B",
      sessionType: "chat",
      updatedAt: 2,
      projectId: "project-a",
      defaultCheckoutId: "checkout-a",
    }, {
      id: "session-a",
      title: "Session A",
      sessionType: "chat",
      updatedAt: 1,
      projectId: "project-a",
      defaultCheckoutId: "checkout-a",
    }]);

    await store.refreshProjectSessions("project-a");

    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2].filter(
      (operation: { resourceKind?: string }) => operation.resourceKind === "session",
    )).toEqual([{
      kind: "placeResource",
      nodeId: expect.any(String),
      resourceKind: "session",
      resourceId: "session-b",
      parentNodeId: "folder:chat",
      position: 2,
    }]);
  });

  it("places a newly created Plan document directly below the Knowledge node", async () => {
    const store = useWorkspaceExplorerStore();
    store.displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments = true;
    await store.loadProject("project-a");
    store.snapshots["project-a"] = {
      ...snapshot(1),
      nodes: [{
        nodeId: "folder:project-notes",
        projectId: "project-a",
        nodeKind: "folder",
        folderName: "Project Notes",
        hidden: false,
        position: 0,
      }, {
        nodeId: "system:knowledge",
        projectId: "project-a",
        nodeKind: "resource",
        parentNodeId: "folder:project-notes",
        resourceKind: "system",
        resourceId: "knowledge",
        hidden: false,
        position: 0,
      }, {
        nodeId: "folder:existing",
        projectId: "project-a",
        nodeKind: "folder",
        parentNodeId: "folder:project-notes",
        folderName: "Existing",
        hidden: false,
        position: 1,
      }],
    };
    const existing = store.resources["project-a"].knowledge[0]!;
    const plan = {
      ...existing,
      id: "plan-rollout",
      type: "plan" as const,
      path: "rollout.md",
      title: "Rollout",
      modifiedAt: 3,
    };
    explorerMocks.projectKnowledgeList.mockResolvedValueOnce([existing, plan]);
    explorerMocks.projectExplorerApplyOperations.mockClear();

    eventListeners.get("locus://workspace-event")?.({
      payload: {
        eventName: "knowledge-changed",
        streamRevision: 2,
        projectId: "project-a",
        checkoutId: "checkout-a",
        workspaceGeneration: 1,
        payload: {
          workingDir: "F:/Project",
          source: "agent_knowledge_tool",
          changedAt: 2,
        },
      },
    });

    await vi.waitFor(() => {
      expect(explorerMocks.projectExplorerApplyOperations).toHaveBeenCalledTimes(1);
    });
    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]).toEqual([{
      kind: "placeResource",
      resourceKind: "knowledge",
      resourceId: "plan-rollout",
      sourceKind: "knowledge",
      parentNodeId: "folder:project-notes",
      position: 1,
    }]);
    expect(store.resources["project-a"].knowledge).toContainEqual(plan);
  });

  it("keeps new Plan and Design documents out of the tree when auto-placement is disabled", async () => {
    const store = useWorkspaceExplorerStore();
    await store.loadProject("project-a");
    store.snapshots["project-a"] = {
      ...snapshot(1),
      nodes: [{
        nodeId: "system:knowledge",
        projectId: "project-a",
        nodeKind: "resource",
        resourceKind: "system",
        resourceId: "knowledge",
        hidden: false,
        position: 0,
      }],
    };
    const existing = store.resources["project-a"].knowledge[0]!;
    explorerMocks.projectKnowledgeList.mockResolvedValueOnce([{
      ...existing,
      id: "design-input",
      type: "design",
      path: "input.md",
      title: "Input",
      modifiedAt: 4,
    }]);
    explorerMocks.projectExplorerApplyOperations.mockClear();
    store.displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments = false;

    await store.refreshProjectKnowledge("project-a");

    expect(explorerMocks.projectExplorerApplyOperations).not.toHaveBeenCalled();
    store.displaySettings.autoPlaceNewPlanDesignKnowledgeDocuments = true;
  });

  it("leaves legacy knowledge placements untouched", async () => {
    explorerMocks.projectExplorerSnapshot.mockResolvedValueOnce({
      ...snapshot(4),
      nodes: [{
        nodeId: "knowledge-type:project-a:memory",
        projectId: "project-a",
        nodeKind: "folder",
        parentNodeId: "folder:custom",
        folderName: "Notes",
        hidden: false,
        position: 0,
      }],
    });
    const store = useWorkspaceExplorerStore();

    await store.loadProject("project-a");

    const operations = (
      explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]
    ) as ProjectExplorerOperation[];
    expect(operations.some((operation) => (
      "nodeId" in operation && operation.nodeId === "knowledge-type:project-a:memory"
    ))).toBe(false);
  });

  it("reloads once and retries the same operation after a revision conflict", async () => {
    const store = useWorkspaceExplorerStore();
    store.snapshots["project-a"] = snapshot(1);
    explorerMocks.projectExplorerApplyOperations
      .mockRejectedValueOnce({
        code: "workspace.explorer_revision_conflict",
        message: "changed",
        retryable: true,
      })
      .mockImplementationOnce((
        projectId: string,
        _revision: number,
        _operations: ProjectExplorerOperation[],
        operationId: string,
      ) => Promise.resolve({
        operationId,
        snapshot: { ...snapshot(3), projectId },
      }));
    explorerMocks.projectExplorerSnapshot.mockResolvedValueOnce(snapshot(2));

    const result = await store.applyOperations("project-a", [{
      kind: "createFolder",
      name: "Tasks",
      position: 0,
    }]);

    expect(result.revision).toBe(3);
    expect(explorerMocks.projectExplorerApplyOperations).toHaveBeenCalledTimes(2);
    expect(explorerMocks.projectExplorerApplyOperations.mock.calls.map((call) => call[1]))
      .toEqual([1, 2]);
    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[3])
      .toBe(explorerMocks.projectExplorerApplyOperations.mock.calls[1]?.[3]);
  });

  it("pins existing files and sessions without moving their original placement", async () => {
    const store = useWorkspaceExplorerStore();
    store.snapshots["project-a"] = { ...snapshot(4), nodes: [
      { nodeId: "file", projectId: "project-a", nodeKind: "resource", sourcePath: "F:/Project/notes.md", parentNodeId: "folder", position: 3, hidden: false },
      { nodeId: "session", projectId: "project-a", nodeKind: "resource", resourceKind: "session", resourceId: "session-a", parentNodeId: "folder", position: 4, hidden: false },
    ] };
    await store.pinResources("project-a", [
      { kind: "mountPath", path: "f:\\Project\\notes.md", position: 0 },
      { kind: "placeResource", resourceKind: "session", resourceId: "session-a", position: 0 },
    ]);
    expect(explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2]).toEqual([
      { kind: "setItemState", nodeId: "file", pinned: true },
      { kind: "setItemState", nodeId: "session", pinned: true },
    ]);
  });

  it("does not overwrite a pin with an earlier mutation response arriving late", async () => {
    const store = useWorkspaceExplorerStore();
    store.snapshots["project-a"] = snapshot(1);
    let finishEarlier!: (result: ProjectExplorerMutationResult) => void;
    explorerMocks.projectExplorerApplyOperations.mockImplementationOnce(() => new Promise((resolve) => { finishEarlier = resolve; }));
    const earlier = store.applyOperations("project-a", [{ kind: "renameFolder", nodeId: "folder", name: "Tasks" }]);
    const pinned = { ...snapshot(3), itemStates: [{ nodeId: "session", pinned: true, highlighted: true }] };
    explorerMocks.projectExplorerApplyOperations.mockResolvedValueOnce({ operationId: "pin", snapshot: pinned });
    await store.applyOperations("project-a", [{ kind: "setItemState", nodeId: "session", pinned: true }]);
    finishEarlier({ operationId: "earlier", snapshot: snapshot(2) });
    await earlier;
    expect(store.snapshots["project-a"]).toEqual(pinned);
  });

  it("creates and pins a new dropped file in one atomic batch and deduplicates it", async () => {
    const store = useWorkspaceExplorerStore();
    store.snapshots["project-a"] = snapshot(2);
    await store.pinResources("project-a", [
      { kind: "mountPath", path: "F:/Project/new.md", position: 0 },
      { kind: "mountPath", path: "F:/Project/new.md", position: 1 },
    ]);
    const operations = explorerMocks.projectExplorerApplyOperations.mock.calls[0]?.[2] as ProjectExplorerOperation[];
    expect(operations).toHaveLength(2);
    expect(operations[0]).toMatchObject({ kind: "mountPath", path: "F:/Project/new.md", nodeId: expect.any(String) });
    expect(operations[1]).toEqual({ kind: "setItemState", nodeId: "nodeId" in operations[0]! ? operations[0].nodeId : "", pinned: true });
    expect(explorerMocks.projectExplorerApplyOperations).toHaveBeenCalledTimes(1);
  });
});
