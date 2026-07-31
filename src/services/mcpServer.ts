import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";

// Locus-as-MCP-server: expose unity tools to external harnesses
// (Claude Code, Codex, OpenCode, ...) over a localhost MCP endpoint.

export interface McpServerSettings {
  enabled: boolean;
  port: number;
  token: string;
  disabledTools: string[];
  callTimeoutMs: number;
}

export interface McpServerStatus {
  running: boolean;
  boundPort: number | null;
  lastError: string | null;
  activeSessions: number;
}

export interface McpServerStateView {
  settings: McpServerSettings;
  status: McpServerStatus;
  endpointUrl: string;
}

export interface McpExposedToolInfo {
  name: string;
  description: string;
  enabled: boolean;
  available: boolean;
  unavailableReason: string | null;
}

export type McpIntegrationState = "absent" | "current" | "stale";

export interface McpIntegrationStatus {
  id: string;
  name: string;
  configPath: string;
  detected: boolean;
  state: McpIntegrationState;
}

export const MCP_SERVER_STATUS_EVENT = "mcp-server-status";

export function mcpServerGetState(): Promise<McpServerStateView> {
  return ipcInvoke<McpServerStateView>("mcp_server_get_state");
}

export function mcpServerUpdateSettings(input: {
  enabled: boolean;
  port: number;
  disabledTools: string[];
  callTimeoutMs: number;
}): Promise<McpServerStateView> {
  return ipcInvoke<McpServerStateView>("mcp_server_update_settings", input);
}

export function mcpServerRegenerateToken(): Promise<McpServerStateView> {
  return ipcInvoke<McpServerStateView>("mcp_server_regenerate_token");
}

export function mcpServerToolInventory(): Promise<McpExposedToolInfo[]> {
  return ipcInvoke<McpExposedToolInfo[]>("mcp_server_tool_inventory");
}

export function mcpServerIntegrations(): Promise<McpIntegrationStatus[]> {
  return ipcInvoke<McpIntegrationStatus[]>("mcp_server_integrations");
}

export function mcpServerIntegrationApply(id: string): Promise<McpIntegrationStatus> {
  return ipcInvoke<McpIntegrationStatus>("mcp_server_integration_apply", { id });
}

export function mcpServerIntegrationRemove(id: string): Promise<McpIntegrationStatus> {
  return ipcInvoke<McpIntegrationStatus>("mcp_server_integration_remove", { id });
}

export function subscribeMcpServerStatus(
  handler: (status: McpServerStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<McpServerStatus>(MCP_SERVER_STATUS_EVENT, handler);
}

/// Snippets for the manual-setup section.
export function claudeCodeCommand(endpointUrl: string, token: string): string {
  return `claude mcp add --transport http locus ${endpointUrl} --header "Authorization: Bearer ${token}"`;
}

export function genericJsonSnippet(endpointUrl: string, token: string): string {
  return JSON.stringify(
    {
      type: "http",
      url: endpointUrl,
      headers: { Authorization: `Bearer ${token}` },
    },
    null,
    2,
  );
}
