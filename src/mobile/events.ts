import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import type { RoutedWorkspaceEvent } from "../services/project";
import {
  publishSessionContentChange,
  publishSessionStreamEvent,
  workspaceStreamEventSource,
} from "../services/sessionStreamEventHub";
import { WORKSPACE_EVENT_NAME } from "../services/workspaceEventHub";
import type { KnowledgeChangedEvent, SessionContentChangedEvent, StreamEvent } from "../types";
import { invalidateSkills } from "../composables/useSkills";

type Listener = (event: RoutedWorkspaceEvent<unknown>) => void;

function skillsMayHaveChanged(change: KnowledgeChangedEvent): boolean {
  return change.docType === "skill" || !change.docType;
}

const listeners = new Set<Listener>();

/** Raw routed events for screens (session list refresh, titles). */
export function onWorkspaceEvent(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The mobile counterpart of the desktop bootstrap's event fan-out: feeds the
 * session stream hub that the shared session controller consumes. Only events
 * matching this client's interests arrive at all (filtered on the desktop). */
export function startRemoteEventDispatch(): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<RoutedWorkspaceEvent<unknown>>(
    WORKSPACE_EVENT_NAME,
    (event) => {
      if (event.eventName === "stream-event") {
        publishSessionStreamEvent({
          event: event.payload as StreamEvent,
          source: workspaceStreamEventSource(event),
        });
      } else if (event.eventName === "session-content-changed") {
        publishSessionContentChange({
          sessionId: (event.payload as SessionContentChangedEvent).sessionId,
          source: workspaceStreamEventSource(event),
        });
      } else if (
        event.eventName === "plugins-changed"
        || (event.eventName === "knowledge-changed" && skillsMayHaveChanged(event.payload as KnowledgeChangedEvent))
      ) {
        // The composer reloads the checkout's skills (slash commands) on next focus.
        invalidateSkills({
          checkoutId: event.checkoutId,
          expectedGeneration: event.workspaceGeneration,
          expectedMaterializationEpoch: event.materializationEpoch ?? null,
        });
      }
      for (const listener of [...listeners]) {
        try {
          listener(event);
        } catch (error) {
          console.error("[remote] workspace event listener failed", error);
        }
      }
    },
    { owner: "mobile.dispatch" },
  );
}
