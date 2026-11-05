import { Channel, invoke } from "@tauri-apps/api/core";
import type { LocusRuntime, RuntimeUnsubscribe } from "../services/locusRuntime";
import type { RoutedWorkspaceEvent } from "../services/project";
import type { RemoteClientTransport, RemoteReference } from "../services/remoteClient";
import type { WindowEventInterests } from "../services/windowEventInterests";
import {
  listenWorkspaceEvent,
  WORKSPACE_EVENT_NAME,
  type WorkspaceEventSubscriptionOptions,
} from "../services/workspaceEventHub";

export type ConnectionState = "connecting" | "connected" | "reconnecting" | "revoked" | "closed";

export interface ConnectionStatus {
  state: ConnectionState;
  desktopName: string;
  deviceName: string;
  /** Last connection failure, for the reconnect banner. */
  error?: string | null;
}

export interface DeviceInfo {
  deviceId: string;
  deviceName: string;
  /** Approve tool calls and change files, git, knowledge and settings. */
  fullControl: boolean;
  desktopId: string;
  desktopName: string;
  buildId: string;
  /** Every command this device may call (decided by the desktop). */
  methods: string[];
}

/** Error of a remote call: a command's own error value is rethrown as is so
 * shared code handles it like a local invoke error. */
interface RemoteCallError {
  code: string;
  message: string;
  data?: unknown;
}

function unwrapRemoteError(error: unknown): unknown {
  if (error && typeof error === "object" && "data" in error && (error as RemoteCallError).data !== undefined) {
    return (error as RemoteCallError).data;
  }
  return error;
}

/** Messages the app's Rust side pushes over the subscription channel. */
type BridgeMessage =
  | { kind: "event"; event: RoutedWorkspaceEvent<unknown> }
  | { kind: "resync"; reason: string }
  | { kind: "status"; status: ConnectionStatus }
  | { kind: "closing"; reason: string };

export class RemoteUnavailableError extends Error {
  readonly code = "remote.unavailable";
  constructor(command: string) {
    super(`"${command}" is not available on this device.`);
    this.name = "RemoteUnavailableError";
  }
}

export interface AppBridge {
  runtime: LocusRuntime;
  transport: RemoteClientTransport;
  status(): Promise<ConnectionStatus>;
  onStatus(listener: (status: ConnectionStatus) => void): () => void;
  /** Events were dropped (queue overflow or reconnect): reload visible state. */
  onResync(listener: (reason: string) => void): () => void;
  /** Device info; also updates which commands are refused locally. */
  deviceInfo(): Promise<DeviceInfo>;
  /** Whether this device may call `command` (unknown before the first
   * device info: then the desktop decides). */
  canCall(command: string): boolean;
  /** Where references opened from conversations go (files, knowledge). */
  setReferenceOpener(opener: ((reference: RemoteReference) => void) | null): void;
  /** Disconnects and returns to the app's desktop picker. */
  leave(): Promise<void>;
  dispose(): void;
}

/** Connects the page to the Locus Remote app, which owns the pinned-TLS
 * connection to the desktop. The page never sees the device token. */
export async function connectAppBridge(): Promise<AppBridge> {
  const eventHandlers = new Set<(event: RoutedWorkspaceEvent<unknown>) => void>();
  const statusListeners = new Set<(status: ConnectionStatus) => void>();
  const resyncListeners = new Set<(reason: string) => void>();
  let disposed = false;

  const notify = <T>(listeners: Set<(value: T) => void>, value: T) => {
    for (const listener of [...listeners]) {
      try {
        listener(value);
      } catch (error) {
        console.error("[remote] listener failed", error);
      }
    }
  };

  const channel = new Channel<BridgeMessage>((message) => {
    if (disposed) return;
    switch (message.kind) {
      case "event":
        notify(eventHandlers, message.event);
        break;
      case "resync":
        notify(resyncListeners, message.reason);
        break;
      case "status":
        notify(statusListeners, message.status);
        break;
      case "closing":
        notify(statusListeners, {
          state: message.reason === "revoked" || message.reason === "unpaired" ? "revoked" : "closed",
          desktopName: "",
          deviceName: "",
          error: message.reason,
        });
        break;
    }
  });
  await invoke("remote_subscribe", { onMessage: channel });

  // Android suspends sockets in the background; reconnect right away on
  // return instead of waiting out the reconnect backoff.
  const onVisibilityChange = () => {
    if (document.visibilityState === "visible") void invoke("remote_wake").catch(() => {});
  };
  const hasDocument = typeof document !== "undefined";
  if (hasDocument) document.addEventListener("visibilitychange", onVisibilityChange);

  const call = <T>(method: string, params?: Record<string, unknown>): Promise<T> =>
    invoke<T>("remote_invoke", { method, params: params ?? {} }).catch((error: unknown) => {
      throw unwrapRemoteError(error);
    });

  // Commands this device may call, from the desktop's device info. Shared
  // components also try desktop-only commands (workbench, Unity); those are
  // refused here without a round trip once the list is known.
  let allowed: Set<string> | null = null;
  let referenceOpener: ((reference: RemoteReference) => void) | null = null;
  const deviceInfo = async () => {
    const info = await call<DeviceInfo>("get_device_info");
    allowed = new Set(info.methods);
    return info;
  };
  void deviceInfo().catch(() => {});

  const runtime: LocusRuntime = {
    kind: "remote",
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      if (allowed && !allowed.has(command)) return Promise.reject(new RemoteUnavailableError(command));
      return call<T>(command, args);
    },
    subscribe<T>(
      eventName: string,
      handler: (payload: T) => unknown,
      options?: WorkspaceEventSubscriptionOptions,
    ): Promise<RuntimeUnsubscribe> {
      if (eventName !== WORKSPACE_EVENT_NAME) return Promise.resolve(() => {});
      return listenWorkspaceEvent<RoutedWorkspaceEvent<unknown>>(
        options?.owner ?? "remote",
        (event) => handler(event.payload as T),
        options,
      );
    },
  };

  const transport: RemoteClientTransport = {
    async setInterests(interests: WindowEventInterests) {
      await call("set_interests", interests as unknown as Record<string, unknown>);
    },
    async setFileWatches(watches) {
      await call("set_file_watches", { watches });
    },
    subscribeWorkspaceEvents(handler) {
      eventHandlers.add(handler);
      return () => eventHandlers.delete(handler);
    },
    openReference(reference) {
      referenceOpener?.(reference);
    },
  };

  return {
    runtime,
    transport,
    status: () => invoke<ConnectionStatus>("remote_status"),
    onStatus(listener) {
      statusListeners.add(listener);
      return () => statusListeners.delete(listener);
    },
    onResync(listener) {
      resyncListeners.add(listener);
      return () => resyncListeners.delete(listener);
    },
    deviceInfo,
    canCall: (command) => allowed === null || allowed.has(command),
    setReferenceOpener(opener) {
      referenceOpener = opener;
    },
    leave: () => invoke("remote_leave"),
    dispose() {
      disposed = true;
      if (hasDocument) document.removeEventListener("visibilitychange", onVisibilityChange);
      eventHandlers.clear();
      statusListeners.clear();
      resyncListeners.clear();
    },
  };
}
