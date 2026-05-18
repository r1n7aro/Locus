import { ipcInvoke } from "./ipc";
import type {
  SessionSummary,
  SessionDetail,
  SessionEventRecord,
  SessionRunSummary,
  TokenUsage,
  TodoSnapshot,
  ImageAttachment,
  AssetRefAttachment,
  UserIntentMeta,
  KnowledgeAccessMode,
  PendingSessionInput,
} from "../types";

export interface ChatParams {
  sessionId?: string | null;
  text: string;
  sessionTitle?: string | null;
  agentId?: string | null;
  model?: string | null;
  effort?: string | null;
  images?: ImageAttachment[] | null;
  assetRefs?: AssetRefAttachment[] | null;
  sessionType?: string | null;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  subagentModels?: Record<string, string> | null;
  knowledgeMode?: KnowledgeAccessMode | null;
}

export interface CreateSessionParams {
  title: string;
  parentSessionId?: string | null;
  sessionType?: string | null;
  agentId?: string | null;
}

export interface ChatLaunchResult {
  sessionId: string;
  runId: string;
}

export interface QueueChatInputParams {
  sessionId: string;
  runId: string;
  mergeGroupId: string;
  text: string;
  displayText?: string | null;
  images?: ImageAttachment[] | null;
  assetRefs?: AssetRefAttachment[] | null;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  clientMessageId?: string | null;
  delivery?: "after_run" | "immediate" | string | null;
}

export function chat(params: ChatParams): Promise<ChatLaunchResult> {
  return ipcInvoke<ChatLaunchResult>("chat", { ...params });
}

export function queueChatInput(params: QueueChatInputParams): Promise<PendingSessionInput> {
  return ipcInvoke<PendingSessionInput>("queue_chat_input", { ...params });
}

export function insertPendingChatInput(
  sessionId: string,
  runId: string,
  pendingInputId?: string | null,
): Promise<PendingSessionInput> {
  return ipcInvoke<PendingSessionInput>("insert_pending_chat_input", {
    sessionId,
    runId,
    pendingInputId: pendingInputId ?? null,
  });
}

export function cancelChat(sessionId: string): Promise<void> {
  return ipcInvoke("cancel_chat", { sessionId });
}

export function staleKnowledgeProposals(sessionId: string): Promise<void> {
  return ipcInvoke("stale_knowledge_proposals", { sessionId });
}

export function ignoreKnowledgeProposal(sessionId: string, proposalId: string): Promise<void> {
  return ipcInvoke("ignore_knowledge_proposal", { sessionId, proposalId });
}

export function applyKnowledgeProposal(
  sessionId: string,
  proposalId: string,
): Promise<void> {
  return ipcInvoke("apply_knowledge_proposal", {
    sessionId,
    proposalId,
  });
}

export function createSession(params: CreateSessionParams): Promise<string> {
  return ipcInvoke<string>("create_session", { ...params });
}

export function forkSession(sessionId: string, title?: string | null): Promise<string> {
  return ipcInvoke<string>("fork_session", { sessionId, title: title ?? null });
}

export function listSessions(): Promise<SessionSummary[]> {
  return ipcInvoke<SessionSummary[]>("list_sessions");
}

export function listArchivedSessions(): Promise<SessionSummary[]> {
  return ipcInvoke<SessionSummary[]>("list_archived_sessions");
}

export function getActiveSessionSelection(): Promise<string | null> {
  return ipcInvoke<string | null>("get_active_session_selection");
}

export function saveActiveSessionSelection(sessionId: string | null): Promise<void> {
  return ipcInvoke("save_active_session_selection", { sessionId });
}

export function loadSession(sessionId: string): Promise<SessionDetail> {
  return ipcInvoke<SessionDetail>("load_session", { sessionId });
}

export function renameSession(sessionId: string, title: string): Promise<void> {
  return ipcInvoke("rename_session", { sessionId, title });
}

export function archiveSession(sessionId: string): Promise<void> {
  return ipcInvoke("archive_session", { sessionId });
}

export function unarchiveSession(sessionId: string): Promise<void> {
  return ipcInvoke("unarchive_session", { sessionId });
}

export function deleteSession(sessionId: string): Promise<void> {
  return ipcInvoke("delete_session", { sessionId });
}

export function undoLatestConversationTurn(sessionId: string): Promise<SessionDetail> {
  return ipcInvoke<SessionDetail>("undo_latest_conversation_turn", { sessionId });
}

export function getSessionUsage(sessionId: string): Promise<TokenUsage> {
  return ipcInvoke<TokenUsage>("get_session_usage", { sessionId });
}

export function getSessionActiveRun(sessionId: string): Promise<SessionRunSummary | null> {
  return ipcInvoke<SessionRunSummary | null>("get_session_active_run", { sessionId });
}

export function listSessionEvents(
  sessionId: string,
  afterSeq?: number | null,
  limit?: number | null,
): Promise<SessionEventRecord[]> {
  return ipcInvoke<SessionEventRecord[]>("list_session_events", {
    sessionId,
    afterSeq: afterSeq ?? null,
    limit: limit ?? null,
  });
}

export function getTodos(sessionId: string): Promise<TodoSnapshot> {
  return ipcInvoke<TodoSnapshot>("get_todos", { sessionId });
}

export function answerQuestion(questionId: string, answer: string): Promise<void> {
  return ipcInvoke("answer_question", { questionId, answer });
}

export function saveRawContext(
  sessionId: string,
  filePath: string,
  includeSystemPrompt = true,
): Promise<string> {
  return ipcInvoke<string>("save_raw_context", { sessionId, filePath, includeSystemPrompt });
}

export function savePlanArtifact(
  sessionId: string,
  agentId: string,
  requestText: string,
  responseText: string,
): Promise<string> {
  return ipcInvoke<string>("save_plan_artifact", { sessionId, agentId, requestText, responseText });
}
