import { onMounted, onUnmounted, ref } from "vue";
import { normalizeAppError } from "../services/errors";
import {
  getServiceSwitches,
  setServiceEnabled,
  subscribeWorkspaceServicesChanged,
  type ServiceKindId,
  type ServiceSwitches,
} from "../services/workspaceServices";
import { useNotificationStore } from "../stores/notification";

/** App-level service switch state for one Settings > Services page. */
export function useServiceSwitches() {
  const notificationStore = useNotificationStore();
  const switches = ref<ServiceSwitches | null>(null);
  const busy = ref(false);
  let disposed = false;
  let release: (() => void) | null = null;

  async function load() {
    try {
      const next = await getServiceSwitches();
      if (!disposed) switches.value = next;
    } catch (cause) {
      const error = normalizeAppError(cause);
      notificationStore.addNotice("error", error.message, {
        code: error.code,
        operation: "loadServiceSwitches",
        replaceOperation: true,
      });
    }
  }

  async function setEnabled(service: ServiceKindId, enabled: boolean) {
    if (!switches.value || busy.value) return;
    busy.value = true;
    try {
      const next = await setServiceEnabled(service, enabled);
      if (!disposed) switches.value = next;
    } catch (cause) {
      const error = normalizeAppError(cause);
      notificationStore.addNotice("error", error.message, {
        code: error.code,
        operation: "setServiceEnabled",
        replaceOperation: true,
      });
      await load();
    } finally {
      busy.value = false;
    }
  }

  onMounted(() => {
    disposed = false;
    void load();
    void Promise.resolve(subscribeWorkspaceServicesChanged((next) => {
      if (!disposed) switches.value = next;
    })).then((unsubscribe) => {
      if (disposed) unsubscribe();
      else release = unsubscribe;
    }).catch(() => {});
  });

  onUnmounted(() => {
    disposed = true;
    release?.();
    release = null;
  });

  return { switches, busy, setEnabled, reload: load };
}
