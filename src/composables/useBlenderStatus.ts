import { computed, onUnmounted, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import {
  getBlenderWorkspaceStatus,
  subscribeBlenderInstallationsChanged,
  subscribeBlenderInstancesChanged,
  type BlenderInstance,
  type BlenderWorkspaceStatus,
} from "../services/blender";
import type { WorkspaceRef } from "../services/project";
import type { RuntimeUnsubscribe } from "../services/locusRuntime";

export type BlenderTone = "success" | "danger" | "warning" | "accent" | "muted";

/** Main-thread silence after which an interactive instance counts as stuck. */
const STUCK_MS = 30_000;
const REFRESH_DEBOUNCE_MS = 120;

/**
 * Blender status for one workspace and session. Event driven: the app emits
 * `blender-instances-changed` / `blender-installations-changed`; nothing
 * polls. Popover-only details (header scan, add-on state) load while
 * `detail` is true.
 */
export function useBlenderStatus(options: {
  workspaceRef: MaybeRefOrGetter<WorkspaceRef | null | undefined>;
  sessionId: MaybeRefOrGetter<string | null | undefined>;
  /** The status item is shown (capability present, surface visible). */
  enabled: MaybeRefOrGetter<boolean>;
  /** The popover is open. */
  detail: MaybeRefOrGetter<boolean>;
}) {
  const status = ref<BlenderWorkspaceStatus | null>(null);
  const error = ref("");
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let unsubscribers: Promise<RuntimeUnsubscribe>[] = [];

  async function refresh() {
    const workspaceRef = toValue(options.workspaceRef);
    if (!workspaceRef || !toValue(options.enabled)) return;
    const version = ++seq;
    try {
      const next = await getBlenderWorkspaceStatus(
        workspaceRef,
        toValue(options.sessionId) ?? null,
        toValue(options.detail),
      );
      if (version !== seq) return;
      // Keep popover details from the last detailed read while closed.
      status.value = toValue(options.detail) ? next : { ...status.value, ...next };
      error.value = "";
    } catch (cause) {
      if (version === seq) error.value = cause instanceof Error ? cause.message : String(cause);
    }
  }

  function scheduleRefresh() {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      void refresh();
    }, REFRESH_DEBOUNCE_MS);
  }

  function stopListening() {
    for (const pending of unsubscribers) void pending.then((unsubscribe) => unsubscribe(), () => undefined);
    unsubscribers = [];
  }

  watch(
    () => Boolean(toValue(options.enabled) && toValue(options.workspaceRef)),
    (active) => {
      stopListening();
      if (!active) return;
      unsubscribers = [
        subscribeBlenderInstancesChanged(scheduleRefresh),
        subscribeBlenderInstallationsChanged(scheduleRefresh),
      ];
      void refresh();
    },
    { immediate: true },
  );

  watch(
    () => [
      toValue(options.workspaceRef)?.checkoutId ?? "",
      toValue(options.sessionId) ?? "",
      toValue(options.detail),
    ],
    () => { void refresh(); },
  );

  onUnmounted(() => {
    stopListening();
    if (timer) clearTimeout(timer);
  });

  const instances = computed<BlenderInstance[]>(() => status.value?.instances ?? []);
  const workspaceInstances = computed(() => instances.value.filter((instance) => instance.inWorkspace));
  const otherInstances = computed(() => instances.value.filter((instance) => !instance.inWorkspace));
  const busy = computed(() => workspaceInstances.value.some((instance) => instance.busy));
  const stuck = computed(() => workspaceInstances.value.some((instance) => (
    instance.mode === "interactive" && (instance.mainThreadStallMs ?? 0) >= STUCK_MS
  )));
  const hostError = computed(() => workspaceInstances.value.some((instance) => instance.connection === "host_error"));
  const addonProblem = computed(() => {
    const addon = status.value?.addon;
    return addon?.status === "missing" || addon?.status === "outdated" || (addon?.outdatedInstances.length ?? 0) > 0;
  });

  const tone = computed<BlenderTone>(() => {
    if (hostError.value) return "danger";
    if (stuck.value || addonProblem.value) return "warning";
    if (busy.value) return "accent";
    if (workspaceInstances.value.some((instance) => instance.connection === "connected")) return "success";
    return "muted";
  });

  return {
    status,
    error,
    refresh,
    instances,
    workspaceInstances,
    otherInstances,
    busy,
    stuck,
    hostError,
    addonProblem,
    tone,
  };
}
