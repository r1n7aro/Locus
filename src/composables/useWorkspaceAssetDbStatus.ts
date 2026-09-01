import {
  computed,
  onScopeDispose,
  ref,
  toValue,
  watch,
  type MaybeRefOrGetter,
} from "vue";
import {
  assetDbLightStatus,
  assetDbScanStart,
  subscribeAssetDbScan,
} from "../services/asset";
import { normalizeAppError } from "../services/errors";
import type { RuntimeUnsubscribe } from "../services/locusRuntime";
import type { WorkspaceRef } from "../services/project";
import type {
  AppErrorPayload,
  AssetDbLightStatus,
  AssetDbScanEvent,
  ScanStats,
} from "../types";

interface WorkspaceAssetDbStatusOptions {
  workspaceRef: MaybeRefOrGetter<WorkspaceRef | null>;
  enabled?: MaybeRefOrGetter<boolean>;
  onScanError?: (error: AppErrorPayload) => void;
}

function workspaceRefKey(workspaceRef: WorkspaceRef | null): string {
  if (!workspaceRef) return "";
  return `${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}`;
}

function isRunningPhase(phase: AssetDbScanEvent | null): boolean {
  return phase != null
    && phase.phase !== "done"
    && phase.phase !== "reconcileDone"
    && phase.phase !== "error";
}

function minimalStatsFromLightStatus(status: AssetDbLightStatus): ScanStats {
  return {
    dirsScanned: 0,
    metaFilesFound: 0,
    yamlAssetsFound: 0,
    nodesAdded: status.nodes,
    edgesAdded: status.edges,
    nodesUpdated: 0,
    nodesDeleted: 0,
    parseFailures: 0,
    elapsedMs: status.lastScanDurationMs ?? 0,
    duplicateGuids: {
      groupCount: 0,
      pathCount: 0,
      assetsOnlyGroups: 0,
      packagesOnlyGroups: 0,
      crossRootGroups: 0,
    },
  };
}

export function useWorkspaceAssetDbStatus(options: WorkspaceAssetDbStatusOptions) {
  const scanPhase = ref<AssetDbScanEvent | null>(null);
  const lastScanStats = ref<ScanStats | null>(null);
  const scanRequestPending = ref(false);
  const enabled = computed(() => options.enabled == null || toValue(options.enabled));
  const currentWorkspaceRef = computed(() => toValue(options.workspaceRef));
  const bindingKey = computed(() => (
    enabled.value ? workspaceRefKey(currentWorkspaceRef.value) : ""
  ));

  let bindingVersion = 0;
  let stateRevision = 0;
  let unsubscribe: RuntimeUnsubscribe | null = null;

  function clearState() {
    stateRevision += 1;
    scanPhase.value = null;
    lastScanStats.value = null;
    scanRequestPending.value = false;
  }

  function applyScanEvent(event: AssetDbScanEvent) {
    stateRevision += 1;
    if (event.phase === "done") {
      scanPhase.value = event;
      lastScanStats.value = event.stats;
      return;
    }
    if (event.phase === "reconcileDone") {
      scanPhase.value = null;
      return;
    }
    scanPhase.value = event;
  }

  function applyLightStatus(status: AssetDbLightStatus) {
    scanPhase.value = status.currentScanPhase ?? null;
    if (status.lastScanStats) {
      lastScanStats.value = status.lastScanStats;
    } else if (status.status === "indexed") {
      lastScanStats.value = minimalStatsFromLightStatus(status);
    } else if (status.status === "none") {
      lastScanStats.value = null;
    }
  }

  function isCurrentBinding(workspaceRef: WorkspaceRef, version: number): boolean {
    return version === bindingVersion
      && enabled.value
      && workspaceRefKey(currentWorkspaceRef.value) === workspaceRefKey(workspaceRef);
  }

  async function refresh(workspaceRef = currentWorkspaceRef.value): Promise<void> {
    if (!workspaceRef || !enabled.value) return;
    const scopedRef = { ...workspaceRef };
    const version = bindingVersion;
    const revision = stateRevision;
    try {
      const status = await assetDbLightStatus(scopedRef);
      if (!isCurrentBinding(scopedRef, version) || revision !== stateRevision) return;
      applyLightStatus(status);
    } catch (error) {
      if (isCurrentBinding(scopedRef, version)) {
        console.warn("[AssetDb] failed to load scoped status:", normalizeAppError(error));
      }
    }
  }

  async function bindWorkspace(): Promise<void> {
    const version = ++bindingVersion;
    unsubscribe?.();
    unsubscribe = null;
    clearState();

    const workspaceRef = currentWorkspaceRef.value;
    if (!workspaceRef || !enabled.value) return;
    const scopedRef = { ...workspaceRef };
    try {
      const release = await subscribeAssetDbScan(scopedRef, (event) => {
        if (!isCurrentBinding(scopedRef, version)) return;
        applyScanEvent(event);
      });
      if (!isCurrentBinding(scopedRef, version)) {
        release();
        return;
      }
      unsubscribe = release;
    } catch (error) {
      if (isCurrentBinding(scopedRef, version)) {
        console.warn("[AssetDb] failed to subscribe scoped scan status:", normalizeAppError(error));
      }
    }
    await refresh(scopedRef);
  }

  async function startScan(): Promise<boolean> {
    const workspaceRef = currentWorkspaceRef.value;
    if (!workspaceRef || !enabled.value) return false;
    if (scanRequestPending.value || isRunningPhase(scanPhase.value)) return false;

    const scopedRef = { ...workspaceRef };
    const version = bindingVersion;
    scanRequestPending.value = true;
    applyScanEvent({ phase: "dirScan" });
    try {
      const result = await assetDbScanStart(scopedRef);
      if (!isCurrentBinding(scopedRef, version)) return false;
      if (!result.started && !result.alreadyRunning) {
        applyScanEvent({ phase: "reconcileDone" });
        await refresh(scopedRef);
        return false;
      }
      if (result.alreadyRunning) await refresh(scopedRef);
      return true;
    } catch (error) {
      if (!isCurrentBinding(scopedRef, version)) return false;
      const normalized = normalizeAppError(error);
      applyScanEvent({ phase: "error", error: normalized });
      options.onScanError?.(normalized);
      return false;
    } finally {
      if (isCurrentBinding(scopedRef, version)) scanRequestPending.value = false;
    }
  }

  watch(bindingKey, () => {
    void bindWorkspace();
  }, { immediate: true });

  onScopeDispose(() => {
    bindingVersion += 1;
    unsubscribe?.();
    unsubscribe = null;
  });

  return {
    scanPhase,
    lastScanStats,
    scanRequestPending,
    refresh,
    startScan,
  };
}
