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
  rules?: PluginComponentSummary[];
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
  ruleFiles?: Array<{
    fileName: string;
    content: string;
  }>;
  projectDependencies: PluginProjectDependency[];
  installAfterExport?: boolean;
  installScope?: PluginInstallScope | null;
  transferOwnership?: boolean;
}

export interface PluginExportResult {
  id: string;
  path: string;
  skillCount: number;
  viewCount: number;
  ruleCount: number;
  fileCount: number;
  byteSize: number;
  installedPlugin?: InstalledPluginSummary | null;
  transferredComponents?: Array<{
    kind: string;
    id: string;
    sourceRoot: string;
    pluginId: string;
  }>;
}

export interface PluginRegistryManifest {
  schemaVersion: number;
  registryVersion: number;
  bucketStrategy: string;
  bucketCount: number;
  entryBasePath: string;
  summaryBasePath: string;
  searchIndexPath: string;
  availableBuckets: string[];
  updatedAt: string;
}

export interface PluginRegistryManifestFetchResult {
  baseUrl: string;
  manifest: PluginRegistryManifest;
}

export type PluginRegistryCacheMode = "default" | "cachePreferred" | "networkPreferred";

export interface PluginRegistryCompatibility {
  minLocusVersion?: string | null;
  projectIndependent?: boolean | null;
}

export interface PluginRegistryIcon {
  type?: string | null;
  id?: string | null;
  url?: string | null;
}

export interface PluginRegistryStat {
  id?: string | null;
  label?: string | null;
  value?: string | number | null;
  icon?: PluginRegistryIcon | null;
}

export interface PluginRegistryDescriptionSource {
  type?: string | null;
  url?: string | null;
  repo?: string | null;
  branch?: string | null;
  path?: string | null;
}

export type PluginRegistryLocalizedText = Record<string, string>;
export type PluginRegistryLocalizedDescriptionSource = Record<string, PluginRegistryDescriptionSource>;

export interface PluginRegistrySummary {
  id: string;
  name: string;
  summary: string;
  summaryI18n?: PluginRegistryLocalizedText;
  author: string;
  tags: string[];
  latestVersion: string;
  updatedAt: string;
  icon?: PluginRegistryIcon | null;
  stats?: PluginRegistryStat[];
  compatibility?: PluginRegistryCompatibility;
}

export interface PluginRegistryShard {
  schemaVersion: number;
  bucket: string;
  plugins: PluginRegistrySummary[];
}

export interface PluginRegistrySearchIndex {
  schemaVersion: number;
  generatedAt: string;
  plugins: PluginRegistrySummary[];
}

export interface PluginRegistryDownload {
  url?: string;
  sha256?: string;
  sizeBytes?: number | null;
}

export interface PluginDownloadSource {
  type?: string | null;
  id?: string | null;
  input?: string | null;
  url?: string | null;
  repo?: string | null;
  ref?: string | null;
  branch?: string | null;
  tag?: string | null;
  commit?: string | null;
  asset?: string | null;
  assetPattern?: string | null;
  sha256?: string | null;
  sizeBytes?: number | null;
  version?: string | null;
}

export interface PluginRegistryEntry extends PluginRegistrySummary {
  schemaVersion: number;
  description: string;
  descriptionI18n?: PluginRegistryLocalizedText;
  descriptionSource?: PluginRegistryDescriptionSource | null;
  descriptionSourceI18n?: PluginRegistryLocalizedDescriptionSource;
  repo: string;
  license: string;
  download?: PluginRegistryDownload | null;
  downloadSource?: PluginDownloadSource | null;
}

export interface PluginRegistryDescriptionFetchResult {
  content: string;
  sourceUrl: string;
}

export interface PluginRegistryInstallRequest {
  id: string;
  latestVersion: string;
  download?: PluginRegistryDownload;
  downloadSource?: PluginDownloadSource;
}

export interface PluginGithubAuthStatus {
  authenticated: boolean;
  account: string;
}

export interface PluginGithubRepoStarStatus {
  repo: string;
  starred: boolean;
  stargazersCount?: number | null;
}

export interface PluginGithubOAuthStartResult {
  userCode: string;
  verificationUri: string;
  deviceCode: string;
  interval: number;
  expiresIn: number;
  message?: string | null;
  auth?: PluginGithubAuthStatus | null;
}

