import { ipcInvoke } from "./ipc";

export type PluginInstallScope = "app" | "project";

export interface PluginComponentSummary {
  id?: string | null;
  path: string;
  root: string;
}

export interface PluginProjectDependency {
  kind: string;
  name: string;
  version?: string | null;
  notes?: string | null;
}

export interface PluginCompatibilitySummary {
  projectIndependent?: boolean | null;
}

export interface PluginDependencySummary {
  project?: PluginProjectDependency[];
}

export interface InstalledPluginSummary {
  id: string;
  name: string;
  version: string;
  scope: PluginInstallScope;
  root: string;
  compatibility?: PluginCompatibilitySummary;
  dependencies?: PluginDependencySummary;
  agents: PluginComponentSummary[];
  skills: PluginComponentSummary[];
  views: PluginComponentSummary[];
}

export interface PluginExportRequest {
  id: string;
  name: string;
  version: string;
  filePath: string;
  skillPackageIds: string[];
  viewIds: string[];
  projectDependencies: PluginProjectDependency[];
}

export interface PluginExportResult {
  id: string;
  path: string;
  skillCount: number;
  viewCount: number;
  fileCount: number;
  byteSize: number;
}

export function pluginListInstalled(): Promise<InstalledPluginSummary[]> {
  return ipcInvoke<InstalledPluginSummary[]>("plugin_list_installed");
}

export function pluginInstallFromPath(
  sourcePath: string,
  scope: PluginInstallScope,
): Promise<InstalledPluginSummary> {
  return ipcInvoke<InstalledPluginSummary>("plugin_install_from_path", {
    sourcePath,
    scope,
  });
}

export function pluginUninstall(
  pluginId: string,
  scope: PluginInstallScope,
): Promise<string> {
  return ipcInvoke<string>("plugin_uninstall", {
    pluginId,
    scope,
  });
}

export function pluginExport(request: PluginExportRequest): Promise<PluginExportResult> {
  return ipcInvoke<PluginExportResult>("plugin_export", {
    request,
  });
}
