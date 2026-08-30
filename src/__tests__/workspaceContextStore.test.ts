import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type {
  ProjectContextDescriptor,
  WindowPaneWorkspaceContext,
  WorkspaceCheckoutDescriptor,
  RoutedWorkspaceEvent,
  WorkspaceRuntimeDescriptor,
} from "../services/project";

const projectServiceMocks = vi.hoisted(() => ({
  getWorkingDir: vi.fn(),
  setWorkingDir: vi.fn(),
  listProjectContexts: vi.fn(),
  listWindowWorkspaceContexts: vi.fn(),
  listWindowWorkspaceIntentEpochs: vi.fn(),
  openWorkspace: vi.fn(),
  focusWorkspace: vi.fn(),
  setActiveWorkspaceSession: vi.fn(),
  detachWorkspacePane: vi.fn(),
  detachWorkspaceWindow: vi.fn(),
}));

vi.mock("../services/project", () => projectServiceMocks);

function runtime(
  checkoutId: string,
  projectId = "project-1",
  generation = 1,
  detectedServices: string[] = [],
): WorkspaceRuntimeDescriptor {
  return {
    projectId,
    checkoutId,
    root: `F:/work/${checkoutId}`,
    workspaceGeneration: generation,
    leaseCount: 1,
    detectedServices,
  };
}

function checkout(
  checkoutId: string,
  options: { projectId?: string; runtime?: WorkspaceRuntimeDescriptor | null } = {},
): WorkspaceCheckoutDescriptor {
  const projectId = options.projectId ?? "project-1";
  return {
    checkoutId,
    projectId,
    root: `F:/work/${checkoutId}`,
    normalizedRoot: `f:/work/${checkoutId}`,
    lastOpenedAt: 1,
    runtime: options.runtime,
  };
}

function project(...checkouts: WorkspaceCheckoutDescriptor[]): ProjectContextDescriptor {
  return {
    projectId: checkouts[0]?.projectId ?? "project-1",
    detectedServices: [],
    checkouts,
  };
}

function paneContext(
  checkoutId: string,
  revision: number,
  options: {
    generation?: number;
    activeSessionId?: string | null;
    windowId?: string;
    paneId?: string;
    intentEpoch?: number;
  } = {},
): WindowPaneWorkspaceContext {
  return {
    windowId: options.windowId ?? "main",
    paneId: options.paneId ?? "main",
    focusedCheckoutId: checkoutId,
    workspaceGeneration: options.generation ?? 1,
    activeSessionId: options.activeSessionId ?? null,
    intentEpoch: options.intentEpoch ?? revision,
    revision,
  };
}

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function workspaceEvent(
  checkoutId: string,
  streamRevision: number,
  options: { projectId?: string; generation?: number; payload?: unknown } = {},
): RoutedWorkspaceEvent {
  return {
    eventName: "knowledge-changed",
    streamRevision,
    projectId: options.projectId ?? "project-1",
    checkoutId,
    workspaceGeneration: options.generation ?? 1,
    serviceInstanceId: null,
    serviceGeneration: null,
    payload: options.payload ?? { checkoutId },
  };
}