export interface PluginGithubOAuthPollResult {
  status: "pending" | "success" | "failed";
  message?: string | null;
  auth?: PluginGithubAuthStatus | null;
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

export function pluginRegistryFetchManifest(
  registryBaseUrl?: string | null,
  cacheMode?: PluginRegistryCacheMode | null,
): Promise<PluginRegistryManifestFetchResult> {
  return ipcInvoke<PluginRegistryManifestFetchResult>("plugin_registry_fetch_manifest", {
    registryBaseUrl: registryBaseUrl ?? null,
    cacheMode: cacheMode ?? null,
  });
}

export function pluginRegistryFetchShard(options: {
  registryBaseUrl?: string | null;
  summaryBasePath?: string | null;
  bucket: string;
  cacheMode?: PluginRegistryCacheMode | null;
}): Promise<PluginRegistryShard> {
  return ipcInvoke<PluginRegistryShard>("plugin_registry_fetch_shard", {
    registryBaseUrl: options.registryBaseUrl ?? null,
    summaryBasePath: options.summaryBasePath ?? null,
    bucket: options.bucket,
    cacheMode: options.cacheMode ?? null,
  });
}

export function pluginRegistryFetchSearchIndex(options: {
  registryBaseUrl?: string | null;
  searchIndexPath?: string | null;
  cacheMode?: PluginRegistryCacheMode | null;
}): Promise<PluginRegistrySearchIndex> {
  return ipcInvoke<PluginRegistrySearchIndex>("plugin_registry_fetch_search_index", {
    registryBaseUrl: options.registryBaseUrl ?? null,
    searchIndexPath: options.searchIndexPath ?? null,
    cacheMode: options.cacheMode ?? null,
  });
}

export function pluginRegistryFetchPlugin(options: {
  registryBaseUrl?: string | null;
  entryBasePath?: string | null;
  pluginId: string;
  cacheMode?: PluginRegistryCacheMode | null;
}): Promise<PluginRegistryEntry> {
  return ipcInvoke<PluginRegistryEntry>("plugin_registry_fetch_plugin", {
    registryBaseUrl: options.registryBaseUrl ?? null,
    entryBasePath: options.entryBasePath ?? null,
    pluginId: options.pluginId,
    cacheMode: options.cacheMode ?? null,
  });
}

export function pluginRegistryFetchDescription(options: {
  repo?: string | null;
  descriptionSource?: PluginRegistryDescriptionSource | null;
  cacheMode?: PluginRegistryCacheMode | null;
}): Promise<PluginRegistryDescriptionFetchResult> {
  return ipcInvoke<PluginRegistryDescriptionFetchResult>("plugin_registry_fetch_description", {
    repo: options.repo ?? null,
    descriptionSource: options.descriptionSource ?? null,
    cacheMode: options.cacheMode ?? null,
  });
}

export function pluginInstallFromRegistry(
  request: PluginRegistryInstallRequest,
  scope: PluginInstallScope,
): Promise<InstalledPluginSummary> {
  return ipcInvoke<InstalledPluginSummary>("plugin_install_from_registry", {
    request,
    scope,
  });
}

export function pluginInstallFromSource(
  source: PluginDownloadSource,
  scope: PluginInstallScope,
): Promise<InstalledPluginSummary> {
  return ipcInvoke<InstalledPluginSummary>("plugin_install_from_source", {
    source,
    scope,
  });
}

export function pluginGithubAuthStatus(): Promise<PluginGithubAuthStatus> {
  return ipcInvoke<PluginGithubAuthStatus>("plugin_github_auth_status");
}

export function pluginGithubRepoStarStatus(repo: string): Promise<PluginGithubRepoStarStatus> {
  return ipcInvoke<PluginGithubRepoStarStatus>("plugin_github_repo_star_status", {
    repo,
  });
}

export function pluginGithubRepoSetStarred(
  repo: string,
  starred: boolean,
): Promise<PluginGithubRepoStarStatus> {
  return ipcInvoke<PluginGithubRepoStarStatus>("plugin_github_repo_set_starred", {
    repo,
    starred,
  });
}

export function pluginGithubAuthSaveToken(token: string): Promise<PluginGithubAuthStatus> {
  return ipcInvoke<PluginGithubAuthStatus>("plugin_github_auth_save_token", {
    token,
  });
}

export function pluginGithubOAuthStart(): Promise<PluginGithubOAuthStartResult> {
  return ipcInvoke<PluginGithubOAuthStartResult>("plugin_github_oauth_start");
}

export function pluginGithubOAuthPoll(deviceCode: string): Promise<PluginGithubOAuthPollResult> {
  return ipcInvoke<PluginGithubOAuthPollResult>("plugin_github_oauth_poll", {
    deviceCode,
  });
}

export function pluginGithubAuthLogout(): Promise<PluginGithubAuthStatus> {
  return ipcInvoke<PluginGithubAuthStatus>("plugin_github_auth_logout");
}
