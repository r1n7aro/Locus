import { reactive } from "vue";
import { getLocusRuntime } from "../services/locusRuntime";
import {
  getSessionUndoEnabled,
  setSessionUndoEnabled,
} from "../services/system";

export const SESSION_UNDO_ENABLED_CHANGED_EVENT = "session-undo-enabled-changed";

interface SessionUndoEnabledChangedEvent {
  enabled: boolean;
}

interface SessionUndoSettingsState {
  enabled: boolean;
  ready: boolean;
  busy: boolean;
}

const state = reactive<SessionUndoSettingsState>({
  enabled: true,
  ready: false,
  busy: false,
});

let loadRequest: Promise<boolean> | null = null;
let subscriptionStarted = false;

function ensureSubscription(): void {
  if (subscriptionStarted) return;
  subscriptionStarted = true;
  void getLocusRuntime()
    .subscribe<SessionUndoEnabledChangedEvent>(
      SESSION_UNDO_ENABLED_CHANGED_EVENT,
      (event) => {
        state.enabled = event.enabled;
        state.ready = true;
      },
    )
    .catch((error: unknown) => {
      subscriptionStarted = false;
      console.warn("[session-undo] failed to subscribe to setting changes:", error);
    });
}

async function load(force = false): Promise<boolean> {
  ensureSubscription();
  if (state.ready && !force) return state.enabled;
  if (loadRequest) return loadRequest;

  loadRequest = getSessionUndoEnabled()
    .then((enabled) => {
      state.enabled = enabled;
      state.ready = true;
      return enabled;
    })
    .catch((error) => {
      state.ready = true;
      throw error;
    })
    .finally(() => {
      loadRequest = null;
    });
  return loadRequest;
}

async function setEnabled(enabled: boolean): Promise<void> {
  ensureSubscription();
  if (state.busy || (state.ready && state.enabled === enabled)) return;

  const previous = state.enabled;
  state.enabled = enabled;
  state.ready = true;
  state.busy = true;
  try {
    await setSessionUndoEnabled(enabled);
  } catch (error) {
    state.enabled = previous;
    throw error;
  } finally {
    state.busy = false;
  }
}

export function useSessionUndoSettings() {
  ensureSubscription();
  return { state, load, setEnabled };
}
