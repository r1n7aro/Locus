import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { hasTauriWindowRuntime } from "./tauriRuntime";
import { listenWindowEvent } from "./windowEventTransport";
import {
  listenWorkspaceEvent,
  WORKSPACE_EVENT_NAME,
  type WorkspaceEventSubscriptionOptions,
} from "./workspaceEventHub";
import type { RoutedWorkspaceEvent } from "./project";

/** `remote`: the Locus Remote mobile app talking to a desktop over the network. */
export type LocusRuntimeKind = "tauri" | "browser" | "unity" | "remote";
export type RuntimeUnsubscribe = () => void;

export interface LocusRuntimeInvokeActivity {
  phase: "started" | "settled";
  id: number;
  command: string;
  startedAtMs: number;
}

export type LocusRuntimeInvokeActivityListener = (
  activity: LocusRuntimeInvokeActivity,
) => void;

export interface LocusRuntime {
  kind: LocusRuntimeKind;
  invoke<T>(command: string, args?: Record<string, unknown>, signal?: AbortSignal): Promise<T>;
  subscribe<T>(
    eventName: string,
    handler: (payload: T) => unknown,
    options?: WorkspaceEventSubscriptionOptions,
  ): Promise<RuntimeUnsubscribe>;
}

const invokeActivityListeners = new Set<LocusRuntimeInvokeActivityListener>();
let installedRuntime: LocusRuntime | null = null;
let invocationSignal: AbortSignal | undefined;
/** Scope the immediate service invocation; the transport owns the signal
 * after dispatch. Never hold ambient state across an asynchronous boundary. */
export function withLocusInvocationSignal<T>(signal: AbortSignal, dispatch: () => T): T {
  const previous = invocationSignal;
  invocationSignal = signal;
  try { return dispatch(); } finally { invocationSignal = previous; }
}
/** A native host supplies transport without impersonating Tauri internals. */
export function installLocusRuntime(runtime: LocusRuntime): () => void {
  if (installedRuntime) throw new Error("A Locus host runtime is already installed.");
  installedRuntime = runtime;
  return () => { if (installedRuntime === runtime) installedRuntime = null; };
}
let nextInvokeActivityId = 1;

function publishInvokeActivity(activity: LocusRuntimeInvokeActivity): void {
  for (const listener of invokeActivityListeners) {
    try {
      listener(activity);
    } catch {
      // Diagnostics are observers and must never affect the IPC request.
    }
  }
}

export function subscribeLocusRuntimeInvokeActivity(
  listener: LocusRuntimeInvokeActivityListener,
): RuntimeUnsubscribe {
  invokeActivityListeners.add(listener);
  return () => invokeActivityListeners.delete(listener);
}

function invokeTauriRuntime<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (invokeActivityListeners.size === 0) {
    return tauriInvoke<T>(command, args);
  }

  const activity = {
    id: nextInvokeActivityId++,
    command,
    startedAtMs: Date.now(),
  };
  publishInvokeActivity({ phase: "started", ...activity });

  let request: Promise<T>;
  try {
    request = tauriInvoke<T>(command, args);
  } catch (error) {
    publishInvokeActivity({ phase: "settled", ...activity });
    throw error;
  }

  void request.then(
    () => publishInvokeActivity({ phase: "settled", ...activity }),
    () => publishInvokeActivity({ phase: "settled", ...activity }),
  );
  return request;
}

function invokeRuntime<T>(
  kind: LocusRuntimeKind,
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (kind === "tauri") {
    return invokeTauriRuntime<T>(command, args);
  }

  return Promise.reject(new Error("Locus runtime is unavailable in this browser context."));
}

export function invokeLocusRuntime<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (installedRuntime) return installedRuntime.invoke<T>(command, args, invocationSignal);
  return invokeRuntime<T>(resolveRuntimeKind(), command, args);
}

function hasTauriInvokeRuntime(): boolean {
  if (typeof window === "undefined") return false;
  const maybeWindow = window as unknown as {
    __TAURI_INTERNALS__?: {
      invoke?: unknown;
    };
  };
  return typeof maybeWindow.__TAURI_INTERNALS__?.invoke === "function";
}

function hasTauriRuntime(): boolean {
  if (hasTauriInvokeRuntime()) return true;
  try {
    return hasTauriWindowRuntime();
  } catch {
    return false;
  }
}

function resolveRuntimeKind(): LocusRuntimeKind {
  if (hasTauriRuntime()) return "tauri";
  return "browser";
}

export function getLocusRuntime(): LocusRuntime {
  if (installedRuntime) {
    const runtime = installedRuntime;
    return { ...runtime, invoke<T>(command: string, args?: Record<string, unknown>, signal?: AbortSignal) {
      return runtime.invoke<T>(command, args, signal ?? invocationSignal);
    } };
  }
  const kind = resolveRuntimeKind();

  return {
    kind,
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      return invokeRuntime<T>(kind, command, args);
    },
    subscribe<T>(
      eventName: string,
      handler: (payload: T) => unknown,
      options?: WorkspaceEventSubscriptionOptions,
    ): Promise<RuntimeUnsubscribe> {
      if (kind === "tauri") {
        if (eventName === WORKSPACE_EVENT_NAME) {
          return listenWorkspaceEvent<RoutedWorkspaceEvent>(
            options?.owner ?? "LocusRuntime.workspace",
            (event) => handler(event.payload as T),
            options,
          );
        }
        if (eventName === "stream-event") {
          return listenWindowEvent<T>(eventName, event => handler(event.payload));
        }
        return tauriListen<T>(eventName, (event) => handler(event.payload))
          .then((release: UnlistenFn) => release);
      }

      return Promise.resolve(() => {});
    },
  };
}
