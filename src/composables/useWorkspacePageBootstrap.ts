import { useAgentStore } from "../stores/agent";
import { useAuthStore } from "../stores/auth";
import { useModelStore } from "../stores/model";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { getCurrentTauriWindowLabel } from "../services/tauriRuntime";
import type { WorkspacePageWindowPayload } from "../services/workspacePageWindow";

/**
 * Bootstrap only the state consumed by a detached workspace page.
 *
 * The main application bootstrap also restores sessions, skills, Unity state,
 * asset database state, and global stream listeners. Detached pages own none
 * of those surfaces, so loading them here would recreate most of the main
 * window before the requested page can render.
 */
export function useWorkspacePageBootstrap() {
  const uiStore = useUiStore();
  const authStore = useAuthStore();
  const modelStore = useModelStore();
  const agentStore = useAgentStore();
  const projectStore = useProjectStore();
  const workspaceContextStore = useWorkspaceContextStore();

  async function loadModelContext() {
    await Promise.all([
      authStore.checkAuth(),
      modelStore.loadModelDefaults(),
      modelStore.loadLastModel(),
      modelStore.loadCodexFastMode(),
      modelStore.loadCustomProviders(),
      modelStore.loadCodexModelConfig(),
    ]);
    await modelStore.loadCodexAvailableModels();
    modelStore.resolveSelectedModel(true);
  }

  async function bindCheckout(payload: Extract<WorkspacePageWindowPayload, { scope: "checkout" }>) {
    const currentWindowId = getCurrentTauriWindowLabel();
    if (!currentWindowId) {
      throw new Error("The checkout window does not have a native window identity.");
    }

    await workspaceContextStore.initialize(currentWindowId, "main");
    await workspaceContextStore.focusWorkspaceRef({
      checkoutId: payload.checkoutId,
      expectedGeneration: payload.workspaceGeneration,
    });

    const focusedRef = workspaceContextStore.focusedWorkspaceRef;
    if (
      !focusedRef
      || focusedRef.checkoutId !== payload.checkoutId
      || focusedRef.expectedGeneration !== payload.workspaceGeneration
      || !workspaceContextStore.focusedRoot
    ) {
      throw new Error("The checkout window could not restore its workspace scope.");
    }
  }

  async function bootstrap(payload: WorkspacePageWindowPayload) {
    const { page } = payload;
    const tasks: Promise<unknown>[] = [uiStore.init()];

    if (payload.scope === "checkout") {
      tasks.push(bindCheckout(payload));
    }

    if (page === "knowledge" || page === "collab" || page === "agent" || page === "settings") {
      tasks.push(loadModelContext());
    }
    if (page === "agent" || page === "settings") {
      tasks.push(agentStore.loadAgents());
    }

    await Promise.all(tasks);
    if (payload.scope === "checkout" && (page === "collab" || page === "agent")) {
      const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
      if (!workspaceRef) {
        throw new Error("The checkout Agent definition scope is unavailable.");
      }
      await agentStore.loadWorkspaceAgents(workspaceRef);
    }
  }

  async function refreshAuthAndModels() {
    await authStore.loadProviderStatus();
    await modelStore.loadCodexAvailableModels();
    modelStore.resolveSelectedModel(true);
  }

  function cleanup() {
    uiStore.cleanup();
  }

  return {
    uiStore,
    authStore,
    modelStore,
    agentStore,
    projectStore,
    workspaceContextStore,
    bootstrap,
    refreshAuthAndModels,
    cleanup,
  };
}
