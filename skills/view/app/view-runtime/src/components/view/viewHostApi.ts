import { searchWorkspaceAssets } from "../../services/asset";
import { normalizeAppError } from "../../services/errors";
import { getLocusRuntime } from "../../services/locusRuntime";
import { getLastEffort, getLastModel, getModelDefaults } from "../../services/model";
import { archiveSession, chat as launchSessionChat, createSession as createLocusSession, deleteSession, forkSession, forkSessionFromMessage, getSessionActiveRun, listArchivedCheckoutSessions, listSessionEvents, listCheckoutSessions, loadSession as loadLocusSession, queueChatInput, renameSession, rollbackSessionToMessage, unarchiveSession, undoLatestConversationTurn } from "../../services/session";
import { applyUnitySerializedProperties, discoverUnitySerializedProperties, readUnitySerializedProperty, writeUnitySerializedProperty } from "../../services/unitySerializedProperty";
import { viewCallScript, viewFsAccess, viewFsAppendFile, viewFsCopyFile, viewFsLstat, viewFsMkdir, viewFsReadFile, viewFsReaddir, viewFsRename, viewFsRm, viewFsStat, viewFsUnlink, viewFsWriteFile, viewOpenFrontendLog, viewReadFrontendLog, viewStorageGet, viewStorageRemove, viewStorageSet, type ViewLlmCallRequest, type ViewLlmCallResult, type ViewSessionChatRequest, type ViewSessionChatResult, type ViewSessionCreateRequest, type ViewSessionQueueInputRequest, type ViewSessionWaitRequest, type ViewSessionWaitResult, type ViewSessionWaitStatus, type ViewRuntimeUpdateEvent } from "../../services/view";
import { WORKSPACE_EVENT_NAME, workspaceMaterializationMatches, type WorkspaceRef, type RoutedWorkspaceEvent } from "../../services/project";
import { useWorkspaceContextStore } from "../../stores/workspaceContext";
import type { AppErrorPayload, ChatMessage, SessionDetail, SessionEventRecord, SessionRunSummary, StreamEvent } from "../../types";
import type { ViewRuntimeApi } from "./viewRuntime";
import type { ViewExecutionScope } from "./viewExecutionScope";

