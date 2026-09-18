import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { effectScope, ref } from "vue";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { useWorkbenchPaneLifecycle } from "../composables/useWorkbenchPaneLifecycle";
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

vi.mock("../services/project", async (importOriginal) => ({
  ...await importOriginal<typeof import("../services/project")>(),
  ...projectServiceMocks,
}));

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
  it("revalidates an explicit open before changing pane focus or cached runtime", async () => {
    setActivePinia(createPinia());
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([paneContext("active", 1)]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([]);
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project(checkout("active", { runtime: runtime("active") })),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();
    projectServiceMocks.openWorkspace.mockRejectedValueOnce(new Error("Directory missing"));
    await expect(store.openCheckout("F:/work/active")).rejects.toThrow("Directory missing");
    expect(store.focusedRoot).toBe("F:/work/active");
    expect(projectServiceMocks.focusWorkspace).not.toHaveBeenCalled();

    projectServiceMocks.openWorkspace.mockResolvedValueOnce(runtime("active", "project-1", 2));
    await store.openCheckout("F:/work/active");
    expect(store.checkoutsById.active.runtime?.workspaceGeneration).toBe(2);
    expect(store.focusedPaneContext?.workspaceGeneration).toBe(1);
    expect(projectServiceMocks.focusWorkspace).not.toHaveBeenCalled();
  });

  it("does not reuse a cached runtime when its directory becomes unavailable", async () => {
    setActivePinia(createPinia());
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([paneContext("old", 1)]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([]);
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project(checkout("old", { runtime: runtime("old") })),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();
    expect(store.focusedWorkspaceRef?.checkoutId).toBe("old");
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project({ ...checkout("old"), available: false }),
    ]);
    await store.initialize();
    expect(store.checkoutsById.old.runtime).toBeNull();
    expect(store.focusedWorkspaceRef).toBeNull();
    expect(store.focusedRoot).toBe("");
    expect(store.checkoutForPane("main", "main")).toBeNull();
  });

  it("leaves unavailable startup history unfocused and opens the renamed directory", async () => {
    setActivePinia(createPinia());
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([]);
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project({ ...checkout("中文工程"), available: false, runtime: null }),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();
    expect(store.focusedRoot).toBe("");
    expect(store.focusedWorkspaceRef).toBeNull();
    projectServiceMocks.openWorkspace.mockResolvedValue(runtime("EnglishProject"));
    projectServiceMocks.focusWorkspace.mockImplementation((_window, _pane, _ref, intentEpoch) =>
      Promise.resolve(paneContext("EnglishProject", 1, { intentEpoch })));
    await store.openAndFocus("F:/work/EnglishProject");
    expect(store.focusedRoot).toBe("F:/work/EnglishProject");
    expect(store.focusedWorkspaceRef?.checkoutId).toBe("EnglishProject");
    expect(store.focusedCheckout?.available).toBe(true);
  });

  it("shares concurrent startup recovery and reuses the completed binding for onboarding", async () => {
    setActivePinia(createPinia());
    const data = deferred<ProjectContextDescriptor[]>();
    projectServiceMocks.listProjectContexts.mockReturnValue(data.promise);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([paneContext("startup", 1)]);
    projectServiceMocks.listWindowWorkspaceIntentEpochs.mockResolvedValue([]);
    const store = useWorkspaceContextStore();
    const first = store.ensureInitialized();
    const second = store.ensureInitialized();
    expect(projectServiceMocks.listProjectContexts).toHaveBeenCalledTimes(1);
    expect(store.initialized).toBe(false);
    data.resolve([project(checkout("startup", { runtime: runtime("startup") }))]);
    await Promise.all([first, second]);
    expect(store.focusedWorkspaceRef?.checkoutId).toBe("startup");
    await store.ensureInitialized();
    expect(projectServiceMocks.listProjectContexts).toHaveBeenCalledTimes(1);
    expect(projectServiceMocks.openWorkspace).not.toHaveBeenCalled();
  });

  it("retries failed context recovery and accepts an empty first-launch workspace", async () => {
    const store = useWorkspaceContextStore();
    projectServiceMocks.listProjectContexts.mockRejectedValueOnce(new Error("Database unavailable"));
    await expect(store.ensureInitialized()).rejects.toThrow("Database unavailable");
    expect(store.initialized).toBe(false);
    await store.ensureInitialized();
    expect(store.initialized).toBe(true);
    expect(store.focusedWorkspaceRef).toBeNull();
    expect(projectServiceMocks.listProjectContexts).toHaveBeenCalledTimes(2);
  });

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

  it("rejects restored panes and explicit handles from an earlier slot assignment", async () => {
    const replacement = { ...runtime("slot"), materializationEpoch: 2 };
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkout("slot", { runtime: replacement }))]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      { ...paneContext("slot", 1), materializationEpoch: 1 },
    ]);
    projectServiceMocks.openWorkspace.mockResolvedValue(replacement);
    const store = useWorkspaceContextStore();
    await store.initialize();
    expect(store.focusedRuntime).toBeNull();
    expect(store.focusedWorkspaceRef).toBeNull();
    for (const expectedMaterializationEpoch of [1, undefined]) {
      await expect(store.focusWorkspaceRefInPane({ checkoutId: "slot", expectedGeneration: 1, expectedMaterializationEpoch }, "main", "main")).rejects.toThrow(/assignment is stale/);
    }
    expect(projectServiceMocks.focusWorkspace).not.toHaveBeenCalled();
  });

  it("captures the verified epoch when focusing a newly selected assignment", async () => {
    const replacement = { ...runtime("slot"), materializationEpoch: 2 };
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkout("slot", { runtime: replacement }))]);
    projectServiceMocks.openWorkspace.mockResolvedValue(replacement);
    projectServiceMocks.focusWorkspace.mockResolvedValue({ ...paneContext("slot", 1), materializationEpoch: 2 });
    const store = useWorkspaceContextStore();
    await store.initialize();
    await store.focusWorkspaceRefInPane({ checkoutId: "slot", expectedGeneration: 1, expectedMaterializationEpoch: 2 }, "main", "main");
    expect(store.focusedWorkspaceRef).toEqual({ checkoutId: "slot", expectedGeneration: 1, expectedMaterializationEpoch: 2 });
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
      { checkoutId: "checkout-b", expectedGeneration: 1, expectedMaterializationEpoch: 0 },
      3,
    );
    expect(store.focusedCheckout?.checkoutId).toBe("checkout-b");
    expect(store.focusedPaneContext?.intentEpoch).toBe(3);
  });

  it.each([false, true])("does not restore a session after its pane is detached (reply pending: %s)", async (pendingReply) => {
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("checkout-a", 1, { paneId: "restored-pane" }),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();
    const restored = store.paneContextAt("main", "restored-pane")!;
    const detaching = deferred<boolean>();
    projectServiceMocks.detachWorkspacePane.mockReturnValueOnce(detaching.promise);
    const detach = store.disposePane("main", "restored-pane");
    if (!pendingReply) { detaching.resolve(true); await detach; }

    expect(await store.setActiveSessionInPane("old-session", "main", "restored-pane", {
      activate: false, expectedIntentEpoch: restored.intentEpoch,
    })).toBeNull();
    expect(projectServiceMocks.setActiveWorkspaceSession).not.toHaveBeenCalled();

    detaching.resolve(true);
    await detach;
    expect(store.paneContextAt("main", "restored-pane")).toBeNull();
  });

  it("only restores the session belonging to the latest focus of a reused pane", async () => {
    projectServiceMocks.listProjectContexts.mockResolvedValue([
      project(checkout("checkout-a"), checkout("checkout-b")),
    ]);
    projectServiceMocks.focusWorkspace.mockImplementation(async (windowId, paneId, ref, intentEpoch) => (
      paneContext(ref.checkoutId, intentEpoch, { windowId, paneId, intentEpoch })
    ));
    projectServiceMocks.setActiveWorkspaceSession.mockImplementation(async (windowId, paneId, sessionId, intentEpoch) => (
      paneContext("checkout-b", intentEpoch, { windowId, paneId, intentEpoch, activeSessionId: sessionId })
    ));
    const store = useWorkspaceContextStore();
    await store.initialize();
    const oldFocus = await store.focusCheckoutInPane("checkout-a", "main", "restored-pane");
    const latestFocus = await store.focusCheckoutInPane("checkout-b", "main", "restored-pane");

    expect(await store.setActiveSessionInPane("session-a", "main", "restored-pane", {
      activate: false, expectedIntentEpoch: oldFocus!.intentEpoch,
    })).toBeNull();
    expect(projectServiceMocks.setActiveWorkspaceSession).not.toHaveBeenCalled();
    const selected = await store.setActiveSessionInPane("session-b", "main", "restored-pane", {
      activate: false, expectedIntentEpoch: latestFocus!.intentEpoch,
    });
    expect(selected?.activeSessionId).toBe("session-b");
    expect(store.paneContextAt("main", "restored-pane")?.focusedCheckoutId).toBe("checkout-b");
  });

  it("reconciles startup recovery and workspace layout switches through the pane lifecycle", async () => {
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("hidden", 1, { paneId: "old-pane" }),
      paneContext("visible", 2, { paneId: "visible-pane" }),
    ]);
    const store = useWorkspaceContextStore();
    const layout = ref(["visible-pane"]);
    const scope = effectScope();
    scope.run(() => useWorkbenchPaneLifecycle("main", () => layout.value));
    try {
      expect(projectServiceMocks.detachWorkspacePane).not.toHaveBeenCalled();
      await store.initialize();
      await vi.waitFor(() => expect(store.paneContextAt("main", "old-pane")).toBeNull());
      expect(store.paneContextAt("main", "visible-pane")).not.toBeNull();

      layout.value = ["next-workspace-pane"];
      await vi.waitFor(() => expect(store.paneContextAt("main", "visible-pane")).toBeNull());
    } finally {
      scope.stop();
    }
  });

  it("releases cached and orphaned panes while retaining visible panes and other windows", async () => {
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("hidden", 1, { paneId: "cached", activeSessionId: "old-session" }),
      paneContext("hidden", 2, { paneId: "orphan" }),
      paneContext("visible", 3, { paneId: "left" }),
      paneContext("visible", 4, { paneId: "right" }),
      paneContext("hidden", 5, { windowId: "secondary", paneId: "other-window" }),
    ]);
    const store = useWorkspaceContextStore();
    await store.initialize();

    await store.reconcileWindowPanes("main", ["left", "right"]);

    expect(projectServiceMocks.detachWorkspacePane.mock.calls.map((call) => call.slice(0, 2)))
      .toEqual([["main", "cached"], ["main", "orphan"]]);
    expect(store.paneContextAt("main", "cached")).toBeNull();
    expect(store.paneContextAt("main", "orphan")).toBeNull();
    expect(store.paneContextAt("main", "left")?.focusedCheckoutId).toBe("visible");
    expect(store.paneContextAt("main", "right")?.focusedCheckoutId).toBe("visible");
    expect(store.paneContextAt("secondary", "other-window")?.focusedCheckoutId).toBe("hidden");
    expect(projectServiceMocks.setActiveWorkspaceSession).not.toHaveBeenCalled();
  });

  it("invalidates a pending hidden-pane focus before workspace opening completes", async () => {
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkout("hidden"))]);
    const opening = deferred<WorkspaceRuntimeDescriptor>();
    projectServiceMocks.openWorkspace.mockReturnValueOnce(opening.promise);
    const store = useWorkspaceContextStore();
    await store.initialize();
    const focus = store.focusCheckoutInPane("hidden", "main", "old-pane");

    await store.reconcileWindowPanes("main", ["new-pane"]);
    opening.resolve(runtime("hidden"));

    expect(await focus).toBeNull();
    expect(projectServiceMocks.focusWorkspace).not.toHaveBeenCalled();
    expect(store.paneContextAt("main", "old-pane")).toBeNull();
  });

  it("preserves a newer focus when a previous layout's detach finishes late", async () => {
    projectServiceMocks.listProjectContexts.mockResolvedValue([project(checkout("visible"))]);
    projectServiceMocks.listWindowWorkspaceContexts.mockResolvedValue([
      paneContext("hidden", 1, { paneId: "reused-pane" }),
    ]);
    const detaching = deferred<boolean>();
    projectServiceMocks.detachWorkspacePane.mockReturnValueOnce(detaching.promise);
    projectServiceMocks.focusWorkspace.mockImplementation(async (windowId, paneId, ref, intentEpoch) => (
      paneContext(ref.checkoutId, 2, { windowId, paneId, intentEpoch })
    ));
    const store = useWorkspaceContextStore();
    await store.initialize();
    const cleanup = store.reconcileWindowPanes("main", ["different-pane"]);
    await store.focusCheckoutInPane("visible", "main", "reused-pane");
    detaching.resolve(true);
    await cleanup;

    expect(store.paneContextAt("main", "reused-pane")?.focusedCheckoutId).toBe("visible");
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
      expectedMaterializationEpoch: 0,
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
      { checkoutId: "checkout-b", expectedGeneration: 9, expectedMaterializationEpoch: 0 },
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
      { checkoutId: "checkout-a", expectedGeneration: 1, expectedMaterializationEpoch: 0 },
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
