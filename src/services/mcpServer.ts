import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";
import type { WorkspaceRef } from "./project";

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
  integrationId: string;
  name: string;
  entryName: string;
  checkoutId: string;
  workspaceGeneration: number;
  endpointUrl: string;
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

export function mcpServerIntegrations(workspaceRef: WorkspaceRef): Promise<McpIntegrationStatus[]> {
  buildScopedMcpServerEntryName(workspaceRef);
  return ipcInvoke<McpIntegrationStatus[]>("mcp_server_integrations", { workspaceRef });
}

export function mcpServerIntegrationApply(
  integrationId: string,
  workspaceRef: WorkspaceRef,
): Promise<McpIntegrationStatus> {
  buildScopedMcpServerEntryName(workspaceRef);
  return ipcInvoke<McpIntegrationStatus>("mcp_server_integration_apply", {
    integrationId,
    workspaceRef,
  });
}

export function mcpServerIntegrationRemove(
  integrationId: string,
  workspaceRef: WorkspaceRef,
): Promise<McpIntegrationStatus> {
  buildScopedMcpServerEntryName(workspaceRef);
  return ipcInvoke<McpIntegrationStatus>("mcp_server_integration_remove", {
    integrationId,
    workspaceRef,
  });
}

export function subscribeMcpServerStatus(
  handler: (status: McpServerStatus) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<McpServerStatus>(MCP_SERVER_STATUS_EVENT, handler);
}

/**
 * Builds the immutable checkout endpoint consumed by the Locus MCP server.
 * The generation is mandatory so copying an endpoint can never silently
 * retarget a newly-created runtime for the same checkout.
 */
export function buildScopedMcpServerEndpoint(
  endpointUrl: string,
  workspaceRef: WorkspaceRef,
): string {
  const checkoutId = workspaceRef.checkoutId.trim();
  const generation = workspaceRef.expectedGeneration;
  if (!checkoutId) throw new Error("A checkout ID is required for an MCP endpoint.");
  if (typeof generation !== "number" || !Number.isSafeInteger(generation) || generation < 0) {
    throw new Error("A checkout generation is required for an MCP endpoint.");
  }
  const endpoint = new URL(endpointUrl);
  endpoint.searchParams.set("checkoutId", checkoutId);
  endpoint.searchParams.set("workspaceGeneration", String(generation));
  return endpoint.toString();
}

export function buildScopedMcpServerEntryName(workspaceRef: WorkspaceRef): string {
  const checkoutId = workspaceRef.checkoutId.trim();
  const generation = workspaceRef.expectedGeneration;
  if (!checkoutId) throw new Error("A checkout ID is required for an MCP entry name.");
  if (typeof generation !== "number" || !Number.isSafeInteger(generation) || generation < 0) {
    throw new Error("A checkout generation is required for an MCP entry name.");
  }
  const normalized = checkoutId
    .toLowerCase()
    .replace(/[^a-z0-9_-]/g, "-")
    .replace(/^-+|-+$/g, "") || "checkout";
  return `locus-${normalized}`;
}

export interface ScopedMcpServerArtifacts {
  entryName: string;
  endpointUrl: string;
  claudeCodeCommand: string;
  jsonSnippet: string;
}

/**
 * Produces only immutable checkout-generation setup artifacts. Keeping this
 * as one operation prevents a caller from pairing a process base URL with a
 * checkout-specific entry name.
 */
export function buildScopedMcpServerArtifacts(
  endpointUrl: string,
  token: string,
  workspaceRef: WorkspaceRef,
): ScopedMcpServerArtifacts {
  const scopedEndpoint = buildScopedMcpServerEndpoint(endpointUrl, workspaceRef);
  const entryName = buildScopedMcpServerEntryName(workspaceRef);
  return {
    entryName,
    endpointUrl: scopedEndpoint,
    claudeCodeCommand: `claude mcp add --transport http ${entryName} "${scopedEndpoint}" --header "Authorization: Bearer ${token}"`,
    jsonSnippet: JSON.stringify(
      {
        mcpServers: {
          [entryName]: {
            type: "http",
            url: scopedEndpoint,
            headers: { Authorization: `Bearer ${token}` },
          },
        },
      },
      null,
      2,
    ),
  };
}
