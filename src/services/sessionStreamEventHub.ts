import type { AsyncTaskUpdatedEvent, StreamEvent } from "../types";
import type { RoutedWorkspaceEvent, WorkspaceRef } from "./project";
import type { SessionExecutionStateChanged } from "./sessionExecutionState";

export type SessionStreamEventSource =
  | { kind: "legacy" }
  | {
    kind: "workspace";
    projectId: string;
    checkoutId: string;
    workspaceGeneration: number;
    streamRevision: number;
  };

export interface SessionStreamEventDispatch {
  event: StreamEvent;
  source: SessionStreamEventSource;
}

export type SessionStreamEventListener = (dispatch: SessionStreamEventDispatch) => void;

interface SessionStreamEventConsumerSubscription {
  resolveConsumer: (dispatch: SessionStreamEventDispatch) => object | null;
  listener: SessionStreamEventListener;
}

const listeners = new Set<SessionStreamEventListener>();
const consumerSubscriptions = new Set<SessionStreamEventConsumerSubscription>();
const boundSessionIds = new Map<string, true>();
const pendingEventsBySessionId = new Map<string, SessionStreamEventDispatch[]>();
const MAX_PENDING_SESSION_COUNT = 64;
const MAX_PENDING_EVENTS_PER_SESSION = 512;
const MAX_BOUND_SESSION_COUNT = 2048;

export type SessionAsyncTaskUpdateListener = (update: AsyncTaskUpdatedEvent) => void;

interface SessionAsyncTaskUpdateConsumerSubscription {
  resolveConsumer: (update: AsyncTaskUpdatedEvent) => object | null;
  listener: SessionAsyncTaskUpdateListener;
}

const asyncTaskUpdateListeners = new Set<SessionAsyncTaskUpdateListener>();
const asyncTaskUpdateConsumerSubscriptions = new Set<SessionAsyncTaskUpdateConsumerSubscription>();
const boundAsyncTaskSessionIds = new Map<string, true>();
const pendingAsyncTaskUpdatesBySessionId = new Map<string, AsyncTaskUpdatedEvent[]>();
const MAX_PENDING_ASYNC_TASK_UPDATES_PER_SESSION = 128;

export type SessionExecutionStateListener = (update: SessionExecutionStateChanged) => void;

interface SessionExecutionStateConsumerSubscription {
  resolveConsumer: (update: SessionExecutionStateChanged) => object | null;
  listener: SessionExecutionStateListener;
}

const executionStateConsumerSubscriptions = new Set<SessionExecutionStateConsumerSubscription>();

function rememberPendingDispatch(
  dispatch: SessionStreamEventDispatch,
  options: { includeBound?: boolean } = {},
): void {
  const sessionId = dispatch.event.sessionId.trim();
  if (!sessionId || (!options.includeBound && boundSessionIds.has(sessionId))) return;
  const pending = pendingEventsBySessionId.get(sessionId) ?? [];
  pending.push(dispatch);
  if (pending.length > MAX_PENDING_EVENTS_PER_SESSION) {
    pending.splice(0, pending.length - MAX_PENDING_EVENTS_PER_SESSION);
  }
  pendingEventsBySessionId.delete(sessionId);
  pendingEventsBySessionId.set(sessionId, pending);
  if (pendingEventsBySessionId.size > MAX_PENDING_SESSION_COUNT) {
    const oldestSessionId = pendingEventsBySessionId.keys().next().value as string | undefined;
    if (oldestSessionId && oldestSessionId !== sessionId) {
      pendingEventsBySessionId.delete(oldestSessionId);
    }
  }
}

/**
 * Frontend-wide fan-out for normalized session stream events.
 *
 * Tauri transport ownership stays in the app bootstrap. Session consumers
 * subscribe here, so panes never create competing raw event listeners and a
 * component remount cannot race asynchronous `listen()` registration.
 */
