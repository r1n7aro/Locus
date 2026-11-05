import type { Event, UnlistenFn } from "@tauri-apps/api/event";
import { listenWindowEvent } from "./windowEventTransport";
import { getCurrentTauriWindowLabel } from "./tauriRuntime";
import { flushWindowEventInterests, retainWindowEventInterest, type WindowEventInterest } from "./windowEventInterests";
import { remoteClientTransport } from "./remoteClient";
import type { RoutedWorkspaceEvent } from "./project";

export const WORKSPACE_EVENT_NAME = "locus://workspace-event";
const TRACE_LIMIT = 256;

export interface WorkspaceEventSubscriptionOptions {
  owner: string;
  signal?: AbortSignal;
  interests?: WindowEventInterest[];
}

type WorkspaceEvent = RoutedWorkspaceEvent<unknown>;
type Handler = (event: Event<WorkspaceEvent>) => unknown;
type Transport = (handler: Handler) => Promise<UnlistenFn>;
type Envelope = Omit<WorkspaceEvent, "payload">;

interface Subscriber {
  id: number;
  owner: string;
  handler: Handler;
  subscribedAt: number;
  deliveries: number;
  errors: number;
  unsubscribe: () => void;
}

interface Trace {
  timestampMs: number;
  action: "subscribe" | "unsubscribe" | "receive" | "error";
  subscriptionId?: number;
  /** receive: delivery targets in dispatch order (one trace per event, not per subscriber). */
  subscriptionIds?: number[];
  /** receive: synchronous time spent in all handlers for this event. */
  dispatchMs?: number;
  owner?: string;
  envelope?: Envelope;
  error?: string;
}

export interface WorkspaceEventSnapshotOptions {
  /** Keep only traces recorded at or after this epoch timestamp. */
  sinceMs?: number;
}

function envelopeOf(event: WorkspaceEvent): Envelope {
  return {
    eventName: event.eventName,
    streamRevision: event.streamRevision,
    projectId: event.projectId,
    checkoutId: event.checkoutId,
    workspaceGeneration: event.workspaceGeneration,
    materializationEpoch: event.materializationEpoch,
    serviceInstanceId: event.serviceInstanceId,
    serviceGeneration: event.serviceGeneration,
  };
}

/** One instance per WebView JS realm, not per project, checkout, pane or component.
 * This layer deliberately does not filter, deduplicate, replay or reorder workspace
 * envelopes. Consumers retain their own worktree and materialization guards.
 */
export function createWorkspaceEventHub(transport: Transport, nativeChannelCount = 1) {
  const subscribers = new Map<number, Subscriber>();
  const traces: Trace[] = [];
  let nextId = 1;
  let registration: Promise<void> | null = null;
  let nativeRelease: UnlistenFn | null = null;
  let disposed = false;
  let diagnosticsEnabled = false;
  let received = 0;

  function record(trace: Omit<Trace, "timestampMs">): Trace | null {
    if (!diagnosticsEnabled) return null;
    if (traces.length === TRACE_LIMIT) traces.shift();
    const recorded: Trace = { timestampMs: Date.now(), ...trace };
    traces.push(recorded);
    return recorded;
  }

  function reportError(subscriber: Subscriber, event: WorkspaceEvent, error: unknown): void {
    subscriber.errors += 1;
    record({
      action: "error", subscriptionId: subscriber.id, owner: subscriber.owner,
      envelope: envelopeOf(event), error: String(error).slice(0, 500),
    });
    console.error("[WorkspaceEvents] subscriber failed", {
      subscriptionId: subscriber.id, owner: subscriber.owner,
      ...envelopeOf(event),
    }, error);
  }

  function dispatch(event: Event<WorkspaceEvent>): void {
    if (disposed) return;
    received += 1;
    // Snapshot excludes subscribers added during delivery; the membership check
    // also skips a subscriber removed by an earlier handler in this same event.
    const targets = [...subscribers.values()];
    const trace = diagnosticsEnabled
      ? record({
        action: "receive", envelope: envelopeOf(event.payload),
        subscriptionIds: targets.map((subscriber) => subscriber.id),
      })
      : null;
    const dispatchStartedAt = trace ? performance.now() : 0;
    for (const subscriber of targets) {
      if (!subscribers.has(subscriber.id)) continue;
      subscriber.deliveries += 1;
      try {
        const result = subscriber.handler(event);
        if (result && typeof (result as PromiseLike<unknown>).then === "function") {
          void Promise.resolve(result).catch((error) => reportError(subscriber, event.payload, error));
        }
      } catch (error) {
        reportError(subscriber, event.payload, error);
      }
    }
    if (trace) trace.dispatchMs = Math.round((performance.now() - dispatchStartedAt) * 100) / 100;
  }

  function ensureRegistered(): Promise<void> {
    if (registration) return registration;
    // Defer transport startup until registration is assigned, so concurrent and
    // reentrant subscribers can never install a second native listener.
    const pending = Promise.resolve().then(() => transport(dispatch)).then((release) => {
      if (disposed) {
        release();
      } else {
        nativeRelease = release;
      }
    });
    registration = pending;
    void pending.catch(() => {
      if (registration === pending) registration = null;
    });
    return pending;
  }

  function subscribe(handler: Handler, options: WorkspaceEventSubscriptionOptions) {
    const id = nextId++;
    const unsubscribe = () => {
      if (!subscribers.delete(id)) return;
      options.signal?.removeEventListener("abort", unsubscribe);
      record({ action: "unsubscribe", subscriptionId: id, owner: options.owner });
      // Keep the native listener even when idle. Calling unlisten here would
      // reintroduce Tauri's race with already queued workspace events.
    };
    if (disposed || options.signal?.aborted) {
      return { id, unsubscribe, ready: Promise.resolve() };
    }
    subscribers.set(id, {
      id, owner: options.owner, handler, subscribedAt: Date.now(),
      deliveries: 0, errors: 0, unsubscribe,
    });
    options.signal?.addEventListener("abort", unsubscribe, { once: true });
    record({ action: "subscribe", subscriptionId: id, owner: options.owner });
    const ready = ensureRegistered().catch((error: unknown) => {
      unsubscribe();
      throw error;
    });
    return { id, unsubscribe, ready };
  }

  return {
    subscribe,
    ensureRegistered,
    setDiagnosticsEnabled(enabled: boolean) {
      diagnosticsEnabled = enabled;
      if (!enabled) traces.length = 0;
    },
    snapshot(options: WorkspaceEventSnapshotOptions = {}) {
      const sinceMs = options.sinceMs;
      const selected = typeof sinceMs === "number" && Number.isFinite(sinceMs)
        ? traces.filter((trace) => trace.timestampMs >= sinceMs)
        : traces;
      return {
        nativeEvent: WORKSPACE_EVENT_NAME,
        state: disposed ? "disposed" : nativeRelease ? "ready" : registration ? "starting" : "idle",
        nativeListenerCount: nativeRelease ? nativeChannelCount : 0,
        received,
        subscribers: [...subscribers.values()].map((subscriber) => ({
          id: subscriber.id,
          owner: subscriber.owner,
          subscribedAt: subscriber.subscribedAt,
          deliveries: subscriber.deliveries,
          errors: subscriber.errors,
        })),
        traces: selected.map((trace) => ({
          ...trace,
          ...(trace.envelope ? { envelope: { ...trace.envelope } } : {}),
          ...(trace.subscriptionIds ? { subscriptionIds: [...trace.subscriptionIds] } : {}),
        })),
      };
    },
    // Explicit teardown is for isolated tests. Real windows let WebView teardown
    // release native resources; pagehide/beforeunload must not start more IPC.
    dispose() {
      disposed = true;
      for (const subscriber of [...subscribers.values()]) subscriber.unsubscribe();
      nativeRelease?.();
      nativeRelease = null;
    },
  };
}

