import { ref } from "vue";
import { getLocusRuntime } from "../services/locusRuntime";
import {
  REMOTE_ACCESS_PAIRING_REQUEST_EVENT,
  REMOTE_ACCESS_PAIRING_SETTLED_EVENT,
  resolveRemotePairing,
  type RemotePairingRequest,
} from "../services/remoteAccess";

/** The pairing request waiting for a decision on this desktop. Shared by the
 * pairing dialog (which answers inline while open) and the app-level prompt
 * (which answers when the dialog is closed). */
const current = ref<RemotePairingRequest | null>(null);
const dialogOpen = ref(false);
const responding = ref(false);
let subscription: Promise<void> | null = null;

function subscribe(): Promise<void> {
  subscription ??= (async () => {
    const runtime = getLocusRuntime();
    await runtime.subscribe<RemotePairingRequest>(REMOTE_ACCESS_PAIRING_REQUEST_EVENT, (request) => {
      current.value = request;
    });
    // Answered elsewhere (another window, timeout): drop the stale prompt.
    await runtime.subscribe<string>(REMOTE_ACCESS_PAIRING_SETTLED_EVENT, (requestId) => {
      if (current.value?.requestId === requestId) current.value = null;
    });
  })().catch((error) => {
    subscription = null;
    console.warn("[remote-access] pairing request subscription failed", error);
  });
  return subscription;
}

export function useRemotePairingRequests() {
  void subscribe();

  async function respond(allow: boolean): Promise<void> {
    const request = current.value;
    if (!request || responding.value) return;
    responding.value = true;
    try {
      await resolveRemotePairing(request.requestId, allow);
    } finally {
      if (current.value?.requestId === request.requestId) current.value = null;
      responding.value = false;
    }
  }

  return { current, dialogOpen, responding, respond };
}
