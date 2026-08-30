import { listen } from "@tauri-apps/api/event";
import type { UnlistenFn } from "@tauri-apps/api/event";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
} from "./project";

type WorkspaceEventSubscriber = (event: RoutedWorkspaceEvent) => void;
type PluginsChangedSubscriber = () => void;

const workspaceSubscribers = new Set<WorkspaceEventSubscriber>();
const pluginsSubscribers = new Set<PluginsChangedSubscriber>();
let workspaceUnlisten: UnlistenFn | null = null;
let pluginsUnlisten: UnlistenFn | null = null;
let startPromise: Promise<void> | null = null;
let listenerGeneration = 0;

async function ensureListeners(): Promise<void> {
  if (workspaceUnlisten && pluginsUnlisten) return;
  if (startPromise) return startPromise;
  const generation = listenerGeneration;
  const pending = Promise.allSettled([
    listen<RoutedWorkspaceEvent>(WORKSPACE_EVENT_NAME, (event) => {
      for (const subscriber of [...workspaceSubscribers]) {
        try {
          subscriber(event.payload);
        } catch (error) {
          console.error("Knowledge workspace event subscriber failed", error);
        }
      }
    }),
    listen<void>("plugins-changed", () => {
      for (const subscriber of [...pluginsSubscribers]) {
        try {
          subscriber();
        } catch (error) {
          console.error("Knowledge plugins event subscriber failed", error);
        }
      }
    }),
  ]).then(([workspaceResult, pluginsResult]) => {
    if (workspaceResult.status === "rejected" || pluginsResult.status === "rejected") {
      if (workspaceResult.status === "fulfilled") workspaceResult.value();
      if (pluginsResult.status === "fulfilled") pluginsResult.value();
      throw workspaceResult.status === "rejected"
        ? workspaceResult.reason
        : (pluginsResult as PromiseRejectedResult).reason;
    }
    const releaseWorkspace = workspaceResult.value;
    const releasePlugins = pluginsResult.value;
    if (
      generation !== listenerGeneration
      || (workspaceSubscribers.size === 0 && pluginsSubscribers.size === 0)
    ) {
      releaseWorkspace();
      releasePlugins();
      return;
    }
    workspaceUnlisten = releaseWorkspace;
    pluginsUnlisten = releasePlugins;
  });
  startPromise = pending;
  void pending.then(
    () => {
      if (startPromise === pending) startPromise = null;
    },
    () => {
      if (startPromise === pending) startPromise = null;
    },
  );
  return pending;
}

function stopListenersWhenIdle(): void {
  if (workspaceSubscribers.size > 0 || pluginsSubscribers.size > 0) return;
  listenerGeneration += 1;
  workspaceUnlisten?.();
  pluginsUnlisten?.();
  workspaceUnlisten = null;
  pluginsUnlisten = null;
  startPromise = null;
}

export async function subscribeKnowledgeWorkspaceEvents(
  workspaceSubscriber: WorkspaceEventSubscriber,
  pluginsSubscriber: PluginsChangedSubscriber,
): Promise<() => void> {
  workspaceSubscribers.add(workspaceSubscriber);
  pluginsSubscribers.add(pluginsSubscriber);
  try {
    await ensureListeners();
  } catch (error) {
    workspaceSubscribers.delete(workspaceSubscriber);
    pluginsSubscribers.delete(pluginsSubscriber);
    stopListenersWhenIdle();
    throw error;
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    workspaceSubscribers.delete(workspaceSubscriber);
    pluginsSubscribers.delete(pluginsSubscriber);
    stopListenersWhenIdle();
  };
}

export function resetKnowledgeWorkspaceEventHubForTests(): void {
  workspaceSubscribers.clear();
  pluginsSubscribers.clear();
  stopListenersWhenIdle();
}