type WorkspaceEventHub = ReturnType<typeof createWorkspaceEventHub>;
let windowHub: WorkspaceEventHub | undefined = import.meta.hot?.data?.workspaceEventHub;

function getWindowHub(): WorkspaceEventHub {
  windowHub ??= createWorkspaceEventHub(
    (handler) => {
      // Mobile remote client: events arrive from the desktop, not Tauri.
      const remote = remoteClientTransport();
      if (remote) {
        return Promise.resolve(remote.subscribeWorkspaceEvents((payload) =>
          handler({ event: WORKSPACE_EVENT_NAME, id: 0, payload } as Event<WorkspaceEvent>)));
      }
      return listenWindowEvent(WORKSPACE_EVENT_NAME, handler);
    },
    !remoteClientTransport() && getCurrentTauriWindowLabel() ? 2 : 1,
  );
  return windowHub;
}

export function ensureWorkspaceEventTransport(): Promise<void> {
  return getWindowHub().ensureRegistered();
}

export function listenWorkspaceEvent<T extends WorkspaceEvent>(
  owner: string,
  handler: (event: Event<T>) => unknown,
  options: { signal?: AbortSignal; interests?: WindowEventInterest[] } = {},
): Promise<UnlistenFn> {
  const subscription = getWindowHub().subscribe(handler as Handler, { owner, ...options });
  const interests = options.signal?.aborted ? [] : options.interests ?? [];
  const releases = interests.map(retainWindowEventInterest);
  const release = () => {
    subscription.unsubscribe();
    releases.forEach(stop => stop());
    options.signal?.removeEventListener("abort", release);
  };
  options.signal?.addEventListener("abort", release, { once: true });
  return subscription.ready.then(async () => {
    if (interests.length) await flushWindowEventInterests();
    return release;
  }).catch(error => { release(); throw error; });
}

export function setWorkspaceEventDiagnosticsEnabled(enabled: boolean): void {
  getWindowHub().setDiagnosticsEnabled(enabled);
}

export function snapshotWorkspaceEvents(options: WorkspaceEventSnapshotOptions = {}) {
  const internals = typeof window === "undefined" ? undefined : (
    window as unknown as { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } }
  ).__TAURI_INTERNALS__;
  return { windowLabel: internals?.metadata?.currentWindow?.label ?? null, ...getWindowHub().snapshot(options) };
}

export function resetWorkspaceEventHubForTests(): void {
  windowHub?.dispose();
  windowHub = undefined;
}

if (typeof window !== "undefined") {
  // Read-only CDP entry: no payloads, callback closures or mutation methods.
  Object.defineProperty(window, "__LOCUS_WORKSPACE_EVENTS__", {
    configurable: true,
    value: { snapshot: snapshotWorkspaceEvents },
  });
}
if (typeof import.meta.hot?.dispose === "function") {
  import.meta.hot.dispose((data) => { data.workspaceEventHub = windowHub; });
}
