import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useWorkspaceExplorerStore } from "../stores/workspaceExplorer";
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
    setActivePinia(createPinia());
    vi.clearAllMocks();
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

  it("anchors newly discovered sessions directly below New Session in an empty layout", async () => {
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
    expect(sessionPlacement?.position).toBe(1);
    expect(operations).toContainEqual(expect.objectContaining({
      kind: "placeResource",
      resourceKind: "system",
      resourceId: "collaboration",
      position: 3,
    }));
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

  it("places a newly created session directly below New Session when none follows", async () => {
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
      position: 1,
    }]);
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
});
