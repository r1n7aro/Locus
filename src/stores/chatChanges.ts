import { computed, ref, triggerRef } from "vue";
import { defineStore } from "pinia";
import { useChatStore } from "./chat";
import * as undoService from "../services/undo";
import {
  buildRounds,
  buildMergedFiles,
  mergeRoundFiles,
  type ChatChangeRound,
  type ChatMergedFileItem,
} from "../services/chatChanges";
import { invalidateDiffCacheForFiles, refetchDiffByKey } from "../services/diff";
import {
  WORKSPACE_EVENT_NAME,
  type RoutedWorkspaceEvent,
} from "../services/project";
import { getLocusRuntime } from "../services/locusRuntime";
import type { ChangedFile, FileDiffPayload } from "../types";
import { useDisplaySettings } from "../composables/useDisplaySettings";

export interface UndoFileRevertedEvent {
  workingDir: string;
  sessionId: string;
  files: ChangedFile[];
}

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

export interface ChatChangesInlineDiffState {
  payload: FileDiffPayload | null;
  loading: boolean;
  error: string | null;
  requestKey: string | null;
  assistantMessageId: string | null;
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

function emptyInlineDiffState(): ChatChangesInlineDiffState {
  return {
    payload: null,
    loading: false,
    error: null,
    requestKey: null,
    assistantMessageId: null,
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
  const inlineDiffSessions = ref(new Map<string, ChatChangesInlineDiffState>());

  const chatStore = useChatStore();

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
  const latestTurnRounds = computed(() => latestTurnRoundsForSession(currentSessionId()));

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
    latestTurnFilesForSession(currentSessionId()),
  );

  const currentFiles = computed(() => filesForSession(currentSessionId()));

  const currentFileCount = computed(() => filesForSession(currentSessionId()).length);

  // Whether any changes exist in any mode (used for button visibility — avoids hiding when one mode is empty)
  const hasAnyChanges = computed(() => hasChangesForSession(currentSessionId()));

  const currentRounds = computed(() => currentState()?.rounds ?? []);
  const currentLoading = computed(() => currentState()?.loading ?? false);
  const currentError = computed(() => currentState()?.error ?? null);

  // ── Inline diff state ──

  function getInlineDiffState(sessionId: string): ChatChangesInlineDiffState {
    let state = inlineDiffSessions.value.get(sessionId);
    if (!state) {
      state = emptyInlineDiffState();
      inlineDiffSessions.value.set(sessionId, state);
    }
    return state;
  }

  function inlineDiffStateForSession(
    sessionId: string | null | undefined,
  ): ChatChangesInlineDiffState | null {
    const normalizedSessionId = sessionId?.trim();
    if (!normalizedSessionId) return null;
    return inlineDiffSessions.value.get(normalizedSessionId) ?? null;
  }

  function updateInlineDiffState(
    sessionId: string | null | undefined,
    update: (state: ChatChangesInlineDiffState) => void,
  ): void {
    const normalizedSessionId = sessionId?.trim();
    if (!normalizedSessionId) return;
    update(getInlineDiffState(normalizedSessionId));
    triggerRef(inlineDiffSessions);
  }

  const inlineDiffPayload = computed<FileDiffPayload | null>({
    get: () => inlineDiffStateForSession(currentSessionId())?.payload ?? null,
    set: (payload) => updateInlineDiffState(currentSessionId(), (state) => {
      state.payload = payload;
    }),
  });
  const inlineDiffLoading = computed(() => (
    inlineDiffStateForSession(currentSessionId())?.loading ?? false
  ));
  const inlineDiffError = computed(() => (
    inlineDiffStateForSession(currentSessionId())?.error ?? null
  ));
  const inlineDiffRequestKey = computed(() => (
    inlineDiffStateForSession(currentSessionId())?.requestKey ?? null
  ));
  /** assistantMessageId for the file currently shown in inline diff (used for Undo) */
  const inlineDiffAssistantMsgId = computed(() => (
    inlineDiffStateForSession(currentSessionId())?.assistantMessageId ?? null
  ));

  function openInlineDiffForSession(
    sessionId: string | null | undefined,
    payload: FileDiffPayload,
    assistantMessageId: string,
  ): void {
    updateInlineDiffState(sessionId, (state) => {
      state.payload = payload;
      state.assistantMessageId = assistantMessageId;
      state.loading = false;
      state.error = null;
      state.requestKey = null;
    });
  }

  function closeInlineDiffForSession(sessionId: string | null | undefined): void {
    const normalizedSessionId = sessionId?.trim();
    if (!normalizedSessionId) return;
    if (inlineDiffSessions.value.delete(normalizedSessionId)) triggerRef(inlineDiffSessions);
  }

  function openInlineDiff(payload: FileDiffPayload, assistantMessageId: string) {
    openInlineDiffForSession(currentSessionId(), payload, assistantMessageId);
  }

  function closeInlineDiff() {
    closeInlineDiffForSession(currentSessionId());
  }

  function sessionState(sessionId: string | null | undefined): ChatChangesSessionState | null {
    const normalizedSessionId = sessionId?.trim();
    if (!normalizedSessionId) return null;
    return sessions.value.get(normalizedSessionId) ?? null;
  }

  function currentSessionId(): string | null {
    return chatStore.activeSessionId?.trim() || null;
  }

  function latestTurnRoundsForSession(
    sessionId: string | null | undefined,
  ): ChatChangeRound[] {
    const state = sessionState(sessionId);
    if (!state || state.rounds.length === 0) return [];
    const currentRunId = state.activeRunId ?? state.latestCompletedRunId;
    if (currentRunId) return state.rounds.filter((round) => round.runId === currentRunId);
    const latestKey = roundTurnKey(state.rounds[state.rounds.length - 1]!);
    return state.rounds.filter((round) => roundTurnKey(round) === latestKey);
  }

  function latestTurnFilesForSession(
    sessionId: string | null | undefined,
  ): ChatMergedFileItem[] {
    return mergeRoundFiles(latestTurnRoundsForSession(sessionId));
  }

  function filesForSession(sessionId: string | null | undefined): ChatMergedFileItem[] {
    const state = sessionState(sessionId);
    if (!state) return [];
    return state.mode === "current"
      ? latestTurnFilesForSession(sessionId)
      : state.mergedFiles;
  }

  function hasChangesForSession(sessionId: string | null | undefined): boolean {
    const state = sessionState(sessionId);
    if (!state) return false;
    return latestTurnFilesForSession(sessionId).length > 0 || state.mergedFiles.length > 0;
  }

  function setInlineDiffLoadingForSession(
    sessionId: string | null | undefined,
    loading: boolean,
    requestKey: string | null = null,
  ): void {
    updateInlineDiffState(sessionId, (state) => {
      state.loading = loading;
      state.requestKey = loading ? requestKey : null;
      if (loading) {
        state.error = null;
        state.payload = null;
      }
    });
  }

  function setInlineDiffLoading(loading: boolean, requestKey: string | null = null) {
    setInlineDiffLoadingForSession(currentSessionId(), loading, requestKey);
  }

  function setInlineDiffErrorForSession(
    sessionId: string | null | undefined,
    error: string,
  ): void {
    updateInlineDiffState(sessionId, (state) => {
      state.error = error;
      state.loading = false;
      state.requestKey = null;
    });
  }

  function setInlineDiffError(error: string) {
    setInlineDiffErrorForSession(currentSessionId(), error);
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

  function togglePanelForSession(sessionId: string | null | undefined): void {
    const sid = sessionId?.trim();
    if (!sid) return;
    const s = getState(sid);
    s.panelVisible = !s.panelVisible;
    triggerRef(sessions);
  }

  function togglePanel() {
    togglePanelForSession(currentSessionId());
  }

  function closePanelForSession(sessionId: string | null | undefined): void {
    const sid = sessionId?.trim();
    if (!sid) return;
    const s = getState(sid);
    if (s.panelVisible) {
      s.panelVisible = false;
      triggerRef(sessions);
    }
  }

  function closePanel() {
    closePanelForSession(currentSessionId());
  }

  function setModeForSession(
    sessionId: string | null | undefined,
    mode: "current" | "all",
  ): void {
    const sid = sessionId?.trim();
    if (!sid) return;
    getState(sid).mode = mode;
    triggerRef(sessions);
  }

  function setMode(mode: "current" | "all") {
    setModeForSession(currentSessionId(), mode);
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
    inlineDiffSessions.value.delete(sessionId);
    triggerRef(sessions);
    triggerRef(inlineDiffSessions);
  }

  /**
   * A single file was reverted to its pre-round state (possibly from another
   * webview, e.g. the diff review window): drop stale diff payloads for it,
   * refresh the inline diff when it shows that file, and reload the panel.
   * All steps are idempotent, so overlapping with the initiator's own refresh
   * is harmless.
   */
  function handleUndoFileReverted(event: UndoFileRevertedEvent) {
    const paths: string[] = [];
    for (const file of event.files) {
      paths.push(file.path);
      if (file.oldPath) paths.push(file.oldPath);
    }
    invalidateDiffCacheForFiles(paths);

    const inline = inlineDiffStateForSession(event.sessionId)?.payload ?? null;
    if (
      inline &&
      (paths.includes(inline.filePath) || (!!inline.oldPath && paths.includes(inline.oldPath)))
    ) {
      void refetchDiffByKey(inline.key)
        .then((updated) => {
          if (updated && inlineDiffStateForSession(event.sessionId)?.payload?.key === inline.key) {
            updateInlineDiffState(event.sessionId, (state) => {
              state.payload = updated;
            });
          }
        })
        .catch((e) => {
          console.warn("[chat-changes] failed to refresh inline diff after file revert", e);
        });
    }

    if (sessions.value.has(event.sessionId) || chatStore.activeSessionId === event.sessionId) {
      void loadChanges(event.sessionId, { allowAutoOpen: false });
    }
  }

  void getLocusRuntime()
    .subscribe<RoutedWorkspaceEvent<UndoFileRevertedEvent>>(
      WORKSPACE_EVENT_NAME,
      (event) => {
        if (event.eventName !== "undo-file-reverted") return;
        handleUndoFileReverted(event.payload);
      },
    )
    .catch((e) => {
      console.warn("[chat-changes] failed to subscribe to undo-file-reverted", e);
    });

  return {
    // State
    sessions,
    inlineDiffSessions,
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
    sessionState,
    latestTurnRoundsForSession,
    latestTurnFilesForSession,
    filesForSession,
    hasChangesForSession,
    // Inline diff
    inlineDiffPayload,
    inlineDiffLoading,
    inlineDiffError,
    inlineDiffRequestKey,
    inlineDiffAssistantMsgId,
    inlineDiffStateForSession,
    openInlineDiff,
    openInlineDiffForSession,
    closeInlineDiff,
    closeInlineDiffForSession,
    setInlineDiffLoading,
    setInlineDiffLoadingForSession,
    setInlineDiffError,
    setInlineDiffErrorForSession,
    // Actions
    loadChanges,
    refresh,
    togglePanel,
    togglePanelForSession,
    closePanel,
    closePanelForSession,
    setMode,
    setModeForSession,
    setActiveRunId,
    setLatestCompletedRunId,
    clear,
  };
});
