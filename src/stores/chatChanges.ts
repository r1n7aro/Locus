import { computed, ref, triggerRef, watch } from "vue";
import { defineStore } from "pinia";
import { useChatStore } from "./chat";
import { useProjectStore } from "./project";
import * as undoService from "../services/undo";
import {
  buildRounds,
  buildMergedFiles,
  mergeRoundFiles,
  type ChatChangeRound,
  type ChatMergedFileItem,
} from "../services/chatChanges";
import type { FileDiffPayload } from "../types";
import { useDisplaySettings } from "../composables/useDisplaySettings";

export interface ChatChangesSessionState {
  panelVisible: boolean;
  mode: "current" | "all";
  rounds: ChatChangeRound[];
  mergedFiles: ChatMergedFileItem[];
  latestCompletedRunId: string | null;
  activeRunId: string | null;
  selectedFileKey: string | null;
  loading: boolean;
  error: string | null;
  lastArrivalEntryKey: string | null;
}

function emptySessionState(): ChatChangesSessionState {
  return {
    panelVisible: false,
    mode: "current",
    rounds: [],
    mergedFiles: [],
    latestCompletedRunId: null,
    activeRunId: null,
    selectedFileKey: null,
    loading: false,
    error: null,
    lastArrivalEntryKey: null,
  };
}

function roundTurnKey(round: ChatChangeRound): string {
  return round.runId || round.assistantMessageId;
}

function latestUndoEntryKey(entries: import("../types").VcsUndoEntry[]): string | null {
  let latest: import("../types").VcsUndoEntry | null = null;
  for (const entry of entries) {
    if (!latest || entry.checkpoint.createdAt > latest.checkpoint.createdAt) {
      latest = entry;
    }
  }
  return latest ? `${latest.id}:${latest.checkpoint.createdAt}` : null;
}

function logChatChangesDebug(message: string, detail?: Record<string, unknown>) {
  console.info(`[chat-changes] ${message}`, detail ?? {});
}