export function publishSessionStreamEvent(dispatch: SessionStreamEventDispatch): void {
  rememberPendingDispatch(dispatch);
  for (const listener of [...listeners]) listener(dispatch);
  const deliveredConsumers = new Set<object>();
  for (const subscription of [...consumerSubscriptions]) {
    const consumer = subscription.resolveConsumer(dispatch);
    if (!consumer || deliveredConsumers.has(consumer)) continue;
    deliveredConsumers.add(consumer);
    subscription.listener(dispatch);
  }
  // A durable session stays marked as bound for the frontend lifetime, while
  // its editor can briefly have no reducer during pane/window reparenting.
  // Preserve events received in that gap so the next mount can catch up.
  const sessionId = dispatch.event.sessionId.trim();
  if (deliveredConsumers.size === 0 && boundSessionIds.has(sessionId)) {
    rememberPendingDispatch(dispatch, { includeBound: true });
  }
}

export function subscribeSessionStreamEvents(listener: SessionStreamEventListener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function rememberPendingAsyncTaskUpdate(
  update: AsyncTaskUpdatedEvent,
  options: { includeBound?: boolean } = {},
): void {
  const sessionId = update.sessionId.trim();
  if (!sessionId || (!options.includeBound && boundAsyncTaskSessionIds.has(sessionId))) return;
  const pending = pendingAsyncTaskUpdatesBySessionId.get(sessionId) ?? [];
  pending.push(update);
  if (pending.length > MAX_PENDING_ASYNC_TASK_UPDATES_PER_SESSION) {
    pending.splice(0, pending.length - MAX_PENDING_ASYNC_TASK_UPDATES_PER_SESSION);
  }
  pendingAsyncTaskUpdatesBySessionId.delete(sessionId);
  pendingAsyncTaskUpdatesBySessionId.set(sessionId, pending);
  if (pendingAsyncTaskUpdatesBySessionId.size > MAX_PENDING_SESSION_COUNT) {
    const oldestSessionId = pendingAsyncTaskUpdatesBySessionId.keys().next().value as string | undefined;
    if (oldestSessionId && oldestSessionId !== sessionId) {
      pendingAsyncTaskUpdatesBySessionId.delete(oldestSessionId);
    }
  }
}

/**
 * Fans out background task lifecycle updates to every mounted representation
 * of the owning session. The app bootstrap remains the sole Tauri listener;
 * scoped editors consume updates through this hub instead of the active
 * legacy-chat projection.
 */
export function publishSessionAsyncTaskUpdate(update: AsyncTaskUpdatedEvent): void {
  rememberPendingAsyncTaskUpdate(update);
  for (const listener of [...asyncTaskUpdateListeners]) listener(update);
  const deliveredConsumers = new Set<object>();
  for (const subscription of [...asyncTaskUpdateConsumerSubscriptions]) {
    const consumer = subscription.resolveConsumer(update);
    if (!consumer || deliveredConsumers.has(consumer)) continue;
    deliveredConsumers.add(consumer);
    subscription.listener(update);
  }
  const sessionId = update.sessionId.trim();
  if (deliveredConsumers.size === 0 && boundAsyncTaskSessionIds.has(sessionId)) {
    rememberPendingAsyncTaskUpdate(update, { includeBound: true });
  }
}

export function subscribeSessionAsyncTaskUpdates(
  listener: SessionAsyncTaskUpdateListener,
): () => void {
  asyncTaskUpdateListeners.add(listener);
  return () => asyncTaskUpdateListeners.delete(listener);
}

export function subscribeSessionAsyncTaskUpdateConsumer(
  resolveConsumer: SessionAsyncTaskUpdateConsumerSubscription["resolveConsumer"],
  listener: SessionAsyncTaskUpdateListener,
): () => void {
  const subscription = { resolveConsumer, listener };
  asyncTaskUpdateConsumerSubscriptions.add(subscription);
  return () => asyncTaskUpdateConsumerSubscriptions.delete(subscription);
}

/**
 * Marks a durable session as owned by a reducer and returns task updates that
 * arrived while its editor was mounting or being reparented.
 */
export function bindSessionAsyncTaskUpdateConsumer(
  sessionId: string,
): AsyncTaskUpdatedEvent[] {
  const normalizedSessionId = sessionId.trim();
  if (!normalizedSessionId) return [];
  boundAsyncTaskSessionIds.delete(normalizedSessionId);
  boundAsyncTaskSessionIds.set(normalizedSessionId, true);
  if (boundAsyncTaskSessionIds.size > MAX_BOUND_SESSION_COUNT) {
    const oldestSessionId = boundAsyncTaskSessionIds.keys().next().value as string | undefined;
    if (oldestSessionId && oldestSessionId !== normalizedSessionId) {
      boundAsyncTaskSessionIds.delete(oldestSessionId);
    }
  }
  const pending = pendingAsyncTaskUpdatesBySessionId.get(normalizedSessionId) ?? [];
  pendingAsyncTaskUpdatesBySessionId.delete(normalizedSessionId);
  return pending;
}

/** Publishes persisted model/effort changes to every mounted pane for that session. */
export function publishSessionExecutionState(update: SessionExecutionStateChanged): void {
  const deliveredConsumers = new Set<object>();
  for (const subscription of [...executionStateConsumerSubscriptions]) {
    const consumer = subscription.resolveConsumer(update);
    if (!consumer || deliveredConsumers.has(consumer)) continue;
    deliveredConsumers.add(consumer);
    subscription.listener(update);
  }
}

export function subscribeSessionExecutionStateConsumer(
  resolveConsumer: SessionExecutionStateConsumerSubscription["resolveConsumer"],
  listener: SessionExecutionStateListener,
): () => void {
  const subscription = { resolveConsumer, listener };
  executionStateConsumerSubscriptions.add(subscription);
  return () => executionStateConsumerSubscriptions.delete(subscription);
}

/**
 * Registers a stateful reducer consumer. Several panes may resolve to the same
 * session state; the hub invokes that shared consumer once for each event.
 */
export function subscribeSessionStreamEventConsumer(
  resolveConsumer: SessionStreamEventConsumerSubscription["resolveConsumer"],
  listener: SessionStreamEventListener,
): () => void {
  const subscription = { resolveConsumer, listener };
  consumerSubscriptions.add(subscription);
  return () => consumerSubscriptions.delete(subscription);
}

/**
 * Binds an exact durable session to a reducer and returns events that arrived
 * before the chat launch IPC resolved with its session id. A session is bound
 * once per frontend lifetime; later panes read the shared state or hydrate the
 * backend runtime snapshot instead of replaying deltas again.
 */
export function bindSessionStreamEventConsumer(sessionId: string): SessionStreamEventDispatch[] {
  const normalizedSessionId = sessionId.trim();
  if (!normalizedSessionId) return [];
  boundSessionIds.delete(normalizedSessionId);
  boundSessionIds.set(normalizedSessionId, true);
  if (boundSessionIds.size > MAX_BOUND_SESSION_COUNT) {
    const oldestSessionId = boundSessionIds.keys().next().value as string | undefined;
    if (oldestSessionId && oldestSessionId !== normalizedSessionId) {
      boundSessionIds.delete(oldestSessionId);
    }
  }
  const pending = pendingEventsBySessionId.get(normalizedSessionId) ?? [];
  pendingEventsBySessionId.delete(normalizedSessionId);
  return pending;
}

export function workspaceStreamEventSource(
  event: RoutedWorkspaceEvent<StreamEvent>,
): SessionStreamEventSource {
  return {
    kind: "workspace",
    projectId: event.projectId,
    checkoutId: event.checkoutId,
    workspaceGeneration: event.workspaceGeneration,
    streamRevision: event.streamRevision,
  };
}

export function sessionStreamSourceMatchesWorkspace(
  source: SessionStreamEventSource,
  workspaceRef: WorkspaceRef | null | undefined,
): boolean {
  if (source.kind === "legacy") return true;
  if (!workspaceRef || source.checkoutId !== workspaceRef.checkoutId) return false;
  return workspaceRef.expectedGeneration == null
    || source.workspaceGeneration === workspaceRef.expectedGeneration;
}
