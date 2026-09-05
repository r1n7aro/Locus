import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { WorkspaceRef } from "../services/project";
import {
  subscribeWorkspaceFileChanges,
  type WorkspaceFileChangedPayload,
} from "../services/workspaceExplorer";
import type { ProjectExplorerFileRevision } from "../types/workbench";

export type FileChangeProbeReason = "activate" | "focus" | "visibility" | "event" | "manual";

interface FileChangeRevalidationOptions {
  active: () => boolean;
  currentRevision: () => ProjectExplorerFileRevision | null;
  probe: () => Promise<ProjectExplorerFileRevision>;
  onBaseline?: (revision: ProjectExplorerFileRevision) => void | Promise<void>;
  onChanged: (
    revision: ProjectExplorerFileRevision,
    reason: FileChangeProbeReason,
  ) => void | Promise<void>;
  onError?: (error: unknown) => void;
  workspaceRef?: () => WorkspaceRef | null | undefined;
  workspacePath?: () => string | null | undefined;
  debounceMs?: number;
  probeOnMount?: boolean;
}

type ForegroundProbeListener = (reason: "focus" | "visibility") => void;

const activeForegroundProbeListeners = new Set<ForegroundProbeListener>();
let foregroundProbeEventsInstalled = false;

function ensureForegroundProbeEvents(): void {
  if (foregroundProbeEventsInstalled) return;
  foregroundProbeEventsInstalled = true;
  window.addEventListener("focus", () => {
    for (const listener of activeForegroundProbeListeners) listener("focus");
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState !== "visible") return;
    for (const listener of activeForegroundProbeListeners) listener("visibility");
  });
}

function normalizedPath(value: string | null | undefined): string {
  const normalized = (value ?? "").trim().replace(/\\/g, "/").replace(/^\.\//, "");
  return normalized.toLocaleLowerCase();
}

export function fileRevisionsEqual(
  left: ProjectExplorerFileRevision | null | undefined,
  right: ProjectExplorerFileRevision | null | undefined,
): boolean {
  return !!left && !!right && left.key === right.key;
}

export function workspaceFileChangeMatches(
  event: {
    checkoutId: string;
    workspaceGeneration: number;
    payload: WorkspaceFileChangedPayload;
  },
  workspaceRef: WorkspaceRef | null | undefined,
  workspacePath: string | null | undefined,
): boolean {
  if (!workspaceRef || event.checkoutId !== workspaceRef.checkoutId) return false;
  if (
    workspaceRef.expectedGeneration != null
    && event.workspaceGeneration !== workspaceRef.expectedGeneration
  ) return false;
  const expectedPath = normalizedPath(workspacePath);
  return !!expectedPath && normalizedPath(event.payload.path) === expectedPath;
}

export function useFileChangeRevalidation(options: FileChangeRevalidationOptions) {
  const checking = ref(false);
  const debounceMs = Math.max(40, options.debounceMs ?? 120);
  let destroyed = false;
  let releaseWorkspaceEvents: (() => void) | null = null;
  let scheduledTimer: ReturnType<typeof setTimeout> | null = null;
  let scheduledReason: FileChangeProbeReason | null = null;
  let activeRequest: Promise<void> | null = null;
  let queuedAfterRequest = false;
  let pendingWhileInactive = false;

  async function runProbe(reason: FileChangeProbeReason): Promise<void> {
    if (destroyed) return;
    if (!options.active() && reason !== "manual") {
      pendingWhileInactive = true;
      return;
    }
    if (activeRequest) {
      queuedAfterRequest = true;
      return activeRequest;
    }

    checking.value = true;
    const request = (async () => {
      try {
        const revision = await options.probe();
        if (destroyed) return;
        const current = options.currentRevision();
        if (!current) {
          await options.onBaseline?.(revision);
          return;
        }
        if (!fileRevisionsEqual(current, revision)) {
          await options.onChanged(revision, reason);
        }
      } catch (error) {
        options.onError?.(error);
      } finally {
        if (!destroyed) checking.value = false;
      }
    })();
    activeRequest = request;
    try {
      await request;
    } finally {
      if (activeRequest === request) activeRequest = null;
      if (queuedAfterRequest && !destroyed) {
        queuedAfterRequest = false;
        scheduleProbe(reason);
      }
    }
  }

  function scheduleProbe(reason: FileChangeProbeReason): void {
    if (destroyed) return;
    if (!options.active() && reason !== "manual") {
      pendingWhileInactive = true;
      return;
    }
    scheduledReason = reason;
    if (scheduledTimer) clearTimeout(scheduledTimer);
    scheduledTimer = setTimeout(() => {
      scheduledTimer = null;
      const nextReason = scheduledReason ?? reason;
      scheduledReason = null;
      void runProbe(nextReason);
    }, reason === "manual" ? 0 : debounceMs);
  }

  const handleForegroundProbe: ForegroundProbeListener = (reason) => scheduleProbe(reason);

  onMounted(() => {
    ensureForegroundProbeEvents();
    if (options.active()) activeForegroundProbeListeners.add(handleForegroundProbe);
    void subscribeWorkspaceFileChanges((event) => {
      if (!workspaceFileChangeMatches(
        event,
        options.workspaceRef?.(),
        options.workspacePath?.(),
      )) return;
      scheduleProbe("event");
    }).then((release) => {
      if (destroyed) release();
      else releaseWorkspaceEvents = release;
    });
    if (options.probeOnMount) scheduleProbe("activate");
  });

  watch(
    options.active,
    (active, wasActive) => {
      if (active) activeForegroundProbeListeners.add(handleForegroundProbe);
      else activeForegroundProbeListeners.delete(handleForegroundProbe);
      if (!active || (wasActive !== false && !pendingWhileInactive)) return;
      pendingWhileInactive = false;
      scheduleProbe("activate");
    },
  );

  onBeforeUnmount(() => {
    destroyed = true;
    if (scheduledTimer) clearTimeout(scheduledTimer);
    scheduledTimer = null;
    releaseWorkspaceEvents?.();
    releaseWorkspaceEvents = null;
    activeForegroundProbeListeners.delete(handleForegroundProbe);
  });

  return {
    checking,
    checkNow: (reason: FileChangeProbeReason = "manual") => runProbe(reason),
    scheduleCheck: scheduleProbe,
  };
}
