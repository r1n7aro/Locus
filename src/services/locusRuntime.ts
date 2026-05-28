import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export type LocusRuntimeKind = "tauri" | "browser";
export type RuntimeUnsubscribe = () => void;

export interface LocusRuntime {
  kind: LocusRuntimeKind;
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  subscribe<T>(eventName: string, handler: (payload: T) => void): Promise<RuntimeUnsubscribe>;
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
  const kind = resolveRuntimeKind();

  return {
    kind,
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      if (kind === "tauri") {
        return tauriInvoke<T>(command, args);
      }

      return Promise.reject(new Error("Locus runtime is unavailable in this browser context."));
    },
    subscribe<T>(eventName: string, handler: (payload: T) => void): Promise<RuntimeUnsubscribe> {
      if (kind === "tauri") {
        return tauriListen<T>(eventName, (event) => handler(event.payload))
          .then((release: UnlistenFn) => release);
      }

      return Promise.resolve(() => {});
    },
  };
}
