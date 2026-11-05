// One project-scope setting of the focused checkout, stored in the project's
// `Locus/config.json`. Same shape as `useSetting`; `ready` stays false while
// no project is focused. The value is reread when the focused checkout
// changes, which includes `workspace-setup-changed` updates from other
// windows and the agent.
import { computed, ref, watch } from "vue";
import { getSettings, setSettings, settingsErrorMessage } from "../services/settings";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import type { UseSetting } from "./useSettingsStore";

export function useProjectSetting<T>(key: string, read: (raw: unknown) => T): UseSetting<T> {
  const workspaceContext = useWorkspaceContextStore();
  const raw = ref<unknown>(undefined);
  const loadedFor = ref<string | null>(null);
  const busy = ref(false);
  const error = ref<string | null>(null);
  let request = 0;

  const checkoutId = computed(() => workspaceContext.focusedWorkspaceRef?.checkoutId ?? null);

  async function load(): Promise<void> {
    const workspaceRef = workspaceContext.focusedWorkspaceRef;
    const current = ++request;
    if (!workspaceRef) {
      raw.value = undefined;
      loadedFor.value = null;
      return;
    }
    try {
      const values = await getSettings([key], workspaceRef);
      if (current !== request) return;
      raw.value = values[key];
      loadedFor.value = workspaceRef.checkoutId;
      error.value = null;
    } catch (cause) {
      if (current !== request) return;
      raw.value = undefined;
      loadedFor.value = null;
      error.value = settingsErrorMessage(cause);
    }
  }

  // The checkout descriptor is replaced whenever its runtime state changes.
  watch(() => [checkoutId.value, workspaceContext.focusedCheckout] as const, () => void load(), {
    immediate: true,
  });

  async function set(value: T): Promise<void> {
    const workspaceRef = workspaceContext.focusedWorkspaceRef;
    if (!workspaceRef) throw new Error("No project is focused.");
    const previous = raw.value;
    raw.value = value;
    busy.value = true;
    try {
      const [change] = await setSettings([{ key, value }], workspaceRef);
      if (change) raw.value = change.new;
      error.value = null;
    } catch (cause) {
      raw.value = previous;
      error.value = settingsErrorMessage(cause);
      throw cause;
    } finally {
      busy.value = false;
    }
  }

  return {
    value: computed(() => read(raw.value)),
    ready: computed(() => loadedFor.value !== null && loadedFor.value === checkoutId.value),
    busy: computed(() => busy.value),
    error: computed(() => error.value),
    set,
  };
}
