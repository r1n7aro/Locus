import { ipcInvoke } from "./ipc";
import type { SessionSummary, SessionDetail, TokenUsage, TodoSnapshot, ImageAttachment, UserIntentMeta } from "../types";

export interface ChatParams {
  sessionId?: string | null;
  text: string;
  sessionTitle?: string | null;
  agentId?: string | null;
  model?: string | null;
  effort?: string | null;
  images?: ImageAttachment[] | null;
  sessionType?: string | null;
  mode?: string | null;
  userIntent?: UserIntentMeta | null;
  subagentModels?: Record<string, string> | null;
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

export function chat(params: ChatParams): Promise<ChatLaunchResult> {
  return ipcInvoke<ChatLaunchResult>("chat", { ...params });
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

export function listSessions(): Promise<SessionSummary[]> {
  return ipcInvoke<SessionSummary[]>("list_sessions");
}

export function listArchivedSessions(): Promise<SessionSummary[]> {
  return ipcInvoke<SessionSummary[]>("list_archived_sessions");
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

export function getSessionUsage(sessionId: string): Promise<TokenUsage> {
  return ipcInvoke<TokenUsage>("get_session_usage", { sessionId });
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
