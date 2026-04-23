import { watch } from "vue";
import { listen } from "@tauri-apps/api/event";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { canvasSetSpec } from "../services/canvas";
import type { UnlistenFn } from "@tauri-apps/api/event";
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
  knowledgeGetLexicalRebuildStatus,
  knowledgeListPage,
} from "../services/knowledge";
import { listAgents, listSubagentDefs } from "../services/agent";
import type {
  StreamEvent,
  AssetDbScanEvent,
  PluginStatus,
  AppErrorPayload,
} from "../types";
import { filterVisibleProviders } from "../config/providerVisibility";
import { t } from "../i18n";
import {
  getKnowledgeLexicalProgressRunKey,
  isKnowledgeLexicalProgressWindowLocation,
  openKnowledgeLexicalProgressWindow,
  shouldAutoOpenKnowledgeLexicalProgressWindow,
} from "../services/knowledgeLexicalProgressWindow";

export function useAppBootstrap() {
  const uiStore = useUiStore();
  const authStore = useAuthStore();
  const agentStore = useAgentStore();
  const modelStore = useModelStore();
  const projectStore = useProjectStore();
  const chatStore = useChatStore();
  const { skillItems, loadSkills } = useSkills();

  const notificationStore = useNotificationStore();

  let unlisten: UnlistenFn | null = null;
  let unlistenUnity: UnlistenFn | null = null;
  let unlistenScan: UnlistenFn | null = null;
  let unlistenPlugin: UnlistenFn | null = null;
  let unlistenAppError: UnlistenFn | null = null;
  let lexicalProgressPollTimer: ReturnType<typeof setTimeout> | null = null;
  let lexicalProgressPollInFlight = false;
  let lastAutoOpenedLexicalProgressRun = "";

  // -- Cross-domain watchers --

  // agent selection -> effort default
  watch(
    () => agentStore.selectedAgentId,
    (agentId) => {
      if (!agentId) return;
      const agent = agentStore.agents.find((a) => a.id === agentId);
      if (agent?.defaultEffort) {
        modelStore.effort = agent.defaultEffort;
      } else {
        modelStore.effort = "none";
      }
    },
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

  // Canvas auto-open (UI shell behavior, not in chat store)
  chatStore.setCanvasAutoOpenCallback(
    async (toolCallId: string, spec: unknown) => {
      try {
        const specId = toolCallId;
        await canvasSetSpec(specId, JSON.stringify(spec));
        const win = new WebviewWindow(`canvas-${specId}`, {
          url: `/canvas?specId=${specId}`,
          title: `Canvas: ${(spec as any).title || "Canvas"}`,
          width: 1200,
          height: 800,
          minWidth: 600,
          minHeight: 400,
          decorations: true,
          resizable: true,
          center: true,
        });
        win.once("tauri://error", (e) =>
          console.error("Canvas auto-open error:", e),
        );
      } catch {
        /* ignore */
      }
    },
  );

  // -- Bootstrap: Critical (first-screen minimum) --
  async function bootstrapCritical() {
    await uiStore.init();
    await Promise.all([
      chatStore.loadToolPermissionMode(),
      modelStore.loadModelDefaults(),
      modelStore.loadLastModel(),
      modelStore.loadCustomEndpoints(),
      modelStore.loadCodexModelConfig(),
    ]);
    const authFailures = await authStore.checkAuth();
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
    modelStore.resolveSelectedModel(true);

    await Promise.all([
      chatStore.refreshSessions(),
      agentStore.loadAgents(),
      projectStore.loadWorkingDir(),
      loadSkills(),
    ]);
    await modelStore.loadLastEffort();
    skillsLoaded = true;
  }

  // -- Bootstrap: Deferred (fire-and-forget after first screen) --
  async function bootstrapDeferred() {
    await Promise.all([projectStore.loadRecentDirs()]);
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
    const schedule = (fn: () => void) => {
      if ("requestIdleCallback" in window) {
        (window as any).requestIdleCallback(fn, { timeout: 2000 });
      } else {
        setTimeout(fn, 50);
      }
    };

    schedule(async () => {
      const warmupGeneration = setScope(projectStore.workingDir);

      // Stage 1: chunk prefetch — 2 concurrent (bottleneck is parse/eval, not download)
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

      // Stage 2: data warmup — 2 concurrent
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
    });
  }

  function clearLexicalProgressPoll() {
    if (!lexicalProgressPollTimer) return;
    clearTimeout(lexicalProgressPollTimer);
    lexicalProgressPollTimer = null;
  }

  function scheduleLexicalProgressPoll(delay = 700) {
    if (isKnowledgeLexicalProgressWindowLocation()) return;
    clearLexicalProgressPoll();
    lexicalProgressPollTimer = setTimeout(() => {
      lexicalProgressPollTimer = null;
      void pollLexicalProgressWindowState();
    }, delay);
  }

  async function maybeOpenLexicalProgressWindow() {
    if (!projectStore.workingDir.trim()) return;
    if (isKnowledgeLexicalProgressWindowLocation()) return;

    const status = await knowledgeGetLexicalRebuildStatus();
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

  async function pollLexicalProgressWindowState() {
    if (lexicalProgressPollInFlight) {
      scheduleLexicalProgressPoll(500);
      return;
    }

    lexicalProgressPollInFlight = true;
    try {
      await maybeOpenLexicalProgressWindow();
    } catch {
      // Ignore transient status read failures and retry on the next tick.
    } finally {
      lexicalProgressPollInFlight = false;
      scheduleLexicalProgressPoll();
    }
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
    unlisten = await listen<StreamEvent>("stream-event", (e) => {
      const handled = chatStore.handleStreamEvent(e.payload);
      if (!handled) return;

      const sessionTitle =
        chatStore.sessions.find((session) => session.id === e.payload.sessionId)?.title ?? null;
      void maybeNotifyStreamEvent(e.payload, { sessionTitle });
    });
    unlistenUnity = await listen<boolean>("unity-connection-status", (e) => {
      projectStore.unityConnected = e.payload;
      if (e.payload) {
        console.log("[Locus] Unity Editor connected!");
      } else {
        console.log("[Locus] Unity Editor disconnected.");
      }
    });
    unlistenScan = await listen<AssetDbScanEvent>("ref-graph-scan", (e) => {
      projectStore.handleScanEvent(e.payload);
    });
    unlistenPlugin = await listen<PluginStatus>("unity-plugin-status", (e) => {
      projectStore.handlePluginStatus(e.payload);
    });
    unlistenAppError = await listen<AppErrorPayload>("app-error", (e) => {
      const payload = normalizeAppError(e.payload);
      notificationStore.addNotice(payload.severity, payload.message, {
        code: payload.code,
        operation: payload.operation,
      });
    });

    // Initial Unity/AssetDb state
    await projectStore.checkUnityConnection();
    await projectStore.checkUnityPlugin();
    await projectStore.loadAssetDbStatus();
    scheduleLexicalProgressPoll(180);
  }

  function cleanup() {
    unlisten?.();
    unlistenUnity?.();
    unlistenScan?.();
    unlistenPlugin?.();
    unlistenAppError?.();
    clearLexicalProgressPoll();
    lexicalProgressPollInFlight = false;
    lastAutoOpenedLexicalProgressRun = "";
    resetSystemNotificationState();
    uiStore.cleanup();
    chatStore.cleanupAnim();
  }

  // -- Workspace management --
  async function applyWorkingDir(path: string) {
    clearWarmup(); // invalidate warmup cache for previous workingDir
    lastAutoOpenedLexicalProgressRun = "";
    resetSystemNotificationState();
    _wpCollab = null;
    _wpKnowledge = null;
    _wpAsset = null;
    _wpAgent = null;
    _wpSettings = null;
    await projectStore.setWorkingDir(path);
    chatStore.newChat();
    await Promise.all([
      chatStore.refreshSessions(),
      projectStore.loadRecentDirs(),
      projectStore.checkUnityConnection(),
      projectStore.checkUnityPlugin(),
      projectStore.loadAssetDbStatus(),
      loadSkills(),
    ]);
  }

  // -- Settings callbacks --
  async function closeSettings() {
    uiStore.setTab("chat");
    await authStore.checkAuth();
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
    modelStore.resolveSelectedModel(true);
    await Promise.all([
      chatStore.refreshSessions(),
      projectStore.loadWorkingDir(),
      projectStore.loadRecentDirs(),
      projectStore.checkUnityConnection(),
      projectStore.checkUnityPlugin(),
      projectStore.loadAssetDbStatus(),
      loadSkills(),
    ]);
    await modelStore.loadLastEffort();
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
