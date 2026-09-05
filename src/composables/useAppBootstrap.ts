import { watch } from "vue";
import { useUiStore } from "../stores/ui";
import { useAuthStore } from "../stores/auth";
import { useAgentStore } from "../stores/agent";
import { useModelStore } from "../stores/model";
import { useProjectStore } from "../stores/project";
import { useWorkspaceContextStore } from "../stores/workspaceContext";
import { useChatStore } from "../stores/chat";
import { useNotificationStore } from "../stores/notification";
import { useDisplaySettings } from "./useDisplaySettings";
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
import { getModelDefaults, getCustomProviders } from "../services/model";
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
  AssetDbScanEvent,
  PluginStatus,
  UnityConnectionStatus,
  AppErrorPayload,
  LexicalRebuildStatus,
  KnowledgeChangedEvent,
  SessionContentChangedEvent,
  SessionTitleUpdatedEvent,
  WorkspaceExecutionLockDiagnostic,
  AsyncTaskUpdatedEvent,
} from "../types";
import { filterVisibleProviders } from "../config/providerVisibility";
import { t } from "../i18n";
import {
  SESSION_EXECUTION_STATE_CHANGED_EVENT,
  type SessionExecutionStateChanged,
} from "../services/sessionExecutionState";
import {
  takeExternalScriptOpenRequest,
  type ExternalScriptOpenRequest,
} from "../services/system";
import { showCurrentTauriWindow } from "../services/tauriRuntime";
import {
  getKnowledgeLexicalProgressRunKey,
  isKnowledgeLexicalProgressWindowLocation,
  KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT,
  openKnowledgeLexicalProgressWindow,
  shouldAutoOpenKnowledgeLexicalProgressWindow,
} from "../services/knowledgeLexicalProgressWindow";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
} from "../services/project";
import {
  publishSessionAsyncTaskUpdate,
  publishSessionExecutionState,
  publishSessionStreamEvent,
  workspaceStreamEventSource,
} from "../services/sessionStreamEventHub";

const WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT = "workspace-execution-lock-diagnostic";

function workspaceLockNoticeOperation(sessionId: string): string {
  return `workspace-lock-wait:${sessionId}`;
}

function workspaceSwitchNowMs(): number {
  return typeof performance !== "undefined" && typeof performance.now === "function"
    ? performance.now()
    : Date.now();
}

export function isPromptCacheInvalidation(
  event: StreamEvent,
): event is Extract<StreamEvent, { type: "usageUpdate" }> {
  return event.type === "usageUpdate" && event.cacheInvalidated === true;
}

function cacheInvalidationReasonLabel(reason?: string): string {
  switch (reason) {
    case "model_changed":
      return t("chat.contextStats.cacheReason.modelChanged");
    case "provider_changed":
      return t("chat.contextStats.cacheReason.providerChanged");
    case "input_growth_exceeds_context_threshold":
      return t("chat.contextStats.cacheReason.inputGrowthExceeded");
    default:
      return t("chat.contextStats.cacheReason.unknown");
  }
}