export function createViewHostApi(options: { viewId: string; workspaceRef: WorkspaceRef; scope: ViewExecutionScope; title(): string; reload(): Promise<void> }): ViewRuntimeApi {
  const workspaceContextStore = useWorkspaceContextStore();
  const requireViewWorkspaceRef = () => options.workspaceRef;
  function pause(delay: number): Promise<void> {
    return new Promise((resolve) => {
      const signal = options.scope.context.signal;
      if (signal.aborted) { resolve(); return; }
      const done = () => { clearTimeout(timer); signal.removeEventListener("abort", done); resolve(); };
      const timer = setTimeout(done, delay);
      signal.addEventListener("abort", done, { once: true });
    });
  }
  function subscribeWorkspaceEvent<T>(eventName: string, handler: (event: T) => void, visibleOnly = false) {
    return options.scope.trackAsync(getLocusRuntime().subscribe<RoutedWorkspaceEvent<T>>(WORKSPACE_EVENT_NAME, (event) => {
      const ref = options.workspaceRef;
      if (options.scope.disposed || (visibleOnly && !options.scope.context.active.value) || event.eventName !== eventName || event.checkoutId !== ref.checkoutId || event.workspaceGeneration !== ref.expectedGeneration || !workspaceMaterializationMatches(ref.expectedMaterializationEpoch, event.materializationEpoch)) return;
      handler(event.payload);
    }, { owner: "ViewRuntimeHost." + eventName, signal: options.scope.context.signal }));
  }
  const subscribeViewWorkspaceStream = (handler: (event: StreamEvent) => void) => subscribeWorkspaceEvent("stream-event", handler);
function nonEmptyString(value: string | null | undefined): string | null {
  const trimmed = value?.trim() ?? "";
  return trimmed ? trimmed : null;
}

function defaultViewSessionTitle(requestTitle?: string | null): string {
  return nonEmptyString(requestTitle)
    ?? nonEmptyString(options.title())
    ?? "View Session";
}

async function resolveViewModel(model?: string | null): Promise<string | null> {
  const explicit = nonEmptyString(model);
  if (explicit) return explicit;

  const [defaultsResult, lastModelResult] = await Promise.allSettled([
    getModelDefaults(),
    getLastModel(),
  ]);
  const defaultModel = defaultsResult.status === "fulfilled"
    ? nonEmptyString(defaultsResult.value.mainModel)
    : null;
  const lastModel = lastModelResult.status === "fulfilled"
    ? nonEmptyString(lastModelResult.value)
    : null;
  return defaultModel ?? lastModel;
}

async function resolveViewEffort(effort?: string | null): Promise<string | null> {
  const explicit = nonEmptyString(effort);
  if (explicit) return explicit;
  try {
    return nonEmptyString(await getLastEffort());
  } catch {
    return null;
  }
}

function waitRequestFromChat(
  launch: { sessionId: string; runId: string },
  wait: ViewSessionChatRequest["wait"],
): ViewSessionWaitRequest | null {
  if (wait === false || wait == null) return null;
  if (wait === true) {
    return { sessionId: launch.sessionId, runId: launch.runId };
  }
  return {
    ...wait,
    sessionId: wait.sessionId || launch.sessionId,
    runId: wait.runId || launch.runId,
  };
}

function terminalStatusFromStreamEvent(
  event: StreamEvent,
  sessionId: string,
  runId?: string | null,
): { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null {
  if (event.sessionId !== sessionId) return null;
  if (runId && event.runId !== runId) return null;
  if (event.type === "done") return { status: "done", error: null };
  if (event.type === "cancelled") return { status: "cancelled", error: null };
  if (event.type === "error") return { status: "error", error: event.error };
  return null;
}

function terminalStatusFromRecord(
  record: SessionEventRecord,
  sessionId: string,
  runId?: string | null,
): { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null {
  if (record.sessionId !== sessionId) return null;
  if (runId && record.runId !== runId) return null;
  const payload = record.payload as { type?: unknown; error?: unknown };
  if (payload.type === "done") return { status: "done", error: null };
  if (payload.type === "cancelled") return { status: "cancelled", error: null };
  if (payload.type === "error") {
    return { status: "error", error: normalizeAppError(payload.error) };
  }
  return null;
}

function assistantTextFromMessage(message: ChatMessage | null): string {
  if (!message) return "";
  if (message.content) return message.content;
  return (message.renderParts ?? [])
    .filter((part) => part.kind === "text")
    .map((part) => part.content)
    .join("");
}

function latestAssistantMessage(detail: SessionDetail): ChatMessage | null {
  for (let index = detail.messages.length - 1; index >= 0; index -= 1) {
    const message = detail.messages[index];
    if (message.role === "assistant") return message;
  }
  return null;
}

function finalTextFromEvents(events: SessionEventRecord[], runId?: string | null): string {
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const record = events[index];
    if (runId && record.runId !== runId) continue;
    const payload = record.payload as { type?: unknown; fullText?: unknown };
    if (payload.type === "done" && typeof payload.fullText === "string") {
      return payload.fullText;
    }
  }
  return "";
}

async function createRuntimeSession(request: ViewSessionCreateRequest = {}): Promise<string> {
  return createLocusSession({
    workspaceRef: requireViewWorkspaceRef(),
    title: defaultViewSessionTitle(request.title),
    parentSessionId: request.parentSessionId ?? null,
    sessionType: request.sessionType ?? "view",
    agentId: request.agentId ?? null,
  });
}

async function showRuntimeSession(sessionId: string): Promise<void> {
  const normalized = nonEmptyString(sessionId);
  if (!normalized) throw new Error("Session id is required.");
  await workspaceContextStore.setActiveSession(normalized);
}

async function finalizeRuntimeSessionWait(
  sessionId: string,
  runId: string | null,
  status: ViewSessionWaitStatus,
  events: SessionEventRecord[],
  includeEvents: boolean,
  activeRun: SessionRunSummary | null,
  error: AppErrorPayload | null,
): Promise<ViewSessionWaitResult> {
  const detail = await loadLocusSession(sessionId);
  const message = latestAssistantMessage(detail);
  const finalText = finalTextFromEvents(events, runId) || assistantTextFromMessage(message);
  return {
    sessionId,
    runId,
    status,
    detail,
    activeRun,
    events: includeEvents ? events : [],
    message,
    finalText,
    error,
  };
}

async function waitRuntimeSession(request: ViewSessionWaitRequest): Promise<ViewSessionWaitResult> {
  const sessionId = nonEmptyString(request.sessionId);
  if (!sessionId) throw new Error("Session id is required.");

  const timeoutMs = Math.max(0, request.timeoutMs ?? 120_000);
  const pollIntervalMs = Math.max(100, request.pollIntervalMs ?? 500);
  const includeEvents = request.includeEvents !== false;
  const returnOnWaitingInput = request.returnOnWaitingInput !== false;
  const events: SessionEventRecord[] = [];
  let afterSeq = Math.max(0, request.afterSeq ?? 0);
  let targetRunId = nonEmptyString(request.runId);
  let terminal: { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null = null;
  let activeRun: SessionRunSummary | null = null;

  // Anchor the wait to a concrete run before subscribing so terminal events
  // from other runs of this session are never matched while targetRunId is
  // still empty.
  if (!targetRunId) {
    activeRun = await getSessionActiveRun(sessionId);
    targetRunId = nonEmptyString(activeRun?.runId);
  }
  const hadRunAnchorAtStart = !!targetRunId;

  const appendNewEvents = async () => {
    const batch = await listSessionEvents(sessionId, afterSeq, 2_000);
    if (batch.length === 0) return;
    events.push(...batch);
    afterSeq = Math.max(afterSeq, ...batch.map((event) => event.seq));
    if (!targetRunId) {
      targetRunId = nonEmptyString(batch[batch.length - 1]?.runId);
    }
    for (const record of batch) {
      terminal = terminalStatusFromRecord(record, sessionId, targetRunId) ?? terminal;
    }
  };

  const unsubscribe = await subscribeViewWorkspaceStream((event) => {
    terminal = terminalStatusFromStreamEvent(event, sessionId, targetRunId) ?? terminal;
  });

  const startedAt = Date.now();
  try {
    await appendNewEvents();
    const settledFromHistory = terminal as
      | { status: ViewSessionWaitStatus; error?: AppErrorPayload | null }
      | null;
    if (settledFromHistory && !hadRunAnchorAtStart) {
      // The terminal state was reconstructed from session history while no run
      // was active. If a run started in the meantime (input queued just before
      // this wait), the caller wants that run — drop the stale terminal and
      // wait on the live run instead.
      const recheck = await getSessionActiveRun(sessionId);
      const liveRunId = nonEmptyString(recheck?.runId);
      if (liveRunId && liveRunId !== targetRunId) {
        activeRun = recheck;
        targetRunId = liveRunId;
        terminal = null;
      }
    }
    while (!terminal && !options.scope.disposed && Date.now() - startedAt <= timeoutMs) {
      activeRun = await getSessionActiveRun(sessionId);
      if (!targetRunId) targetRunId = nonEmptyString(activeRun?.runId);
      if (activeRun?.status === "waiting_input" && returnOnWaitingInput) {
        return finalizeRuntimeSessionWait(
          sessionId,
          targetRunId,
          "waiting_input",
          events,
          includeEvents,
          activeRun,
          null,
        );
      }
      await pause(pollIntervalMs);
      await appendNewEvents();
    }
  } finally {
    unsubscribe();
  }

  const terminalResult = terminal as { status: ViewSessionWaitStatus; error?: AppErrorPayload | null } | null;
  if (terminalResult) {
    return finalizeRuntimeSessionWait(
      sessionId,
      targetRunId,
      terminalResult.status,
      events,
      includeEvents,
      activeRun,
      terminalResult.error ?? null,
    );
  }

  return finalizeRuntimeSessionWait(
    sessionId,
    targetRunId,
    "timeout",
    events,
    includeEvents,
    activeRun,
    null,
  );
}

async function sendRuntimeSessionMessage(
  request: ViewSessionChatRequest,
): Promise<ViewSessionChatResult> {
  const text = request.text ?? "";
  if (!text.trim()) throw new Error("Session message text is required.");
  const model = await resolveViewModel(request.model);
  if (!model) throw new Error("No model configured for View LLM calls.");
  const effort = await resolveViewEffort(request.effort);
  const launch = await launchSessionChat({
    workspaceRef: requireViewWorkspaceRef(),
    sessionId: request.sessionId ?? null,
    text,
    sessionTitle: request.sessionTitle ?? request.title ?? defaultViewSessionTitle(null),
    agentId: request.agentId ?? null,
    model,
    effort,
    images: request.images ?? null,
    assetRefs: request.assetRefs ?? null,
    sessionType: request.sessionType ?? "view",
    mode: request.mode ?? null,
    userIntent: request.userIntent ?? null,
    subagentModels: request.subagentModels ?? null,
    subagentEfforts: request.subagentEfforts ?? null,
    subagentFastModes: request.subagentFastModes ?? null,
    knowledgeMode: request.knowledgeMode ?? null,
  });

  if (request.show) {
    await showRuntimeSession(launch.sessionId);
  }

  const waitRequest = waitRequestFromChat(launch, request.wait);
  const result = waitRequest ? await waitRuntimeSession(waitRequest) : null;
  return { ...launch, result };
}

async function callRuntimeLlm(request: ViewLlmCallRequest): Promise<ViewLlmCallResult> {
  const launch = await sendRuntimeSessionMessage({
    ...request,
    text: request.prompt,
    wait: request.wait ?? {
      sessionId: request.sessionId ?? "",
      timeoutMs: request.timeoutMs ?? undefined,
    },
  });

  if (!launch.result) {
    return {
      sessionId: launch.sessionId,
      runId: launch.runId,
      status: "running",
      text: "",
      detail: null,
      events: [],
      message: null,
      error: null,
    };
  }

  return {
    sessionId: launch.sessionId,
    runId: launch.runId,
    status: launch.result.status,
    text: launch.result.finalText,
    detail: launch.result.detail,
    events: launch.result.events,
    message: launch.result.message ?? null,
    error: launch.result.error ?? null,
  };
}


return {
            workspaceRef: requireViewWorkspaceRef(),
            callScript: (scriptName, method, args) =>
              viewCallScript(requireViewWorkspaceRef(), { viewId: options.viewId, scriptName, method, args }),
            unityPropertyRead: (request) => readUnitySerializedProperty(requireViewWorkspaceRef(), request),
            unityPropertyDiscover: (request) => discoverUnitySerializedProperties(requireViewWorkspaceRef(), request),
            unityPropertyWrite: (request) => writeUnitySerializedProperty(requireViewWorkspaceRef(), request),
            unityPropertyApply: (request) => applyUnitySerializedProperties(requireViewWorkspaceRef(), request),
            searchAssets: (query, roots, limit) =>
              searchWorkspaceAssets(
                query,
                roots?.length ? roots : ["Assets", "Packages"],
                limit,
                requireViewWorkspaceRef(),
              ),
            createSession: (request) => createRuntimeSession(request),
            showSession: (sessionId) => showRuntimeSession(sessionId),
            loadSession: (sessionId) => loadLocusSession(sessionId),
            getSessionActiveRun: (sessionId) => getSessionActiveRun(sessionId),
            listSessionEvents: (sessionId, afterSeq, limit) =>
              listSessionEvents(sessionId, afterSeq, limit),
            queueSessionInput: (request: ViewSessionQueueInputRequest) => queueChatInput(request),
            sendSessionMessage: (request) => sendRuntimeSessionMessage(request),
            waitSession: (request) => waitRuntimeSession(request),
            forkSession: (sessionId, title) => forkSession(sessionId, title),
            forkSessionFromMessage: (sessionId, messageId, title) =>
              forkSessionFromMessage(sessionId, messageId, title),
            listSessions: () => listCheckoutSessions(requireViewWorkspaceRef()),
            listArchivedSessions: () => listArchivedCheckoutSessions(requireViewWorkspaceRef()),
            renameSession: (sessionId, title) => renameSession(sessionId, title),
            archiveSession: (sessionId) => archiveSession(sessionId),
            unarchiveSession: (sessionId) => unarchiveSession(sessionId),
            deleteSession: (sessionId) => deleteSession(sessionId),
            undoSessionTurn: (sessionId) => undoLatestConversationTurn(sessionId),
            rollbackSessionToMessage: (sessionId, messageId) =>
              rollbackSessionToMessage(sessionId, messageId),
            callLlm: (request) => callRuntimeLlm(request),
            onSessionEvent: (handler) => subscribeWorkspaceEvent<StreamEvent>("stream-event", handler, true),
            readFrontendLog: (limit) => viewReadFrontendLog(requireViewWorkspaceRef(), { viewId: options.viewId, limit }),
            openFrontendLog: () => viewOpenFrontendLog(requireViewWorkspaceRef(), options.viewId),
            storageGet: (key) => viewStorageGet(requireViewWorkspaceRef(), { viewId: options.viewId, key }),
            storageSet: (key, value) => viewStorageSet(requireViewWorkspaceRef(), { viewId: options.viewId, key, value }),
            storageRemove: (key) => viewStorageRemove(requireViewWorkspaceRef(), { viewId: options.viewId, key }),
            fsReadFile: (path, encoding) => viewFsReadFile(requireViewWorkspaceRef(), { path, encoding }),
            fsWriteFile: (path, data, encoding) => viewFsWriteFile(requireViewWorkspaceRef(), { path, data, encoding }),
            fsAppendFile: (path, data, encoding) => viewFsAppendFile(requireViewWorkspaceRef(), { path, data, encoding }),
            fsMkdir: (path, options) => viewFsMkdir(requireViewWorkspaceRef(), { path, recursive: options?.recursive }),
            fsReaddir: (path, options) => viewFsReaddir(requireViewWorkspaceRef(), { path, withFileTypes: options?.withFileTypes }),
            fsStat: (path) => viewFsStat(requireViewWorkspaceRef(), { path }),
            fsLstat: (path) => viewFsLstat(requireViewWorkspaceRef(), { path }),
            fsAccess: (path) => viewFsAccess(requireViewWorkspaceRef(), { path }),
            fsUnlink: (path) => viewFsUnlink(requireViewWorkspaceRef(), { path }),
            fsRm: (path, options) =>
              viewFsRm(requireViewWorkspaceRef(), { path, recursive: options?.recursive, force: options?.force }),
            fsRename: (oldPath, newPath) => viewFsRename(requireViewWorkspaceRef(), { oldPath, newPath }),
            fsCopyFile: (src, dest) => viewFsCopyFile(requireViewWorkspaceRef(), { src, dest }),
            onUpdate: (handler) =>
              subscribeWorkspaceEvent<ViewRuntimeUpdateEvent>("unity-editor-update", handler, true),
            reload: options.reload,
          };
}
