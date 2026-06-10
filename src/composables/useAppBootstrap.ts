import { watch } from "vue";
import { useUiStore } from "../stores/ui";
import { useAuthStore } from "../stores/auth";
import { useAgentStore } from "../stores/agent";
import { useModelStore } from "../stores/model";
import { useProjectStore } from "../stores/project";
import { useChatStore } from "../stores/chat";
import { useNotificationStore } from "../stores/notification";
import { useSkills } from "./useSkills";
import { normalizeAppError } from "../services/errors";
import {
  maybeNotifyStreamEvent,
  resetSystemNotificationState,
} from "../services/systemNotifications";
import { getLocusRuntime, type RuntimeUnsubscribe } from "../services/locusRuntime";
import { markStartupPhase, measureStartupAsync } from "../services/startupPerf";
import { setScope, setWarmup, clearWarmup } from "./warmupCache";
import {
  getProviders,
  codexStatus as fetchCodexStatus,
} from "../services/auth";
import { getModelDefaults, getCustomEndpoints } from "../services/model";
import { getToolPermissions } from "../services/permissions";
import {
  gitProbe,
  gitHistorySnapshot,
  gitStatus,
  gitBranches,
  gitSubmodules,
} from "../services/git";
import { assetDbOverview, getWatcherTuning } from "../services/asset";
import {
  knowledgeListPage,
} from "../services/knowledge";
import { listAgents, listSubagentDefs } from "../services/agent";
import type {
  StreamEvent,
  ActiveSessionSelectionChanged,
  AssetDbScanEvent,
  PluginStatus,
  UnityConnectionStatus,
  AppErrorPayload,
  LexicalRebuildStatus,
  KnowledgeChangedEvent,
  SessionContentChangedEvent,
} from "../types";
import { filterVisibleProviders } from "../config/providerVisibility";
import { t } from "../i18n";
import {
  getKnowledgeLexicalProgressRunKey,
  isKnowledgeLexicalProgressWindowLocation,
  KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT,
  openKnowledgeLexicalProgressWindow,
  shouldAutoOpenKnowledgeLexicalProgressWindow,
} from "../services/knowledgeLexicalProgressWindow";

