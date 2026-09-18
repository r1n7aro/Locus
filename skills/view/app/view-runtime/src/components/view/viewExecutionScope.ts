import { inject, readonly, type InjectionKey, type Ref } from "vue";
import type { WorkspaceRef } from "../../services/project";

export interface ViewContext {
  viewId: string;
  editorId: string;
  windowLabel: string;
  workspaceRef: WorkspaceRef;
  active: Readonly<Ref<boolean>>;
  signal: AbortSignal;
  onDispose(dispose: () => void): () => void;
}
export const VIEW_CONTEXT: InjectionKey<ViewContext> = Symbol("LocusViewContext");
export function useOptionalViewContext(): ViewContext | null { return inject(VIEW_CONTEXT, null); }
export function useViewContext(): ViewContext {
  const context = useOptionalViewContext();
  if (!context) throw new Error("View context is only available inside a Locus View.");
  return context;
}

export function createViewExecutionScope(options: {
  viewId: string;
  editorId: string;
  windowLabel?: string;
  ownerWindow?: Window;
  workspaceRef: WorkspaceRef;
  active: Ref<boolean>;
  state: Map<string, unknown>;
  log(level: "debug" | "log" | "info" | "warn" | "error", args: unknown[]): void;
}) {
  const window = options.ownerWindow ?? globalThis.window;
  const document = window.document;
  const controller = new AbortController();
  const disposers = new Set<() => void>();
  const timers = new Map<number, () => void>();
  const frames = new Map<number, () => void>();
  let stateSequence = 0;
  function track(dispose: () => void): () => void {
    let live = true;
    const release = () => {
      if (!live) return;
      live = false; disposers.delete(release); dispose();
    };
    if (controller.signal.aborted) release(); else disposers.add(release);
    return release;
  }
  async function trackAsync(registration: Promise<() => void>): Promise<() => void> {
    return track(await registration);
  }
  function vueRuntime(vue: typeof import("vue")): typeof import("vue") {
    // Vue owns component watches; also retain module-level and post-await
    // watches so hot replacement and temporary tool executions release them.
    function managed<T extends (...args: any[]) => { stop(): void }>(create: T, detached = false): T {
      return ((...args: Parameters<T>) => {
        if (controller.signal.aborted) throw new Error("View execution was disposed.");
        const resource = create(...args);
        // Component-owned watches already stop with their Vue scope. Retaining
        // them here would retain removed child components until the View closes.
        if (!detached && vue.getCurrentScope()) return resource;
        const release = track(() => resource.stop());
        if (vue.getCurrentScope()) vue.onScopeDispose(release);
        return resource;
      }) as T;
    }
    return { ...vue, watch: managed(vue.watch), watchEffect: managed(vue.watchEffect), watchPostEffect: managed(vue.watchPostEffect), watchSyncEffect: managed(vue.watchSyncEffect), effectScope: managed(vue.effectScope, true) };
  }
  const context: ViewContext = {
    viewId: options.viewId, editorId: options.editorId,
    windowLabel: options.windowLabel ?? "main",
    workspaceRef: Object.freeze({ ...options.workspaceRef }), active: readonly(options.active),
    signal: controller.signal, onDispose: track,
  };
  const logger = Object.fromEntries(["debug", "log", "info", "warn", "error"].map((level) => [level, (...args: unknown[]) => {
    if (!controller.signal.aborted) options.log(level as Parameters<typeof options.log>[0], args);
  }])) as Pick<Console, "debug" | "log" | "info" | "warn" | "error">;
  const scopedConsole = new Proxy(console, { get(target, key) {
    if (key in logger) return logger[key as keyof typeof logger];
    const value = Reflect.get(target, key); return typeof value === "function" ? value.bind(target) : value;
  } });
  const clearTimer = (id: number) => { timers.get(id)?.(); timers.delete(id); };
  const cancelFrame = (id: number) => { frames.get(id)?.(); frames.delete(id); };
  const timeout = (handler: (...args: unknown[]) => void, delay = 0, ...args: unknown[]) => {
    if (controller.signal.aborted) return 0;
    const id = window.setTimeout(() => { timers.delete(id); if (!controller.signal.aborted) handler(...args); }, delay);
    timers.set(id, () => window.clearTimeout(id)); return id;
  };
  const interval = (handler: (...args: unknown[]) => void, delay = 0, ...args: unknown[]) => {
    if (controller.signal.aborted) return 0;
    const id = window.setInterval(() => { if (options.active.value && !controller.signal.aborted) handler(...args); }, delay);
    timers.set(id, () => window.clearInterval(id)); return id;
  };
  const frame = (handler: FrameRequestCallback) => {
    if (controller.signal.aborted) return 0;
    const id = window.requestAnimationFrame((time) => { frames.delete(id); if (!controller.signal.aborted) handler(time); });
    frames.set(id, () => window.cancelAnimationFrame(id)); return id;
  };
  function globals(locus: unknown): Record<string, unknown> {
    function listenerMethods(target: EventTarget) {
      const eventListeners = new Map<EventListenerOrEventListenerObject, Map<string, () => void>>();
      return {
        addEventListener(type: string, handler: EventListenerOrEventListenerObject, settings?: boolean | AddEventListenerOptions) {
          if (controller.signal.aborted) return;
          target.addEventListener(type, handler, settings);
          const release = track(() => target.removeEventListener(type, handler, settings));
          let map = eventListeners.get(handler); if (!map) eventListeners.set(handler, map = new Map());
          map.set(`${type}:${typeof settings === "boolean" ? settings : !!settings?.capture}`, release);
        },
        removeEventListener(type: string, handler: EventListenerOrEventListenerObject, settings?: boolean | EventListenerOptions) {
          eventListeners.get(handler)?.get(`${type}:${typeof settings === "boolean" ? settings : !!settings?.capture}`)?.();
        },
      };
    }
    const documentMethods = listenerMethods(document);
    const scopedDocument = new Proxy(document, { get(target, key) {
      if (key === "defaultView") return scopedWindow;
      if (key === "addEventListener" || key === "removeEventListener") return documentMethods[key];
      const value = Reflect.get(target, key, target); return typeof value === "function" && !value.prototype ? value.bind(target) : value;
    } });
    const overrides: Record<string, unknown> = {
      locus, document: scopedDocument, console: scopedConsole, setTimeout: timeout, setInterval: interval,
      clearTimeout: clearTimer, clearInterval: clearTimer, requestAnimationFrame: frame, cancelAnimationFrame: cancelFrame,
      ...listenerMethods(window),
    };
    const scopedWindow = new Proxy(window, { get(target, key) {
      if (key === "window" || key === "self" || key === "globalThis") return scopedWindow;
      if (typeof key === "string" && key in overrides) return overrides[key];
      const value = Reflect.get(target, key, target); return typeof value === "function" && !value.prototype ? value.bind(target) : value;
    } });
    return { ...overrides, window: scopedWindow, self: scopedWindow, globalThis: scopedWindow };
  }
  return {
    context, ownerWindow: window, track, trackAsync, globals, vueRuntime,
    state<T>(initial: T, key = `state-${stateSequence++}`): T {
      if (!options.state.has(key)) options.state.set(key, initial);
      return options.state.get(key) as T;
    },
    get disposed() { return controller.signal.aborted; },
    dispose() {
      if (controller.signal.aborted) return;
      controller.abort();
      for (const cancel of timers.values()) cancel(); timers.clear();
      for (const cancel of frames.values()) cancel(); frames.clear();
      for (const dispose of [...disposers]) {
        try { dispose(); } catch (error) { console.warn("[view] cleanup failed", error); }
      }
    },
  };
}
export type ViewExecutionScope = ReturnType<typeof createViewExecutionScope>;
