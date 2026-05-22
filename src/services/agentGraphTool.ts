import type { GraphData } from "../components/graph";
import { ipcInvoke } from "./ipc";

export const AGENT_GRAPH_TOOL_WINDOW_PATH = "/agent-graph";
export const AGENT_GRAPH_TOOL_WINDOW_FLAG = "agentGraph";

export interface AgentGraphToolOption {
  label: string;
  description: string;
  value?: string | null;
}

export interface AgentGraphToolPayload {
  requestId: string;
  toolCallId: string;
  title: string;
  description?: string | null;
  editable: boolean;
  graph: GraphData;
  options: AgentGraphToolOption[];
}

export interface AgentGraphToolSelectedOption {
  label: string;
  description: string;
  value?: string | null;
}

export interface AgentGraphToolSubmitRequest {
  requestId: string;
  option?: AgentGraphToolSelectedOption | null;
  graph: GraphData;
}

export interface AgentGraphToolSubmitResult {
  status: string;
}

export function isAgentGraphToolWindowLocation(
  locationLike: Pick<Location, "pathname" | "search"> = window.location,
): boolean {
  return locationLike.pathname === AGENT_GRAPH_TOOL_WINDOW_PATH
    || locationLike.search.includes(`${AGENT_GRAPH_TOOL_WINDOW_FLAG}=1`);
}

export function agentGraphToolRequestIdFromLocation(search = window.location.search): string {
  return new URLSearchParams(search).get("id")?.trim() || "";
}

export function buildAgentGraphToolWindowUrl(requestId: string): string {
  const params = new URLSearchParams({
    id: requestId,
    [AGENT_GRAPH_TOOL_WINDOW_FLAG]: "1",
  });
  return `${AGENT_GRAPH_TOOL_WINDOW_PATH}?${params.toString()}`;
}

export function agentGraphToolRequest(requestId: string): Promise<AgentGraphToolPayload> {
  return ipcInvoke<AgentGraphToolPayload>("agent_graph_tool_request", { requestId });
}

export function agentGraphToolSubmit(
  request: AgentGraphToolSubmitRequest,
): Promise<AgentGraphToolSubmitResult> {
  return ipcInvoke<AgentGraphToolSubmitResult>("agent_graph_tool_submit", { request });
}

export function agentGraphToolCancel(requestId: string): Promise<AgentGraphToolSubmitResult> {
  return ipcInvoke<AgentGraphToolSubmitResult>("agent_graph_tool_cancel", { requestId });
}
