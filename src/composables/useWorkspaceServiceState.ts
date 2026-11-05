import { computed, ref, toValue, watch, type MaybeRefOrGetter } from "vue";
import {
  getWorkspaceServiceStates,
  type WorkspaceRef,
  type WorkspaceServiceStateSnapshot,
  type WorkspaceServiceStatus,
} from "../services/project";
import { startWorkspaceService, type ServiceKindId } from "../services/workspaceServices";
import { useWorkspaceContextStore } from "../stores/workspaceContext";

/**
 * Mode and lifecycle of one workspace service for status items. The initial
 * state is read once per workspace; later starts and stops arrive as the
 * `<kind>-service-state` workspace event, so nothing polls.
 */
export function useWorkspaceServiceState(
  workspaceRef: MaybeRefOrGetter<WorkspaceRef | null | undefined>,
  service: ServiceKindId,
) {
  const workspaceContextStore = useWorkspaceContextStore();
  const fetched = ref<WorkspaceServiceStateSnapshot | null>(null);
  const starting = ref(false);
  let loadVersion = 0;

  const checkoutId = computed(() => toValue(workspaceRef)?.checkoutId ?? "");
  const mode = computed(() => (
    workspaceContextStore.checkoutsById[checkoutId.value]?.runtime?.serviceModes?.[service] ?? null
  ));
  const pushed = computed(() => {
    const event = workspaceContextStore.workspaceStateByCheckout[checkoutId.value]
      ?.events[`${service}-service-state`];
    return (event?.payload as WorkspaceServiceStateSnapshot | undefined) ?? null;
  });
  const status = computed<WorkspaceServiceStatus | null>(() => (
    pushed.value?.status ?? fetched.value?.status ?? null
  ));
  /** Lazy mode and not started yet (or reaped after idling). */
  const dormant = computed(() => (
    mode.value === "lazy"
    && !starting.value
    && status.value !== "running"
    && status.value !== "starting"
  ));

  async function load() {
    const ref = toValue(workspaceRef);
    const version = ++loadVersion;
    fetched.value = null;
    if (!ref || !mode.value) return;
    try {
      const states = await getWorkspaceServiceStates(ref);
      if (version === loadVersion) {
        fetched.value = states.find((state) => state.serviceKind === service) ?? null;
      }
    } catch {
      // Status items fall back to their own connection state.
    }
  }

  async function start() {
    const ref = toValue(workspaceRef);
    if (!ref || starting.value) return;
    starting.value = true;
    try {
      await startWorkspaceService(ref, service);
      await load();
    } finally {
      starting.value = false;
    }
  }

  watch(
    () => `${checkoutId.value}:${toValue(workspaceRef)?.expectedGeneration ?? ""}:${mode.value ?? ""}`,
    () => { void load(); },
    { immediate: true },
  );

  return { mode, status, dormant, starting, start, reload: load };
}