export const useChatChangesStore = defineStore("chatChanges", () => {
  const sessions = ref(new Map<string, ChatChangesSessionState>());

  const chatStore = useChatStore();
  const projectStore = useProjectStore();

  // ── Helpers ──

  function getState(sessionId: string): ChatChangesSessionState {
    let s = sessions.value.get(sessionId);
    if (!s) {
      s = emptySessionState();
      sessions.value.set(sessionId, s);
    }
    return s;
  }

  function currentState(): ChatChangesSessionState | null {
    const sid = chatStore.activeSessionId;
    if (!sid) return null;
    return sessions.value.get(sid) ?? null;
  }

  // ── Computed (shortcuts for current session) ──

  const currentPanelVisible = computed(() => currentState()?.panelVisible ?? false);

  const currentMode = computed(() => currentState()?.mode ?? "current");

  /** All rounds belonging to the latest conversation turn (same runId when available). */
  const latestTurnRounds = computed(() => {
    const s = currentState();
    if (!s || s.rounds.length === 0) return [];
    const currentRunId = s.activeRunId ?? s.latestCompletedRunId;
    if (currentRunId) {
      return s.rounds.filter((r) => r.runId === currentRunId);
    }
    const latestKey = roundTurnKey(s.rounds[s.rounds.length - 1]);
    return s.rounds.filter((r) => roundTurnKey(r) === latestKey);
  });

  /**
   * Net-merged file list for the latest conversation turn.
   *
   * Uses the same identity merge as the "all changes" view so statuses are
   * net relative to the run's first checkpoint (matching the diff anchor):
   * a file created and deleted within the run disappears instead of showing
   * a stale per-round letter, A→M stays A, D→A becomes M, rename chains
   * collapse.
   */
  const latestTurnFiles = computed<ChatMergedFileItem[]>(() =>
    mergeRoundFiles(latestTurnRounds.value),
  );

  const currentFiles = computed(() => {
    const s = currentState();
    if (!s) return [];
    if (s.mode === "current") {
      return latestTurnFiles.value;
    }
    return s.mergedFiles;
  });

  const currentFileCount = computed(() => {
    const s = currentState();
    if (!s) return 0;
    if (s.mode === "current") {
      return latestTurnFiles.value.length;
    }
    return s.mergedFiles.length;
  });

  // Whether any changes exist in any mode (used for button visibility — avoids hiding when one mode is empty)
  const hasAnyChanges = computed(() => {
    const s = currentState();
    if (!s) return false;
    return latestTurnFiles.value.length > 0 || s.mergedFiles.length > 0;
  });

  const currentRounds = computed(() => currentState()?.rounds ?? []);
  const currentLoading = computed(() => currentState()?.loading ?? false);
  const currentError = computed(() => currentState()?.error ?? null);

  // ── Inline diff state (app-level, not per-session) ──

  const inlineDiffPayload = ref<FileDiffPayload | null>(null);
  const inlineDiffLoading = ref(false);
  const inlineDiffError = ref<string | null>(null);
  /** assistantMessageId for the file currently shown in inline diff (used for Undo) */
  const inlineDiffAssistantMsgId = ref<string | null>(null);

  function openInlineDiff(payload: FileDiffPayload, assistantMessageId: string) {
    inlineDiffPayload.value = payload;
    inlineDiffAssistantMsgId.value = assistantMessageId;
    inlineDiffLoading.value = false;
    inlineDiffError.value = null;
  }

  function closeInlineDiff() {
    inlineDiffPayload.value = null;
    inlineDiffLoading.value = false;
    inlineDiffError.value = null;
    inlineDiffAssistantMsgId.value = null;
  }

  function setInlineDiffLoading(loading: boolean) {
    inlineDiffLoading.value = loading;
    if (loading) {
      inlineDiffError.value = null;
      inlineDiffPayload.value = null;
    }
  }

  function setInlineDiffError(error: string) {
    inlineDiffError.value = error;
    inlineDiffLoading.value = false;
  }

  // ── Actions ──

  async function loadChanges(
    sessionId: string,
    options?: { allowAutoOpen?: boolean },
  ): Promise<import("../types").VcsUndoEntry[]> {
    const s = getState(sessionId);
    const allowAutoOpen = options?.allowAutoOpen ?? true;
    const previousArrivalKey = s.lastArrivalEntryKey;
    const previousPanelVisible = s.panelVisible;
    s.loading = true;
    s.error = null;
    triggerRef(sessions);
    logChatChangesDebug("loading undo entries", {
      sessionId,
      allowAutoOpen,
      previousArrivalKey,
      previousPanelVisible,
    });
    try {
      const entries = await undoService.undoList(sessionId);
      s.rounds = buildRounds(entries);
      s.mergedFiles = buildMergedFiles(entries);
      const latestEntryKey = latestUndoEntryKey(entries);
      let changesAutoOpenEnabled: boolean | null = null;
      let autoOpened = false;
      if (allowAutoOpen && latestEntryKey && latestEntryKey !== s.lastArrivalEntryKey) {
        const { state: displaySettings } = useDisplaySettings();
        changesAutoOpenEnabled = displaySettings.changesAutoOpen;
        if (displaySettings.changesAutoOpen) {
          s.panelVisible = true;
          autoOpened = true;
        }
        s.lastArrivalEntryKey = latestEntryKey;
      }
      logChatChangesDebug("loaded undo entries", {
        sessionId,
        allowAutoOpen,
        entryCount: entries.length,
        roundCount: s.rounds.length,
        mergedFileCount: s.mergedFiles.length,
        latestEntryKey,
        previousArrivalKey,
        currentArrivalKey: s.lastArrivalEntryKey,
        activeRunId: s.activeRunId,
        latestCompletedRunId: s.latestCompletedRunId,
        changesAutoOpenEnabled,
        autoOpened,
        panelVisible: s.panelVisible,
      });
      return entries;
    } catch (e: unknown) {
      s.error = e instanceof Error ? e.message : String(e);
      s.rounds = [];
      s.mergedFiles = [];
      console.warn("[chat-changes] failed to load undo entries", {
        sessionId,
        allowAutoOpen,
        error: s.error,
      });
      return [];
    } finally {
      s.loading = false;
      triggerRef(sessions);
    }
  }

  async function refresh(sessionId: string | null, options?: { allowAutoOpen?: boolean }) {
    if (!sessionId) return;
    await loadChanges(sessionId, options);
  }

  function togglePanel() {
    const sid = chatStore.activeSessionId;
    if (!sid) return;
    const s = getState(sid);
    s.panelVisible = !s.panelVisible;
    triggerRef(sessions);
  }

  function closePanel() {
    const sid = chatStore.activeSessionId;
    if (!sid) return;
    const s = getState(sid);
    if (s.panelVisible) {
      s.panelVisible = false;
      triggerRef(sessions);
    }
  }

  function setMode(mode: "current" | "all") {
    const sid = chatStore.activeSessionId;
    if (!sid) return;
    getState(sid).mode = mode;
    triggerRef(sessions);
  }

  function setLatestCompletedRunId(sessionId: string | null, runId: string | null | undefined) {
    if (!sessionId) return;
    const s = getState(sessionId);
    s.latestCompletedRunId = runId ?? null;
    if (!runId || s.activeRunId === runId) {
      s.activeRunId = null;
    }
    triggerRef(sessions);
  }

  function setActiveRunId(sessionId: string | null, runId: string | null | undefined) {
    if (!sessionId) return;
    const s = getState(sessionId);
    s.activeRunId = runId ?? null;
    triggerRef(sessions);
  }

  function clear(sessionId: string | null) {
    if (!sessionId) return;
    sessions.value.delete(sessionId);
    closeInlineDiff();
    triggerRef(sessions);
  }

  // ── Watchers ──

  // Clear all session states when working directory changes
  watch(
    () => projectStore.workingDir,
    () => {
      sessions.value.clear();
      closeInlineDiff();
      triggerRef(sessions);
    },
  );

  return {
    // State
    sessions,
    // Computed
    currentPanelVisible,
    currentMode,
    currentFiles,
    currentFileCount,
    hasAnyChanges,
    latestTurnRounds,
    latestTurnFiles,
    currentRounds,
    currentLoading,
    currentError,
    // Inline diff
    inlineDiffPayload,
    inlineDiffLoading,
    inlineDiffError,
    inlineDiffAssistantMsgId,
    openInlineDiff,
    closeInlineDiff,
    setInlineDiffLoading,
    setInlineDiffError,
    // Actions
    loadChanges,
    refresh,
    togglePanel,
    closePanel,
    setMode,
    setActiveRunId,
    setLatestCompletedRunId,
    clear,
  };
});
