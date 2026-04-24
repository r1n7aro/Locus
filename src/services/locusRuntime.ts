import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { hasTauriWindowRuntime } from "./tauriRuntime";

export type LocusRuntimeKind = "tauri" | "unity" | "browser";
export type RuntimeUnsubscribe = () => void;

export interface LocusRuntime {
  kind: LocusRuntimeKind;
  unityBridgeUrl: string | null;
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  request<T>(path: string, init?: RequestInit): Promise<T>;
  subscribe<T>(eventName: string, handler: (payload: T) => void): Promise<RuntimeUnsubscribe>;
}

interface UnityInvokePayload {
  command: string;
  args?: Record<string, unknown>;
}

function searchParams(): URLSearchParams {
  if (typeof window === "undefined") return new URLSearchParams();
  return new URLSearchParams(window.location.search);
}

export function isUnityHostLocation(): boolean {
  return searchParams().get("host") === "unity";
}

function getUnityBridgeUrl(): string | null {
  const value = searchParams().get("locusUnityBridge");
  if (!value) return null;
  return value.replace(/\/+$/, "");
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
  if (isUnityHostLocation()) return "unity";
  if (hasTauriRuntime()) return "tauri";
  return "browser";
}

function buildUnityBridgeUrl(path: string): string {
  const baseUrl = getUnityBridgeUrl();
  if (!baseUrl) {
    throw new Error("Unity bridge URL is missing.");
  }
  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  return `${baseUrl}${normalizedPath}`;
}

async function requestJson<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    ...init,
    headers: {
      ...(init?.body ? { "Content-Type": "application/json" } : {}),
      ...init?.headers,
    },
  });
  const text = await response.text();
  const data = text ? JSON.parse(text) : null;

  if (!response.ok) {
    const message = data && typeof data.error === "string"
      ? data.error
      : `Request failed with ${response.status}`;
    throw new Error(message);
  }

  return data as T;
}

function subscribeBridgeEvent<T>(
  baseUrl: string | null,
  eventName: string,
  handler: (payload: T) => void,
): Promise<RuntimeUnsubscribe> {
  if (!baseUrl) return Promise.resolve(() => {});
  if (typeof EventSource === "undefined") return Promise.resolve(() => {});

  const url = new URL(`${baseUrl}/events`);
  url.searchParams.set("topic", eventName);
  const source = new EventSource(url.toString());
  source.onmessage = (event) => {
    try {
      handler(JSON.parse(event.data) as T);
    } catch (error) {
      console.warn(`[Locus] ignored malformed runtime event '${eventName}'`, error);
    }
  };
  return Promise.resolve(() => source.close());
}

export function getLocusRuntime(): LocusRuntime {
  const kind = resolveRuntimeKind();
  const unityBridgeUrl = getUnityBridgeUrl();

  return {
    kind,
    unityBridgeUrl,
    invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
      if (kind === "tauri") {
        return tauriInvoke<T>(command, args);
      }

      if (kind === "unity") {
        const payload: UnityInvokePayload = { command, args };
        return requestJson<T>(buildUnityBridgeUrl("/invoke"), {
          method: "POST",
          body: JSON.stringify(payload),
        });
      }

      return Promise.reject(new Error("Locus runtime is unavailable in this browser context."));
    },
    request<T>(path: string, init?: RequestInit): Promise<T> {
      if (kind === "unity") {
        return requestJson<T>(buildUnityBridgeUrl(path), init);
      }

      return Promise.reject(new Error(`Runtime request is not supported for ${kind}.`));
    },
    subscribe<T>(eventName: string, handler: (payload: T) => void): Promise<RuntimeUnsubscribe> {
      if (kind === "tauri") {
        return tauriListen<T>(eventName, (event) => handler(event.payload))
          .then((release: UnlistenFn) => release);
      }

      return subscribeBridgeEvent<T>(unityBridgeUrl, eventName, handler);
    },
  };
}