describe("workspace context store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    projectServiceMocks.listProjectContexts.mockResolvedValue([]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([]);
    projectServiceMocks.openWorkspace.mockImplementation((path: string) => {
      const segments = path.replaceAll("\\", "/").split("/");
      const checkoutId = segments[segments.length - 1] || "checkout-opened";
      return Promise.resolve(runtime(checkoutId));
    });
    projectServiceMocks.detachWorkspacePane.mockResolvedValue(true);
    projectServiceMocks.detachWorkspaceWindow.mockResolvedValue(0);
  });

  it("restores a pane and exposes its checkout runtime through explicit scope", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a", "project-a", 7) });
    checkoutA.projectId = "project-a";
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 4, { generation: 7, windowId: "window-1", paneId: "pane-1" }),
    ]);
    const store = useWorkspaceContextStore();

    await store.initialize("window-1", "pane-1");

    expect(store.projectsById["project-a"]?.checkouts).toHaveLength(1);
    expect(store.checkoutsById["checkout-a"]?.runtime?.workspaceGeneration).toBe(7);
    expect(store.focusedPaneContext?.revision).toBe(4);
    expect(store.focusedRoot).toBe("F:/work/checkout-a");
    expect(store.focusedWorkspaceRef).toEqual({
      checkoutId: "checkout-a",
      expectedGeneration: 7,
    });
  });

  it("focuses and updates active sessions for independent editor panes", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.focusWorkspace.mockImplementation(
      (
        targetWindowId: string,
        targetPaneId: string,
        workspaceRef: { checkoutId: string },
        intentEpoch: number,
      ) => Promise.resolve(paneContext(workspaceRef.checkoutId, 1, {
        windowId: targetWindowId,
        paneId: targetPaneId,
        intentEpoch,
      })),
    );
    projectServiceMocks.setActiveWorkspaceSession.mockImplementation(
      (
        targetWindowId: string,
        targetPaneId: string,
        sessionId: string,
        intentEpoch: number,
      ) => {
        const checkoutId = targetPaneId === "left" ? "checkout-a" : "checkout-b";
        return Promise.resolve(paneContext(checkoutId, 2, {
          windowId: targetWindowId,
          paneId: targetPaneId,
          activeSessionId: sessionId,
          intentEpoch,
        }));
      },
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.focusCheckoutInPane("checkout-a", "main", "left", { activate: false });
    await store.focusCheckoutInPane("checkout-b", "main", "right");
    await store.setActiveSessionInPane("session-a", "main", "left", { activate: false });
    await store.setActiveSessionInPane("session-b", "main", "right");

    expect(store.workspaceRefForPane("main", "left")).toEqual({
      checkoutId: "checkout-a",
      expectedGeneration: 1,
    });
    expect(store.workspaceRefForPane("main", "right")).toEqual({
      checkoutId: "checkout-b",
      expectedGeneration: 1,
    });
    expect(store.paneContextAt("main", "left")?.activeSessionId).toBe("session-a");
    expect(store.paneContextAt("main", "right")?.activeSessionId).toBe("session-b");
    expect(store.paneId).toBe("right");
    expect(store.focusedCheckout?.checkoutId).toBe("checkout-b");
  });

  it("keeps the latest checkout intent when focus responses complete in reverse order", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1),
    ]);
    const slowA = deferred<WindowPaneWorkspaceContext>();
    projectServiceMocks.focusWorkspace.mockImplementation(
      (
        _windowId: string,
        _paneId: string,
        workspaceRef: { checkoutId: string },
        intentEpoch: number,
      ) => (
        workspaceRef.checkoutId === "checkout-a"
          ? slowA.promise
          : Promise.resolve(paneContext("checkout-b", 2, { intentEpoch }))
      ),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    const first = store.focusCheckout("checkout-a");
    const second = store.focusCheckout("checkout-b");
    await second;
    // Even a numerically newer stale response belongs to the older local intent.
    slowA.resolve(paneContext("checkout-a", 3, { intentEpoch: 2 }));
    expect(await first).toBeNull();

    expect(store.focusedCheckout?.checkoutId).toBe("checkout-b");
    expect(store.focusedPaneContext?.revision).toBe(2);
  });

  it("shares one epoch across session and focus mutations completed in reverse order", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1),
    ]);
    const slowSession = deferred<WindowPaneWorkspaceContext>();
    projectServiceMocks.setActiveWorkspaceSession.mockReturnValue(slowSession.promise);
    projectServiceMocks.focusWorkspace.mockImplementation(
      (
        _windowId: string,
        _paneId: string,
        workspaceRef: { checkoutId: string },
        intentEpoch: number,
      ) => Promise.resolve(paneContext(workspaceRef.checkoutId, 2, { intentEpoch })),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    const sessionIntent = store.setActiveSession("session-a");
    const focusIntent = store.focusCheckout("checkout-b");
    await focusIntent;
    slowSession.resolve(paneContext("checkout-a", 3, {
      activeSessionId: "session-a",
      intentEpoch: 2,
    }));

    expect(await sessionIntent).toBeNull();
    expect(projectServiceMocks.setActiveWorkspaceSession).toHaveBeenCalledWith(
      "main",
      "main",
      "session-a",
      2,
    );
    expect(projectServiceMocks.focusWorkspace).toHaveBeenCalledWith(
      "main",
      "main",
      { checkoutId: "checkout-b", expectedGeneration: 1 },
      3,
    );
    expect(store.focusedCheckout?.checkoutId).toBe("checkout-b");
    expect(store.focusedPaneContext?.intentEpoch).toBe(3);
  });

  it("keeps a pane detached when an older focus response arrives afterwards", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1),
    ]);
    const slowFocus = deferred<WindowPaneWorkspaceContext>();
    projectServiceMocks.focusWorkspace.mockReturnValue(slowFocus.promise);
    const store = useWorkspaceContextStore();
    await store.initialize();

    const focusIntent = store.focusCheckout("checkout-b");
    expect(await store.disposePane()).toBe(true);
    slowFocus.resolve(paneContext("checkout-b", 2, { intentEpoch: 2 }));

    expect(await focusIntent).toBeNull();
    expect(projectServiceMocks.detachWorkspacePane).toHaveBeenCalledWith("main", "main", 3);
    expect(store.focusedPaneContext).toBeNull();
  });

  it("rejects a response older than the current backend revision", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-b", 8),
    ]);
    projectServiceMocks.focusWorkspace.mockResolvedValue(
      paneContext("checkout-a", 7, { intentEpoch: 9 }),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    expect(await store.focusCheckout("checkout-a")).toBeNull();
    expect(store.focusedCheckout?.checkoutId).toBe("checkout-b");
    expect(store.focusedPaneContext?.revision).toBe(8);
  });

  it("opens an unloaded persisted checkout before focusing it", async () => {
    const checkoutA = checkout("checkout-a", { runtime: null });
    const openedRuntime = runtime("checkout-a", "project-1", 5);
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    projectServiceMocks.openWorkspace.mockResolvedValue(openedRuntime);
    projectServiceMocks.focusWorkspace.mockResolvedValue(
      paneContext("checkout-a", 1, { generation: 5 }),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.focusCheckout(checkoutA);

    expect(projectServiceMocks.openWorkspace).toHaveBeenCalledWith("F:/work/checkout-a");
    expect(projectServiceMocks.focusWorkspace).toHaveBeenCalledWith("main", "main", {
      checkoutId: "checkout-a",
      expectedGeneration: 5,
    }, 1);
    expect(store.focusedRuntime).toEqual(openedRuntime);
  });

  it("re-registers a cached background checkout before focusing it", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a", "project-1", 2) });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b", "project-1", 3) });
    const reopenedRuntime = runtime("checkout-b", "project-1", 9);
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1, { generation: 2 }),
    ]);
    projectServiceMocks.openWorkspace.mockResolvedValue(reopenedRuntime);
    projectServiceMocks.focusWorkspace.mockImplementation(
      (
        _windowId: string,
        _paneId: string,
        workspaceRef: { checkoutId: string },
        intentEpoch: number,
      ) => Promise.resolve(paneContext(workspaceRef.checkoutId, 2, {
        generation: 9,
        intentEpoch,
      })),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.focusCheckout("checkout-b");

    expect(projectServiceMocks.openWorkspace).toHaveBeenCalledWith("F:/work/checkout-b");
    expect(projectServiceMocks.focusWorkspace).toHaveBeenCalledWith(
      "main",
      "main",
      { checkoutId: "checkout-b", expectedGeneration: 9 },
      2,
    );
    expect(store.focusedWorkspaceRef).toEqual({
      checkoutId: "checkout-b",
      expectedGeneration: 9,
    });
  });

  it("continues above a detached window tombstone when a window label is recreated", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([{
      windowId: "main",
      paneId: null,
      intentEpoch: 8,
    }]);
    projectServiceMocks.focusWorkspace.mockImplementation(
      (
        windowId: string,
        paneId: string,
        workspaceRef: { checkoutId: string },
        intentEpoch: number,
      ) => Promise.resolve(paneContext(workspaceRef.checkoutId, 1, {
        windowId,
        paneId,
        intentEpoch,
      })),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.focusCheckout("checkout-a");

    expect(projectServiceMocks.focusWorkspace).toHaveBeenCalledWith(
      "main",
      "main",
      { checkoutId: "checkout-a", expectedGeneration: 1 },
      9,
    );
    expect(store.focusedPaneContext?.intentEpoch).toBe(9);
  });

  it("does not restore a pane snapshot older than a concurrently observed window tombstone", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 4, { intentEpoch: 5 }),
    ]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([{
      windowId: "main",
      paneId: null,
      intentEpoch: 6,
    }]);
    const store = useWorkspaceContextStore();

    await store.initialize();

    expect(store.focusedPaneContext).toBeNull();
  });

  it("treats an explicit null runtime as retirement and a missing field as no update", async () => {
    const activeRuntime = runtime("checkout-a", "project-1", 7);
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project(checkout("checkout-a", { runtime: activeRuntime })),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();
    expect(store.applyWorkspaceEvent(workspaceEvent("checkout-a", 1, { generation: 7 }))).toBe(true);

    const checkoutWithoutRuntime = checkout("checkout-a");
    delete checkoutWithoutRuntime.runtime;
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project(checkoutWithoutRuntime),
    ]);
    await store.initialize();
    expect(store.checkoutsById["checkout-a"]?.runtime).toEqual(activeRuntime);
    expect(store.workspaceStateByCheckout["checkout-a"]?.workspaceGeneration).toBe(7);

    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project(checkout("checkout-a", { runtime: null })),
    ]);
    await store.initialize();
    expect(store.checkoutsById["checkout-a"]?.runtime).toBeNull();
    expect(store.workspaceStateByCheckout["checkout-a"]).toBeUndefined();
  });

  it("registers and focuses a path without touching the global working directory", async () => {
    const openedRuntime = runtime("checkout-new", "project-new", 9, ["unity"]);
    projectServiceMocks.openWorkspace.mockResolvedValue(openedRuntime);
    projectServiceMocks.focusWorkspace.mockResolvedValue(
      paneContext("checkout-new", 1, { generation: 9 }),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.openAndFocus("F:/work/checkout-new");

    expect(store.projectsById["project-new"]?.checkouts[0]?.checkoutId).toBe("checkout-new");
    expect(store.projectsById["project-new"]?.detectedServices).toEqual(["unity"]);
    expect(store.focusedWorkspaceRef).toEqual({
      checkoutId: "checkout-new",
      expectedGeneration: 9,
    });
    expect(projectServiceMocks.setWorkingDir).not.toHaveBeenCalled();
    expect(projectServiceMocks.getWorkingDir).not.toHaveBeenCalled();
  });

  it("updates the pane active session through the scoped command", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1),
    ]);
    projectServiceMocks.setActiveWorkspaceSession.mockResolvedValue(
      paneContext("checkout-a", 2, { activeSessionId: "session-a" }),
    );
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.setActiveSession("session-a");

    expect(projectServiceMocks.setActiveWorkspaceSession).toHaveBeenCalledWith(
      "main",
      "main",
      "session-a",
      2,
    );
    expect(store.focusedPaneContext?.activeSessionId).toBe("session-a");
    expect(store.focusedPaneContext?.revision).toBe(2);
  });

  it("disposes pane and window contexts through their backend owners", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1),
      paneContext("checkout-a", 1, { windowId: "secondary", paneId: "pane-a" }),
    ]);
    projectServiceMocks.detachWorkspaceWindow.mockResolvedValue(1);
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.disposePane();
    expect(projectServiceMocks.detachWorkspacePane).toHaveBeenCalledWith("main", "main", 2);
    expect(store.focusedPaneContext).toBeNull();

    await store.disposeWindow("secondary");
    expect(projectServiceMocks.detachWorkspaceWindow).toHaveBeenCalledWith("secondary", 2);
    expect(Object.values(store.paneContexts)).toHaveLength(0);
  });

  it("reduces background checkout events without projecting through the focused checkout", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a") });
    const checkoutB = checkout("checkout-b", { runtime: runtime("checkout-b") });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA, checkoutB)]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();

    expect(store.applyWorkspaceEvent(workspaceEvent("checkout-b", 4))).toBe(true);
    expect(store.focusedCheckout?.checkoutId).toBe("checkout-a");
    expect(store.workspaceStateByCheckout["checkout-b"]?.events["knowledge-changed"]?.payload)
      .toEqual({ checkoutId: "checkout-b" });
  });

  it("rejects stale event revisions and stale runtime generations", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a", "project-1", 3) });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    const store = useWorkspaceContextStore();
    await store.initialize();

    expect(store.applyWorkspaceEvent(workspaceEvent("checkout-a", 8, { generation: 3 }))).toBe(true);
    expect(store.applyWorkspaceEvent(workspaceEvent("checkout-a", 7, { generation: 3 }))).toBe(false);
    expect(store.applyWorkspaceEvent(workspaceEvent("checkout-a", 9, { generation: 2 }))).toBe(false);
    expect(store.workspaceStateByCheckout["checkout-a"]?.lastStreamRevision).toBe(8);
  });

  it("orders revisions within each event stream without dropping another event kind", async () => {
    const checkoutA = checkout("checkout-a", { runtime: runtime("checkout-a", "project-1", 3) });
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkoutA)]);
    const store = useWorkspaceContextStore();
    await store.initialize();

    const newerKnowledge = workspaceEvent("checkout-a", 8, { generation: 3 });
    const earlierStream = {
      ...workspaceEvent("checkout-a", 7, { generation: 3, payload: { delta: "kept" } }),
      eventName: "stream-event",
    };
    expect(store.applyWorkspaceEvent(newerKnowledge)).toBe(true);
    expect(store.applyWorkspaceEvent(earlierStream)).toBe(true);
    expect(store.workspaceStateByCheckout["checkout-a"]?.events["stream-event"]?.payload)
      .toEqual({ delta: "kept" });
    expect(store.workspaceStateByCheckout["checkout-a"]?.lastStreamRevision).toBe(8);
  });
});