function workspaceSwitchNowMs(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

function formatWorkspaceSwitchDetail(detail?: Record<string, unknown>): string {
  if (!detail) return "";
  const parts = Object.entries(detail)
    .filter(([, value]) => value !== undefined && value !== null && value !== "")
    .map(([key, value]) => `${key}=${String(value)}`);
  return parts.length ? ` ${parts.join(" ")}` : "";
}

async function measureWorkspaceSwitchAsync<T>(
  phase: string,
  task: () => Promise<T>,
  detail?: Record<string, unknown>,
): Promise<T> {
  const startedAt = workspaceSwitchNowMs();
  console.info(`[workspace-switch] phase=${phase}_start${formatWorkspaceSwitchDetail(detail)}`);
  try {
    return await task();
  } finally {
    console.info(
      `[workspace-switch] phase=${phase}_done elapsed_ms=${Math.round(
        workspaceSwitchNowMs() - startedAt,
      )}${formatWorkspaceSwitchDetail(detail)}`,
    );
  }
}

export function useAppBootstrap() {
  const uiStore = useUiStore();
  const authStore = useAuthStore();
  const agentStore = useAgentStore();
  const modelStore = useModelStore();
  const projectStore = useProjectStore();
  const chatStore = useChatStore();
  const { skillItems, loadSkills } = useSkills();

  const notificationStore = useNotificationStore();

  let unlisten: RuntimeUnsubscribe | null = null;
  let unlistenUnity: RuntimeUnsubscribe | null = null;
  let unlistenUnityDetail: RuntimeUnsubscribe | null = null;
  let unlistenScan: RuntimeUnsubscribe | null = null;
  let unlistenPlugin: RuntimeUnsubscribe | null = null;
  let unlistenActiveSessionSelection: RuntimeUnsubscribe | null = null;
  let unlistenAppError: RuntimeUnsubscribe | null = null;
  let unlistenLexicalRebuildStatus: RuntimeUnsubscribe | null = null;
  let unlistenKnowledgeChanged: RuntimeUnsubscribe | null = null;
  let unlistenSessionContentChanged: RuntimeUnsubscribe | null = null;
  let unlistenPluginsChanged: RuntimeUnsubscribe | null = null;
  let lastAutoOpenedLexicalProgressRun = "";

  // -- Cross-domain watchers --

  function syncEffortForChatContext() {
    const agentId = agentStore.selectedAgentId;
    if (!agentId) return;
    if (!chatStore.activeSessionId) {
      modelStore.restoreDefaultEffort();
      return;
    }
    if (modelStore.hasUserDefaultEffort) {
      modelStore.restoreDefaultEffort();
      return;
    }
    const agent = agentStore.agents.find((a) => a.id === agentId);
    modelStore.applyContextEffort(agent?.defaultEffort ?? "none");
  }

  function normalizeWorkspacePath(path: string | null | undefined): string {
    return (path ?? "").trim().replace(/\\/g, "/").replace(/\/+$/g, "").toLowerCase();
  }

  function knowledgeChangeBelongsToCurrentWorkspace(change: KnowledgeChangedEvent): boolean {
    const eventWorkspace = normalizeWorkspacePath(change.workingDir);
    const currentWorkspace = normalizeWorkspacePath(projectStore.workingDir);
    return !!eventWorkspace && eventWorkspace === currentWorkspace;
  }

  function sessionContentChangeBelongsToCurrentWorkspace(change: SessionContentChangedEvent): boolean {
    const eventWorkspace = normalizeWorkspacePath(change.workingDir);
    const currentWorkspace = normalizeWorkspacePath(projectStore.workingDir);
    return !!eventWorkspace && eventWorkspace === currentWorkspace;
  }

  function knowledgeChangeMayAffectSkills(change: KnowledgeChangedEvent): boolean {
    if (change.docType === "skill") return true;

    const source = (change.source ?? "").trim();
    return !change.docType && (
      source === "agent_knowledge_tool" ||
      source === "create_skill_scaffold" ||
      source === "delete_skill_package" ||
      source === "import_skill_package" ||
      source === "knowledge_create" ||
      source === "knowledge_edit" ||
      source === "knowledge_move" ||
      source === "knowledge_delete" ||
      source.startsWith("plugin_") ||
      source === "undo_perform"
    );
  }

  function handleKnowledgeChanged(change: KnowledgeChangedEvent) {
    if (!knowledgeChangeBelongsToCurrentWorkspace(change)) return;
    if (!knowledgeChangeMayAffectSkills(change)) return;
    void loadSkills();
  }

  function handleSessionContentChanged(change: SessionContentChangedEvent) {
    if (!sessionContentChangeBelongsToCurrentWorkspace(change)) return;
    void chatStore.refreshSessionAfterExternalChange(change.sessionId);
  }

  // active session/agent selection -> current effort, preserving the user's saved default.
  watch(
    () => [agentStore.selectedAgentId, chatStore.activeSessionId, agentStore.agents.length] as const,
    syncEffortForChatContext,
    { immediate: true },
  );

  // tab switch -> load skills on chat tab (only once after initial load)
  let skillsLoaded = false;
  watch(
    () => uiStore.activeTab,
    (tab) => {
      if (tab === "chat" && !skillsLoaded) {
        loadSkills();
        skillsLoaded = true;
      }
    },
  );

  // -- Bootstrap: Critical (first-screen minimum) --
  async function bootstrapCritical() {
    await measureStartupAsync("bootstrap_ui_init", async () => {
      await uiStore.init();
    });
    await measureStartupAsync("bootstrap_model_config", async () => {
      await Promise.all([
        chatStore.loadToolPermissionMode(),
        modelStore.loadModelDefaults(),
        modelStore.loadLastModel(),
        modelStore.loadCustomEndpoints(),
        modelStore.loadCodexModelConfig(),
      ]);
    });
    const authFailures = await measureStartupAsync("bootstrap_auth_check", async () => {
      return authStore.checkAuth();
    });
    const authFailureList = Array.isArray(authFailures) ? authFailures : [];
    for (const failure of authFailureList) {
      const isCodexFailure = failure.target === "codex";
      notificationStore.addNotice(
        "error",
        isCodexFailure
          ? t("app.startup.codexStatusFailed", failure.error.message)
          : t("app.startup.providersStatusFailed", failure.error.message),
        {
          code: failure.error.code,
          operation: isCodexFailure
            ? "startup-auth-codex"
            : "startup-auth-providers",
          sticky: true,
          replaceOperation: true,
        },
      );
    }
    await measureStartupAsync("bootstrap_codex_available_models", async () => {
      await modelStore.loadCodexAvailableModels();
    });
    markStartupPhase("bootstrap_resolve_selected_model_start");
    modelStore.resolveSelectedModel(true);
    markStartupPhase("bootstrap_resolve_selected_model_done");

    await measureStartupAsync("bootstrap_shell_data", async () => {
      await Promise.all([
        chatStore.refreshSessions(),
        agentStore.loadAgents(),
        projectStore.loadWorkingDir(),
        loadSkills(),
      ]);
    });
    await measureStartupAsync("bootstrap_last_effort", async () => {
      await modelStore.loadLastEffort();
    });
    markStartupPhase("bootstrap_sync_effort_start");
    syncEffortForChatContext();
    markStartupPhase("bootstrap_sync_effort_done");
    skillsLoaded = true;
    markStartupPhase("bootstrap_critical_ready");
  }

  // -- Bootstrap: Deferred (fire-and-forget after first screen) --
  async function bootstrapDeferred() {
    await measureStartupAsync("bootstrap_deferred_recent_dirs", async () => {
      await Promise.all([projectStore.loadRecentDirs()]);
    });
  }

  // -- Background preloading --

  /** Run tasks with limited concurrency */
  function runQueue(
    tasks: Array<() => Promise<any>>,
    concurrency: number,
  ): Promise<void> {
    return new Promise((resolve) => {
      let running = 0;
      let idx = 0;
      function next() {
        if (idx >= tasks.length && running === 0) {
          resolve();
          return;
        }
        while (running < concurrency && idx < tasks.length) {
          const task = tasks[idx++];
          running++;
          task()
            .catch(() => {})
            .finally(() => {
              running--;
              next();
            });
        }
      }
      next();
    });
  }

  function preloadTabsInBackground() {
    markStartupPhase("preload_tabs_schedule_start");
    const schedule = (fn: () => void) => {
      if ("requestIdleCallback" in window) {
        (window as any).requestIdleCallback(fn, { timeout: 2000 });
      } else {
        setTimeout(fn, 50);
      }
    };

    schedule(async () => {
      markStartupPhase("preload_tabs_task_start");
      const warmupGeneration = setScope(projectStore.workingDir);

      // Stage 1: chunk prefetch — 2 concurrent (bottleneck is parse/eval, not download)
      markStartupPhase("preload_tabs_chunks_start");
      await runQueue(
        [
          () => import("../components/SettingsView.vue"),
          () => import("../components/CollabView.vue"),
          () => import("../components/KnowledgeView.vue"),
          () => import("../components/AssetView.vue"),
          () => import("../components/AgentView.vue"),
        ],
        2,
      ).catch(() => {});
      markStartupPhase("preload_tabs_chunks_done");

      // Stage 2: data warmup — 2 concurrent
      markStartupPhase("preload_tabs_data_start");
      await runQueue(
        [
          () => warmupSettings(warmupGeneration),
          () => warmupCollab(warmupGeneration),
          () => warmupKnowledge(warmupGeneration),
          () => warmupAsset(warmupGeneration),
          () => warmupAgent(warmupGeneration),
        ],
        2,
      ).catch(() => {});
      markStartupPhase("preload_tabs_data_done");
    });
    markStartupPhase("preload_tabs_schedule_done");
  }

  async function maybeOpenLexicalProgressWindow(status: LexicalRebuildStatus) {
    if (!projectStore.workingDir.trim()) return;
    if (isKnowledgeLexicalProgressWindowLocation()) return;

    const runKey = getKnowledgeLexicalProgressRunKey(status);
    if (!runKey) {
      lastAutoOpenedLexicalProgressRun = "";
      return;
    }
    if (!shouldAutoOpenKnowledgeLexicalProgressWindow(status)) return;

    if (runKey === lastAutoOpenedLexicalProgressRun) return;

    lastAutoOpenedLexicalProgressRun = runKey;
    await openKnowledgeLexicalProgressWindow(status);
  }

  // -- Warmup functions (idempotent, reusable promise) --

  let _wpSettings: Promise<void> | null = null;
  function warmupSettings(generation: number): Promise<void> {
    if (_wpSettings) return _wpSettings;
    _wpSettings = (async () => {
      const [providers, codex, defaults, perms, endpoints] = await Promise.all([
        getProviders(),
        fetchCodexStatus(),
        getModelDefaults(),
        getToolPermissions(),
        getCustomEndpoints(),
      ]);
      setWarmup(
        "settings:providers",
        filterVisibleProviders(providers),
        generation,
      );
      setWarmup("settings:codexStatus", codex, generation);
      setWarmup("settings:modelDefaults", defaults, generation);
      setWarmup("settings:toolPermissions", perms, generation);
      setWarmup("settings:customEndpoints", endpoints, generation);
    })();
    return _wpSettings;
  }

  let _wpCollab: Promise<void> | null = null;
  function warmupCollab(generation: number): Promise<void> {
    if (_wpCollab) return _wpCollab;
    _wpCollab = (async () => {
      const probe = await gitProbe();
      setWarmup("collab:probe", probe, generation);
      if (probe.available) {
        const [snapshot, status, branches, submoduleList] = await Promise.all([
          gitHistorySnapshot(0, 30),
          gitStatus(),
          gitBranches(),
          gitSubmodules(),
        ]);
        setWarmup("collab:snapshot", snapshot, generation);
        setWarmup("collab:status", status, generation);
        setWarmup("collab:branches", branches, generation);
        setWarmup("collab:submodules", submoduleList, generation);
      }
    })();
    return _wpCollab;
  }

  let _wpKnowledge: Promise<void> | null = null;
  function warmupKnowledge(generation: number): Promise<void> {
    if (_wpKnowledge) return _wpKnowledge;
    _wpKnowledge = (async () => {
      const page = await knowledgeListPage({ type: "design", limit: 64 });
      setWarmup("knowledge:documents", page.items, generation);
    })();
    return _wpKnowledge;
  }

  let _wpAsset: Promise<void> | null = null;
  function warmupAsset(generation: number): Promise<void> {
    if (_wpAsset) return _wpAsset;
    _wpAsset = (async () => {
      if (!projectStore.workingDir.trim()) return;
      const [overview, tuning] = await Promise.all([
        assetDbOverview(),
        getWatcherTuning(),
      ]);
      setWarmup("asset:dbOverview", overview, generation);
      setWarmup("asset:watcherTuning", tuning, generation);
    })();
    return _wpAsset;
  }

  let _wpAgent: Promise<void> | null = null;
  function warmupAgent(generation: number): Promise<void> {
    if (_wpAgent) return _wpAgent;
    _wpAgent = (async () => {
      const [agents, subagents] = await Promise.all([
        listAgents(),
        listSubagentDefs(),
      ]);
      setWarmup("agent:agents", agents, generation);
      setWarmup("agent:subagents", subagents, generation);
    })();
    return _wpAgent;
  }

  // -- Event listener registration --
  async function registerListeners() {
    markStartupPhase("register_listeners_enter");
    const runtime = getLocusRuntime();
    markStartupPhase("register_listeners_runtime_ready", { runtime: runtime.kind });
    if (runtime.kind === "browser") {
      markStartupPhase("register_listeners_skipped");
      return;
    }
    unlisten = await runtime.subscribe<StreamEvent>("stream-event", (payload) => {
      const handled = chatStore.handleStreamEvent(payload);
      if (!handled) return;

      const session = chatStore.sessions.find((item) => item.id === payload.sessionId);
      const notificationContext = {
        sessionTitle: session?.title ?? null,
        ...(session?.parentSessionId ? { isSubagent: true } : {}),
      };
      void maybeNotifyStreamEvent(payload, notificationContext);
    });
    unlistenActiveSessionSelection = await runtime.subscribe<ActiveSessionSelectionChanged>(
      "active-session-selection-changed",
      (payload) => {
        void chatStore.syncActiveSessionSelection(payload.sessionId);
      },
    );
    unlistenUnity = await runtime.subscribe<boolean>("unity-connection-status", (payload) => {
      projectStore.handleUnityConnectionStatus(payload);
      if (payload) {
        console.log("[Locus] Unity Editor connected!");
      } else {
        console.log("[Locus] Unity Editor disconnected.");
      }
    });
    unlistenUnityDetail = await runtime.subscribe<UnityConnectionStatus>(
      "unity-connection-status-detail",
      (payload) => {
        projectStore.handleUnityConnectionStatusDetail(payload);
      },
    );
    unlistenScan = await runtime.subscribe<AssetDbScanEvent>("ref-graph-scan", (payload) => {
      projectStore.handleScanEvent(payload);
    });
    unlistenPlugin = await runtime.subscribe<PluginStatus>("unity-plugin-status", (payload) => {
      projectStore.handlePluginStatus(payload);
    });
    unlistenAppError = await runtime.subscribe<AppErrorPayload>("app-error", (eventPayload) => {
      const payload = normalizeAppError(eventPayload);
      notificationStore.addNotice(payload.severity, payload.message, {
        code: payload.code,
        operation: payload.operation,
      });
    });
    unlistenLexicalRebuildStatus = await runtime.subscribe<LexicalRebuildStatus>(
      KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT,
      (status) => {
        void maybeOpenLexicalProgressWindow(status);
      },
    );
    unlistenKnowledgeChanged = await runtime.subscribe<KnowledgeChangedEvent>(
      "knowledge-changed",
      handleKnowledgeChanged,
    );
    unlistenSessionContentChanged = await runtime.subscribe<SessionContentChangedEvent>(
      "session-content-changed",
      handleSessionContentChanged,
    );
    unlistenPluginsChanged = await runtime.subscribe<void>("plugins-changed", () => {
      void agentStore.loadAgents();
      void loadSkills();
    });
    markStartupPhase("register_listeners_subscriptions_ready");

    // Initial Unity/AssetDb state
    await measureStartupAsync("register_listeners_initial_state", async () => {
      await Promise.all([
        projectStore.checkUnityConnection(),
        projectStore.checkUnityPlugin(),
        projectStore.loadAssetDbStatus(),
      ]);
    });
  }

  function cleanup() {
    unlisten?.();
    unlistenUnity?.();
    unlistenUnityDetail?.();
    unlistenScan?.();
    unlistenPlugin?.();
    unlistenActiveSessionSelection?.();
    unlistenAppError?.();
    unlistenLexicalRebuildStatus?.();
    unlistenKnowledgeChanged?.();
    unlistenSessionContentChanged?.();
    unlistenPluginsChanged?.();
    lastAutoOpenedLexicalProgressRun = "";
    resetSystemNotificationState();
    uiStore.cleanup();
    chatStore.cleanupAnim();
  }

  // -- Workspace management --
  async function applyWorkingDir(path: string) {
    const switchStartedAt = workspaceSwitchNowMs();
    console.info(`[workspace-switch] phase=apply_start target=${path}`);
    clearWarmup(); // invalidate warmup cache for previous workingDir
    lastAutoOpenedLexicalProgressRun = "";
    resetSystemNotificationState();
    _wpCollab = null;
    _wpKnowledge = null;
    _wpAsset = null;
    _wpAgent = null;
    _wpSettings = null;
    try {
      await measureWorkspaceSwitchAsync(
        "set_working_dir",
        () => projectStore.setWorkingDir(path),
        { target: path },
      );
      chatStore.newChat({ persistSelection: false });
      console.info(`[workspace-switch] phase=new_chat_done target=${path}`);
      await Promise.all([
        measureWorkspaceSwitchAsync("refresh_sessions", () => chatStore.refreshSessions(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync("load_recent_dirs", () => projectStore.loadRecentDirs(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync("load_agents", () => agentStore.loadAgents(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync(
          "check_unity_connection",
          () => projectStore.checkUnityConnection(),
          { target: path },
        ),
        measureWorkspaceSwitchAsync("check_unity_plugin", () => projectStore.checkUnityPlugin(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync("load_asset_db_status", () => projectStore.loadAssetDbStatus(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync("load_skills", () => loadSkills(), { target: path }),
      ]);
    } finally {
      console.info(
        `[workspace-switch] phase=apply_done elapsed_ms=${Math.round(
          workspaceSwitchNowMs() - switchStartedAt,
        )} target=${path}`,
      );
    }
  }

  // -- Settings callbacks --
  async function closeSettings() {
    uiStore.setTab("chat");
    await authStore.checkAuth();
    await modelStore.loadCodexAvailableModels();
    modelStore.resolveSelectedModel(true);
  }

  async function onOnboardingCompleted() {
    uiStore.completeOnboarding();
    lastAutoOpenedLexicalProgressRun = "";
    await Promise.all([
      authStore.checkAuth(),
      modelStore.loadModelDefaults(),
      modelStore.loadLastModel(),
      modelStore.loadCustomEndpoints(),
      modelStore.loadCodexModelConfig(),
    ]);
    await modelStore.loadCodexAvailableModels();
    modelStore.resolveSelectedModel(true);
    await Promise.all([
      chatStore.refreshSessions(),
      agentStore.loadAgents(),
      projectStore.loadWorkingDir(),
      projectStore.loadRecentDirs(),
      projectStore.checkUnityConnection(),
      projectStore.checkUnityPlugin(),
      projectStore.loadAssetDbStatus(),
      loadSkills(),
    ]);
    await modelStore.loadLastEffort();
    syncEffortForChatContext();
  }

  return {
    skillItems,
    loadSkills,
    bootstrapCritical,
    bootstrapDeferred,
    preloadTabsInBackground,
    registerListeners,
    cleanup,
    applyWorkingDir,
    closeSettings,
    onOnboardingCompleted,
  };
}
