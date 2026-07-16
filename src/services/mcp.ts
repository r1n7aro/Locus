import { ipcInvoke } from "./ipc";
import { getLocusRuntime, type RuntimeUnsubscribe } from "./locusRuntime";

export type McpTransport = "stdio" | "http";
export type McpLoadMode = "lazy" | "direct";

export interface McpServerConfig {
  id: string;
  name: string;
  transport: McpTransport;
  command: string;
  args: string[];
  env: Record<string, string>;
  cwd: string;
  url: string;
  headers: Record<string, string>;
  enabled: boolean;
  callTimeoutMs: number;
  autoRestart: boolean;
  loadMode: McpLoadMode;
  toolAllowlist: string[];
  toolDenylist: string[];
}

export interface McpToolSummary {
  name: string;
  description: string;
}

export interface McpServerTestResult {
  ok: boolean;
  protocolVersion: string | null;
  serverName: string | null;
  serverVersion: string | null;
  tools: McpToolSummary[];
  error: string | null;
  elapsedMs: number;
}

export const DEFAULT_MCP_CALL_TIMEOUT_MS = 120_000;

export function emptyMcpServerConfig(): McpServerConfig {
  return {
    id: "",
    name: "",
    transport: "stdio",
    command: "",
    args: [],
    env: {},
    cwd: "",
    url: "",
    headers: {},
    enabled: true,
    callTimeoutMs: DEFAULT_MCP_CALL_TIMEOUT_MS,
    autoRestart: false,
    loadMode: "lazy",
    toolAllowlist: [],
    toolDenylist: [],
  };
}

/// Built-in templates for common servers. `noteKey` points at an i18n string
/// describing the software-side step (installing the Blender addon, enabling
/// the Figma MCP server, ...).
export interface McpPreset {
  id: string;
  label: string;
  noteKey: string;
  config: Partial<McpServerConfig>;
}

export const MCP_PRESETS: McpPreset[] = [
  {
    id: "blender",
    label: "Blender",
    noteKey: "settings.mcp.preset.blenderNote",
    config: {
      name: "Blender",
      transport: "stdio",
      command: "uvx",
      args: ["blender-mcp"],
      // Locus's managed-Python variables can leak into the child interpreter;
      // empty overrides isolate the server's own Python.
      env: { PYTHONHOME: "", PYTHONPATH: "" },
      // blender-mcp keeps a 180s socket timeout towards Blender; the outer
      // timeout must stay above it.
      callTimeoutMs: 240_000,
    },
  },
  {
    id: "figma-desktop",
    label: "Figma Desktop",
    noteKey: "settings.mcp.preset.figmaNote",
    config: {
      name: "Figma",
      transport: "http",
      url: "http://127.0.0.1:3845/mcp",
    },
  },
  {
    id: "python-uvx",
    label: "Python (uvx)",
    noteKey: "settings.mcp.preset.pythonNote",
    config: {
      transport: "stdio",
      command: "uvx",
      env: { PYTHONHOME: "", PYTHONPATH: "" },
    },
  },
  {
    id: "node-npx",
    label: "Node (npx)",
    noteKey: "settings.mcp.preset.nodeNote",
    config: {
      transport: "stdio",
      command: "npx",
      args: ["-y"],
    },
  },
];

export function mcpServersGet(): Promise<McpServerConfig[]> {
  return ipcInvoke<McpServerConfig[]>("mcp_servers_get");
}

export function mcpServersUpsert(server: McpServerConfig): Promise<McpServerConfig[]> {
  return ipcInvoke<McpServerConfig[]>("mcp_servers_upsert", { server });
}

export function mcpServersRemove(id: string): Promise<McpServerConfig[]> {
  return ipcInvoke<McpServerConfig[]>("mcp_servers_remove", { id });
}

export function mcpServerTest(server: McpServerConfig): Promise<McpServerTestResult> {
  return ipcInvoke<McpServerTestResult>("mcp_server_test", { server });
}

export interface McpImportCandidate {
  source: "claude_desktop" | "claude_code" | "cursor" | string;
  sourcePath: string;
  server: McpServerConfig;
  duplicateOf: string | null;
}

export function mcpImportScan(): Promise<McpImportCandidate[]> {
  return ipcInvoke<McpImportCandidate[]>("mcp_import_scan");
}

export function mcpImportApply(servers: McpServerConfig[]): Promise<McpServerConfig[]> {
  return ipcInvoke<McpServerConfig[]>("mcp_import_apply", { servers });
}

export function mcpServerWireTools(id: string): Promise<string[]> {
  return ipcInvoke<string[]>("mcp_server_wire_tools", { id });
}

export interface McpServerRuntimeStatus {
  id: string;
  name: string;
  enabled: boolean;
  connected: boolean;
  toolsCount: number;
  error: string | null;
}

export const MCP_STATUS_EVENT = "mcp-status";

export function mcpGetStatus(): Promise<McpServerRuntimeStatus[]> {
  return ipcInvoke<McpServerRuntimeStatus[]>("mcp_get_status");
}

export function mcpServerSetEnabled(
  id: string,
  enabled: boolean,
): Promise<McpServerConfig[]> {
  return ipcInvoke<McpServerConfig[]>("mcp_server_set_enabled", { id, enabled });
}

export function subscribeMcpStatus(
  handler: (status: McpServerRuntimeStatus[]) => void,
): Promise<RuntimeUnsubscribe> {
  return getLocusRuntime().subscribe<McpServerRuntimeStatus[]>(MCP_STATUS_EVENT, handler);
}
