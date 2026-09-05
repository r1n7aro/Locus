import { computed, ref } from "vue";
import { defineStore } from "pinia";
import * as projectService from "../services/project";
import type {
  ProjectContextDescriptor,
  WindowPaneWorkspaceContext,
  WorkspaceCheckoutDescriptor,
  WorkspaceRef,
  WorkspaceRuntimeDescriptor,
  RoutedWorkspaceEvent,
} from "../services/project";

const DEFAULT_WINDOW_ID = "main";
const DEFAULT_PANE_ID = "main";

export interface WorkspaceEventProjection {
  projectId: string;
  checkoutId: string;
  workspaceGeneration: number;
  lastStreamRevision: number;
  events: Record<string, RoutedWorkspaceEvent>;
}

function paneContextKey(windowId: string, paneId: string): string {
  return `${windowId}\u0000${paneId}`;
}

function requireCheckout(
  checkout: WorkspaceCheckoutDescriptor | undefined,
  checkoutId: string,
): WorkspaceCheckoutDescriptor {
  if (checkout) return checkout;
  throw new Error(`Unknown workspace checkout: ${checkoutId}`);
}

/**
 * Process-level workspace catalog plus the checkout focus owned by each window pane.
 *
 * This store deliberately has no dependency on the legacy working-dir projection.
 * Callers address backend work through `focusedWorkspaceRef` or an explicit checkout.
 */