function cacheExcessInputTokens(
  event: Extract<StreamEvent, { type: "usageUpdate" }>,
): number {
  const baselineTokens = event.cacheBaselineTokens ?? 0;
  const effectiveContextTokens =
    event.inputTokens + event.cacheReadTokens + event.cacheWriteTokens;
  const contextGrowthTokens = Math.max(0, effectiveContextTokens - baselineTokens);
  return Math.max(0, event.inputTokens - contextGrowthTokens);
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

export interface AppBootstrapOptions {
  /** Keep this window pinned to its own session instead of following the
   *  main window's globally persisted active-session selection. */
  syncActiveSessionSelection?: boolean;
  /** Route Unity external-editor open requests into this window's asset tab. */
  handleExternalScriptOpen?: boolean;
}

export function useAppBootstrap(options: AppBootstrapOptions = {}) {
  const uiStore = useUiStore();
  const authStore = useAuthStore();
  const agentStore = useAgentStore();
  const modelStore = useModelStore();
  const projectStore = useProjectStore();
  const workspaceContextStore = useWorkspaceContextStore();
  const chatStore = useChatStore();
  const { skillItems, loadSkills } = useSkills();

  const notificationStore = useNotificationStore();
  const { state: displaySettings } = useDisplaySettings();

  let unlisten: RuntimeUnsubscribe | null = null;
  let unlistenUnity: RuntimeUnsubscribe | null = null;
  let unlistenUnityDetail: RuntimeUnsubscribe | null = null;
  let unlistenScan: RuntimeUnsubscribe | null = null;
  let unlistenPlugin: RuntimeUnsubscribe | null = null;
  let unlistenSessionExecutionState: RuntimeUnsubscribe | null = null;
  let unlistenAppError: RuntimeUnsubscribe | null = null;
  let unlistenLexicalRebuildStatus: RuntimeUnsubscribe | null = null;
  let unlistenKnowledgeChanged: RuntimeUnsubscribe | null = null;
  let unlistenSessionContentChanged: RuntimeUnsubscribe | null = null;
  let unlistenSessionTitleUpdated: RuntimeUnsubscribe | null = null;
  let unlistenAsyncTaskUpdated: RuntimeUnsubscribe | null = null;
  let unlistenPluginsChanged: RuntimeUnsubscribe | null = null;
  let unlistenAgentsChanged: RuntimeUnsubscribe | null = null;
  let unlistenExternalScriptOpen: RuntimeUnsubscribe | null = null;
  let lastAutoOpenedLexicalProgressRun = "";
  const workspaceLockNoticeOperations = new Set<string>();

  function sessionDiagnosticLabel(sessionId: string): string {
    const title = chatStore.sessions.find((session) => session.id === sessionId)?.title?.trim();
    if (title) return title;
    return sessionId.length > 12 ? `${sessionId.slice(0, 12)}…` : sessionId;
  }

  function handleWorkspaceLockDiagnostic(payload: WorkspaceExecutionLockDiagnostic) {
    const operation = workspaceLockNoticeOperation(payload.sessionId);
    if (!payload.active) {
      notificationStore.clearByOperation(operation);
      workspaceLockNoticeOperations.delete(operation);
      return;
    }

    const mode = payload.mode === "write" || payload.mode === "path_write"
      ? t("chat.workspaceLock.mode.write")
      : t("chat.workspaceLock.mode.read");
    const tools = payload.tools.slice(0, 3).join(", ") || t("chat.workspaceLock.noTools");
    const blockerLabels = Array.from(new Set(
      payload.blockers.map((blocker) => sessionDiagnosticLabel(blocker.sessionId)),
    ));
    const blockers = blockerLabels.slice(0, 2).join(", ")
      || t("chat.workspaceLock.unknownBlocker");
    const waitedSeconds = Math.max(1, Math.floor(payload.waitedMs / 1_000));

    notificationStore.addNotice(
      "error",
      t(
        "chat.workspaceLock.waiting",
        sessionDiagnosticLabel(payload.sessionId),
        mode,
        waitedSeconds,
        tools,
        blockers,
      ),
      {
        code: "workspace_lock_wait",
        operation,
        sticky: true,
        replaceOperation: true,
      },
    );
    workspaceLockNoticeOperations.add(operation);
  }

  // -- Cross-domain watchers --

  function syncEffortForChatContext() {
    const agentId = agentStore.selectedAgentId;
    if (!agentId) return;
    const agent = agentStore.agents.find((item) => item.id === agentId);
    const fallbackEffort = modelStore.hasUserDefaultEffort
      ? modelStore.defaultEffort
      : (agent?.defaultEffort ?? "none");
    if (!chatStore.activeSessionId) {
      modelStore.activateAgentPreference(agentId, fallbackEffort, true);
      return;
    }
    modelStore.activateAgentPreference(agentId, fallbackEffort, false);
    if (chatStore.sessionEffort) {
      modelStore.applyContextEffort(chatStore.sessionEffort);
      return;
    }
    if (modelStore.hasUserDefaultEffort) {
      modelStore.restoreDefaultEffort();
      return;
    }
    modelStore.applyContextEffort(agent?.defaultEffort ?? "none");
  }

  function syncFastModeForChatContext() {
    if (chatStore.activeSessionId && chatStore.sessionFastMode !== null) {
      modelStore.applyContextCodexFastMode(chatStore.sessionFastMode);
      return;
    }
    modelStore.restoreDefaultCodexFastMode();
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
    // A knowledge event is an explicit cache invalidation. Once the initial
    // list has loaded, a regular loadSkills() call intentionally reuses that
    // snapshot and would leave slash commands stale until the next workspace
    // switch or app restart.
    void loadSkills({ force: true });
  }

  function handleSessionContentChanged(change: SessionContentChangedEvent) {
    if (!sessionContentChangeBelongsToCurrentWorkspace(change)) return;
    void chatStore.refreshSessionAfterExternalChange(change.sessionId);
  }

  function handleExternalScriptOpen(request: ExternalScriptOpenRequest) {
    if (!request.projectPath?.trim()) return;
    if (request.assetPath?.trim()) {
      uiStore.stageExternalScriptOpen({
        projectPath: request.projectPath,
        assetPath: request.assetPath,
        line: Math.max(1, Math.floor(request.line || 1)),
        column: Math.max(1, Math.floor(request.column || 1)),
      });
    }
    uiStore.setPage("development");
    void showCurrentTauriWindow();
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
        modelStore.loadDebugMode(),
        modelStore.loadModelDefaults(),
        modelStore.loadLastModel(),
        modelStore.loadAgentModelPreferences(),
        modelStore.loadCodexFastMode(),
        modelStore.loadCustomProviders(),
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

  function preloadTabsInBackground(viewLoaders: Array<() => Promise<void>> = []) {
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

      // Stage 1: chunk prefetch — 2 concurrent (bottleneck is parse/eval, not download).
      // Prefer the lazy-view loaders (ensureLoaded) over bare imports: they also
      // fill the view's `component` ref, so the first click on a tab mounts it
      // immediately instead of flashing the "loading" placeholder for a frame.
      markStartupPhase("preload_tabs_chunks_start");
      const chunkTasks: Array<() => Promise<unknown>> = viewLoaders.length
        ? viewLoaders
        : [
            () => import("../components/SettingsView.vue"),
            () => import("../components/CollabView.vue"),
            () => import("../components/KnowledgeView.vue"),
            () => import("../components/AssetView.vue"),
            () => import("../components/AgentView.vue"),
          ];
      await runQueue(chunkTasks, 2).catch(() => {});
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
    const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
    if (!workspaceRef) return;
    await openKnowledgeLexicalProgressWindow(workspaceRef, status);
  }

  // -- Warmup functions (idempotent, reusable promise) --

  let _wpSettings: Promise<void> | null = null;
  function warmupSettings(generation: number): Promise<void> {
    if (_wpSettings) return _wpSettings;
    _wpSettings = (async () => {
      const [providers, codex, defaults, perms, customProviders] = await Promise.all([
        getProviders(),
        fetchCodexStatus(),
        getModelDefaults(),
        getToolPermissions(),
        getCustomProviders(),
      ]);
      setWarmup(
        "settings:providers",
        filterVisibleProviders(providers),
        generation,
      );
      setWarmup("settings:codexStatus", codex, generation);
      setWarmup("settings:modelDefaults", defaults, generation);
      setWarmup("settings:toolPermissions", perms, generation);
      setWarmup("settings:customProviders", customProviders, generation);
    })();
    return _wpSettings;
  }

  let _wpCollab: Promise<void> | null = null;
  function warmupCollab(generation: number): Promise<void> {
    if (_wpCollab) return _wpCollab;
    _wpCollab = (async () => {
      const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
      if (!workspaceRef) return;
      const scopeKey = `${workspaceRef.checkoutId}@${workspaceRef.expectedGeneration ?? "current"}`;
      const probe = await gitProbe(workspaceRef);
      setWarmup(`collab:probe:${scopeKey}`, probe, generation);
      if (probe.available) {
        const [snapshot, status, branches, submoduleList] = await Promise.all([
          gitHistorySnapshot(0, 30, workspaceRef),
          gitStatus(workspaceRef),
          gitBranches(workspaceRef),
          gitSubmodules(workspaceRef),
        ]);
        setWarmup(`collab:snapshot:${scopeKey}`, snapshot, generation);
        setWarmup(`collab:status:${scopeKey}`, status, generation);
        setWarmup(`collab:branches:${scopeKey}`, branches, generation);
        setWarmup(`collab:submodules:${scopeKey}`, submoduleList, generation);
      }
    })();
    return _wpCollab;
  }

  let _wpKnowledge: Promise<void> | null = null;
  function warmupKnowledge(generation: number): Promise<void> {
    if (_wpKnowledge) return _wpKnowledge;
    _wpKnowledge = (async () => {
      const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
      if (!workspaceRef) return;
      const page = await knowledgeListPage({ type: "design", limit: 64 }, workspaceRef);
      setWarmup("knowledge:documents", page.items, generation);
    })();
    return _wpKnowledge;
  }

  let _wpAsset: Promise<void> | null = null;
  function warmupAsset(generation: number): Promise<void> {
    if (_wpAsset) return _wpAsset;
    _wpAsset = (async () => {
      const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
      if (!projectStore.isUnityProject || !workspaceRef) return;
      const [overview, tuning] = await Promise.all([
        assetDbOverview(workspaceRef),
        getWatcherTuning(),
      ]);
      setWarmup(`asset:dbOverview:${workspaceRef.checkoutId}:${workspaceRef.expectedGeneration ?? "current"}`, overview, generation);
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
  function handleStreamEvent(payload: StreamEvent) {
    const handled = chatStore.handleStreamEvent(payload);
    if (!handled) return;

    if (
      displaySettings.cacheInvalidationWarningsEnabled
      && isPromptCacheInvalidation(payload)
    ) {
      notificationStore.addNotice(
        "warning",
        t(
          "notifications.cacheInvalidationWarning",
          sessionDiagnosticLabel(payload.sessionId),
          cacheInvalidationReasonLabel(payload.cacheInvalidationReason),
          payload.cacheBaselineTokens ?? 0,
          cacheExcessInputTokens(payload),
        ),
        {
          code: "prompt_cache_miss",
          operation: `prompt-cache-miss:${payload.runId}`,
          replaceOperation: true,
        },
      );
    }

    const session = chatStore.sessions.find((item) => item.id === payload.sessionId);
    const notificationContext = {
      sessionTitle: session?.title ?? null,
      ...(session?.parentSessionId ? { isSubagent: true } : {}),
    };
    void maybeNotifyStreamEvent(payload, notificationContext);
  }

  function handleLegacyStreamEvent(payload: StreamEvent) {
    publishSessionStreamEvent({
      event: payload,
      source: { kind: "legacy" },
    });
    handleStreamEvent(payload);
  }

  async function registerListeners() {
    markStartupPhase("register_listeners_enter");
    const runtime = getLocusRuntime();
    markStartupPhase("register_listeners_runtime_ready", { runtime: runtime.kind });
    if (runtime.kind === "browser") {
      markStartupPhase("register_listeners_skipped");
      return;
    }
    // Bare stream events are retained only for migrated historical runs that
    // do not have a checkout binding. Current runs arrive through the scoped
    // workspace event stream below.
    unlisten = await runtime.subscribe<StreamEvent>("stream-event", handleLegacyStreamEvent);
    unlistenSessionExecutionState = await runtime.subscribe<SessionExecutionStateChanged>(
      SESSION_EXECUTION_STATE_CHANGED_EVENT,
      (payload) => {
        chatStore.applyActiveSessionExecutionState(
          payload.sessionId,
          payload.modelId,
          payload.effort,
          payload.fastMode,
          payload.multiAgentEnabled,
        );
        publishSessionExecutionState(payload);
      },
    );
    unlistenUnity = null;
    unlistenUnityDetail = null;
    unlistenScan = null;
    unlistenPlugin = null;
    unlistenAppError = await runtime.subscribe<AppErrorPayload>("app-error", (eventPayload) => {
      const payload = normalizeAppError(eventPayload);
      notificationStore.addNotice(payload.severity, payload.message, {
        code: payload.code,
        operation: payload.operation,
      });
    });
    unlistenAsyncTaskUpdated = await runtime.subscribe<AsyncTaskUpdatedEvent>(
      "async-task-updated",
      (payload) => {
        chatStore.applyAsyncTaskUpdate(payload);
        publishSessionAsyncTaskUpdate(payload);
      },
    );
    unlistenKnowledgeChanged = await runtime.subscribe<RoutedWorkspaceEvent>(
      WORKSPACE_EVENT_NAME,
      (event) => {
        if (!workspaceContextStore.applyWorkspaceEvent(event)) return;
        if (event.eventName === "stream-event") {
          const streamEvent = event as RoutedWorkspaceEvent<StreamEvent>;
          publishSessionStreamEvent({
            event: streamEvent.payload,
            source: workspaceStreamEventSource(streamEvent),
          });
        }
        const workspaceRef = workspaceContextStore.focusedWorkspaceRef;
        if (
          !workspaceRef
          || event.checkoutId !== workspaceRef.checkoutId
          || event.workspaceGeneration !== workspaceRef.expectedGeneration
        ) return;
        if (event.eventName === "unity-connection-status") {
          const connected = event.payload as boolean;
          projectStore.handleUnityConnectionStatus(connected);
          console.log(
            connected
              ? "[Locus] Unity Editor connected!"
              : "[Locus] Unity Editor disconnected.",
          );
        } else if (event.eventName === "unity-connection-status-detail") {
          projectStore.handleUnityConnectionStatusDetail(event.payload as UnityConnectionStatus);
        } else if (event.eventName === "ref-graph-scan") {
          projectStore.handleScanEvent(event.payload as AssetDbScanEvent);
        } else if (event.eventName === "unity-plugin-status") {
          projectStore.handlePluginStatus(event.payload as PluginStatus);
        } else if (event.eventName === KNOWLEDGE_LEXICAL_REBUILD_STATUS_EVENT) {
          void maybeOpenLexicalProgressWindow(event.payload as LexicalRebuildStatus);
        } else if (event.eventName === "knowledge-changed") {
          handleKnowledgeChanged(event.payload as KnowledgeChangedEvent);
        } else if (event.eventName === "stream-event") {
          handleStreamEvent(event.payload as StreamEvent);
        } else if (event.eventName === "session-content-changed") {
          handleSessionContentChanged(event.payload as SessionContentChangedEvent);
        } else if (event.eventName === "session-title-updated") {
          const payload = event.payload as SessionTitleUpdatedEvent;
          chatStore.applySessionTitleUpdate(payload.sessionId, payload.title);
        } else if (event.eventName === WORKSPACE_EXECUTION_LOCK_DIAGNOSTIC_EVENT) {
          handleWorkspaceLockDiagnostic(event.payload as WorkspaceExecutionLockDiagnostic);
        } else if (event.eventName === "plugins-changed") {
          void agentStore.loadWorkspaceAgents(workspaceRef);
          void loadSkills({ force: true });
        } else if (event.eventName === "agents-changed") {
          void agentStore.loadWorkspaceAgents(workspaceRef);
        } else if (
          event.eventName === "locus-open-script"
          && options.handleExternalScriptOpen === true
        ) {
          handleExternalScriptOpen(event.payload as ExternalScriptOpenRequest);
        }
      },
    );
    unlistenLexicalRebuildStatus = null;
    unlistenSessionContentChanged = null;
    unlistenSessionTitleUpdated = await runtime.subscribe<SessionTitleUpdatedEvent>(
      "session-title-updated",
      ({ sessionId, title }) => chatStore.applySessionTitleUpdate(sessionId, title),
    );
    unlistenPluginsChanged = await runtime.subscribe<void>("plugins-changed", () => {
      void agentStore.loadAgents();
      void loadSkills({ force: true });
    });
    unlistenAgentsChanged = await runtime.subscribe<void>("agents-changed", () => {
      void agentStore.loadAgents();
    });
    if (options.handleExternalScriptOpen === true) {
      unlistenExternalScriptOpen = null;
      const startupRequest = await takeExternalScriptOpenRequest();
      if (startupRequest) handleExternalScriptOpen(startupRequest);
    }
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
    unlistenSessionExecutionState?.();
    unlistenAppError?.();
    unlistenLexicalRebuildStatus?.();
    unlistenKnowledgeChanged?.();
    unlistenSessionContentChanged?.();
    unlistenSessionTitleUpdated?.();
    unlistenAsyncTaskUpdated?.();
    unlistenPluginsChanged?.();
    unlistenAgentsChanged?.();
    unlistenExternalScriptOpen?.();
    for (const operation of workspaceLockNoticeOperations) {
      notificationStore.clearByOperation(operation);
    }
    workspaceLockNoticeOperations.clear();
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
        "open_and_focus_workspace",
        async () => {
          const context = await workspaceContextStore.openAndFocus(path);
          if (!context) throw new Error("The workspace focus request was superseded.");
        },
        { target: path },
      );
      chatStore.newChat({ persistSelection: false, resetRestoreAttempt: true });
      console.info(`[workspace-switch] phase=new_chat_done target=${path}`);
      await Promise.all([
        measureWorkspaceSwitchAsync("refresh_sessions", () => chatStore.refreshSessions(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync("load_recent_dirs", () => projectStore.loadRecentDirs(), {
          target: path,
        }),
        measureWorkspaceSwitchAsync(
          "load_agents",
          () => agentStore.loadWorkspaceAgents(projectStore.requireWorkspaceRef()),
          { target: path },
        ),
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
        // force: a non-forced call would adopt a still-in-flight scan from
        // the previous workspace and commit its stale skill list here.
        measureWorkspaceSwitchAsync("load_skills", () => loadSkills({ force: true }), {
          target: path,
        }),
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
  // 设置项改动已通过各自事件即时生效；离开设置页时再做一次兜底刷新（原 closeSettings 的副作用）。
  async function refreshAfterSettings() {
    // Fallback refresh only — real auth/config changes are already pushed via
    // SettingsView's auth-changed / codex-transport-changed events. A light
    // status probe is enough here; the full checkAuth() (providers + codex)
    // costs three IPC round-trips on every settings exit.
    await authStore.checkAuthLight();
    await modelStore.loadCodexAvailableModels();
    modelStore.resolveSelectedModel(!chatStore.activeSessionId);
  }

  async function onOnboardingCompleted() {
    uiStore.completeOnboarding();
    lastAutoOpenedLexicalProgressRun = "";
    await Promise.all([
      authStore.checkAuth(),
      modelStore.loadModelDefaults(),
      modelStore.loadDebugMode(),
      modelStore.loadLastModel(),
      modelStore.loadAgentModelPreferences(),
      modelStore.loadCodexFastMode(),
      modelStore.loadCustomProviders(),
      modelStore.loadCodexModelConfig(),
    ]);
    await modelStore.loadCodexAvailableModels();
    modelStore.resolveSelectedModel(true);
    await Promise.all([
      chatStore.refreshSessions(),
      agentStore.loadAgents(),
      projectStore.loadRecentDirs(),
      projectStore.checkUnityConnection(),
      projectStore.checkUnityPlugin(),
      projectStore.loadAssetDbStatus(),
      loadSkills(),
    ]);
    await modelStore.loadLastEffort();
    syncEffortForChatContext();
    syncFastModeForChatContext();
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
    refreshAfterSettings,
    onOnboardingCompleted,
  };
}
