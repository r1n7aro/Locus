import { useAgentStore } from "../stores/agent";
import { useAuthStore } from "../stores/auth";
import { useModelStore } from "../stores/model";
import { useProjectStore } from "../stores/project";
import { useUiStore } from "../stores/ui";
import type { WorkspacePageId } from "../services/workspacePageWindow";

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

  async function bootstrap(page: WorkspacePageId) {
    const tasks: Promise<unknown>[] = [
      uiStore.init(),
      projectStore.loadWorkingDir(),
    ];

    if (page === "knowledge" || page === "collab" || page === "settings") {
      tasks.push(loadModelContext());
    }
    if (page === "collab" || page === "agent" || page === "settings") {
      tasks.push(agentStore.loadAgents());
    }

    await Promise.all(tasks);
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
    bootstrap,
    refreshAuthAndModels,
    cleanup,
  };
}