export const useWorkspaceContextStore = defineStore("workspaceContext", () => {
  const projectsById = ref<Record<string, ProjectContextDescriptor>>({});
  const checkoutsById = ref<Record<string, WorkspaceCheckoutDescriptor>>({});
  const projectOrder = ref<string[]>([]);
  const paneContexts = ref<Record<string, WindowPaneWorkspaceContext>>({});
  const workspaceStateByCheckout = ref<Record<string, WorkspaceEventProjection>>({});
  const windowId = ref(DEFAULT_WINDOW_ID);
  const paneId = ref(DEFAULT_PANE_ID);
  const initialized = ref(false);

  // Focus, active-session selection, and detach share one client-owned CAS
  // sequence per pane. The window high-water mark lets a window detach outrun
  // every pane mutation already issued by this renderer.
  const mutationIntentEpochs = new Map<string, number>();
  const windowIntentEpochs = new Map<string, number>();
  const windowTombstoneEpochs = new Map<string, number>();
  const runtimeOpenRequests = new Map<string, Promise<WorkspaceRuntimeDescriptor>>();

  const focusedPaneContext = computed<WindowPaneWorkspaceContext | null>(() => (
    paneContexts.value[paneContextKey(windowId.value, paneId.value)] ?? null
  ));
  const focusedCheckout = computed<WorkspaceCheckoutDescriptor | null>(() => {
    const checkoutId = focusedPaneContext.value?.focusedCheckoutId;
    return checkoutId ? checkoutsById.value[checkoutId] ?? null : null;
  });
  const focusedRuntime = computed<WorkspaceRuntimeDescriptor | null>(() => (
    focusedCheckout.value?.runtime ?? null
  ));
  const focusedWorkspaceRef = computed<WorkspaceRef | null>(() => {
    const runtime = focusedRuntime.value;
    if (!runtime) return null;
    return {
      checkoutId: runtime.checkoutId,
      expectedGeneration: runtime.workspaceGeneration,
    };
  });
  const focusedRoot = computed(() => focusedCheckout.value?.root ?? "");
  const focusedProject = computed<ProjectContextDescriptor | null>(() => {
    const projectId = focusedCheckout.value?.projectId;
    return projectId ? projectsById.value[projectId] ?? null : null;
  });
  const projects = computed(() => projectOrder.value
    .map((projectId) => projectsById.value[projectId])
    .filter((project): project is ProjectContextDescriptor => Boolean(project)));

  function paneContextAt(
    targetWindowId: string,
    targetPaneId: string,
  ): WindowPaneWorkspaceContext | null {
    return paneContexts.value[paneContextKey(targetWindowId, targetPaneId)] ?? null;
  }

  function checkoutForPane(
    targetWindowId: string,
    targetPaneId: string,
  ): WorkspaceCheckoutDescriptor | null {
    const checkoutId = paneContextAt(targetWindowId, targetPaneId)?.focusedCheckoutId;
    return checkoutId ? checkoutsById.value[checkoutId] ?? null : null;
  }

  function workspaceRefForPane(
    targetWindowId: string,
    targetPaneId: string,
  ): WorkspaceRef | null {
    const context = paneContextAt(targetWindowId, targetPaneId);
    if (!context) return null;
    return {
      checkoutId: context.focusedCheckoutId,
      expectedGeneration: context.workspaceGeneration,
    };
  }

  function activatePane(targetWindowId: string, targetPaneId: string): void {
    windowId.value = targetWindowId;
    paneId.value = targetPaneId;
  }

  function currentIntentEpoch(key: string): number {
    return mutationIntentEpochs.get(key) ?? 0;
  }

  function checkedNextIntentEpoch(current: number): number {
    const next = current + 1;
    if (!Number.isSafeInteger(next) || next <= 0) {
      throw new Error("Workspace intent epoch exhausted.");
    }
    return next;
  }

  function nextPaneIntentEpoch(targetWindowId: string, targetPaneId: string): number {
    const key = paneContextKey(targetWindowId, targetPaneId);
    const epoch = checkedNextIntentEpoch(Math.max(
      currentIntentEpoch(key),
      windowIntentEpochs.get(targetWindowId) ?? 0,
    ));
    mutationIntentEpochs.set(key, epoch);
    windowIntentEpochs.set(targetWindowId, epoch);
    return epoch;
  }

  function nextWindowIntentEpoch(targetWindowId: string): number {
    const epoch = checkedNextIntentEpoch(windowIntentEpochs.get(targetWindowId) ?? 0);
    windowIntentEpochs.set(targetWindowId, epoch);
    windowTombstoneEpochs.set(targetWindowId, epoch);
    const prefix = `${targetWindowId}\u0000`;
    for (const key of mutationIntentEpochs.keys()) {
      if (key.startsWith(prefix)) mutationIntentEpochs.set(key, epoch);
    }
    return epoch;
  }

  function observePaneIntent(context: WindowPaneWorkspaceContext) {
    const epoch = context.intentEpoch;
    if (!Number.isSafeInteger(epoch) || epoch <= 0) return;
    const key = paneContextKey(context.windowId, context.paneId);
    mutationIntentEpochs.set(key, Math.max(currentIntentEpoch(key), epoch));
    windowIntentEpochs.set(
      context.windowId,
      Math.max(windowIntentEpochs.get(context.windowId) ?? 0, epoch),
    );
  }

  function upsertProjectCheckout(checkout: WorkspaceCheckoutDescriptor) {
    checkoutsById.value[checkout.checkoutId] = checkout;

    const existingProject = projectsById.value[checkout.projectId];
    if (existingProject) {
      if (!projectOrder.value.includes(checkout.projectId)) {
        projectOrder.value.push(checkout.projectId);
      }
      const detectedServices = checkout.runtime?.detectedServices ?? [];
      if (detectedServices.length > 0) {
        existingProject.detectedServices = Array.from(new Set([
          ...existingProject.detectedServices,
          ...detectedServices,
        ])).sort();
      }
      const index = existingProject.checkouts.findIndex(
        (candidate) => candidate.checkoutId === checkout.checkoutId,
      );
      if (index >= 0) {
        existingProject.checkouts[index] = checkout;
      } else {
        existingProject.checkouts.push(checkout);
      }
      return;
    }

    projectsById.value[checkout.projectId] = {
      projectId: checkout.projectId,
      detectedServices: [...(checkout.runtime?.detectedServices ?? [])].sort(),
      checkouts: [checkout],
    };
    projectOrder.value.push(checkout.projectId);
  }

  function upsertRuntime(runtime: WorkspaceRuntimeDescriptor): WorkspaceCheckoutDescriptor {
    const existing = checkoutsById.value[runtime.checkoutId];
    if (
      existing?.runtime
      && existing.runtime.workspaceGeneration !== runtime.workspaceGeneration
    ) {
      delete workspaceStateByCheckout.value[runtime.checkoutId];
    }
    const checkout: WorkspaceCheckoutDescriptor = {
      checkoutId: runtime.checkoutId,
      projectId: runtime.projectId,
      root: runtime.root,
      normalizedRoot: existing?.normalizedRoot ?? runtime.root,
      lastOpenedAt: existing?.lastOpenedAt ?? Date.now(),
      runtime,
    };
    upsertProjectCheckout(checkout);
    return checkout;
  }

  function replaceProjects(projectContexts: ProjectContextDescriptor[]) {
    const nextProjects: Record<string, ProjectContextDescriptor> = {};
    const nextCheckouts: Record<string, WorkspaceCheckoutDescriptor> = {};
    const nextOrder: string[] = [];

    for (const project of projectContexts) {
      const checkouts = project.checkouts.map((checkout) => {
        const knownRuntime = checkoutsById.value[checkout.checkoutId]?.runtime;
        return {
          ...checkout,
          runtime: checkout.runtime === null
            ? null
            : checkout.runtime ?? knownRuntime ?? null,
        };
      });
      const projectCopy = { ...project, checkouts };
      nextProjects[project.projectId] = projectCopy;
      nextOrder.push(project.projectId);
      for (const checkout of checkouts) {
        nextCheckouts[checkout.checkoutId] = checkout;
      }
    }

    projectsById.value = nextProjects;
    checkoutsById.value = nextCheckouts;
    projectOrder.value = nextOrder;
    for (const [checkoutId, projection] of Object.entries(workspaceStateByCheckout.value)) {
      const runtime = nextCheckouts[checkoutId]?.runtime;
      if (!runtime || runtime.workspaceGeneration !== projection.workspaceGeneration) {
        delete workspaceStateByCheckout.value[checkoutId];
      }
    }
  }

  function applyPaneContext(
    context: WindowPaneWorkspaceContext,
    expectedIntentEpoch?: number,
  ): boolean {
    const key = paneContextKey(context.windowId, context.paneId);
    if (
      !Number.isSafeInteger(context.intentEpoch)
      || context.intentEpoch <= 0
      || context.intentEpoch <= (windowTombstoneEpochs.get(context.windowId) ?? 0)
      || (
        expectedIntentEpoch != null
        && (
          currentIntentEpoch(key) !== expectedIntentEpoch
          || context.intentEpoch !== expectedIntentEpoch
        )
      )
      || context.intentEpoch < currentIntentEpoch(key)
    ) {
      return false;
    }

    const current = paneContexts.value[key];
    if (current && context.revision < current.revision) return false;
    if (
      current
      && context.revision === current.revision
      && (
        context.focusedCheckoutId !== current.focusedCheckoutId
        || context.workspaceGeneration !== current.workspaceGeneration
        || context.activeSessionId !== current.activeSessionId
      )
    ) {
      return false;
    }

    paneContexts.value[key] = context;
    observePaneIntent(context);
    return true;
  }

  function applyWorkspaceEvent(event: RoutedWorkspaceEvent): boolean {
    if (
      !event.checkoutId
      || !event.projectId
      || !event.eventName
      || !Number.isSafeInteger(event.streamRevision)
      || event.streamRevision <= 0
    ) {
      return false;
    }
    const checkout = checkoutsById.value[event.checkoutId];
    const runtime = checkout?.runtime;
    if (
      !checkout
      || checkout.projectId !== event.projectId
      || !runtime
      || runtime.workspaceGeneration !== event.workspaceGeneration
    ) {
      return false;
    }
    const current = workspaceStateByCheckout.value[event.checkoutId];
    const currentEvent = current?.events[event.eventName];
    if (currentEvent && event.streamRevision <= currentEvent.streamRevision) return false;
    workspaceStateByCheckout.value[event.checkoutId] = {
      projectId: event.projectId,
      checkoutId: event.checkoutId,
      workspaceGeneration: event.workspaceGeneration,
      lastStreamRevision: Math.max(current?.lastStreamRevision ?? 0, event.streamRevision),
      events: {
        ...(current?.events ?? {}),
        [event.eventName]: event,
      },
    };
    return true;
  }

  async function initialize(
    nextWindowId = DEFAULT_WINDOW_ID,
    nextPaneId = DEFAULT_PANE_ID,
  ) {
    windowId.value = nextWindowId;
    paneId.value = nextPaneId;
    const key = paneContextKey(nextWindowId, nextPaneId);
    const initialIntentEpoch = currentIntentEpoch(key);
    const [projectContexts, restoredPaneContexts, restoredIntentEpochs] = await Promise.all([
      projectService.listProjectContexts(),
      projectService.listWindowWorkspaceContexts(),
      projectService.listWindowWorkspaceIntentEpochs(),
    ]);

    replaceProjects(projectContexts);
    for (const snapshot of restoredIntentEpochs) {
      if (!Number.isSafeInteger(snapshot.intentEpoch) || snapshot.intentEpoch <= 0) continue;
      windowIntentEpochs.set(
        snapshot.windowId,
        Math.max(windowIntentEpochs.get(snapshot.windowId) ?? 0, snapshot.intentEpoch),
      );
      if (snapshot.paneId) {
        const snapshotKey = paneContextKey(snapshot.windowId, snapshot.paneId);
        mutationIntentEpochs.set(
          snapshotKey,
          Math.max(currentIntentEpoch(snapshotKey), snapshot.intentEpoch),
        );
      } else {
        windowTombstoneEpochs.set(
          snapshot.windowId,
          Math.max(windowTombstoneEpochs.get(snapshot.windowId) ?? 0, snapshot.intentEpoch),
        );
      }
    }
    for (const context of restoredPaneContexts) {
      const isCurrentPane = context.windowId === nextWindowId && context.paneId === nextPaneId;
      if (
        isCurrentPane
        && initialIntentEpoch > 0
        && currentIntentEpoch(key) !== initialIntentEpoch
      ) continue;
      applyPaneContext(context);
    }
    initialized.value = true;
  }

  async function ensureRuntime(
    checkout: WorkspaceCheckoutDescriptor,
    targetWindowId = windowId.value,
    targetPaneId = paneId.value,
  ): Promise<WorkspaceRuntimeDescriptor> {
    const currentRuntime = checkoutsById.value[checkout.checkoutId]?.runtime ?? checkout.runtime;
    const focusedContext = paneContextAt(targetWindowId, targetPaneId);
    if (
      currentRuntime
      && focusedContext?.focusedCheckoutId === checkout.checkoutId
      && focusedContext.workspaceGeneration === currentRuntime.workspaceGeneration
    ) {
      upsertRuntime(currentRuntime);
      return currentRuntime;
    }

    const existingRequest = runtimeOpenRequests.get(checkout.checkoutId);
    if (existingRequest) return existingRequest;

    // Cached descriptors for background checkouts can outlive their backend
    // runtimes after resource-policy eviction. openWorkspace is idempotent for
    // live runtimes and re-registers an evicted checkout with a fresh generation.
    const request = projectService.openWorkspace(checkout.root)
      .then((runtime) => {
        upsertRuntime(runtime);
        return runtime;
      })
      .finally(() => {
        runtimeOpenRequests.delete(checkout.checkoutId);
      });
    runtimeOpenRequests.set(checkout.checkoutId, request);
    return request;
  }

  async function focusRuntime(
    runtime: WorkspaceRuntimeDescriptor,
    targetWindowId: string,
    targetPaneId: string,
    expectedIntentEpoch: number,
    activate = true,
  ): Promise<WindowPaneWorkspaceContext | null> {
    const key = paneContextKey(targetWindowId, targetPaneId);
    if (currentIntentEpoch(key) !== expectedIntentEpoch) return null;

    try {
      const context = await projectService.focusWorkspace(targetWindowId, targetPaneId, {
        checkoutId: runtime.checkoutId,
        expectedGeneration: runtime.workspaceGeneration,
      }, expectedIntentEpoch);
      if (!applyPaneContext(context, expectedIntentEpoch)) return null;
      if (activate) activatePane(targetWindowId, targetPaneId);
      return context;
    } catch (error) {
      if (currentIntentEpoch(key) !== expectedIntentEpoch) return null;
      throw error;
    }
  }

  async function focusCheckoutInPane(
    checkoutOrId: WorkspaceCheckoutDescriptor | string,
    targetWindowId: string,
    targetPaneId: string,
    options: { activate?: boolean } = {},
  ): Promise<WindowPaneWorkspaceContext | null> {
    const checkoutId = typeof checkoutOrId === "string"
      ? checkoutOrId
      : checkoutOrId.checkoutId;
    const checkout = requireCheckout(
      checkoutsById.value[checkoutId]
        ?? (typeof checkoutOrId === "string" ? undefined : checkoutOrId),
      checkoutId,
    );
    const epoch = nextPaneIntentEpoch(targetWindowId, targetPaneId);
    const runtime = await ensureRuntime(checkout, targetWindowId, targetPaneId);
    return focusRuntime(
      runtime,
      targetWindowId,
      targetPaneId,
      epoch,
      options.activate !== false,
    );
  }

  async function focusCheckout(
    checkoutOrId: WorkspaceCheckoutDescriptor | string,
  ): Promise<WindowPaneWorkspaceContext | null> {
    return focusCheckoutInPane(checkoutOrId, windowId.value, paneId.value);
  }

  async function focusWorkspaceRefInPane(
    workspaceRef: WorkspaceRef,
    targetWindowId: string,
    targetPaneId: string,
    options: { activate?: boolean } = {},
  ): Promise<WindowPaneWorkspaceContext | null> {
    const checkout = requireCheckout(
      checkoutsById.value[workspaceRef.checkoutId],
      workspaceRef.checkoutId,
    );
    const epoch = nextPaneIntentEpoch(targetWindowId, targetPaneId);
    const runtime = await ensureRuntime(checkout, targetWindowId, targetPaneId);
    if (
      workspaceRef.expectedGeneration != null
      && runtime.workspaceGeneration !== workspaceRef.expectedGeneration
    ) {
      throw new Error("The requested checkout runtime generation is stale.");
    }
    return focusRuntime(
      runtime,
      targetWindowId,
      targetPaneId,
      epoch,
      options.activate !== false,
    );
  }

  async function focusWorkspaceRef(
    workspaceRef: WorkspaceRef,
  ): Promise<WindowPaneWorkspaceContext | null> {
    return focusWorkspaceRefInPane(workspaceRef, windowId.value, paneId.value);
  }

  async function openAndFocusInPane(
    path: string,
    targetWindowId: string,
    targetPaneId: string,
    options: { activate?: boolean } = {},
  ): Promise<WindowPaneWorkspaceContext | null> {
    const epoch = nextPaneIntentEpoch(targetWindowId, targetPaneId);
    const runtime = await projectService.openWorkspace(path);
    upsertRuntime(runtime);
    return focusRuntime(
      runtime,
      targetWindowId,
      targetPaneId,
      epoch,
      options.activate !== false,
    );
  }

  async function openAndFocus(path: string): Promise<WindowPaneWorkspaceContext | null> {
    return openAndFocusInPane(path, windowId.value, paneId.value);
  }

  async function removeProject(projectId: string): Promise<boolean> {
    const removed = await projectService.removeWorkspace(projectId);
    if (!removed) return false;
    projectOrder.value = projectOrder.value.filter((candidate) => candidate !== projectId);
    return true;
  }

  async function setActiveSessionInPane(
    activeSessionId: string | null,
    targetWindowId: string,
    targetPaneId: string,
    options: { activate?: boolean } = {},
  ): Promise<WindowPaneWorkspaceContext | null> {
    const key = paneContextKey(targetWindowId, targetPaneId);
    const intentEpoch = nextPaneIntentEpoch(targetWindowId, targetPaneId);

    try {
      const context = await projectService.setActiveWorkspaceSession(
        targetWindowId,
        targetPaneId,
        activeSessionId,
        intentEpoch,
      );
      if (!applyPaneContext(context, intentEpoch)) return null;
      if (options.activate !== false) activatePane(targetWindowId, targetPaneId);
      return context;
    } catch (error) {
      if (currentIntentEpoch(key) !== intentEpoch) return null;
      throw error;
    }
  }

  async function setActiveSession(
    activeSessionId: string | null,
  ): Promise<WindowPaneWorkspaceContext | null> {
    return setActiveSessionInPane(
      activeSessionId,
      windowId.value,
      paneId.value,
    );
  }

  async function disposePane(
    targetWindowId = windowId.value,
    targetPaneId = paneId.value,
  ): Promise<boolean> {
    const key = paneContextKey(targetWindowId, targetPaneId);
    const intentEpoch = nextPaneIntentEpoch(targetWindowId, targetPaneId);
    try {
      const removed = await projectService.detachWorkspacePane(
        targetWindowId,
        targetPaneId,
        intentEpoch,
      );
      if (currentIntentEpoch(key) === intentEpoch) delete paneContexts.value[key];
      return currentIntentEpoch(key) === intentEpoch ? removed : false;
    } catch (error) {
      if (currentIntentEpoch(key) !== intentEpoch) return false;
      throw error;
    }
  }

  async function disposeWindow(targetWindowId = windowId.value): Promise<number> {
    const intentEpoch = nextWindowIntentEpoch(targetWindowId);
    try {
      const removed = await projectService.detachWorkspaceWindow(targetWindowId, intentEpoch);
      if (windowIntentEpochs.get(targetWindowId) !== intentEpoch) return 0;
      for (const [key, context] of Object.entries(paneContexts.value)) {
        if (context.windowId === targetWindowId) delete paneContexts.value[key];
      }
      return removed;
    } catch (error) {
      if (windowIntentEpochs.get(targetWindowId) !== intentEpoch) return 0;
      throw error;
    }
  }

  return {
    projectsById,
    checkoutsById,
    paneContexts,
    workspaceStateByCheckout,
    windowId,
    paneId,
    initialized,
    projects,
    focusedPaneContext,
    focusedProject,
    focusedCheckout,
    focusedRuntime,
    focusedWorkspaceRef,
    focusedRoot,
    paneContextAt,
    checkoutForPane,
    workspaceRefForPane,
    activatePane,
    applyWorkspaceEvent,
    initialize,
    openAndFocus,
    openAndFocusInPane,
    removeProject,
    focusCheckout,
    focusCheckoutInPane,
    focusWorkspaceRef,
    focusWorkspaceRefInPane,
    setActiveSession,
    setActiveSessionInPane,
    disposePane,
    disposeWindow,
  };
});
